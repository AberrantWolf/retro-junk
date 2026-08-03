//! Thin dispatch to `retro_junk_backend::ops::organize`. Scheduling and
//! message delivery only — planning and moving live in the backend.

use retro_junk_backend::ops::OpCtx;
use retro_junk_lib::organize::OrganizePlan;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// Plan the organize operation for a console on a background thread and send
/// the plan to the UI for preview. Execution happens only after the user
/// confirms, via [`execute_organize_plan`].
pub fn organize_console(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.browser.consoles[console_idx];
    let folder_name = console.folder_name.clone();
    let folder_path = console.folder_path.clone();
    let platform = console.platform;
    let context = app.context.clone();

    let description = format!("Organizing disc files in {folder_name}");
    let ctx = ctx.clone();
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::Other,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = crate::backend::worker::forward_phases(op_id, tx.clone());
            let outcome = retro_junk_backend::ops::organize::plan(
                &context,
                platform,
                &folder_path,
                &OpCtx::new(&cancel, &progress),
            );
            match outcome {
                Ok(plan) => {
                    if !cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = tx.send(AppMessage::OrganizePlanReady { folder_name, plan });
                    }
                }
                Err(error) => {
                    let _ = tx.send(AppMessage::OrganizeComplete {
                        folder_name,
                        rescan_target: None,
                        jobs_executed: 0,
                        files_moved: 0,
                        unmatched: 0,
                        errors: vec![error],
                    });
                }
            }
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}

/// Execute an already-planned organize operation (called after user confirms preview).
pub fn execute_organize_plan(
    app: &mut RetroJunkApp,
    folder_name: String,
    plan: OrganizePlan,
    ctx: &egui::Context,
) {
    let rescan_target = app
        .browser
        .find_by_folder(&folder_name)
        .and_then(|index| crate::backend::scan::ConsoleScanTarget::durable(app, index));
    let description = format!("Moving disc files in {folder_name}");
    let ctx = ctx.clone();
    let scope = folder_name.clone();

    spawn_background_op(
        app,
        description,
        OperationKind::Other,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = crate::backend::worker::forward_phases(op_id, tx.clone());
            let outcome = retro_junk_backend::ops::organize::execute_plan(
                &plan,
                &OpCtx::new(&cancel, &progress),
            );
            let _ = tx.send(AppMessage::OrganizeComplete {
                folder_name,
                rescan_target,
                jobs_executed: outcome.jobs_executed,
                files_moved: outcome.files_moved,
                unmatched: outcome.unmatched,
                errors: outcome.errors,
            });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}
