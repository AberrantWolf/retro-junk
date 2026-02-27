use std::path::PathBuf;
use std::sync::atomic::Ordering;

use retro_junk_lib::hasher;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::AppMessage;

/// A single unit of hash work: either a whole entry or one disc of a multi-disc entry.
enum HashWork {
    SingleFile {
        entry_index: usize,
        path: PathBuf,
    },
    Disc {
        entry_index: usize,
        disc_path: PathBuf,
    },
}

/// Compute hashes for selected entries in the active console.
pub fn compute_hashes_for_selection(app: &mut RetroJunkApp, console_idx: usize) {
    let console = &app.library.consoles[console_idx];
    let platform = console.platform;
    let folder_name = console.folder_name.clone();

    // Collect work items — single-file entries get one item,
    // multi-disc entries get one item per disc (skipping already-hashed discs).
    let work: Vec<HashWork> = app
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| console.entries.get(i).map(|e| (i, e)))
        .flat_map(|(i, entry)| {
            if let Some(ref discs) = entry.disc_identifications {
                // Multi-disc: hash each disc individually
                discs
                    .iter()
                    .map(|d| HashWork::Disc {
                        entry_index: i,
                        disc_path: d.path.clone(),
                    })
                    .collect::<Vec<_>>()
            } else {
                // Single-file: hash the analysis path
                vec![HashWork::SingleFile {
                    entry_index: i,
                    path: entry.game_entry.analysis_path().to_path_buf(),
                }]
            }
        })
        .collect();

    if work.is_empty() {
        return;
    }

    let context = app.context.clone();
    let description = format!("Computing hashes ({} files)", work.len());

    spawn_background_op(app, description, move |op_id, cancel, tx| {
        let registered = match context.get_by_platform(platform) {
            Some(r) => r,
            None => {
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                return;
            }
        };

        for (file_num, item) in work.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                return;
            }

            let _ = tx.send(AppMessage::OperationProgress {
                op_id,
                current: file_num as u64,
                total: work.len() as u64,
            });

            let (entry_index, path) = match item {
                HashWork::SingleFile { entry_index, path } => (*entry_index, path),
                HashWork::Disc {
                    entry_index,
                    disc_path,
                } => (*entry_index, disc_path),
            };

            match std::fs::File::open(path) {
                Ok(mut file) => {
                    match hasher::compute_crc32_sha1(&mut file, registered.analyzer.as_ref()) {
                        Ok(hashes) => {
                            let msg = match item {
                                HashWork::SingleFile { .. } => AppMessage::HashComplete {
                                    folder_name: folder_name.clone(),
                                    index: entry_index,
                                    hashes,
                                },
                                HashWork::Disc { disc_path, .. } => AppMessage::DiscHashComplete {
                                    folder_name: folder_name.clone(),
                                    entry_index,
                                    disc_path: disc_path.clone(),
                                    hashes,
                                },
                            };
                            let _ = tx.send(msg);
                        }
                        Err(e) => {
                            let _ = tx.send(AppMessage::HashFailed {
                                folder_name: folder_name.clone(),
                                index: entry_index,
                                error: e.to_string(),
                            });
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::HashFailed {
                        folder_name: folder_name.clone(),
                        index: entry_index,
                        error: e.to_string(),
                    });
                }
            }
        }

        let _ = tx.send(AppMessage::OperationComplete { op_id });
    });
}
