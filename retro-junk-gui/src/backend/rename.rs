use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use retro_junk_dat::DatIndex;
use retro_junk_lib::context::AnalysisContext;
use retro_junk_lib::disc_set::DiscSetOutcome;
use retro_junk_lib::rename::{DiscMatchData, ExecutionContext, execute_single_rename};
use retro_junk_lib::scanner::GameEntry;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{self, AppMessage, OperationKind, ProgressDisplay, RenameOutcome, RenameResult};

/// A single-file rename job. Target is resolved on the background thread.
struct RenameJob {
    entry_name: String,
    source: PathBuf,
    /// Raw DAT rom name (e.g., "Game (USA).iso") — extension corrected at rename time.
    dat_rom_name: String,
}

/// A cue/bin disc set job: planned and verified on the background thread.
struct CueSetJob {
    entry_name: String,
    cue: PathBuf,
}

/// An M3U rename job that needs background resolution of disc files.
struct M3uJob {
    entry_name: String,
    /// All disc files in this multi-disc set (from the playlist)
    files: Vec<PathBuf>,
    /// Already-resolved non-cue disc data — `target_filename` holds raw DAT
    /// `rom_name`, corrected on the background thread before execution.
    resolved_discs: Vec<DiscMatchData>,
    /// Non-cue file paths that still need hash-based resolution
    unresolved_files: Vec<PathBuf>,
    /// Cue files inside the folder — each becomes a verified disc set
    cue_files: Vec<PathBuf>,
    /// Pre-resolved game name from catalog DB (skips `derive_base_game_name`).
    /// Empty = derive from per-disc DAT names.
    game_name_override: String,
}

/// Rename selected entries to their DAT-matched filenames.
///
/// Collects DAT match info on the UI thread, then spawns a background thread
/// to verify disc sets, compute correct extensions, and execute renames.
/// All execution goes through the shared transactional engine in
/// `retro_junk_lib::rename` — companion media files and gamelist.xml entries
/// move inside each game's transaction.
pub fn rename_selected_entries(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.browser.consoles[console_idx];
    let folder_name = console.folder_name.clone();

    let mut jobs: Vec<RenameJob> = Vec::new();
    let mut cue_jobs: Vec<CueSetJob> = Vec::new();
    let mut m3u_jobs: Vec<M3uJob> = Vec::new();
    let mut results: Vec<RenameResult> = Vec::new();

    let dat_index = app.dat_indices.get(&folder_name);

    // First pass: which selected entries are cue files (their sets rename
    // as a unit, covering their track files).
    let selected_cues: std::collections::HashSet<PathBuf> = app
        .ui_state
        .selected_entries
        .iter()
        .filter_map(|&id| console.entry_by_id(id))
        .filter_map(|e| match &e.game_entry {
            GameEntry::SingleFile(p) if is_cue(p) => Some(p.clone()),
            _ => None,
        })
        .collect();

    // Lazy per-directory map of track file → owning cue, so a track that
    // belongs to a disc set is never renamed on its own (which would break
    // the cue's FILE references).
    let mut dir_membership: HashMap<PathBuf, HashMap<PathBuf, PathBuf>> = HashMap::new();

    for &i in &app.ui_state.selected_entries {
        let Some(entry) = console.entry_by_id(i) else {
            continue;
        };

        let entry_name = entry.game_entry.display_name().to_string();

        match &entry.game_entry {
            GameEntry::SingleFile(path) => {
                if is_cue(path) {
                    // Cue/bin sets are planned (and verified) on the
                    // background thread against the DAT index.
                    if dat_index.is_some() {
                        cue_jobs.push(CueSetJob {
                            entry_name,
                            cue: path.clone(),
                        });
                    } else {
                        results.push(RenameResult {
                            entry_name,
                            outcome: RenameOutcome::NoMatch {
                                reason: "No DAT loaded for disc-set rename".to_string(),
                            },
                        });
                    }
                    continue;
                }

                // Track files owned by a sibling cue only ever move with
                // their set.
                if let Some(dir) = path.parent() {
                    let membership = dir_membership
                        .entry(dir.to_path_buf())
                        .or_insert_with(|| cue_membership_for_dir(dir));
                    if let Some(cue) = membership.get(path) {
                        if !selected_cues.contains(cue) {
                            let cue_name = cue.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                            results.push(RenameResult {
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

                // Determine target ROM name from existing dat_match or try DAT lookup
                let rom_name = get_target_rom_name(app, &folder_name, entry);

                match rom_name {
                    Some(dat_rom_name) => {
                        let source = entry.game_entry.analysis_path().to_path_buf();
                        jobs.push(RenameJob {
                            entry_name,
                            source,
                            dat_rom_name,
                        });
                    }
                    None => {
                        results.push(RenameResult {
                            entry_name,
                            outcome: RenameOutcome::NoMatch {
                                reason: format!(
                                    "No DAT match for '{}'",
                                    entry.game_entry.display_name()
                                ),
                            },
                        });
                    }
                }
            }
            GameEntry::MultiDisc { files, .. } => {
                if dat_index.is_none() {
                    results.push(RenameResult {
                        entry_name,
                        outcome: RenameOutcome::NoMatch {
                            reason: "No DAT loaded for multi-disc rename".to_string(),
                        },
                    });
                    continue;
                }

                // Partition: cues become disc sets (planned on the background
                // thread); non-cue files use cached matches or hash resolution.
                let mut disc_resolved: HashMap<PathBuf, DiscMatchData> = HashMap::new();
                if let Some(ref discs) = entry.disc_identifications {
                    for disc in discs {
                        if let Some(ref dm) = disc.dat_match {
                            disc_resolved.insert(
                                disc.path.clone(),
                                DiscMatchData {
                                    file_path: disc.path.clone(),
                                    game_name: dm.game_name.clone(),
                                    target_filename: dm.rom_name.clone(),
                                },
                            );
                        }
                    }
                }

                let mut cue_files = Vec::new();
                let mut resolved = Vec::new();
                let mut unresolved = Vec::new();
                for f in files {
                    if is_cue(f) {
                        cue_files.push(f.clone());
                    } else if let Some(disc_data) = disc_resolved.remove(f) {
                        resolved.push(disc_data);
                    } else {
                        unresolved.push(f.clone());
                    }
                }

                if cue_files.is_empty() && resolved.is_empty() && unresolved.is_empty() {
                    results.push(RenameResult {
                        entry_name,
                        outcome: RenameOutcome::NoMatch {
                            reason: format!(
                                "No DAT match for '{}'",
                                entry.game_entry.display_name()
                            ),
                        },
                    });
                    continue;
                }

                m3u_jobs.push(M3uJob {
                    entry_name,
                    files: files.clone(),
                    resolved_discs: resolved,
                    unresolved_files: unresolved,
                    cue_files,
                    game_name_override: entry
                        .dat_match
                        .as_ref()
                        .map(|dm| dm.game_name.clone())
                        .unwrap_or_default(),
                });
            }
        }
    }

    let has_work = !jobs.is_empty() || !cue_jobs.is_empty() || !m3u_jobs.is_empty();
    if !has_work {
        // Nothing to rename on disk — still show results if there are any
        if !results.is_empty() {
            let _ = app.message_tx.send(AppMessage::RenameComplete {
                folder_name,
                results,
            });
            ctx.request_repaint();
        }
        return;
    }

    let ctx = ctx.clone();
    let total_work = jobs.len() + cue_jobs.len() + m3u_jobs.len();

    // Clone the Arc<DatIndex> for the background thread
    let dat_index_arc = dat_index.cloned();
    let context = app.context.clone();
    let platform = console.platform;
    let description = format!("Renaming {total_work} entries");

    // Resolve companion locations for this console: media directory and
    // gamelist.xml. Both move inside each game's rename transaction.
    let media_dir = app
        .root_path
        .as_ref()
        .and_then(|rp| {
            state::asset_dir_for_console(rp, &folder_name, &app.settings.general.assets_dir)
        })
        .filter(|d| d.is_dir());
    let gamelist_path = app
        .root_path
        .as_ref()
        .map(|rp| {
            state::metadata_dir_for_console(rp, &folder_name, &app.settings.general.metadata_dir)
                .join("gamelist.xml")
        })
        .filter(|p| p.is_file());

    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::Rename,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let gamelist_rewriter = |stem_map: &HashMap<String, String>| -> Vec<(PathBuf, String)> {
                gamelist_path
                    .as_ref()
                    .and_then(|p| retro_junk_frontend::esde::plan_gamelist_rewrite(p, stem_map))
                    .into_iter()
                    .collect()
            };
            let exec = ExecutionContext {
                media_dir: media_dir.clone(),
                gamelist_rewriter: Some(&gamelist_rewriter),
            };

            let mut file_num = 0usize;
            let send_progress = |file_num: &mut usize| {
                let _ = tx.send(AppMessage::OperationProgress {
                    op_id,
                    current: *file_num as u64,
                    total: total_work as u64,
                });
                *file_num += 1;
            };

            // Step 1: Execute single-file renames (one transaction each)
            for job in &jobs {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                send_progress(&mut file_num);

                // Detect actual format extension from file content
                let detected_ext = sniff_detected_extension(&job.source, &context, platform);
                let target_name = retro_junk_lib::rename::target_filename_for_rename(
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
                            entry_name: job.entry_name.clone(),
                            outcome: RenameOutcome::Renamed {
                                source: job.source.clone(),
                                target,
                            },
                        });
                    }
                    Err(e) => {
                        results.push(RenameResult {
                            entry_name: job.entry_name.clone(),
                            outcome: RenameOutcome::Error {
                                message: format!(
                                    "Failed to rename '{}': {}",
                                    job.source.display(),
                                    e
                                ),
                            },
                        });
                    }
                }
            }

            // Step 2: Plan and execute cue/bin disc sets (one transaction each)
            for job in &cue_jobs {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                send_progress(&mut file_num);

                let outcome = plan_cue_set(&job.cue, dat_index_arc.as_ref(), &context, platform);
                results.push(RenameResult {
                    entry_name: job.entry_name.clone(),
                    outcome: execute_cue_set_outcome(outcome, &job.cue, &exec),
                });
            }

            // Step 3: Resolve and execute multi-disc renames
            for m3u_job in &m3u_jobs {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                send_progress(&mut file_num);

                // Fix up target_filename extensions for resolved non-cue discs
                let mut all_discs: Vec<DiscMatchData> = m3u_job
                    .resolved_discs
                    .iter()
                    .map(|d| {
                        let detected_ext =
                            sniff_detected_extension(&d.file_path, &context, platform);
                        DiscMatchData {
                            file_path: d.file_path.clone(),
                            game_name: d.game_name.clone(),
                            target_filename: retro_junk_lib::rename::target_filename_for_rename(
                                &d.target_filename,
                                &d.file_path,
                                &detected_ext,
                            ),
                        }
                    })
                    .collect();

                // Resolve unresolved non-cue disc files via hashing
                if let Some(ref di) = dat_index_arc {
                    for file_path in &m3u_job.unresolved_files {
                        match resolve_disc_file(file_path, di, &context, platform) {
                            Some(disc_data) => all_discs.push(disc_data),
                            None => {
                                log::warn!("Could not resolve disc file: {}", file_path.display());
                            }
                        }
                    }
                }

                // Plan a verified disc set for each cue in the folder
                let mut disc_sets = Vec::new();
                let mut set_errors = Vec::new();
                for cue in &m3u_job.cue_files {
                    match plan_cue_set(cue, dat_index_arc.as_ref(), &context, platform) {
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
                        entry_name: m3u_job.entry_name.clone(),
                        outcome: RenameOutcome::M3uRenamed {
                            target_folder: m3u_result.final_folder,
                            discs_renamed: m3u_result.discs_renamed,
                            playlist_written: m3u_result.playlist_written,
                            folder_renamed: m3u_result.folder_renamed,
                            errors: m3u_result.errors,
                        },
                    });
                } else if m3u_result.errors.is_empty() {
                    results.push(RenameResult {
                        entry_name: m3u_job.entry_name.clone(),
                        outcome: RenameOutcome::AlreadyCorrect,
                    });
                } else {
                    results.push(RenameResult {
                        entry_name: m3u_job.entry_name.clone(),
                        outcome: RenameOutcome::Error {
                            message: m3u_result.errors.join("; "),
                        },
                    });
                }
            }

            let _ = tx.send(AppMessage::RenameComplete {
                folder_name,
                results,
            });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
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

/// Plan a disc set for a cue file against the loaded DAT index.
/// Returns `None` when no DAT index or analyzer is available.
fn plan_cue_set(
    cue: &Path,
    dat_index: Option<&Arc<DatIndex>>,
    context: &Arc<AnalysisContext>,
    platform: retro_junk_lib::Platform,
) -> Option<DiscSetOutcome> {
    let di = dat_index?;
    let registered = context.get_by_platform(platform)?;
    Some(retro_junk_lib::disc_set::plan_disc_set(
        cue,
        registered.analyzer.as_ref(),
        di,
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

/// Resolve a single disc file by hashing it and matching against the `DatIndex`.
/// Runs on the background thread, so it can sniff the format extension directly.
fn resolve_disc_file(
    file_path: &PathBuf,
    dat_index: &DatIndex,
    context: &Arc<AnalysisContext>,
    platform: retro_junk_lib::Platform,
) -> Option<DiscMatchData> {
    let registered = context.get_by_platform(platform)?;
    let mut file = std::fs::File::open(file_path).ok()?;
    let hashes = retro_junk_lib::hasher::compute_crc32_sha1(
        &mut file,
        registered.analyzer.as_ref(),
        Some(file_path.as_path()),
    )
    .ok()?;
    let m = dat_index
        .match_by_hash(hashes.data_size, &hashes)
        .into_iter()
        .next()?;
    let game = &dat_index.games[m.game_index];
    let rom = &game.roms[m.rom_index];

    let detected_ext = sniff_detected_extension(file_path, context, platform);
    Some(DiscMatchData {
        file_path: file_path.clone(),
        game_name: game.name.clone(),
        target_filename: retro_junk_lib::rename::target_filename_for_rename(
            &rom.name,
            file_path,
            &detected_ext,
        ),
    })
}

/// Try to determine the target ROM filename for an entry.
///
/// Priority:
/// 1. Cached `dat_match.rom_name` (already resolved)
/// 2. Hash lookup against loaded DAT index
/// 3. Serial lookup against loaded DAT index
fn get_target_rom_name(
    app: &RetroJunkApp,
    folder_name: &str,
    entry: &crate::state::LibraryEntry,
) -> Option<String> {
    // 1. Use cached rom_name from dat_match if available (skip cross-region matches)
    if let Some(ref dm) = entry.dat_match
        && !dm.rom_name.is_empty()
        && !dm.cross_region
    {
        return Some(dm.rom_name.clone());
    }

    // 2. Try hash lookup
    let dat_index = app.dat_indices.get(folder_name)?;
    if let Some(ref hashes) = entry.hashes
        && let Some(m) = dat_index
            .match_by_hash(hashes.data_size, hashes)
            .into_iter()
            .next()
    {
        return Some(dat_index.games[m.game_index].roms[m.rom_index].name.clone());
    }

    // 3. Try serial lookup
    if let Some(ref id) = entry.identification {
        let serial = &id.serial_number;
        if !serial.is_empty() {
            let console = app
                .browser
                .consoles
                .iter()
                .find(|c| c.folder_name == folder_name)?;
            let registered = app.context.get_by_platform(console.platform)?;
            let game_code = registered.analyzer.extract_dat_game_code(serial);
            if let retro_junk_dat::SerialLookupResult::Match(m) =
                dat_index.match_by_serial(serial, game_code.as_deref())
            {
                return Some(dat_index.games[m.game_index].roms[m.rom_index].name.clone());
            }
        }
    }

    None
}

/// Quick-analyze a file to detect its format extension.
///
/// Opens the file, runs a quick analysis, and returns the detected extension
/// (e.g., "rvz", "iso", "chd"). Used on the background thread to determine
/// the correct file extension regardless of what the file is currently named.
fn sniff_detected_extension(
    file_path: &Path,
    context: &AnalysisContext,
    platform: retro_junk_lib::Platform,
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
