use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use retro_junk_core::RomAnalyzer;
use retro_junk_disc::cue::check_cue_compat;
use retro_junk_lib::AnalysisOptions;
use retro_junk_lib::rename;
use retro_junk_lib::scanner;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{
    AppMessage, CueCompatIssue, EntryStatus, LibraryEntry, OperationKind, ProgressDisplay,
};

/// Scan a root folder for console subfolders on a background thread.
pub fn scan_root_folder(app: &mut RetroJunkApp, root: PathBuf, ctx: &egui::Context) {
    let context = app.context.clone();
    let ctx = ctx.clone();
    let archive_root = app
        .settings
        .library
        .active_profile()
        .filter(|profile| profile.playable_root == root)
        .map(|profile| profile.archive_root.clone());

    spawn_background_op(
        app,
        "Scanning folders...".to_string(),
        OperationKind::UiFetch,
        String::new(),
        ProgressDisplay::Count,
        move |_op_id, cancel, tx| {
            let result = context.scan_console_folders(&root, None);
            match result {
                Ok(mut scan) => {
                    if let Some(archive_root) = archive_root {
                        add_archive_only_console_folders(
                            &context,
                            &root,
                            &archive_root,
                            &mut scan.matches,
                        );
                    }
                    for cf in scan.matches {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        if let Some(registered) = context.get_by_platform(cf.platform) {
                            let _ = tx.send(AppMessage::ConsoleFolderFound {
                                platform: cf.platform,
                                folder_name: cf.folder_name,
                                folder_path: cf.path,
                                manufacturer: registered.metadata.manufacturer,
                                platform_name: registered.metadata.platform_name,
                            });
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to scan root folder: {e}");
                }
            }
            let _ = tx.send(AppMessage::FolderScanComplete);
            ctx.request_repaint();
        },
    );
}

/// Add empty playable projection folders for archive platforms that otherwise
/// would have no selectable console in the Library view. The folders contain
/// no authoritative data; they are merely stable destinations for future
/// derivatives and let archived-only carriers surface before the first build.
fn add_archive_only_console_folders(
    context: &retro_junk_lib::AnalysisContext,
    playable_root: &std::path::Path,
    archive_root: &std::path::Path,
    matches: &mut Vec<retro_junk_lib::ConsoleFolder>,
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
        if matches
            .iter()
            .any(|folder| folder.folder_name.eq_ignore_ascii_case(&platform_id))
        {
            continue;
        }
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
        let path = playable_root.join(&platform_id);
        if let Err(error) = std::fs::create_dir_all(&path) {
            log::warn!(
                "Could not create playable projection folder {}: {error}",
                path.display()
            );
            continue;
        }
        matches.push(retro_junk_lib::ConsoleFolder {
            path,
            folder_name: platform_id,
            platform: registered.metadata.platform,
        });
    }
    matches.sort_by(|a, b| a.folder_name.cmp(&b.folder_name));
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
        tag: None,
    }
}

/// Join filesystem discovery to durable rows by source key/fingerprint. The
/// active UI page is deliberately not consulted.
fn scan_entry_snapshots(
    game_entries: Vec<scanner::GameEntry>,
    folder_path: &std::path::Path,
    console_id: Option<retro_junk_db::LibraryConsoleId>,
    db_path: Option<&std::path::Path>,
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
                        crate::cache::detail_to_entry(detail)
                            .map(|entry| (key, (fingerprint, entry)))
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

/// Quick-scan a single console folder: discover game entries, then analyze each.
///
/// The index is resolved only at this UI boundary; persisted work uses durable IDs.
pub fn quick_scan_console(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    if app.browser.consoles[console_idx].scan_status != crate::state::ScanStatus::NotScanned {
        return;
    }
    app.browser.consoles[console_idx].scan_status = crate::state::ScanStatus::Scanning;

    let Some(target) = ConsoleScanTarget::from_app(app, console_idx) else {
        app.browser.consoles[console_idx].scan_status = crate::state::ScanStatus::NotScanned;
        return;
    };
    start_console_scan(app, target, ctx);
}

/// Everything a durable scan needs, captured before the worker starts. The
/// database ID is the identity; names and paths are immutable job inputs and
/// are never resolved again through the active UI projection.
#[derive(Debug, Clone)]
pub struct ConsoleScanTarget {
    pub console_id: Option<retro_junk_db::LibraryConsoleId>,
    pub root_path: PathBuf,
    pub platform: retro_junk_core::Platform,
    pub folder_name: String,
    pub folder_path: PathBuf,
    pub platform_name: String,
}

impl ConsoleScanTarget {
    pub fn from_app(app: &RetroJunkApp, console_idx: usize) -> Option<Self> {
        let console = app.browser.consoles.get(console_idx)?;
        Some(Self {
            console_id: console.id,
            root_path: app.root_path.clone()?,
            platform: console.platform,
            folder_name: console.folder_name.clone(),
            folder_path: console.folder_path.clone(),
            platform_name: console.platform_name.to_owned(),
        })
    }

    pub fn durable(app: &RetroJunkApp, console_idx: usize) -> Option<Self> {
        let target = Self::from_app(app, console_idx)?;
        target.console_id?;
        Some(target)
    }
}

pub fn restart_console_scan(
    app: &mut RetroJunkApp,
    target: ConsoleScanTarget,
    ctx: &egui::Context,
) {
    if let Some(console_id) = target.console_id {
        app.submit_store(
            crate::backend::library_store::LibraryStoreRequest::MarkConsoleStale(console_id),
            ctx,
        );
        if let Some(console) = app
            .browser
            .consoles
            .iter_mut()
            .find(|console| console.id == Some(console_id))
        {
            console.scan_status = crate::state::ScanStatus::Scanning;
            console.fingerprint = None;
            console.loose_disc_files.clear();
        }
    }
    start_console_scan(app, target, ctx);
}

fn start_console_scan(app: &mut RetroJunkApp, target: ConsoleScanTarget, ctx: &egui::Context) {
    let ConsoleScanTarget {
        console_id,
        root_path,
        platform,
        folder_name,
        folder_path,
        platform_name,
    } = target;

    let context = app.context.clone();
    let db_path = app.db_path.clone();
    let ctx = ctx.clone();

    let description = format!("Scanning {platform_name} ({folder_name})");
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::Scan,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let Some(registered) = context.get_by_platform(platform) else {
                let _ = tx.send(AppMessage::ConsoleScanFailed {
                    folder_name,
                    error: Some(format!("No analyzer is registered for {platform:?}")),
                });
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                ctx.request_repaint();
                return;
            };

            let extensions = scanner::extension_set(registered.analyzer.file_extensions());
            let entries = match scanner::scan_game_entries(&folder_path, &extensions) {
                Ok(e) => e,
                Err(e) => {
                    log::warn!("Failed to scan {}: {}", folder_path.display(), e);
                    let _ = tx.send(AppMessage::ConsoleScanFailed {
                        folder_name,
                        error: Some(format!("Failed to scan {}: {e}", folder_path.display())),
                    });
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    ctx.request_repaint();
                    return;
                }
            };

            // Detect loose disc entry-point files (for organize feature)
            let loose_disc_files: Vec<PathBuf> = entries
                .iter()
                .filter_map(|e| {
                    if let scanner::GameEntry::SingleFile(path) = e {
                        let filename = path.file_name()?.to_string_lossy();
                        if retro_junk_lib::rename::is_m3u_entry_point(&filename) {
                            return Some(path.clone());
                        }
                    }
                    None
                })
                .collect();

            let snapshots =
                scan_entry_snapshots(entries, &folder_path, console_id, db_path.as_deref());
            let Some(analyzed) = analyze_entry_snapshots(
                snapshots,
                registered.analyzer.as_ref(),
                &tx,
                op_id,
                &cancel,
                &ctx,
                db_path.as_deref(),
            ) else {
                let _ = tx.send(AppMessage::ConsoleScanFailed {
                    folder_name,
                    error: None,
                });
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                ctx.request_repaint();
                return;
            };

            let fingerprint = crate::fingerprint::compute_fingerprint(&folder_path);
            let result = analyzed
                .iter()
                .map(|entry| crate::cache::scanned_entry_for_folder(&folder_path, entry))
                .collect::<Result<Vec<_>, _>>()
                .map(
                    |entries| crate::backend::library_store::CompletedConsoleScan {
                        root_path: root_path.to_string_lossy().into_owned(),
                        platform: platform.short_name().to_owned(),
                        folder_name: folder_name.clone(),
                        folder_path: folder_path.to_string_lossy().into_owned(),
                        fingerprint_hash: fingerprint.name_hash.clone(),
                        entries,
                    },
                )
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::ScanSnapshotPrepared {
                folder_name: folder_name.clone(),
                console_id,
                result,
            });
            let _ = tx.send(AppMessage::ScanProjectionInfo {
                folder_name: folder_name.clone(),
                loose_disc_files,
                fingerprint,
            });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}

/// Re-analyze selected entries without rediscovering the folder.
pub fn rescan_selected_entries(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.browser.consoles[console_idx];
    let selected: Vec<LibraryEntry> = app
        .ui_state
        .selected_entries
        .iter()
        .copied()
        .filter_map(|id| console.entry_by_id(id).cloned())
        .collect();

    if selected.is_empty() {
        return;
    }

    let context = app.context.clone();
    let folder_name = console.folder_name.clone();
    let platform = console.platform;
    let db_path = app.db_path.clone();
    let ctx = ctx.clone();

    let count = selected.len();
    let noun = if count == 1 { "entry" } else { "entries" };
    let description = format!("Rescanning {count} {noun}");
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::Scan,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let Some(registered) = context.get_by_platform(platform) else {
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                ctx.request_repaint();
                return;
            };

            if let Some(entries) = analyze_entry_snapshots(
                selected,
                registered.analyzer.as_ref(),
                &tx,
                op_id,
                &cancel,
                &ctx,
                db_path.as_deref(),
            ) {
                let _ = tx.send(AppMessage::EntryAnalysisSnapshotsComplete {
                    folder_name,
                    entries,
                });
            }

            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}

fn analyze_entry_snapshots(
    mut entries: Vec<crate::state::LibraryEntry>,
    analyzer: &dyn RomAnalyzer,
    tx: &crate::state::AppMessageSender,
    op_id: u64,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ctx: &egui::Context,
    db_path: Option<&std::path::Path>,
) -> Option<Vec<crate::state::LibraryEntry>> {
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
        if cancel.load(Ordering::Relaxed) {
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

        let _ = tx.send(AppMessage::OperationProgress {
            op_id,
            current: (entry_index + 1) as u64,
            total: total as u64,
        });
        if entry_index % 10 == 0 {
            ctx.request_repaint();
        }
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
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        match analysis {
            PendingAnalysis::Single {
                entry_index,
                result,
                query,
            } => crate::state::apply_single_analysis_result(
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
                crate::state::apply_multi_disc_analysis_results(
                    &mut entries[entry_index],
                    &results,
                    &candidates,
                );
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
        if !report.is_standard() {
            issues.push(CueCompatIssue {
                file_name,
                summary: report.summary(),
                can_auto_fix: report.can_auto_fix(),
            });
        } else {
            let layout = retro_junk_disc::cue::parse_cue(&content).and_then(|sheet| {
                retro_junk_disc::track_layout::cue_track_spans(
                    &sheet,
                    path.parent().unwrap_or(std::path::Path::new(".")),
                )
            });
            if let Err(error) = layout {
                issues.push(CueCompatIssue {
                    file_name,
                    summary: format!("Invalid logical track layout: {error}"),
                    can_auto_fix: false,
                });
            }
        }
    }
    issues
}
