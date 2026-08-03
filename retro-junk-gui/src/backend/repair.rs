//! Thin dispatch to `retro_junk_backend::ops::repair`. Scheduling and message
//! delivery only — planning and rewriting live in the backend.

use retro_junk_backend::ops::OpCtx;

use crate::app::RetroJunkApp;
use crate::backend::worker::{forward_phases, spawn_background_op};
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// Plan repairs for one console and hand the plan to the UI.
///
/// Nothing is written here: a repair rewrites files in place, so the user
/// sees what would change and answers before it happens.
pub fn plan_console_repairs(app: &mut RetroJunkApp, console_idx: usize) {
    let console = &app.browser.consoles[console_idx];
    let folder_name = console.folder_name.clone();
    let platform_short_name = console.platform.short_name().to_owned();
    let Some(root) = app.root_path.clone() else {
        return;
    };
    let context = app.context.clone();
    spawn_background_op(
        app,
        format!("Checking {folder_name} for repairable files"),
        OperationKind::Other,
        folder_name.clone(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = forward_phases(op_id, tx.clone());
            let report = retro_junk_backend::ops::repair::plan(
                &context,
                &root,
                &[(folder_name.clone(), platform_short_name)],
                &retro_junk_lib::repair::RepairOptions::default(),
                &OpCtx::new(&cancel, &progress),
            );
            let _ = tx.send(AppMessage::RepairPlanReady {
                folder_name,
                prompt: Box::new(crate::state::RepairPrompt::from_report(&report)),
            });
        },
    );
}

/// Carry out a repair the user confirmed.
pub fn execute_console_repairs(app: &mut RetroJunkApp, folder_name: String, create_backup: bool) {
    let Some(root) = app.root_path.clone() else {
        return;
    };
    let Some(console) = app
        .browser
        .consoles
        .iter()
        .find(|console| console.folder_name == folder_name)
    else {
        return;
    };
    let platform_short_name = console.platform.short_name().to_owned();
    let context = app.context.clone();
    spawn_background_op(
        app,
        format!("Repairing files in {folder_name}"),
        OperationKind::Other,
        folder_name.clone(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = forward_phases(op_id, tx.clone());
            let ctx = OpCtx::new(&cancel, &progress);
            // Re-planned rather than carried over from the preview: the files
            // may have changed since the user was shown the plan, and a
            // repair writes to them.
            let planned = retro_junk_backend::ops::repair::plan(
                &context,
                &root,
                &[(folder_name.clone(), platform_short_name)],
                &retro_junk_lib::repair::RepairOptions::default(),
                &ctx,
            );
            let summaries = retro_junk_backend::ops::repair::execute(&planned, create_backup, &ctx);
            let repaired = summaries
                .iter()
                .map(|console| console.summary.repaired)
                .sum::<usize>();
            let failed = summaries
                .iter()
                .map(|console| console.summary.errors.len())
                .sum::<usize>();
            let _ = tx.send(AppMessage::RepairComplete {
                folder_name,
                repaired,
                failed,
            });
        },
    );
}
