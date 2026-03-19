use std::path::PathBuf;

use retro_junk_disc::cue::{check_cue_compat, convert_cue_to_standard};

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{AppMessage, CueFixOutcome, CueFixResult};

/// Fix CUE sheets for the selected entries in a console.
///
/// For each selected entry whose analysis path is a `.cue` file, checks for
/// CDRWin compatibility issues and converts to standard CUE format.
pub fn fix_cue_for_selection(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.library.consoles[console_idx];
    let folder_name = console.folder_name.clone();

    // Collect CUE file paths from selected entries
    let mut cue_files: Vec<(String, PathBuf)> = Vec::new();
    for &i in &app.selected_entries {
        let entry = match console.entries.get(i) {
            Some(e) => e,
            None => continue,
        };

        let entry_name = entry.game_entry.display_name().to_string();

        // Collect all CUE files for this entry
        match &entry.game_entry {
            retro_junk_lib::scanner::GameEntry::SingleFile(path) => {
                if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("cue"))
                {
                    cue_files.push((entry_name, path.clone()));
                }
            }
            retro_junk_lib::scanner::GameEntry::MultiDisc { files, .. } => {
                for file in files {
                    if file
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("cue"))
                    {
                        cue_files.push((entry_name.clone(), file.clone()));
                    }
                }
            }
        }
    }

    if cue_files.is_empty() {
        return;
    }

    let ctx = ctx.clone();
    let total = cue_files.len();
    let description = format!("Fixing {} CUE file(s)", total);

    spawn_background_op(app, description, move |op_id, cancel, tx| {
        let mut results: Vec<CueFixResult> = Vec::new();

        for (i, (entry_name, cue_path)) in cue_files.iter().enumerate() {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            let _ = tx.send(AppMessage::OperationProgress {
                op_id,
                current: i as u64,
                total: total as u64,
            });

            let file_name = cue_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&entry_name)
                .to_string();

            // Read CUE content
            let content = match std::fs::read_to_string(&cue_path) {
                Ok(c) => c,
                Err(e) => {
                    results.push(CueFixResult {
                        file_name,
                        outcome: CueFixOutcome::Error {
                            message: format!("Cannot read: {e}"),
                        },
                    });
                    continue;
                }
            };

            // Check compatibility
            let report = check_cue_compat(&content);

            if report.is_standard() {
                results.push(CueFixResult {
                    file_name,
                    outcome: CueFixOutcome::AlreadyStandard,
                });
                continue;
            }

            if let Some(reason) = &report.unfixable_reason {
                results.push(CueFixResult {
                    file_name,
                    outcome: CueFixOutcome::Unfixable {
                        reason: reason.to_string(),
                    },
                });
                continue;
            }

            let summary = report.summary();
            let cue_dir = cue_path.parent().unwrap_or(cue_path.as_path());

            // Convert
            match convert_cue_to_standard(&content, cue_dir) {
                Ok(converted) => {
                    // Create backup
                    let backup_path = cue_path.with_extension("cue.bak");
                    if let Err(e) = std::fs::copy(&cue_path, &backup_path) {
                        results.push(CueFixResult {
                            file_name,
                            outcome: CueFixOutcome::Error {
                                message: format!("Backup failed: {e}"),
                            },
                        });
                        continue;
                    }

                    // Write converted
                    match std::fs::write(&cue_path, &converted) {
                        Ok(()) => {
                            results.push(CueFixResult {
                                file_name,
                                outcome: CueFixOutcome::Fixed { summary },
                            });
                        }
                        Err(e) => {
                            results.push(CueFixResult {
                                file_name,
                                outcome: CueFixOutcome::Error {
                                    message: format!("Write failed: {e}"),
                                },
                            });
                        }
                    }
                }
                Err(e) => {
                    results.push(CueFixResult {
                        file_name,
                        outcome: CueFixOutcome::Error {
                            message: format!("Conversion failed: {e}"),
                        },
                    });
                }
            }
        }

        let _ = tx.send(AppMessage::CueFixComplete {
            folder_name,
            results,
        });
        let _ = tx.send(AppMessage::OperationComplete { op_id });
        ctx.request_repaint();
    });
}
