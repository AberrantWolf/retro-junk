use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use retro_junk_core::RomAnalyzer;
use retro_junk_dat::FileHashes;
use retro_junk_lib::hasher;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{AppMessage, LibraryEntry, OperationKind, ProgressDisplay};

/// 4 MB throttle — only send a progress update when at least this many new bytes
/// have been processed since the last report.
const PROGRESS_THROTTLE: u64 = 4 * 1024 * 1024;

/// A single unit of hash work: either a whole entry or one disc of a multi-disc entry.
struct HashWork {
    entry_name: String,
    path: PathBuf,
    file_size: u64,
    is_disc: bool,
}

/// Flatten entries into hash work items — single-file entries get one item,
/// multi-disc entries get one item per disc.
fn collect_hash_work<'a>(entries: impl Iterator<Item = &'a LibraryEntry>) -> Vec<HashWork> {
    entries
        .flat_map(|entry| {
            let name = entry.game_entry.display_name().to_string();
            log::debug!(
                "compute_hashes: entry '{}', disc_identifications={}, status={:?}",
                name,
                entry
                    .disc_identifications
                    .as_ref()
                    .map_or(0, std::vec::Vec::len),
                entry.status
            );
            if let Some(ref discs) = entry.disc_identifications {
                discs
                    .iter()
                    .map(|d| {
                        let file_size = std::fs::metadata(&d.path).map_or(0, |m| m.len());
                        log::info!(
                            "compute_hashes: disc path={}, file_size={}",
                            d.path.display(),
                            file_size
                        );
                        HashWork {
                            entry_name: name.clone(),
                            path: d.path.clone(),
                            file_size,
                            is_disc: true,
                        }
                    })
                    .collect::<Vec<_>>()
            } else {
                let path = entry.game_entry.analysis_path().to_path_buf();
                let file_size = std::fs::metadata(&path).map_or(0, |m| m.len());
                log::info!(
                    "compute_hashes: single file path={}, file_size={}",
                    path.display(),
                    file_size
                );
                vec![HashWork {
                    entry_name: name,
                    path,
                    file_size,
                    is_disc: false,
                }]
            }
        })
        .collect()
}

/// Open and hash a single work item, forwarding scaled byte progress
/// (absolute across the whole operation) to `progress`.
fn hash_one(
    item: &HashWork,
    analyzer: &dyn RomAnalyzer,
    file_base: u64,
    progress: &dyn Fn(u64),
) -> Result<FileHashes, String> {
    log::debug!("compute_hashes: opening file {}", item.path.display());
    let mut file = std::fs::File::open(&item.path).map_err(|e| e.to_string())?;
    let item_file_size = item.file_size;
    log::debug!("compute_hashes: calling hasher for {}", item.path.display());
    hasher::compute_crc32_sha1_with_progress(
        &mut file,
        analyzer,
        &|file_bytes_done, file_total| {
            // Scale progress proportionally: container formats (CHD) may hash
            // far fewer bytes than the file size. Map hash progress to the
            // file's share of total_bytes.
            let scaled = if file_total > 0 && file_total != item_file_size {
                (file_bytes_done as f64 / file_total as f64 * item_file_size as f64) as u64
            } else {
                file_bytes_done
            };
            progress(file_base + scaled);
        },
        Some(&item.path),
    )
    .map_err(|e| e.to_string())
}

/// Compute hashes for selected entries in the active console.
pub fn compute_hashes_for_selection(app: &mut RetroJunkApp, console_idx: usize) {
    let console = &app.library.consoles[console_idx];
    let platform = console.platform;
    let folder_name = console.folder_name.clone();

    log::debug!(
        "compute_hashes_for_selection: console_idx={}, selected_entries={:?}, total_entries={}",
        console_idx,
        app.selected_entries,
        console.entries.len()
    );

    let work = collect_hash_work(
        app.selected_entries
            .iter()
            .copied()
            .filter_map(|i| console.entries.get(i)),
    );

    if work.is_empty() {
        log::warn!("compute_hashes_for_selection: work list is empty, returning early");
        return;
    }

    let total_bytes: u64 = work.iter().map(|w| w.file_size).sum();
    let context = app.context.clone();
    let description = format!("Computing hashes ({} files)", work.len());
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::Hash,
        scope,
        ProgressDisplay::Bytes,
        move |op_id, cancel, tx| {
            let Some(registered) = context.get_by_platform(platform) else {
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                return;
            };

            let mut bytes_completed: u64 = 0;
            let last_reported = Cell::new(0u64);

            for item in &work {
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    return;
                }

                let file_base = bytes_completed;
                let throttled_progress = |current: u64| {
                    if current - last_reported.get() >= PROGRESS_THROTTLE {
                        last_reported.set(current);
                        let _ = tx.send(AppMessage::OperationProgress {
                            op_id,
                            current,
                            total: total_bytes,
                        });
                    }
                };

                let msg = match hash_one(
                    item,
                    registered.analyzer.as_ref(),
                    file_base,
                    &throttled_progress,
                ) {
                    Ok(hashes) => {
                        log::debug!(
                            "compute_hashes: success for {}, crc32={}, data_size={}",
                            item.path.display(),
                            hashes.crc32,
                            hashes.data_size
                        );
                        if item.is_disc {
                            AppMessage::DiscHashComplete {
                                folder_name: folder_name.clone(),
                                entry_name: item.entry_name.clone(),
                                disc_path: item.path.clone(),
                                hashes,
                            }
                        } else {
                            AppMessage::HashComplete {
                                folder_name: folder_name.clone(),
                                entry_name: item.entry_name.clone(),
                                hashes,
                            }
                        }
                    }
                    Err(error) => AppMessage::HashFailed {
                        folder_name: folder_name.clone(),
                        entry_name: item.entry_name.clone(),
                        error,
                    },
                };
                let _ = tx.send(msg);

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
        },
    );
}
