//! Thin dispatch to `retro_junk_backend::ops::archive`. Scheduling and
//! message delivery only — the operations themselves live in the backend.

use retro_junk_backend::ops::OpCtx;

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

pub(crate) fn start_archive_operation(
    app: &mut RetroJunkApp,
    profile: &retro_junk_archive::CollectionProfile,
    verify: bool,
) {
    let profile = profile.clone();
    let db_path = app.db_path.clone();
    crate::backend::worker::spawn_background_op(
        app,
        if verify {
            "Verifying archive"
        } else {
            "Refreshing archive index"
        }
        .to_owned(),
        OperationKind::Hash,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress = crate::backend::worker::forward_phases(op_id, sender.clone());
            let result = retro_junk_backend::ops::archive::refresh_archive(
                &profile,
                db_path.as_deref(),
                verify,
                &OpCtx::new(&cancel, &progress),
            );
            crate::backend::worker::deliver_result(&sender, result, |result| {
                AppMessage::ArchiveOperationComplete { result }
            })
        },
    );
}

pub(crate) fn start_catalog_identification_operation(
    app: &mut RetroJunkApp,
    profile: &retro_junk_archive::CollectionProfile,
) {
    let Some(db_path) = app.db_path.clone() else {
        app.push_error(
            "Identify archived carriers",
            "Catalog database is unavailable",
        );
        return;
    };
    let profile = profile.clone();
    crate::backend::worker::spawn_background_op(
        app,
        "Identifying archived carriers".to_owned(),
        OperationKind::Hash,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress = crate::backend::worker::forward_phases(op_id, sender.clone());
            let result = retro_junk_backend::ops::archive::identify_carriers(
                &profile,
                &db_path,
                &OpCtx::new(&cancel, &progress),
            );
            crate::backend::worker::deliver_result(&sender, result, |result| {
                AppMessage::ArchiveOperationComplete { result }
            })
        },
    );
}

pub(crate) fn start_add_release_file(
    app: &mut RetroJunkApp,
    profile: &retro_junk_archive::CollectionProfile,
    request: retro_junk_backend::ops::archive::AddReleaseFileRequest,
) {
    let Some(db_path) = app.db_path.clone() else {
        app.push_error("Release artwork", "Catalog database is unavailable");
        return;
    };
    let profile = profile.clone();
    crate::backend::worker::spawn_background_op(
        app,
        "Archiving release artwork".to_owned(),
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |_op_id, cancel, sender| {
            let added = retro_junk_backend::ops::archive::add_release_file(
                &profile, &db_path, &request, &cancel,
            )?;
            if added {
                let _ = sender.send(AppMessage::ArchiveAssetsChanged);
            }
            Ok(())
        },
    );
}

pub(crate) fn start_add_physical_copy_file(
    app: &mut RetroJunkApp,
    profile: &retro_junk_archive::CollectionProfile,
    request: retro_junk_backend::ops::archive::AddPhysicalCopyFileRequest,
) {
    let Some(db_path) = app.db_path.clone() else {
        app.push_error("Physical-copy file", "Catalog database is unavailable");
        return;
    };
    let profile = profile.clone();
    crate::backend::worker::spawn_background_op(
        app,
        "Archiving physical-copy file".to_owned(),
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |_op_id, cancel, sender| {
            retro_junk_backend::ops::archive::add_physical_copy_file(
                &profile, &db_path, &request, &cancel,
            )?;
            let _ = sender.send(AppMessage::ArchiveAssetsChanged);
            Ok(())
        },
    );
}

pub(crate) fn start_update_collection_details(
    app: &mut RetroJunkApp,
    profile: &retro_junk_archive::CollectionProfile,
    request: retro_junk_backend::ops::archive::UpdateCollectionDetailsRequest,
) {
    let Some(db_path) = app.db_path.clone() else {
        app.push_error("Collection details", "Catalog database is unavailable");
        return;
    };
    let profile = profile.clone();
    crate::backend::worker::spawn_background_op(
        app,
        "Saving collection details".to_owned(),
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress = crate::backend::worker::forward_phases(op_id, sender.clone());
            let result = retro_junk_backend::ops::archive::update_collection_details(
                &profile,
                &db_path,
                &request,
                &OpCtx::new(&cancel, &progress),
            );
            crate::backend::worker::deliver_result(&sender, result, |result| {
                AppMessage::ArchiveOperationComplete { result }
            })
        },
    );
}
