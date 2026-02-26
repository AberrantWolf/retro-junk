use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use retro_junk_lib::hasher;

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, BackgroundOperation, next_operation_id};

/// Compute hashes for selected entries in the active console.
pub fn compute_hashes_for_selection(app: &mut RetroJunkApp, console_idx: usize) {
    let console = &app.library.consoles[console_idx];
    let platform = console.platform;
    let folder_name = console.folder_name.clone();

    // Collect indices and paths of selected entries that don't already have hashes
    let work: Vec<(usize, std::path::PathBuf)> = app
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| {
            let entry = console.entries.get(i)?;
            if entry.hashes.is_some() {
                return None; // Already hashed
            }
            Some((i, entry.game_entry.analysis_path().to_path_buf()))
        })
        .collect();

    if work.is_empty() {
        return;
    }

    let tx = app.message_tx.clone();
    let context = app.context.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let op_id = next_operation_id();

    app.operations.push(BackgroundOperation::new(
        op_id,
        format!("Computing hashes ({} files)", work.len()),
        cancel.clone(),
    ));

    std::thread::spawn(move || {
        let registered = match context.get_by_platform(platform) {
            Some(r) => r,
            None => {
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                return;
            }
        };

        for (file_num, (entry_idx, path)) in work.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                return;
            }

            let _ = tx.send(AppMessage::OperationProgress {
                op_id,
                current: file_num as u64,
                total: work.len() as u64,
            });

            match std::fs::File::open(path) {
                Ok(mut file) => {
                    match hasher::compute_crc32_sha1(&mut file, registered.analyzer.as_ref()) {
                        Ok(hashes) => {
                            let _ = tx.send(AppMessage::HashComplete {
                                folder_name: folder_name.clone(),
                                index: *entry_idx,
                                hashes,
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(AppMessage::HashFailed {
                                folder_name: folder_name.clone(),
                                index: *entry_idx,
                                error: e.to_string(),
                            });
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::HashFailed {
                        folder_name: folder_name.clone(),
                        index: *entry_idx,
                        error: e.to_string(),
                    });
                }
            }
        }

        let _ = tx.send(AppMessage::OperationComplete { op_id });
    });
}
