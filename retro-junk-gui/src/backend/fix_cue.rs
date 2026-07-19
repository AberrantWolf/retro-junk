use std::path::{Path, PathBuf};

use retro_junk_disc::cue::{check_cue_compat, convert_cue_to_standard};

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{AppMessage, CueFixOutcome, CueFixResult, OperationKind, ProgressDisplay};

/// Fix CUE sheets for the selected entries in a console.
///
/// For each selected entry whose analysis path is a `.cue` file, checks for
/// `CDRWin` compatibility issues and converts to standard CUE format.
pub fn fix_cue_for_selection(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.library.consoles[console_idx];
    let folder_name = console.folder_name.clone();

    // Collect CUE file paths from selected entries
    let mut cue_files: Vec<(String, PathBuf)> = Vec::new();
    for &i in &app.ui_state.selected_entries {
        let Some(entry) = console.entries.get(i) else {
            continue;
        };
        let entry_name = entry.game_entry.display_name().to_string();
        for path in entry.game_entry.cue_files() {
            cue_files.push((entry_name.clone(), path.to_path_buf()));
        }
    }

    if cue_files.is_empty() {
        return;
    }

    let ctx = ctx.clone();
    let total = cue_files.len();
    let description = format!("Fixing {total} CUE file(s)");
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::CueFix,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
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
                    .unwrap_or(entry_name)
                    .to_string();

                results.push(CueFixResult {
                    file_name,
                    outcome: fix_one_cue(cue_path),
                });
            }

            let _ = tx.send(AppMessage::CueFixComplete {
                folder_name,
                results,
            });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}

/// Check one CUE file and, when fixable, back it up and rewrite it in
/// standard format. Returns the outcome for the results dialog.
fn fix_one_cue(cue_path: &Path) -> CueFixOutcome {
    // Read CUE content
    let content = match std::fs::read_to_string(cue_path) {
        Ok(c) => c,
        Err(e) => {
            return CueFixOutcome::Error {
                message: format!("Cannot read: {e}"),
            };
        }
    };

    // Check compatibility
    let report = check_cue_compat(&content);

    if report.is_standard() {
        return CueFixOutcome::AlreadyStandard;
    }

    if let Some(reason) = report.blocked_reason() {
        return CueFixOutcome::Unfixable {
            reason: reason.to_string(),
        };
    }

    let summary = report.summary();
    let cue_dir = cue_path.parent().unwrap_or(cue_path);

    // Convert
    let converted = match convert_cue_to_standard(&content, cue_dir) {
        Ok(c) => c,
        Err(e) => {
            return CueFixOutcome::Error {
                message: format!("Conversion failed: {e}"),
            };
        }
    };

    // Create backup
    let backup_path = cue_path.with_extension("cue.bak");
    if let Err(e) = std::fs::copy(cue_path, &backup_path) {
        return CueFixOutcome::Error {
            message: format!("Backup failed: {e}"),
        };
    }

    // Write converted
    match std::fs::write(cue_path, &converted) {
        Ok(()) => CueFixOutcome::Fixed { summary },
        Err(e) => CueFixOutcome::Error {
            message: format!("Write failed: {e}"),
        },
    }
}
