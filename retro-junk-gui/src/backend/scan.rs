use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

use retro_junk_core::RomAnalyzer;
use retro_junk_lib::AnalysisOptions;
use retro_junk_lib::rename;
use retro_junk_lib::scanner;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::AppMessage;

/// Scan a root folder for console subfolders on a background thread.
pub fn scan_root_folder(app: &mut RetroJunkApp, root: PathBuf, ctx: &egui::Context) {
    let context = app.context.clone();
    let ctx = ctx.clone();

    spawn_background_op(
        app,
        "Scanning folders...".to_string(),
        move |_op_id, cancel, tx| {
            let result = context.scan_console_folders(&root, None);
            match result {
                Ok(scan) => {
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
                    log::warn!("Failed to scan root folder: {}", e);
                }
            }
            let _ = tx.send(AppMessage::FolderScanComplete);
            ctx.request_repaint();
        },
    );
}

/// Quick-scan a single console folder: discover game entries, then analyze each.
///
/// Identified by `console_idx` (position in `library.consoles`) to avoid
/// ambiguity when multiple folders map to the same platform.
pub fn quick_scan_console(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &mut app.library.consoles[console_idx];
    if console.scan_status != crate::state::ScanStatus::NotScanned {
        return;
    }
    console.scan_status = crate::state::ScanStatus::Scanning;

    let context = app.context.clone();
    let folder_path = console.folder_path.clone();
    let folder_name = console.folder_name.clone();
    let platform = console.platform;
    let ctx = ctx.clone();

    let platform_name = console.platform_name.to_string();
    let description = format!("Scanning {} ({})", platform_name, folder_name);

    spawn_background_op(app, description, move |op_id, cancel, tx| {
        let registered = match context.get_by_platform(platform) {
            Some(r) => r,
            None => {
                let _ = tx.send(AppMessage::ConsoleScanDone { folder_name });
                ctx.request_repaint();
                return;
            }
        };

        let extensions = scanner::extension_set(registered.analyzer.file_extensions());
        let entries = match scanner::scan_game_entries(&folder_path, &extensions) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Failed to scan {}: {}", folder_path.display(), e);
                let _ = tx.send(AppMessage::ConsoleScanDone { folder_name });
                ctx.request_repaint();
                return;
            }
        };

        // Send the full entry list so the UI can show file names immediately
        let _ = tx.send(AppMessage::ConsoleScanComplete {
            folder_name: folder_name.clone(),
            entries: entries.clone(),
        });
        ctx.request_repaint();

        // Analyze each entry
        let indexed: Vec<(usize, &scanner::GameEntry)> = entries.iter().enumerate().collect();
        analyze_entries(
            &indexed,
            registered.analyzer.as_ref(),
            &tx,
            &folder_name,
            op_id,
            &cancel,
            &ctx,
        );

        let _ = tx.send(AppMessage::ConsoleScanDone {
            folder_name: folder_name.clone(),
        });
        let _ = tx.send(AppMessage::OperationComplete { op_id });
        ctx.request_repaint();

        // Auto-load DAT after scan completes
        crate::backend::dat::load_dat_for_console(tx, context, platform, folder_name, ctx);
    });
}

/// Re-analyze selected entries without rediscovering the folder.
pub fn rescan_selected_entries(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.library.consoles[console_idx];
    let selected: Vec<(usize, scanner::GameEntry)> = app
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| console.entries.get(i).map(|e| (i, e.game_entry.clone())))
        .collect();

    if selected.is_empty() {
        return;
    }

    let context = app.context.clone();
    let folder_name = console.folder_name.clone();
    let platform = console.platform;
    let ctx = ctx.clone();

    let count = selected.len();
    let noun = if count == 1 { "entry" } else { "entries" };
    let description = format!("Rescanning {} {}", count, noun);

    spawn_background_op(app, description, move |op_id, cancel, tx| {
        let registered = match context.get_by_platform(platform) {
            Some(r) => r,
            None => {
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                ctx.request_repaint();
                return;
            }
        };

        let refs: Vec<(usize, &scanner::GameEntry)> =
            selected.iter().map(|(i, e)| (*i, e)).collect();
        analyze_entries(
            &refs,
            registered.analyzer.as_ref(),
            &tx,
            &folder_name,
            op_id,
            &cancel,
            &ctx,
        );

        let _ = tx.send(AppMessage::OperationComplete { op_id });
        ctx.request_repaint();
    });
}

/// Check broken references for all entries that haven't been checked yet.
///
/// Runs on a background thread so cache loading stays fast. Sends
/// `BrokenRefsChecked` for each entry whose `broken_references` is `None`.
pub fn check_broken_refs_background(
    tx: mpsc::Sender<AppMessage>,
    entries: Vec<(String, usize, scanner::GameEntry)>,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        for (folder_name, index, entry) in &entries {
            let broken = rename::check_broken_references(entry);
            let _ = tx.send(AppMessage::BrokenRefsChecked {
                folder_name: folder_name.clone(),
                index: *index,
                broken_refs: broken,
            });
        }
        ctx.request_repaint();
    });
}

/// Analyze a set of (index, entry) pairs and send results via the message channel.
///
/// Shared by `quick_scan_console` (all entries) and `rescan_selected_entries` (subset).
fn analyze_entries(
    entries: &[(usize, &scanner::GameEntry)],
    analyzer: &dyn RomAnalyzer,
    tx: &mpsc::Sender<AppMessage>,
    folder_name: &str,
    op_id: u64,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    ctx: &egui::Context,
) {
    let options = AnalysisOptions::new().quick(true);
    let total = entries.len();

    for (progress_idx, &(entry_idx, entry)) in entries.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        match entry {
            scanner::GameEntry::SingleFile(_) => {
                let path = entry.analysis_path();
                let result = match std::fs::File::open(path) {
                    Ok(mut file) => {
                        let file_options = AnalysisOptions {
                            file_path: Some(path.to_path_buf()),
                            ..options.clone()
                        };
                        analyzer.analyze(&mut file, &file_options)
                    }
                    Err(e) => Err(retro_junk_lib::AnalysisError::Io(e)),
                };

                let _ = tx.send(AppMessage::EntryAnalyzed {
                    folder_name: folder_name.to_string(),
                    index: entry_idx,
                    result,
                });
            }
            scanner::GameEntry::MultiDisc { files, .. } => {
                let disc_results: Vec<(
                    std::path::PathBuf,
                    Result<retro_junk_lib::RomIdentification, retro_junk_lib::AnalysisError>,
                )> = files
                    .iter()
                    .map(|path| {
                        let result = match std::fs::File::open(path) {
                            Ok(mut file) => {
                                let file_options = AnalysisOptions {
                                    file_path: Some(path.to_path_buf()),
                                    ..options.clone()
                                };
                                analyzer.analyze(&mut file, &file_options)
                            }
                            Err(e) => Err(retro_junk_lib::AnalysisError::Io(e)),
                        };
                        (path.clone(), result)
                    })
                    .collect();

                let _ = tx.send(AppMessage::MultiDiscAnalyzed {
                    folder_name: folder_name.to_string(),
                    index: entry_idx,
                    disc_results,
                });
            }
        }

        // Check for broken CUE/M3U references
        let broken_refs = rename::check_broken_references(entry);
        let _ = tx.send(AppMessage::BrokenRefsChecked {
            folder_name: folder_name.to_string(),
            index: entry_idx,
            broken_refs,
        });

        let _ = tx.send(AppMessage::OperationProgress {
            op_id,
            current: (progress_idx + 1) as u64,
            total: total as u64,
        });

        if progress_idx % 10 == 0 {
            ctx.request_repaint();
        }
    }
}
