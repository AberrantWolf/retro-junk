//! Thin dispatch for the stale-name repair.

use retro_junk_backend::ops::OpCtx;

use crate::app::RetroJunkApp;
use crate::backend::worker::{forward_phases, spawn_background_op};
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// Rename the misnamed playables of one archive release, or of the whole
/// active profile when no release is named.
pub(crate) fn start(app: &mut RetroJunkApp, only_release: Option<String>, _ctx: &egui::Context) {
    let Some(profile) = app.settings.library.active_profile().cloned() else {
        app.push_error("Rename playables", "No active collection profile");
        return;
    };
    let Some(db_path) = app.db_path.clone() else {
        app.push_error("Rename playables", "Catalog database is unavailable");
        return;
    };
    let media_root = app
        .settings
        .frontend_roots(&profile.playable_root)
        .media_root;
    let expected_assets = app.ui_state.expected_assets.clone();
    let description = if only_release.is_some() {
        "Renaming this release's playables to their catalog names".to_owned()
    } else {
        "Renaming playables to their catalog names".to_owned()
    };
    spawn_background_op(
        app,
        description,
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = forward_phases(op_id, tx.clone());
            let result = retro_junk_backend::ops::rename_playable::rename_stale_playables(
                &profile,
                &db_path,
                Some(&media_root),
                &expected_assets,
                only_release.as_deref(),
                &OpCtx::new(&cancel, &progress),
            )
            .map(|report| {
                let mut lines = vec![format!("Renamed {} playable(s)", report.renamed.len())];
                lines.extend(report.failures);
                lines.join("\n")
            });
            let _ = tx.send(AppMessage::ArchiveOperationComplete { op_id, result });
        },
    );
}
