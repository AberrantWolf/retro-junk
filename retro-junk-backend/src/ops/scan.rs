//! Library scanning: root-folder console discovery and per-console entry
//! scans.
//!
//! Discovery walks the playable root for recognizable console folders (and
//! surfaces archive-only platforms). A console scan lists the folder's game
//! entries, joins them to their durable database rows, quick-analyzes each
//! file, resolves catalog candidates in bulk, and returns a snapshot ready to
//! be committed by the store. The frontend only schedules the call and
//! renders what comes back.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use retro_junk_core::RomAnalyzer;
use retro_junk_disc::cue::check_cue_compat;
use retro_junk_io::ProgressUnit;
use retro_junk_lib::rename;
use retro_junk_lib::scanner;
use retro_junk_lib::{AnalysisContext, AnalysisOptions, ConsoleFolder, Platform};

use super::OpCtx;
use crate::fingerprint::FolderFingerprint;
use crate::library::{
    CueCompatIssue, EntryStatus, LibraryEntry, apply_multi_disc_analysis_results,
    apply_single_analysis_result, detail_to_entry, scanned_entry_for_folder,
};
use crate::store::CompletedConsoleScan;

// ── Root discovery ──────────────────────────────────────────────────────────

/// Discover console folders under `root`, hiding empty aliases and surfacing
/// archive-only platforms.
///
/// **Side effect:** when `archive_root` is given, platforms that exist only
/// in the archive get an empty playable projection folder **created on disk**
/// (`create_dir_all`) under `root`, so they appear as selectable consoles
/// before their first build. Discovery is therefore not read-only.
pub fn discover_console_folders(
    context: &AnalysisContext,
    root: &Path,
    archive_root: Option<&Path>,
) -> std::io::Result<Vec<ConsoleFolder>> {
    let mut scan = context.scan_console_folders(root, None)?;
    suppress_empty_platform_aliases(&mut scan.matches);
    if let Some(archive_root) = archive_root {
        add_archive_only_console_folders(context, root, archive_root, &mut scan.matches);
    }
    Ok(scan.matches)
}

/// If two recognized folders are aliases for the same console, hide only an
/// empty alias when another alias actually contains the playable library.
/// Two populated aliases remain visible because silently merging their
/// contents would conceal real user data.
fn suppress_empty_platform_aliases(matches: &mut Vec<ConsoleFolder>) {
    let mut counts = std::collections::HashMap::new();
    for folder in matches.iter() {
        *counts.entry(folder.platform).or_insert(0_usize) += 1;
    }
    let populated_paths = matches
        .iter()
        .filter(|folder| {
            std::fs::read_dir(&folder.path).is_ok_and(|mut entries| entries.next().is_some())
        })
        .map(|folder| folder.path.clone())
        .collect::<std::collections::HashSet<_>>();
    let populated_aliases = matches
        .iter()
        .filter(|folder| populated_paths.contains(&folder.path))
        .map(|folder| projection_alias_key(folder.platform, &folder.folder_name))
        .collect::<std::collections::HashSet<_>>();
    matches.retain(|folder| {
        populated_paths.contains(&folder.path)
            || !populated_aliases
                .contains(&projection_alias_key(folder.platform, &folder.folder_name))
            || counts.get(&folder.platform).copied().unwrap_or(0) <= 1
    });
}

/// Regional hardware/library identities intentionally remain distinct even
/// when they share an analyzer. Other names are interchangeable folder aliases.
pub fn projection_alias_key(platform: Platform, folder_name: &str) -> String {
    match folder_name.to_ascii_lowercase().as_str() {
        "famicom" | "fc" => "famicom".to_owned(),
        "sfc" | "super-famicom" | "super famicom" => "super-famicom".to_owned(),
        "megadrive" | "mega drive" | "megadrivejp" | "md" => "megadrive".to_owned(),
        "genesis" | "gen" => "genesis".to_owned(),
        "megacd" | "mega cd" => "megacd".to_owned(),
        "segacd" | "sega cd" => "segacd".to_owned(),
        "mark iii" | "mark-iii" => "mark-iii".to_owned(),
        "pce" | "pc engine" | "pc-engine" | "pcengine" => "pc-engine".to_owned(),
        "tg16" | "tg-16" | "turbografx" | "turbografx-16" | "turbo grafx 16" => {
            "turbografx-16".to_owned()
        }
        "saturnjp" => "saturnjp".to_owned(),
        _ => platform.short_name().to_owned(),
    }
}

/// Add empty playable projection folders for archive platforms that otherwise
/// would have no selectable console in the Library view. The folders contain
/// no authoritative data; they are merely stable destinations for future
/// derivatives and let archived-only carriers surface before the first build.
///
/// **Side effect:** creates the projection folders on disk (`create_dir_all`)
/// as part of discovery.
fn add_archive_only_console_folders(
    context: &AnalysisContext,
    playable_root: &Path,
    archive_root: &Path,
    matches: &mut Vec<ConsoleFolder>,
) {
    let Ok(entries) = std::fs::read_dir(archive_root) else {
        return;
    };
    // Platform directories are part of the portable archive layout. Listing
    // this one level avoids walking and hashing every manifest on a network
    // archive merely to populate the console tree.
    let mut platform_ids = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect::<Vec<_>>();
    platform_ids.sort();
    platform_ids.dedup();
    for platform_id in platform_ids {
        let analyzer_name = match platform_id.as_str() {
            "super-famicom" => "snes",
            other => other,
        };
        let Some(registered) = context.find_by_folder(analyzer_name).into_iter().next() else {
            log::warn!(
                "Archive platform {platform_id} has no registered library analyzer; it remains available in Collection"
            );
            continue;
        };
        let archive_alias = projection_alias_key(registered.metadata.platform, &platform_id);
        if matches.iter().any(|folder| {
            projection_alias_key(folder.platform, &folder.folder_name) == archive_alias
        }) {
            continue;
        }
        let projection_folder = retro_junk_frontend::esde::system_directory(&platform_id, None);
        let path = playable_root.join(&projection_folder);
        if let Err(error) = std::fs::create_dir_all(&path) {
            log::warn!(
                "Could not create playable projection folder {}: {error}",
                path.display()
            );
            continue;
        }
        matches.push(ConsoleFolder {
            path,
            folder_name: projection_folder,
            platform: registered.metadata.platform,
        });
    }
    matches.sort_by(|a, b| a.folder_name.cmp(&b.folder_name));
}

// ── Console scan ────────────────────────────────────────────────────────────

/// Everything one console scan needs. Names and paths are immutable job
/// inputs; nothing is resolved through frontend state once the scan starts.
pub struct ConsoleScanRequest {
    /// Durable console identity, when the console has been reconciled before.
    pub console_id: Option<retro_junk_db::LibraryConsoleId>,
    pub root_path: PathBuf,
    pub platform: Platform,
    pub folder_name: String,
    pub folder_path: PathBuf,
}

/// What a completed console scan hands back to the frontend.
pub struct ConsoleScanOutcome {
    /// The reconciliation-ready snapshot for the store, or the serialization
    /// error that prevented building it.
    pub snapshot: Result<CompletedConsoleScan, String>,
    /// Loose disc entry-point files at the top level (not inside .m3u
    /// folders) — non-empty means the user may want the Organize command.
    pub loose_disc_files: Vec<PathBuf>,
    pub fingerprint: FolderFingerprint,
}

pub enum ConsoleScanError {
    /// The scan could not run; the message is user-facing.
    Failed(String),
    /// The operation was cancelled mid-scan.
    Cancelled,
}

/// Scan one console folder: discover game entries, join them to durable rows
/// (opening the catalog database from `db_path` on this thread), analyze each
/// file, and resolve catalog candidates in bulk.
pub fn scan_console(
    context: &AnalysisContext,
    db_path: Option<&Path>,
    request: &ConsoleScanRequest,
    ctx: &OpCtx,
) -> Result<ConsoleScanOutcome, ConsoleScanError> {
    let Some(registered) = context.get_by_platform(request.platform) else {
        return Err(ConsoleScanError::Failed(format!(
            "No analyzer is registered for {:?}",
            request.platform
        )));
    };

    let extensions = scanner::extension_set(registered.analyzer.file_extensions());
    let entries = scanner::scan_game_entries(&request.folder_path, &extensions).map_err(|e| {
        log::warn!("Failed to scan {}: {}", request.folder_path.display(), e);
        ConsoleScanError::Failed(format!(
            "Failed to scan {}: {e}",
            request.folder_path.display()
        ))
    })?;

    // Detect loose disc entry-point files (for organize feature)
    let loose_disc_files: Vec<PathBuf> = entries
        .iter()
        .filter_map(|e| {
            if let scanner::GameEntry::SingleFile(path) = e {
                let filename = path.file_name()?.to_string_lossy();
                if rename::is_m3u_entry_point(&filename) {
                    return Some(path.clone());
                }
            }
            None
        })
        .collect();

    let snapshots =
        scan_entry_snapshots(entries, &request.folder_path, request.console_id, db_path);
    let analyzed = analyze_entry_snapshots(snapshots, registered.analyzer.as_ref(), db_path, ctx)
        .ok_or(ConsoleScanError::Cancelled)?;

    let fingerprint = crate::fingerprint::compute_fingerprint(&request.folder_path);
    let snapshot = analyzed
        .iter()
        .map(|entry| scanned_entry_for_folder(&request.folder_path, entry))
        .collect::<Result<Vec<_>, _>>()
        .map(|entries| CompletedConsoleScan {
            root_path: request.root_path.to_string_lossy().into_owned(),
            platform: request.platform.short_name().to_owned(),
            folder_name: request.folder_name.clone(),
            folder_path: request.folder_path.to_string_lossy().into_owned(),
            fingerprint_hash: fingerprint.name_hash.clone(),
            entries,
        })
        .map_err(|error| error.to_string());

    Ok(ConsoleScanOutcome {
        snapshot,
        loose_disc_files,
        fingerprint,
    })
}

/// Re-analyze the given entries without rediscovering the folder. Returns
/// `None` when cancelled or when no analyzer is registered for the platform.
pub fn analyze_entries(
    context: &AnalysisContext,
    platform: Platform,
    db_path: Option<&Path>,
    entries: Vec<LibraryEntry>,
    ctx: &OpCtx,
) -> Option<Vec<LibraryEntry>> {
    let registered = context.get_by_platform(platform)?;
    analyze_entry_snapshots(entries, registered.analyzer.as_ref(), db_path, ctx)
}

fn fresh_scan_entry(game_entry: scanner::GameEntry) -> LibraryEntry {
    LibraryEntry {
        id: None,
        revision: 0,
        source_revision: 0,
        game_entry,
        identification: None,
        hashes: None,
        disc_verification: Default::default(),
        dat_match: None,
        status: EntryStatus::Unknown,
        ambiguous_candidates: Vec::new(),
        asset_paths: None,
        region_override: None,
        cover_title: String::new(),
        screen_title: String::new(),
        disc_identifications: None,
        broken_references: None,
        cue_compat_issues: None,
        disambiguation: None,
        tag: None,
    }
}

/// Join filesystem discovery to durable rows by source key/fingerprint.
///
/// Opens its own connection to the catalog database from `db_path` — on the
/// scan worker thread, never a UI thread — and drops it before analysis
/// begins. Frontend state is deliberately not consulted.
fn scan_entry_snapshots(
    game_entries: Vec<scanner::GameEntry>,
    folder_path: &Path,
    console_id: Option<retro_junk_db::LibraryConsoleId>,
    db_path: Option<&Path>,
) -> Vec<LibraryEntry> {
    let mut existing: HashMap<String, (String, LibraryEntry)> = console_id
        .zip(db_path)
        .and_then(|(console_id, path)| {
            let conn = retro_junk_db::open_database(path).ok()?;
            let details = retro_junk_db::load_entry_details_for_console(&conn, console_id).ok()?;
            Some(
                details
                    .into_iter()
                    .filter_map(|detail| {
                        let key = detail.entry_key.as_str().to_owned();
                        let fingerprint = detail.source_fingerprint.clone();
                        detail_to_entry(detail).map(|entry| (key, (fingerprint, entry)))
                    })
                    .collect(),
            )
        })
        .unwrap_or_default();

    game_entries
        .into_iter()
        .map(|game_entry| {
            let current = serde_json::to_string(&game_entry).ok().and_then(|json| {
                let key =
                    retro_junk_db::source_key_from_game_entry_json(&json, folder_path).ok()?;
                let fingerprint =
                    retro_junk_db::source_fingerprint_from_game_entry_json(&json, folder_path)
                        .ok()?;
                Some((key.as_str().to_owned(), fingerprint))
            });
            if let Some((key, fingerprint)) = current
                && let Some((stored_fingerprint, mut entry)) = existing.remove(&key)
                && stored_fingerprint == fingerprint
            {
                entry.game_entry = game_entry;
                return entry;
            }
            fresh_scan_entry(game_entry)
        })
        .collect()
}

/// Quick-analyze every entry, then resolve catalog candidates for the whole
/// batch with clustered queries (opening the catalog database from `db_path`
/// on this thread). Returns `None` when cancelled.
fn analyze_entry_snapshots(
    mut entries: Vec<LibraryEntry>,
    analyzer: &dyn RomAnalyzer,
    db_path: Option<&Path>,
    ctx: &OpCtx,
) -> Option<Vec<LibraryEntry>> {
    enum PendingAnalysis {
        Single {
            entry_index: usize,
            result: Result<retro_junk_lib::RomIdentification, retro_junk_lib::AnalysisError>,
            query: usize,
        },
        Multi {
            entry_index: usize,
            results: Vec<(
                PathBuf,
                Result<retro_junk_lib::RomIdentification, retro_junk_lib::AnalysisError>,
            )>,
            queries: Vec<usize>,
        },
    }
    struct EvidenceQuery {
        serial: String,
        hash: retro_junk_db::CatalogHashQuery,
    }

    fn hash_query(hashes: Option<&retro_junk_core::FileHashes>) -> retro_junk_db::CatalogHashQuery {
        hashes.map_or_else(
            || retro_junk_db::CatalogHashQuery {
                file_size: 0,
                crc32: String::new(),
                sha1: String::new(),
            },
            |hashes| retro_junk_db::CatalogHashQuery {
                file_size: hashes.data_size,
                crc32: hashes.crc32.clone(),
                sha1: hashes.sha1.clone().unwrap_or_default(),
            },
        )
    }

    let options = AnalysisOptions::new().quick(true);
    let total = entries.len();
    let mut pending = Vec::with_capacity(total);
    let mut evidence = Vec::new();

    for (entry_index, entry) in entries.iter_mut().enumerate() {
        if ctx.cancelled() {
            return None;
        }
        entry.broken_references = Some(rename::check_broken_references(&entry.game_entry));
        entry.cue_compat_issues = Some(check_cue_compat_for_entry(&entry.game_entry));

        match &entry.game_entry {
            scanner::GameEntry::SingleFile(_) => {
                let path = entry.game_entry.analysis_path();
                let result = match std::fs::File::open(path) {
                    Ok(mut file) => analyzer.analyze(
                        &mut file,
                        &AnalysisOptions {
                            file_path: Some(path.to_path_buf()),
                            ..options.clone()
                        },
                    ),
                    Err(error) => Err(retro_junk_lib::AnalysisError::Io(error)),
                };
                let serial = result
                    .as_ref()
                    .ok()
                    .and_then(|identification| {
                        retro_junk_lib::catalog_match::catalog_serial_key(analyzer, identification)
                    })
                    .unwrap_or_default();
                let query = evidence.len();
                evidence.push(EvidenceQuery {
                    serial,
                    hash: hash_query(entry.hashes.as_ref()),
                });
                pending.push(PendingAnalysis::Single {
                    entry_index,
                    result,
                    query,
                });
            }
            scanner::GameEntry::MultiDisc { files, .. } => {
                let results: Vec<_> = files
                    .iter()
                    .map(|path| {
                        let result = match std::fs::File::open(path) {
                            Ok(mut file) => analyzer.analyze(
                                &mut file,
                                &AnalysisOptions {
                                    file_path: Some(path.clone()),
                                    ..options.clone()
                                },
                            ),
                            Err(error) => Err(retro_junk_lib::AnalysisError::Io(error)),
                        };
                        (path.clone(), result)
                    })
                    .collect();
                let queries = results
                    .iter()
                    .map(|(path, result)| {
                        let serial = result
                            .as_ref()
                            .ok()
                            .and_then(|identification| {
                                retro_junk_lib::catalog_match::catalog_serial_key(
                                    analyzer,
                                    identification,
                                )
                            })
                            .unwrap_or_default();
                        let hashes = entry
                            .disc_identifications
                            .as_ref()
                            .and_then(|discs| discs.iter().find(|disc| disc.path == *path))
                            .and_then(|disc| disc.hashes.as_ref());
                        let query = evidence.len();
                        evidence.push(EvidenceQuery {
                            serial,
                            hash: hash_query(hashes),
                        });
                        query
                    })
                    .collect();
                pending.push(PendingAnalysis::Multi {
                    entry_index,
                    results,
                    queries,
                });
            }
        }

        (ctx.progress)(
            "Analyzing entries",
            ProgressUnit::Items,
            (entry_index + 1) as u64,
            total as u64,
        );
    }

    let matches = db_path
        .and_then(|path| retro_junk_db::open_database(path).ok())
        .and_then(|conn| {
            let serials: Vec<_> = evidence.iter().map(|query| query.serial.clone()).collect();
            let hashes: Vec<_> = evidence.iter().map(|query| query.hash.clone()).collect();
            let mut serial_matches = Vec::with_capacity(evidence.len());
            for cluster in serials.chunks(200) {
                serial_matches.extend(
                    retro_junk_db::match_media_by_serials(&conn, analyzer.short_name(), cluster)
                        .ok()?,
                );
            }
            let mut hash_matches = Vec::with_capacity(evidence.len());
            for cluster in hashes.chunks(200) {
                hash_matches.extend(
                    retro_junk_db::match_media_by_hashes(&conn, analyzer.short_name(), cluster)
                        .ok()?,
                );
            }
            Some(
                hash_matches
                    .into_iter()
                    .zip(serial_matches)
                    .map(|(mut hashes, serials)| {
                        for candidate in serials {
                            if !hashes
                                .iter()
                                .any(|existing| existing.media.id == candidate.media.id)
                            {
                                hashes.push(candidate);
                            }
                        }
                        hashes
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_else(|| vec![Vec::new(); evidence.len()]);

    for analysis in pending {
        if ctx.cancelled() {
            return None;
        }
        match analysis {
            PendingAnalysis::Single {
                entry_index,
                result,
                query,
            } => apply_single_analysis_result(
                &mut entries[entry_index],
                result,
                matches.get(query).map_or(&[], Vec::as_slice),
            ),
            PendingAnalysis::Multi {
                entry_index,
                results,
                queries,
            } => {
                let candidates: Vec<_> = queries
                    .into_iter()
                    .map(|query| matches.get(query).cloned().unwrap_or_default())
                    .collect();
                apply_multi_disc_analysis_results(&mut entries[entry_index], &results, &candidates);
            }
        }
    }
    Some(entries)
}

/// Check CUE sheet compatibility for an entry's CUE files.
fn check_cue_compat_for_entry(entry: &scanner::GameEntry) -> Vec<CueCompatIssue> {
    let mut issues = Vec::new();
    for path in entry.cue_files() {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let report = check_cue_compat(&content);
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        if report.is_standard() {
            let layout = retro_junk_disc::cue::parse_cue(&content).and_then(|sheet| {
                retro_junk_disc::track_layout::cue_track_spans(
                    &sheet,
                    path.parent().unwrap_or(Path::new(".")),
                )
            });
            if let Err(error) = layout {
                issues.push(CueCompatIssue {
                    file_name,
                    summary: format!("Invalid logical track layout: {error}"),
                    can_auto_fix: false,
                });
            }
        } else {
            issues.push(CueCompatIssue {
                file_name,
                summary: report.summary(),
                can_auto_fix: report.can_auto_fix(),
            });
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::suppress_empty_platform_aliases;

    #[test]
    fn empty_alias_is_hidden_beside_populated_console_folder() {
        let temp = tempfile::tempdir().unwrap();
        let ps1 = temp.path().join("ps1");
        let psx = temp.path().join("psx");
        std::fs::create_dir_all(&ps1).unwrap();
        std::fs::create_dir_all(&psx).unwrap();
        std::fs::write(psx.join("game.chd"), b"game").unwrap();
        let mut folders = vec![
            retro_junk_lib::ConsoleFolder {
                path: ps1,
                folder_name: "ps1".to_owned(),
                platform: retro_junk_core::Platform::Ps1,
            },
            retro_junk_lib::ConsoleFolder {
                path: psx.clone(),
                folder_name: "psx".to_owned(),
                platform: retro_junk_core::Platform::Ps1,
            },
        ];
        suppress_empty_platform_aliases(&mut folders);
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].path, psx);
    }

    #[test]
    fn regional_hardware_names_remain_distinct() {
        let temp = tempfile::tempdir().unwrap();
        let nes = temp.path().join("nes");
        let famicom = temp.path().join("famicom");
        std::fs::create_dir_all(&nes).unwrap();
        std::fs::create_dir_all(&famicom).unwrap();
        std::fs::write(nes.join("game.nes"), b"game").unwrap();
        let mut folders = vec![
            retro_junk_lib::ConsoleFolder {
                path: nes,
                folder_name: "nes".to_owned(),
                platform: retro_junk_core::Platform::Nes,
            },
            retro_junk_lib::ConsoleFolder {
                path: famicom.clone(),
                folder_name: "famicom".to_owned(),
                platform: retro_junk_core::Platform::Nes,
            },
        ];
        suppress_empty_platform_aliases(&mut folders);
        assert_eq!(folders.len(), 2);
        assert!(folders.iter().any(|folder| folder.path == famicom));
    }
}
