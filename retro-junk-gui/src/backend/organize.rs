use retro_junk_lib::organize::{
    OrganizeOptions, OrganizePlan, OrganizeProgress, execute_organize_job, plan_organize,
};

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// Plan and execute the organize operation for a console.
///
/// Phase 1: plan on background thread, send plan to UI for preview.
/// Phase 2 (after user confirmation): execute jobs on background thread.
pub fn organize_console(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.library.consoles[console_idx];
    let folder_name = console.folder_name.clone();
    let folder_path = console.folder_path.clone();
    let platform = console.platform;
    let context = app.context.clone();

    let description = format!("Organizing disc files in {}", folder_name);
    let ctx = ctx.clone();
    let scope = Some(folder_name.clone());

    spawn_background_op(
        app,
        description,
        OperationKind::Other,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let registered = match context.get_by_platform(platform) {
                Some(r) => r,
                None => {
                    let _ = tx.send(AppMessage::OrganizeComplete {
                        folder_name,
                        jobs_executed: 0,
                        files_moved: 0,
                        unmatched: 0,
                        errors: vec!["No analyzer registered for platform".to_string()],
                    });
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    ctx.request_repaint();
                    return;
                }
            };

            let options = OrganizeOptions {
                dat_dir: None,
                limit: None,
                include_single_disc: false,
                hash_fallback: false,
            };

            let progress_callback = |progress: OrganizeProgress| {
                match &progress {
                    OrganizeProgress::Scanning { file_count } => {
                        let _ = tx.send(AppMessage::OperationProgress {
                            op_id,
                            current: 0,
                            total: *file_count as u64,
                        });
                    }
                    OrganizeProgress::ReadingDisc { index, total, .. } => {
                        let _ = tx.send(AppMessage::OperationProgress {
                            op_id,
                            current: *index as u64,
                            total: *total as u64,
                        });
                    }
                    _ => {}
                }
                ctx.request_repaint();
            };

            let plan = match plan_organize(
                &folder_path,
                registered.analyzer.as_ref(),
                &options,
                &progress_callback,
            ) {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(AppMessage::OrganizeComplete {
                        folder_name,
                        jobs_executed: 0,
                        files_moved: 0,
                        unmatched: 0,
                        errors: vec![format!("Failed to plan: {}", e)],
                    });
                    let _ = tx.send(AppMessage::OperationComplete { op_id });
                    ctx.request_repaint();
                    return;
                }
            };

            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = tx.send(AppMessage::OperationComplete { op_id });
                ctx.request_repaint();
                return;
            }

            // Send plan for preview
            let _ = tx.send(AppMessage::OrganizePlanReady {
                folder_name: folder_name.clone(),
                plan,
            });
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
    let description = format!("Moving disc files in {}", folder_name);
    let ctx = ctx.clone();
    let scope = Some(folder_name.clone());

    spawn_background_op(
        app,
        description,
        OperationKind::Other,
        scope,
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let mut jobs_executed = 0usize;
            let mut files_moved = 0usize;
            let mut errors: Vec<String> = Vec::new();
            let unmatched = plan.unmatched.len();

            let total_jobs = plan.jobs.len();
            for (i, job) in plan.jobs.iter().enumerate() {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }

                let _ = tx.send(AppMessage::OperationProgress {
                    op_id,
                    current: i as u64,
                    total: total_jobs as u64,
                });

                let result = execute_organize_job(job);
                if result.folder_created {
                    jobs_executed += 1;
                }
                files_moved += result.files_moved;
                errors.extend(result.errors);
            }

            let _ = tx.send(AppMessage::OrganizeComplete {
                folder_name,
                jobs_executed,
                files_moved,
                unmatched,
                errors,
            });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}
