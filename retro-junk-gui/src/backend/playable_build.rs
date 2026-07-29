use std::path::PathBuf;

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// Run one release's build through the shared convergence executor — the
/// same claim → archive-lock → build → reconcile path the CLI and daemon
/// use. Cross-process claims replace the old process-local build queue: a
/// release the daemon is already building surfaces as a friendly error
/// instead of a duplicate build, and the whole-archive lock keeps builds
/// serialized.
pub fn start(
    app: &mut RetroJunkApp,
    release: retro_junk_db::ArchivedPlayableGap,
    format: &retro_junk_archive::RepresentationFormat,
    playable_platform_id: String,
    ctx: &egui::Context,
) {
    let Some(profile) = app.settings.library.active_profile().cloned() else {
        app.push_error("Archive action", "No active collection profile".to_owned());
        return;
    };
    let Some(db_path) = app.db_path.clone() else {
        app.push_error(
            "Archive action",
            "Catalog database is unavailable".to_owned(),
        );
        return;
    };
    let release_label = if release.region.trim().is_empty() {
        release.title.clone()
    } else {
        format!("{} ({})", release.title, release.region)
    };
    let description = if release.needs_playable {
        format!("Queued playable build for {release_label}")
    } else {
        format!("Queued archive verification for {release_label}")
    };
    let profile_id = profile.profile_id.to_string();
    let exec = retro_junk_work::ExecContext {
        roots: retro_junk_lib::archive_ops::FrontendRoots::from_settings(
            &profile.playable_root,
            &app.settings.general.assets_dir,
            &app.settings.general.metadata_dir,
        ),
        profile,
        db_path,
        tools: retro_junk_work::ToolPaths {
            chdman: PathBuf::from(app.settings.general.chdman_path.trim()),
            redumper: PathBuf::new(),
            dolphin_tool: PathBuf::new(),
        },
        analyzers: app.context.clone(),
        owner: retro_junk_work::ExecContext::owner_string("gui"),
        lock: retro_junk_work::LockEtiquette::InteractiveWait,
        reconcile: retro_junk_work::ReconcileMode::PerAction,
    };
    // The user's explicit format choice overrides the stored policy for
    // this run; the executor reads the format from the gap.
    let mut gap = release;
    gap.preferred_format = Some(format.key().to_owned());
    let action = retro_junk_db::convergence::ProposedAction {
        kind: retro_junk_db::convergence::ActionKind::BuildPlayable,
        target: retro_junk_db::convergence::WorkTarget::Release(gap.archive_release_id.clone()),
        profile_id,
        platform_id: String::new(),
        playable_platform_id,
        label: release_label,
        blocked: None,
        build: Some(gap),
    };
    crate::backend::worker::spawn_background_op(
        app,
        description,
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress_sender = sender.clone();
            let result = retro_junk_work::execute_action(
                &exec,
                &action,
                &|description, current, total| {
                    let display = if total == 0 {
                        ProgressDisplay::Count
                    } else {
                        ProgressDisplay::Bytes
                    };
                    let _ = progress_sender.send(AppMessage::OperationPhase {
                        op_id,
                        description: description.to_owned(),
                        display,
                        current,
                        total,
                    });
                },
                &cancel,
            );
            let result = match result {
                Ok(retro_junk_work::ActionOutcome::Completed { mut outputs }) => Ok(outputs.pop()),
                Ok(retro_junk_work::ActionOutcome::ClaimHeld(held)) => Err(format!(
                    "{} is already being handled by {} (since {})",
                    action.label, held.owner, held.since
                )),
                Ok(retro_junk_work::ActionOutcome::ArchiveBusy) => Err(format!(
                    "the archive is busy; {} will be retried",
                    action.label
                )),
                Ok(retro_junk_work::ActionOutcome::Blocked(reason)) => Err(reason),
                Ok(retro_junk_work::ActionOutcome::Cancelled) => {
                    Err("Playable build was cancelled".to_owned())
                }
                Err(error) => Err(error.to_string()),
            };
            let _ = sender.send(AppMessage::PlayableBuildComplete { op_id, result });
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}
