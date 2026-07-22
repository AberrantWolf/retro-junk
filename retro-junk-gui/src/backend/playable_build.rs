use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, BackgroundOperation, OperationKind, ProgressDisplay};

pub fn start(
    app: &mut RetroJunkApp,
    dump_id: String,
    format: retro_junk_archive::RepresentationFormat,
    title: String,
    allow_unverified: bool,
    retain_intermediate: bool,
    ctx: &egui::Context,
) {
    let Some(profile) = app.settings.library.active_profile().cloned() else {
        app.push_error(
            "Build playable copy",
            "No active collection profile".to_owned(),
        );
        return;
    };
    let Some(db_path) = app.db_path.clone() else {
        app.push_error(
            "Build playable copy",
            "Catalog database is unavailable".to_owned(),
        );
        return;
    };
    let op_id = crate::state::next_operation_id();
    let cancel = Arc::new(AtomicBool::new(false));
    app.operations.push(BackgroundOperation::new(
        op_id,
        format!("Making {title} playable"),
        Arc::clone(&cancel),
        OperationKind::Other,
        title,
        ProgressDisplay::Count,
    ));
    let sender = app.message_tx.clone();
    let chdman = PathBuf::from(app.settings.general.chdman_path.trim());
    let request = retro_junk_lib::playable_build::PlayableBuildRequest {
        archive_root: profile.archive_root.clone(),
        playable_root: profile.playable_root.clone(),
        workspace_root: profile.workspace_root.clone(),
        dump_id,
        format,
        chdman_path: chdman,
        allow_unverified,
        retain_intermediate,
    };
    let handle = std::thread::spawn(move || {
        let progress_sender = sender.clone();
        let result = retro_junk_lib::playable_build::build_playable(
            &request,
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
        )
        .and_then(|outcome| {
            let snapshot =
                retro_junk_archive::scan_archive(&request.archive_root).map_err(|error| {
                    retro_junk_lib::playable_build::PlayableBuildError::Message(error.to_string())
                })?;
            let mut connection = retro_junk_db::open_database(&db_path).map_err(|error| {
                retro_junk_lib::playable_build::PlayableBuildError::Message(error.to_string())
            })?;
            retro_junk_db::reconcile_archive_snapshot(
                &mut connection,
                &snapshot,
                &request.playable_root,
                &request.workspace_root,
            )
            .map_err(|error| {
                retro_junk_lib::playable_build::PlayableBuildError::Message(error.to_string())
            })?;
            Ok(outcome.output)
        })
        .map_err(|error| error.to_string());
        let _ = sender.send(AppMessage::PlayableBuildComplete { op_id, result });
    });
    app.op_threads.insert(op_id, handle);
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}
