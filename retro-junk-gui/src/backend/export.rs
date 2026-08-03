//! Thin dispatch to `retro_junk_backend::ops::export`. Scheduling and
//! message delivery only — the export itself lives in the backend.

use retro_junk_backend::ops::OpCtx;

use crate::app::RetroJunkApp;
use crate::backend::worker::spawn_background_op;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// Generate a gamelist.xml (ES-DE format) for a console on a background thread.
pub fn generate_gamelist(app: &mut RetroJunkApp, console_idx: usize, ctx: &egui::Context) {
    let console = &app.browser.consoles[console_idx];
    let folder_name = console.folder_name.clone();
    let Some(console_id) = console.id else {
        app.push_error(
            "Export",
            "This console has not been committed to the library yet",
        );
        return;
    };

    let Some(root_path) = app.root_path.clone() else {
        return;
    };

    let Some(db_path) = app.db_path.clone() else {
        app.push_error("Export", "The library database is unavailable");
        return;
    };

    let metadata_dir_setting = app.settings.general.metadata_dir.clone();
    let media_dir_setting = app.settings.general.assets_dir.clone();
    let ctx = ctx.clone();
    let description = format!("Exporting gamelist.xml for {folder_name}");

    spawn_background_op(
        app,
        description,
        OperationKind::Other,
        folder_name.clone(),
        ProgressDisplay::Count,
        move |op_id, cancel, tx| {
            let progress = crate::backend::worker::forward_phases(op_id, tx.clone());
            let result = retro_junk_backend::ops::export::generate_gamelist(
                &root_path,
                &folder_name,
                &db_path,
                console_id,
                &metadata_dir_setting,
                &media_dir_setting,
                &OpCtx::new(&cancel, &progress),
            );
            let _ = tx.send(AppMessage::ExportComplete {
                folder_name,
                result,
            });
            let _ = tx.send(AppMessage::OperationComplete { op_id });
            ctx.request_repaint();
        },
    );
}
