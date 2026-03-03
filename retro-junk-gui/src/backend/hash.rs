use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use retro_junk_lib::hasher;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::AppMessage;

/// 4 MB throttle — only send a progress update when at least this many new bytes
/// have been processed since the last report.
const PROGRESS_THROTTLE: u64 = 4 * 1024 * 1024;

/// A single unit of hash work: either a whole entry or one disc of a multi-disc entry.
struct HashWork {
    entry_index: usize,
    path: PathBuf,
    file_size: u64,
    is_disc: bool,
}

/// Compute hashes for selected entries in the active console.
pub fn compute_hashes_for_selection(app: &mut RetroJunkApp, console_idx: usize) {
    let console = &app.library.consoles[console_idx];
    let platform = console.platform;
    let folder_name = console.folder_name.clone();

    // Collect work items — single-file entries get one item,
    // multi-disc entries get one item per disc.
    let work: Vec<HashWork> = app
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| console.entries.get(i).map(|e| (i, e)))
        .flat_map(|(i, entry)| {
            if let Some(ref discs) = entry.disc_identifications {
                discs
                    .iter()
                    .map(|d| {
                        let file_size = std::fs::metadata(&d.path).map(|m| m.len()).unwrap_or(0);
                        HashWork {
                            entry_index: i,
                            path: d.path.clone(),
                            file_size,
                            is_disc: true,
                        }
                    })
                    .collect::<Vec<_>>()
            } else {
                let path = entry.game_entry.analysis_path().to_path_buf();
                let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                vec![HashWork {
                    entry_index: i,
                    path,
                    file_size,
                    is_disc: false,
                }]
            }
        })
        .collect();

    if work.is_empty() {
        return;
    }

    let total_bytes: u64 = work.iter().map(|w| w.file_size).sum();
    let context = app.context.clone();
    let description = format!("Computing hashes ({} files)", work.len());

    let op_id = spawn_background_op(app, description, move |op_id, cancel, tx| {
        let registered = match context.get_by_platform(platform) {
            Some(r) => r,
            None => {
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                return;
            }
        };

        let mut bytes_completed: u64 = 0;
        let last_reported = Cell::new(0u64);

        for item in &work {
            if cancel.load(Ordering::Relaxed) {
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                return;
            }

            let file_base = bytes_completed;

            match std::fs::File::open(&item.path) {
                Ok(mut file) => {
                    match hasher::compute_crc32_sha1_with_progress(
                        &mut file,
                        registered.analyzer.as_ref(),
                        &|file_bytes_done, _file_total| {
                            let current = file_base + file_bytes_done;
                            if current - last_reported.get() >= PROGRESS_THROTTLE {
                                last_reported.set(current);
                                let _ = tx.send(AppMessage::OperationProgress {
                                    op_id,
                                    current,
                                    total: total_bytes,
                                });
                            }
                        },
                        Some(&item.path),
                    ) {
                        Ok(hashes) => {
                            let msg = if item.is_disc {
                                AppMessage::DiscHashComplete {
                                    folder_name: folder_name.clone(),
                                    entry_index: item.entry_index,
                                    disc_path: item.path.clone(),
                                    hashes,
                                }
                            } else {
                                AppMessage::HashComplete {
                                    folder_name: folder_name.clone(),
                                    index: item.entry_index,
                                    hashes,
                                }
                            };
                            let _ = tx.send(msg);
                        }
                        Err(e) => {
                            let _ = tx.send(AppMessage::HashFailed {
                                folder_name: folder_name.clone(),
                                index: item.entry_index,
                                error: e.to_string(),
                            });
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::HashFailed {
                        folder_name: folder_name.clone(),
                        index: item.entry_index,
                        error: e.to_string(),
                    });
                }
            }

            // Always advance past this file (even on failure)
            bytes_completed += item.file_size;
            let _ = tx.send(AppMessage::OperationProgress {
                op_id,
                current: bytes_completed,
                total: total_bytes,
            });
            last_reported.set(bytes_completed);
        }

        let _ = tx.send(AppMessage::OperationComplete { op_id });
    });

    // Mark this operation as byte-level progress
    if let Some(op) = app.operations.iter_mut().find(|op| op.id == op_id) {
        op.progress_is_bytes = true;
    }
}
