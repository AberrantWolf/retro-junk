use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use retro_junk_dat::DatIndex;
use retro_junk_lib::context::AnalysisContext;
use retro_junk_lib::rename::DiscMatchData;
use retro_junk_lib::scanner::GameEntry;

use crate::app::RetroJunkApp;
use crate::state::{
    AppMessage, BackgroundOperation, RenameOutcome, RenameResult, next_operation_id,
};

/// A single rename job prepared on the UI thread.
struct RenameJob {
    entry_index: usize,
    source: PathBuf,
    target: PathBuf,
}

/// An M3U rename job that needs background resolution of disc files.
struct M3uJob {
    entry_index: usize,
    /// All disc files in this multi-disc set
    files: Vec<PathBuf>,
    /// Already-resolved disc data (from cached dat_match on the primary disc)
    resolved_discs: Vec<DiscMatchData>,
    /// File paths that still need hash-based resolution
    unresolved_files: Vec<PathBuf>,
    /// Pre-resolved game name from catalog DB (skips derive_base_game_name)
    game_name_override: Option<String>,
}

/// Rename selected entries to their DAT-matched filenames.
///
/// Determines target filenames on the UI thread (using cached dat_match/hashes/serial),
/// then spawns a background thread to execute the filesystem renames.
pub fn rename_selected_entries(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.library.consoles[console_idx];
    let folder_name = console.folder_name.clone();

    let mut jobs: Vec<RenameJob> = Vec::new();
    let mut m3u_jobs: Vec<M3uJob> = Vec::new();
    let mut results: Vec<RenameResult> = Vec::new();

    let dat_index = app.dat_indices.get(&folder_name);

    for &i in &app.selected_entries {
        let entry = match console.entries.get(i) {
            Some(e) => e,
            None => continue,
        };

        match &entry.game_entry {
            GameEntry::SingleFile(_) => {
                // Determine target ROM name from existing dat_match or try DAT lookup
                let rom_name = get_target_rom_name(app, &folder_name, entry);

                match rom_name {
                    Some(target_name) => {
                        let source = entry.game_entry.analysis_path().to_path_buf();
                        let target = source.parent().unwrap_or(&source).join(&target_name);

                        if source == target {
                            results.push(RenameResult {
                                entry_index: i,
                                outcome: RenameOutcome::AlreadyCorrect,
                            });
                        } else {
                            jobs.push(RenameJob {
                                entry_index: i,
                                source,
                                target,
                            });
                        }
                    }
                    None => {
                        results.push(RenameResult {
                            entry_index: i,
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
                let Some(_di) = dat_index else {
                    results.push(RenameResult {
                        entry_index: i,
                        outcome: RenameOutcome::NoMatch {
                            reason: "No DAT loaded for multi-disc rename".to_string(),
                        },
                    });
                    continue;
                };

                // Build what we can from per-disc cached dat_match (set by serial or hash matching)
                let mut resolved = Vec::new();
                let mut unresolved = Vec::new();

                let mut disc_resolved: std::collections::HashMap<
                    std::path::PathBuf,
                    DiscMatchData,
                > = std::collections::HashMap::new();
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

                for f in files {
                    if let Some(disc_data) = disc_resolved.remove(f) {
                        resolved.push(disc_data);
                    } else {
                        unresolved.push(f.clone());
                    }
                }

                if resolved.is_empty() && unresolved.is_empty() {
                    results.push(RenameResult {
                        entry_index: i,
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
                    entry_index: i,
                    files: files.clone(),
                    resolved_discs: resolved,
                    unresolved_files: unresolved,
                    game_name_override: entry.dat_match.as_ref().map(|dm| dm.game_name.clone()),
                });
            }
        }
    }

    let has_work = !jobs.is_empty() || !m3u_jobs.is_empty();
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

    let tx = app.message_tx.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let op_id = next_operation_id();
    let ctx = ctx.clone();
    let total_work = jobs.len() + m3u_jobs.len();

    // Clone the Arc<DatIndex> for the background thread
    let dat_index_arc = dat_index.cloned();
    let context = app.context.clone();
    let platform = console.platform;

    app.operations.push(BackgroundOperation::new(
        op_id,
        format!("Renaming {} entries", total_work),
        cancel.clone(),
    ));

    std::thread::spawn(move || {
        let mut file_num = 0usize;

        // Step 1: Execute single-file renames
        for job in jobs.iter() {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            let _ = tx.send(AppMessage::OperationProgress {
                op_id,
                current: file_num as u64,
                total: total_work as u64,
            });
            file_num += 1;

            match std::fs::rename(&job.source, &job.target) {
                Ok(()) => {
                    results.push(RenameResult {
                        entry_index: job.entry_index,
                        outcome: RenameOutcome::Renamed {
                            source: job.source.clone(),
                            target: job.target.clone(),
                        },
                    });
                }
                Err(e) => {
                    results.push(RenameResult {
                        entry_index: job.entry_index,
                        outcome: RenameOutcome::Error {
                            message: format!("Failed to rename '{}': {}", job.source.display(), e),
                        },
                    });
                }
            }
        }

        // Step 2: Resolve and execute multi-disc renames
        for m3u_job in &m3u_jobs {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            let _ = tx.send(AppMessage::OperationProgress {
                op_id,
                current: file_num as u64,
                total: total_work as u64,
            });
            file_num += 1;

            let mut all_discs = m3u_job.resolved_discs.clone();

            // Resolve unresolved disc files via hashing
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

            if all_discs.is_empty() {
                results.push(RenameResult {
                    entry_index: m3u_job.entry_index,
                    outcome: RenameOutcome::NoMatch {
                        reason: "Could not resolve any disc files".to_string(),
                    },
                });
                continue;
            }

            // Rename individual disc files first
            let mut disc_rename_errors = Vec::new();
            let source_folder = match m3u_job.files[0].parent() {
                Some(p) => p.to_path_buf(),
                None => continue,
            };

            let mut rename_map = std::collections::HashMap::new();
            for disc in &all_discs {
                let target = source_folder.join(&disc.target_filename);
                if disc.file_path == target {
                    continue; // Already correctly named
                }
                let old_name = disc
                    .file_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if let Err(e) = std::fs::rename(&disc.file_path, &target) {
                    disc_rename_errors.push(format!(
                        "Failed to rename '{}': {}",
                        disc.file_path.display(),
                        e
                    ));
                } else {
                    rename_map.insert(old_name, disc.target_filename.clone());
                }
            }

            // Fix CUE file references broken by the renames above
            retro_junk_lib::rename::fix_cue_references_in_dir(
                &source_folder,
                &rename_map,
                &mut disc_rename_errors,
            );

            // Fix M3U playlist entries broken by the renames above
            retro_junk_lib::rename::fix_m3u_references_in_dir(
                &source_folder,
                &rename_map,
                &mut disc_rename_errors,
            );

            // Plan and execute M3U action (folder rename + playlist)
            let mut m3u_errors = Vec::new();
            if let Some(action) = retro_junk_lib::rename::plan_m3u_action(
                &source_folder,
                &all_discs,
                None,
                m3u_job.game_name_override.as_deref(),
            ) {
                // If playlist_entries is empty (e.g., CUE/BIN where .bin isn't an
                // entry point), rename any misnamed inner .m3u file before the
                // folder rename moves it.
                if action.playlist_entries.is_empty() {
                    let expected = format!("{}.m3u", action.game_name);
                    if let Some((src, dst)) =
                        retro_junk_lib::rename::detect_misnamed_m3u(&source_folder, &expected)
                    {
                        if let Err(e) = std::fs::rename(&src, &dst) {
                            m3u_errors.push(format!("Failed to rename inner playlist: {}", e));
                        }
                    }
                }

                let m3u_result =
                    retro_junk_lib::rename::execute_m3u_action(&action, &mut m3u_errors);

                // If playlist was written, the inner .m3u is correct. If not
                // (CUE/BIN case), and the folder was renamed, fix the inner .m3u
                // name inside the new folder.
                let final_folder = if m3u_result.folder_renamed {
                    action.target_folder.clone()
                } else {
                    source_folder.clone()
                };

                results.push(RenameResult {
                    entry_index: m3u_job.entry_index,
                    outcome: RenameOutcome::M3uRenamed {
                        target_folder: final_folder,
                        discs_renamed: all_discs.len(),
                        playlist_written: m3u_result.playlist_written,
                        folder_renamed: m3u_result.folder_renamed,
                        errors: [disc_rename_errors, m3u_errors].concat(),
                    },
                });
            } else if disc_rename_errors.is_empty() {
                results.push(RenameResult {
                    entry_index: m3u_job.entry_index,
                    outcome: RenameOutcome::AlreadyCorrect,
                });
            } else {
                results.push(RenameResult {
                    entry_index: m3u_job.entry_index,
                    outcome: RenameOutcome::Error {
                        message: disc_rename_errors.join("; "),
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
    });
}

/// Resolve a single disc file by hashing it and matching against the DatIndex.
fn resolve_disc_file(
    file_path: &PathBuf,
    dat_index: &DatIndex,
    context: &Arc<AnalysisContext>,
    platform: retro_junk_lib::Platform,
) -> Option<DiscMatchData> {
    let registered = context.get_by_platform(platform)?;
    let mut file = std::fs::File::open(file_path).ok()?;
    let hashes =
        retro_junk_lib::hasher::compute_crc32_sha1(&mut file, registered.analyzer.as_ref()).ok()?;
    let m = dat_index.match_by_hash(hashes.data_size, &hashes)?;
    let game = &dat_index.games[m.game_index];
    let rom = &game.roms[m.rom_index];
    Some(DiscMatchData {
        file_path: file_path.clone(),
        game_name: game.name.clone(),
        target_filename: rom.name.clone(),
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
    // 1. Use cached rom_name from dat_match if available
    if let Some(ref dm) = entry.dat_match {
        if !dm.rom_name.is_empty() {
            return Some(dm.rom_name.clone());
        }
    }

    // 2. Try hash lookup
    let dat_index = app.dat_indices.get(folder_name)?;
    if let Some(ref hashes) = entry.hashes {
        if let Some(m) = dat_index.match_by_hash(hashes.data_size, hashes) {
            return Some(dat_index.games[m.game_index].roms[m.rom_index].name.clone());
        }
    }

    // 3. Try serial lookup
    if let Some(ref id) = entry.identification {
        if let Some(ref serial) = id.serial_number {
            let console = app
                .library
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
