//! Thin dispatch to `retro_junk_backend::ops::fix_cue`. Collects the CUE
//! paths from the current selection, schedules the fix, and delivers the
//! results — the repair itself lives in the backend.

use std::path::PathBuf;

use retro_junk_backend::ops::OpCtx;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// Fix CUE sheets for the selected entries in a console.
///
/// For each selected entry whose analysis path is a `.cue` file, checks for
/// `CDRWin` compatibility issues and converts to standard CUE format.
pub fn fix_cue_for_selection(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.browser.consoles[console_idx];
    let folder_name = console.folder_name.clone();
    let rescan_target = crate::backend::scan::ConsoleScanTarget::durable(app, console_idx);

    // Collect CUE file paths from selected entries
    let mut cue_files: Vec<(String, PathBuf)> = Vec::new();
    for &i in &app.ui_state.selected_entries {
        let Some(entry) = console.entry_by_id(i) else {
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
    let description = format!("Fixing {} CUE file(s)", cue_files.len());
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::CueFix,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = crate::backend::worker::forward_phases(op_id, tx.clone());
            let results = retro_junk_backend::ops::fix_cue::fix_cues(
                &cue_files,
                &OpCtx::new(&cancel, &progress),
            );
            let _ = tx.send(AppMessage::CueFixComplete {
                folder_name,
                rescan_target,
                results,
            });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}
