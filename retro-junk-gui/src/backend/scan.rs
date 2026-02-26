use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use retro_junk_lib::AnalysisOptions;
use retro_junk_lib::scanner;

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, BackgroundOperation, next_operation_id};

/// Scan a root folder for console subfolders on a background thread.
pub fn scan_root_folder(app: &mut RetroJunkApp, root: PathBuf, ctx: &egui::Context) {
    let tx = app.message_tx.clone();
    let context = app.context.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let op_id = next_operation_id();
    let ctx = ctx.clone();

    app.operations.push(BackgroundOperation::new(
        op_id,
        "Scanning folders...".to_string(),
        cancel.clone(),
    ));

    std::thread::spawn(move || {
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
    });
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

    let tx = app.message_tx.clone();
    let context = app.context.clone();
    let folder_path = console.folder_path.clone();
    let folder_name = console.folder_name.clone();
    let platform = console.platform;
    let cancel = Arc::new(AtomicBool::new(false));
    let op_id = next_operation_id();
    let ctx = ctx.clone();

    let platform_name = console.platform_name.to_string();
    app.operations.push(BackgroundOperation::new(
        op_id,
        format!("Scanning {} ({})", platform_name, folder_name),
        cancel.clone(),
    ));

    std::thread::spawn(move || {
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

        // Now analyze each entry
        let options = AnalysisOptions::new().quick(true);

        for (i, entry) in entries.iter().enumerate() {
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
                            registered.analyzer.analyze(&mut file, &file_options)
                        }
                        Err(e) => Err(retro_junk_lib::AnalysisError::Io(e)),
                    };

                    let _ = tx.send(AppMessage::EntryAnalyzed {
                        folder_name: folder_name.clone(),
                        index: i,
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
                                    registered.analyzer.analyze(&mut file, &file_options)
                                }
                                Err(e) => Err(retro_junk_lib::AnalysisError::Io(e)),
                            };
                            (path.clone(), result)
                        })
                        .collect();

                    let _ = tx.send(AppMessage::MultiDiscAnalyzed {
                        folder_name: folder_name.clone(),
                        index: i,
                        disc_results,
                    });
                }
            }

            let _ = tx.send(AppMessage::OperationProgress {
                op_id,
                current: (i + 1) as u64,
                total: entries.len() as u64,
            });

            if i % 10 == 0 {
                ctx.request_repaint();
            }
        }

        let _ = tx.send(AppMessage::ConsoleScanDone {
            folder_name: folder_name.clone(),
        });
        let _ = tx.send(AppMessage::OperationComplete { op_id });
        ctx.request_repaint();

        // Auto-load DAT after scan completes
        crate::backend::dat::load_dat_for_console(tx, context, platform, folder_name, ctx);
    });
}
