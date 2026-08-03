//! Rename selected library entries to their DAT-matched filenames.
//!
//! The frontend hands over the selected entries with whatever match data it
//! already had cached; everything else — cue/bin set classification, catalog
//! lookups, disc-set verification, extension sniffing, and the renames
//! themselves — happens here, on the worker thread. All execution goes
//! through the shared transactional engine in `retro_junk_lib::rename`, so
//! companion media files and gamelist.xml entries move inside each game's
//! transaction.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use retro_junk_dat::{DatFile, DatGame, DatIndex, DatRom, FileHashes};
use retro_junk_io::ProgressUnit;
use retro_junk_lib::disc_set::DiscSetOutcome;
use retro_junk_lib::rename::{DiscMatchData, ExecutionContext, execute_single_rename};
use retro_junk_lib::{AnalysisContext, Platform, RomIdentification};

use super::OpCtx;

/// The outcome for one selected entry, keyed by its durable library identity.
pub struct RenameResult {
    pub entry_id: retro_junk_db::LibraryEntryId,
    pub entry_name: String,
    pub outcome: RenameOutcome,
}

pub enum RenameOutcome {
    Renamed {
        source: PathBuf,
        target: PathBuf,
    },
    AlreadyCorrect,
    NoMatch {
        reason: String,
    },
    Error {
        message: String,
    },
    M3uRenamed {
        /// Folder the set lived in before the rename, so its library row's
        /// identity can follow it to the new `set:` key.
        source_folder: PathBuf,
        target_folder: PathBuf,
        discs_renamed: usize,
        playlist_written: bool,
        folder_renamed: bool,
        errors: Vec<String>,
    },
}

/// One selected entry, carried across with the match data the frontend
/// already had cached. Nothing here requires I/O to build.
pub struct RenameEntry {
    pub entry_id: retro_junk_db::LibraryEntryId,
    pub entry_name: String,
    pub kind: RenameEntryKind,
}

pub enum RenameEntryKind {
    SingleFile {
        path: PathBuf,
        /// Cached (DAT game name, DAT rom name) when the frontend already
        /// resolved a same-region match; `None` falls back to catalog lookup
        /// by hash/serial.
        cached_names: Option<(String, String)>,
        /// Already-computed file hashes, for catalog hash lookup.
        hashes: Option<FileHashes>,
        /// Already-computed identification, for catalog serial lookup.
        identification: Option<RomIdentification>,
    },
    MultiDisc {
        /// All disc files in this multi-disc set (from the playlist).
        files: Vec<PathBuf>,
        /// Already-resolved non-cue disc data — `target_filename` holds the
        /// raw DAT `rom_name`, corrected here before execution.
        resolved_discs: Vec<DiscMatchData>,
        /// Pre-resolved game name (skips `derive_base_game_name`).
        /// Empty = derive from per-disc DAT names.
        game_name_override: String,
    },
}

/// Everything a rename run needs beyond the entries themselves.
pub struct RenameRequest {
    pub platform: Platform,
    pub entries: Vec<RenameEntry>,
    /// Candidate media directory for this console; companion assets in it
    /// move inside each rename transaction. Used only when it is a directory.
    pub media_dir: Option<PathBuf>,
    /// Candidate gamelist.xml path; rewritten inside each rename
    /// transaction. Used only when it is a file.
    pub gamelist_path: Option<PathBuf>,
}

/// A single-file rename job. Target is resolved at execution time.
struct RenameJob {
    entry_id: retro_junk_db::LibraryEntryId,
    entry_name: String,
    source: PathBuf,
    /// Disc-level DAT game name; empty when the match did not carry one. A
    /// whole-disc container is named from this, never from a member track.
    dat_game_name: String,
    /// Raw DAT rom name (e.g., "Game (USA).iso") — extension corrected at rename time.
    dat_rom_name: String,
}

/// A cue/bin disc set job: planned and verified against the catalog.
struct CueSetJob {
    entry_id: retro_junk_db::LibraryEntryId,
    entry_name: String,
    cue: PathBuf,
}

/// An M3U rename job that needs resolution of its disc files.
struct M3uJob {
    entry_id: retro_junk_db::LibraryEntryId,
    entry_name: String,
    files: Vec<PathBuf>,
    resolved_discs: Vec<DiscMatchData>,
    /// Non-cue file paths that still need hash-based resolution.
    unresolved_files: Vec<PathBuf>,
    /// Cue files inside the folder — each becomes a verified disc set.
    cue_files: Vec<PathBuf>,
    game_name_override: String,
}

/// Rename the requested entries to their DAT-matched filenames.
///
/// First classifies each entry — cue sets rename as a unit and cover their
/// track files; loose files resolve through cached names or the catalog —
/// then verifies disc sets, corrects extensions by sniffing the actual
/// format, and executes each rename as one transaction. Every requested
/// entry gets a reported outcome.
pub fn rename_entries(
    context: &AnalysisContext,
    db_path: Option<&Path>,
    request: &RenameRequest,
    ctx: &OpCtx,
) -> Vec<RenameResult> {
    let conn = db_path.and_then(|p| retro_junk_db::open_database(p).ok());
    let platform = request.platform;

    let mut jobs: Vec<RenameJob> = Vec::new();
    let mut cue_jobs: Vec<CueSetJob> = Vec::new();
    let mut m3u_jobs: Vec<M3uJob> = Vec::new();
    let mut results: Vec<RenameResult> = Vec::new();

    // First pass: which selected entries are cue files (their sets rename
    // as a unit, covering their track files).
    let selected_cues: std::collections::HashSet<&Path> = request
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            RenameEntryKind::SingleFile { path, .. } if is_cue(path) => Some(path.as_path()),
            _ => None,
        })
        .collect();

    // Lazy per-directory map of track file → owning cue, so a track that
    // belongs to a disc set is never renamed on its own (which would break
    // the cue's FILE references).
    let mut dir_membership: HashMap<PathBuf, HashMap<PathBuf, PathBuf>> = HashMap::new();

    for entry in &request.entries {
        let entry_id = entry.entry_id;
        let entry_name = entry.entry_name.clone();

        match &entry.kind {
            RenameEntryKind::SingleFile {
                path,
                cached_names,
                hashes,
                identification,
            } => {
                if is_cue(path) {
                    // Cue/bin sets are planned (and verified) against the
                    // catalog projection below.
                    cue_jobs.push(CueSetJob {
                        entry_id,
                        entry_name,
                        cue: path.clone(),
                    });
                    continue;
                }

                // Track files owned by a sibling cue only ever move with
                // their set.
                if let Some(dir) = path.parent() {
                    let membership = dir_membership
                        .entry(dir.to_path_buf())
                        .or_insert_with(|| cue_membership_for_dir(dir));
                    if let Some(cue) = membership.get(path) {
                        if !selected_cues.contains(cue.as_path()) {
                            let cue_name = cue.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                            results.push(RenameResult {
                                entry_id,
                                entry_name,
                                outcome: RenameOutcome::NoMatch {
                                    reason: format!(
                                        "Part of disc set '{cue_name}' — select the cue to rename the whole set"
                                    ),
                                },
                            });
                        }
                        // When the cue is selected, its set covers this file.
                        continue;
                    }
                }

                // Determine target names from the cached match or catalog lookup.
                let target_names = get_target_names(
                    conn.as_ref(),
                    context,
                    platform,
                    cached_names.as_ref(),
                    hashes.as_ref(),
                    identification.as_ref(),
                );

                match target_names {
                    Some((dat_game_name, dat_rom_name)) => {
                        jobs.push(RenameJob {
                            entry_id,
                            entry_name,
                            source: path.clone(),
                            dat_game_name,
                            dat_rom_name,
                        });
                    }
                    None => {
                        results.push(RenameResult {
                            entry_id,
                            entry_name: entry_name.clone(),
                            outcome: RenameOutcome::NoMatch {
                                reason: format!("No DAT match for '{entry_name}'"),
                            },
                        });
                    }
                }
            }
            RenameEntryKind::MultiDisc {
                files,
                resolved_discs,
                game_name_override,
            } => {
                // Partition: cues become disc sets; non-cue files use cached
                // matches or hash resolution.
                let mut disc_resolved: HashMap<&Path, &DiscMatchData> = resolved_discs
                    .iter()
                    .map(|d| (d.file_path.as_path(), d))
                    .collect();

                let mut cue_files = Vec::new();
                let mut resolved = Vec::new();
                let mut unresolved = Vec::new();
                for f in files {
                    if is_cue(f) {
                        cue_files.push(f.clone());
                    } else if let Some(disc_data) = disc_resolved.remove(f.as_path()) {
                        resolved.push(disc_data.clone());
                    } else {
                        unresolved.push(f.clone());
                    }
                }

                if cue_files.is_empty() && resolved.is_empty() && unresolved.is_empty() {
                    results.push(RenameResult {
                        entry_id,
                        entry_name: entry_name.clone(),
                        outcome: RenameOutcome::NoMatch {
                            reason: format!("No DAT match for '{entry_name}'"),
                        },
                    });
                    continue;
                }

                m3u_jobs.push(M3uJob {
                    entry_id,
                    entry_name,
                    files: files.clone(),
                    resolved_discs: resolved,
                    unresolved_files: unresolved,
                    cue_files,
                    game_name_override: game_name_override.clone(),
                });
            }
        }
    }

    // Companion locations: only used when they actually exist on disk.
    let media_dir = request.media_dir.clone().filter(|d| d.is_dir());
    let gamelist_path = request.gamelist_path.clone().filter(|p| p.is_file());

    let gamelist_rewriter = |stem_map: &HashMap<String, String>| -> Vec<(PathBuf, String)> {
        gamelist_path
            .as_ref()
            .and_then(|p| retro_junk_frontend::esde::plan_gamelist_rewrite(p, stem_map))
            .into_iter()
            .collect()
    };
    let exec = ExecutionContext {
        media_dir,
        gamelist_rewriter: Some(&gamelist_rewriter),
    };

    let total_work = (jobs.len() + cue_jobs.len() + m3u_jobs.len()) as u64;
    let mut file_num: u64 = 0;
    let send_progress = |file_num: &mut u64| {
        (ctx.progress)("Renaming", ProgressUnit::Items, *file_num, total_work);
        *file_num += 1;
    };

    // Step 1: Execute single-file renames (one transaction each)
    for job in &jobs {
        if ctx.cancelled() {
            break;
        }
        send_progress(&mut file_num);

        // Detect actual format extension from file content
        let detected_ext = sniff_detected_extension(&job.source, context, platform);
        let target_name = retro_junk_lib::rename::target_filename_for_rename(
            &job.dat_game_name,
            &job.dat_rom_name,
            &job.source,
            &detected_ext,
        );
        let target = job
            .source
            .parent()
            .unwrap_or(&job.source)
            .join(&target_name);

        if job.source == target {
            results.push(RenameResult {
                entry_id: job.entry_id,
                entry_name: job.entry_name.clone(),
                outcome: RenameOutcome::AlreadyCorrect,
            });
            continue;
        }

        match execute_single_rename(&job.source, &target, &exec) {
            Ok(single) => {
                for warning in &single.warnings {
                    log::warn!("{}: {}", job.entry_name, warning);
                }
                results.push(RenameResult {
                    entry_id: job.entry_id,
                    entry_name: job.entry_name.clone(),
                    outcome: RenameOutcome::Renamed {
                        source: job.source.clone(),
                        target,
                    },
                });
            }
            Err(e) => {
                results.push(RenameResult {
                    entry_id: job.entry_id,
                    entry_name: job.entry_name.clone(),
                    outcome: RenameOutcome::Error {
                        message: format!("Failed to rename '{}': {}", job.source.display(), e),
                    },
                });
            }
        }
    }

    // Step 2: Plan and execute cue/bin disc sets (one transaction each)
    for job in &cue_jobs {
        if ctx.cancelled() {
            break;
        }
        send_progress(&mut file_num);

        let outcome = plan_cue_set(&job.cue, conn.as_ref(), context, platform);
        results.push(RenameResult {
            entry_id: job.entry_id,
            entry_name: job.entry_name.clone(),
            outcome: execute_cue_set_outcome(outcome, &job.cue, &exec),
        });
    }

    // Step 3: Resolve and execute multi-disc renames
    for m3u_job in &m3u_jobs {
        if ctx.cancelled() {
            break;
        }
        send_progress(&mut file_num);

        // Fix up target_filename extensions for resolved non-cue discs
        let mut all_discs: Vec<DiscMatchData> = m3u_job
            .resolved_discs
            .iter()
            .map(|d| {
                let detected_ext = sniff_detected_extension(&d.file_path, context, platform);
                DiscMatchData {
                    file_path: d.file_path.clone(),
                    game_name: d.game_name.clone(),
                    target_filename: retro_junk_lib::rename::target_filename_for_rename(
                        &d.game_name,
                        &d.target_filename,
                        &d.file_path,
                        &detected_ext,
                    ),
                }
            })
            .collect();

        // Resolve unresolved non-cue disc files via hashing
        for file_path in &m3u_job.unresolved_files {
            log::warn!(
                "Could not resolve uncached disc file: {}",
                file_path.display()
            );
        }

        // Plan a verified disc set for each cue in the folder
        let mut disc_sets = Vec::new();
        let mut set_errors = Vec::new();
        for cue in &m3u_job.cue_files {
            match plan_cue_set(cue, conn.as_ref(), context, platform) {
                Some(DiscSetOutcome::Planned(plan)) => {
                    all_discs.push(DiscMatchData {
                        file_path: cue.clone(),
                        game_name: plan.game_name.clone(),
                        target_filename: plan.cue_target_filename.clone(),
                    });
                    disc_sets.push(plan);
                }
                Some(DiscSetOutcome::AlreadyCorrect { game_name, .. }) => {
                    if let Some(name) = cue.file_name().and_then(|n| n.to_str()) {
                        all_discs.push(DiscMatchData {
                            file_path: cue.clone(),
                            game_name,
                            target_filename: name.to_string(),
                        });
                    }
                }
                Some(other) => {
                    set_errors.push(describe_set_failure(cue, &other));
                }
                None => {
                    set_errors.push(format!(
                        "{}: no DAT index or analyzer available",
                        cue.display()
                    ));
                }
            }
        }

        if all_discs.is_empty() && disc_sets.is_empty() {
            results.push(RenameResult {
                entry_id: m3u_job.entry_id,
                entry_name: m3u_job.entry_name.clone(),
                outcome: RenameOutcome::NoMatch {
                    reason: if set_errors.is_empty() {
                        "Could not resolve any disc files".to_string()
                    } else {
                        set_errors.join("; ")
                    },
                },
            });
            continue;
        }

        let source_folder = match m3u_job.files[0].parent() {
            Some(p) => p.to_path_buf(),
            None => continue,
        };

        let lib_job = retro_junk_lib::rename::M3uRenameJob {
            source_folder: source_folder.clone(),
            discs: all_discs.clone(),
            disc_sets,
            game_name_override: m3u_job.game_name_override.clone(),
        };
        let mut m3u_result = retro_junk_lib::rename::execute_m3u_rename(&lib_job, &exec);
        m3u_result.errors.extend(set_errors);

        let any_work = m3u_result.discs_renamed > 0
            || m3u_result.playlist_written
            || m3u_result.playlist_renamed
            || m3u_result.folder_renamed
            || m3u_result.cue_files_updated > 0
            || m3u_result.m3u_references_updated > 0;

        if any_work {
            results.push(RenameResult {
                entry_id: m3u_job.entry_id,
                entry_name: m3u_job.entry_name.clone(),
                outcome: RenameOutcome::M3uRenamed {
                    source_folder: m3u_job
                        .files
                        .first()
                        .and_then(|disc| disc.parent())
                        .map_or_else(
                            || m3u_result.final_folder.clone(),
                            std::path::Path::to_path_buf,
                        ),
                    target_folder: m3u_result.final_folder,
                    discs_renamed: m3u_result.discs_renamed,
                    playlist_written: m3u_result.playlist_written,
                    folder_renamed: m3u_result.folder_renamed,
                    errors: m3u_result.errors,
                },
            });
        } else if m3u_result.errors.is_empty() {
            results.push(RenameResult {
                entry_id: m3u_job.entry_id,
                entry_name: m3u_job.entry_name.clone(),
                outcome: RenameOutcome::AlreadyCorrect,
            });
        } else {
            results.push(RenameResult {
                entry_id: m3u_job.entry_id,
                entry_name: m3u_job.entry_name.clone(),
                outcome: RenameOutcome::Error {
                    message: m3u_result.errors.join("; "),
                },
            });
        }
    }

    results
}

/// Map every existing track file referenced by a `.cue` in `dir` to its cue.
fn cue_membership_for_dir(dir: &Path) -> HashMap<PathBuf, PathBuf> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && is_cue(&path)
            && let Ok(set) = retro_junk_lib::disc_set::expand_disc_set(&path)
        {
            for track in set.tracks {
                map.insert(track, path.clone());
            }
        }
    }
    map
}

/// Returns true if the path has a .cue extension.
fn is_cue(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cue"))
}

/// Build a small projection containing only catalog media implicated by this
/// cue's track hashes, then reuse the verified set planner.
fn plan_cue_set(
    cue: &Path,
    conn: Option<&retro_junk_db::Connection>,
    context: &AnalysisContext,
    platform: Platform,
) -> Option<DiscSetOutcome> {
    let registered = context.get_by_platform(platform)?;
    let conn = conn?;
    let set = retro_junk_lib::disc_set::expand_disc_set(cue).ok()?;
    let mut media_ids = std::collections::BTreeSet::new();
    for track in &set.tracks {
        let mut file = std::fs::File::open(track).ok()?;
        let hashes = retro_junk_lib::hasher::compute_crc32_sha1(
            &mut file,
            registered.analyzer.as_ref(),
            Some(track),
        )
        .ok()?;
        for id in retro_junk_db::match_media_ids_by_track_hash(
            conn,
            registered.analyzer.short_name(),
            hashes.data_size,
            &hashes.crc32,
            hashes.sha1.as_deref(),
        )
        .ok()?
        {
            media_ids.insert(id);
        }
    }
    let mut games = Vec::new();
    for media_id in media_ids {
        let media = retro_junk_db::get_media_by_id(conn, &media_id).ok()??;
        let release = retro_junk_db::get_release_by_id(conn, &media.release_id).ok()??;
        let tracks = retro_junk_db::find_media_tracks(conn, &media_id).ok()?;
        games.push(DatGame {
            name: media.dat_name,
            region: Some(release.region),
            serial: (!media.media_serial.is_empty()).then_some(media.media_serial.clone()),
            version: (!media.revision.is_empty()).then_some(media.revision),
            category: None,
            roms: tracks
                .into_iter()
                .map(|track| DatRom {
                    name: track.track_name,
                    size: u64::try_from(track.file_size).unwrap_or(0),
                    crc: track.crc32,
                    sha1: (!track.sha1.is_empty()).then_some(track.sha1),
                    md5: (!track.md5.is_empty()).then_some(track.md5),
                    serial: None,
                })
                .collect(),
        });
    }
    let index = DatIndex::from_dat(DatFile {
        name: String::new(),
        description: "SQLite catalog cue projection".to_string(),
        version: String::new(),
        games,
    });
    Some(retro_junk_lib::disc_set::plan_disc_set(
        cue,
        registered.analyzer.as_ref(),
        &index,
        &|_, _, _| {},
    ))
}

/// Execute a planned cue-set outcome and convert it to a `RenameOutcome`.
fn execute_cue_set_outcome(
    outcome: Option<DiscSetOutcome>,
    cue: &Path,
    exec: &ExecutionContext<'_>,
) -> RenameOutcome {
    match outcome {
        Some(DiscSetOutcome::Planned(plan)) => {
            let target = plan.cue_target();
            let result = retro_junk_lib::rename::execute_disc_set(&plan, exec);
            if result.errors.is_empty() {
                RenameOutcome::Renamed {
                    source: cue.to_path_buf(),
                    target,
                }
            } else {
                RenameOutcome::Error {
                    message: result.errors.join("; "),
                }
            }
        }
        Some(DiscSetOutcome::AlreadyCorrect { .. }) => RenameOutcome::AlreadyCorrect,
        Some(other) => RenameOutcome::NoMatch {
            reason: describe_set_failure(cue, &other),
        },
        None => RenameOutcome::NoMatch {
            reason: "No DAT index or analyzer available".to_string(),
        },
    }
}

/// Human-readable description of a non-renameable disc-set outcome.
fn describe_set_failure(cue: &Path, outcome: &DiscSetOutcome) -> String {
    let cue_name = cue.file_name().and_then(|n| n.to_str()).unwrap_or("?");
    match outcome {
        DiscSetOutcome::Broken { missing } => {
            format!(
                "{cue_name}: cue references missing files: {}",
                missing.join(", ")
            )
        }
        DiscSetOutcome::NotVerified { game_name, issues } => format!(
            "{cue_name}: identified as \"{}\" but not verified: {}",
            if game_name.is_empty() {
                "unknown"
            } else {
                game_name
            },
            issues.join("; "),
        ),
        DiscSetOutcome::Unmatched { issues } => {
            format!("{cue_name}: no DAT match ({})", issues.join("; "))
        }
        DiscSetOutcome::Planned(_) | DiscSetOutcome::AlreadyCorrect { .. } => String::new(),
    }
}

/// Try to determine the target (game name, ROM filename) for an entry.
///
/// Priority:
/// 1. Cached names the frontend already resolved
/// 2. Hash lookup against the `SQLite` catalog
/// 3. Serial lookup against the `SQLite` catalog
fn get_target_names(
    conn: Option<&retro_junk_db::Connection>,
    context: &AnalysisContext,
    platform: Platform,
    cached_names: Option<&(String, String)>,
    hashes: Option<&FileHashes>,
    identification: Option<&RomIdentification>,
) -> Option<(String, String)> {
    // 1. Use the cached names when the frontend already resolved a match.
    if let Some((game_name, rom_name)) = cached_names {
        return Some((game_name.clone(), rom_name.clone()));
    }

    let conn = conn?;
    let platform_id = platform.short_name();
    let mut candidates = Vec::new();
    if let Some(hashes) = hashes
        && let Ok(matches) = retro_junk_db::match_media_by_hash(
            conn,
            platform_id,
            hashes.data_size,
            (!hashes.crc32.is_empty()).then_some(hashes.crc32.as_str()),
            hashes.sha1.as_deref(),
        )
    {
        candidates.extend(matches);
    }
    if let Some(id) = identification
        && let Some(registered) = context.get_by_platform(platform)
        && let Some(lookup_serial) =
            retro_junk_lib::catalog_match::catalog_serial_key(registered.analyzer.as_ref(), id)
        && let Ok(matches) = retro_junk_db::match_media_by_serial(conn, platform_id, &lookup_serial)
    {
        for candidate in matches {
            if !candidates
                .iter()
                .any(|existing| existing.media.id == candidate.media.id)
            {
                candidates.push(candidate);
            }
        }
    }
    if let retro_junk_lib::catalog_match::CatalogMatchResolution::Match { candidate, .. } =
        retro_junk_lib::catalog_match::resolve_catalog_match(&candidates, identification, hashes)
    {
        return Some((
            candidate.media.dat_name.clone(),
            candidate.media.rom_name.clone(),
        ));
    }

    None
}

/// Quick-analyze a file to detect its format extension.
///
/// Opens the file, runs a quick analysis, and returns the detected extension
/// (e.g., "rvz", "iso", "chd"). Used to determine the correct file extension
/// regardless of what the file is currently named.
fn sniff_detected_extension(
    file_path: &Path,
    context: &AnalysisContext,
    platform: Platform,
) -> String {
    let Some(registered) = context.get_by_platform(platform) else {
        return String::new();
    };
    let Ok(mut file) = std::fs::File::open(file_path) else {
        return String::new();
    };
    let opts = retro_junk_lib::AnalysisOptions::new()
        .quick(true)
        .file_path(file_path);
    let Ok(info) = registered.analyzer.analyze(&mut file, &opts) else {
        return String::new();
    };
    info.extra
        .get("detected_extension")
        .cloned()
        .unwrap_or_default()
}

/// Carry each renamed entry's stored identity across to its new path.
///
/// Library entries are keyed by path, so a rename plus a rescan looks exactly
/// like "one file vanished, another appeared" — and the new row starts with no
/// digests, no DAT match, and no identification. A rename cannot change
/// content, so the identity follows the file instead of being re-derived.
///
/// Best-effort: a row that cannot be re-keyed (its destination already exists,
/// or the path is outside the console) simply gets re-read by the rescan, which
/// is the old behaviour.
pub fn carry_identity_across_renames(
    conn: &retro_junk_db::Connection,
    console_id: retro_junk_db::LibraryConsoleId,
    console_folder: &Path,
    results: &[RenameResult],
) {
    let key = |path: &Path, directory: bool| {
        let relative = path.strip_prefix(console_folder).ok()?;
        let key = if directory {
            retro_junk_db::set_source_key(relative)
        } else {
            retro_junk_db::file_source_key(relative)
        };
        key.ok().map(|value| value.as_str().to_owned())
    };
    for result in results {
        let (from, to, directory) = match &result.outcome {
            RenameOutcome::Renamed { source, target } => (source, target, false),
            RenameOutcome::M3uRenamed {
                source_folder,
                target_folder,
                ..
            } => (source_folder, target_folder, true),
            _ => continue,
        };
        let (Some(old_key), Some(new_key)) = (key(from, directory), key(to, directory)) else {
            continue;
        };
        match retro_junk_db::rekey_library_entry(conn, console_id, &old_key, &new_key) {
            Ok(true) => log::debug!("Carried library identity {old_key} -> {new_key}"),
            Ok(false) => {}
            Err(error) => log::warn!("Could not carry library identity for {old_key}: {error}"),
        }
    }
}
