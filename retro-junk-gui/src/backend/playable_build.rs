use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// Playable builds have a long parallelizable preparation phase, but currently
/// also publish archive evidence, frontend metadata, and a database projection.
/// Keep whole jobs FIFO-ish and exclusive until those commit phases are split
/// from conversion.
static PLAYABLE_BUILD_QUEUE: OnceLock<Mutex<()>> = OnceLock::new();

pub fn start(
    app: &mut RetroJunkApp,
    release: retro_junk_db::ArchivedPlayableGap,
    format: retro_junk_archive::RepresentationFormat,
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
    let chdman = PathBuf::from(app.settings.general.chdman_path.trim());
    let roots = retro_junk_lib::archive_ops::FrontendRoots::from_settings(
        &profile.playable_root,
        &app.settings.general.assets_dir,
        &app.settings.general.metadata_dir,
    );
    crate::backend::worker::spawn_background_op(
        app,
        description,
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let result = (|| -> Result<Option<PathBuf>, String> {
                let _queue_turn = PLAYABLE_BUILD_QUEUE
                    .get_or_init(|| Mutex::new(()))
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("Playable build was cancelled while queued".to_owned());
                }
                send_progress(
                    &sender,
                    op_id,
                    &format!("Starting playable build for {release_label}"),
                    0,
                    0,
                );
                let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
                    .map_err(|error| error.to_string())?;
                let connection =
                    retro_junk_db::open_database(&db_path).map_err(|error| error.to_string())?;
                let request = retro_junk_lib::archive_ops::ReleaseBuildRequest {
                    gap: &release,
                    archive_root: profile.archive_root.clone(),
                    workspace_root: profile.processing_workspace_root(),
                    roots,
                    format,
                    playable_platform_id,
                    chdman_path: chdman,
                    redumper_path: PathBuf::new(),
                    dolphin_tool_path: PathBuf::new(),
                    options: std::collections::BTreeMap::new(),
                    project_assets: true,
                    update_gamelist: true,
                };
                let progress_sender = sender.clone();
                let outcome = retro_junk_lib::archive_ops::build_release_playable(
                    &request,
                    &connection,
                    &|description, current, total| {
                        send_progress(&progress_sender, op_id, description, current, total);
                    },
                    &cancel,
                )
                .map_err(|error| error.to_string())?;
                drop(connection);
                let mut connection =
                    retro_junk_db::open_database(&db_path).map_err(|error| error.to_string())?;
                retro_junk_db::reconcile_archive_snapshot(
                    &mut connection,
                    &outcome.snapshot,
                    &profile.playable_root,
                    &profile.workspace_root,
                )
                .map_err(|error| error.to_string())?;
                Ok(outcome.playlist.or_else(|| outcome.built.last().cloned()))
            })();
            let _ = sender.send(AppMessage::PlayableBuildComplete { op_id, result });
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

fn send_progress(
    sender: &crate::state::AppMessageSender,
    op_id: u64,
    description: &str,
    current: u64,
    total: u64,
) {
    let display = if total == 0 {
        ProgressDisplay::Count
    } else {
        ProgressDisplay::Bytes
    };
    let _ = sender.send(AppMessage::OperationPhase {
        op_id,
        description: description.to_owned(),
        display,
        current,
        total,
    });
}
