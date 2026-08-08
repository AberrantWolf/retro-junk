//! Thin dispatch to `retro_junk_backend::ops::inbox`. Scheduling, message
//! delivery, and reads of UI selection state only — loading the inbox and
//! deciding suggestions live in the backend, on the same paths
//! `retro-junk suggestions apply|dismiss|ignore` takes, so a decision taken
//! here and one taken at the terminal are the same decision.

use retro_junk_backend::ops::OpCtx;
use retro_junk_db::work::IncomingPackage;

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

pub use retro_junk_backend::ops::inbox::{InboxContents, InboxItem, InboxSort, target_path};

/// Load the inbox off the render thread.
pub fn load(app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    if app.ui_state.inbox_loading {
        return;
    }
    app.ui_state.inbox_loading = true;
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    let playable_root = app
        .settings
        .library
        .active_profile()
        .map(|profile| profile.playable_root.clone());
    let collection_root = app.collection_root();
    std::thread::spawn(move || {
        let result = retro_junk_backend::ops::inbox::load_inbox(
            &db_path,
            playable_root,
            collection_root.as_deref(),
        );
        let _ = sender.send(AppMessage::InboxReady { result });
        repaint.request_repaint();
    });
}

/// Execute a suggestion through the shared dispatch, answering its question if
/// it asked one.
pub fn apply(
    app: &mut RetroJunkApp,
    id: i64,
    choice: Option<String>,
    label: &str,
    ctx: &egui::Context,
) {
    let exec = match crate::backend::convergence::exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Apply suggestion", error);
            return;
        }
    };
    crate::backend::worker::spawn_background_op(
        app,
        format!("Applying {label}"),
        OperationKind::ArchiveImport,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |_op_id, cancel, sender| {
            let result =
                retro_junk_backend::apply_suggestion_choice(&exec, id, choice.as_deref(), &cancel)
                    .map_err(|error| error.to_string());
            let _ = sender.send(AppMessage::InboxChanged);
            crate::backend::worker::deliver_result(&sender, result, |result| {
                AppMessage::ArchiveOperationComplete { result }
            })
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Apply many suggestions, one after another, refreshing the inbox as each
/// one lands.
pub fn apply_many(app: &mut RetroJunkApp, ids: Vec<i64>, ctx: &egui::Context) {
    let exec = match crate::backend::convergence::exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Apply suggestions", error);
            return;
        }
    };
    crate::backend::worker::spawn_background_op(
        app,
        format!("Applying {} suggestion(s)", ids.len()),
        OperationKind::ArchiveImport,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress = crate::backend::worker::forward_phases(op_id, sender.clone());
            let on_applied = || {
                let _ = sender.send(AppMessage::InboxChanged);
            };
            let result = retro_junk_backend::ops::inbox::apply_many(
                &exec,
                &ids,
                &OpCtx::new(&cancel, &progress),
                on_applied,
            );
            let _ = sender.send(AppMessage::InboxChanged);
            crate::backend::worker::deliver_result(&sender, result, |result| {
                AppMessage::ArchiveOperationComplete { result }
            })
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Resolve suggestions without executing them.
///
/// The ids that were actually closed come back on [`AppMessage::InboxDismissed`]
/// so the view can offer to put exactly those back.
pub fn dismiss(app: &mut RetroJunkApp, ids: Vec<i64>, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        match retro_junk_backend::ops::inbox::dismiss(&db_path, &ids) {
            Ok(dismissed) => {
                let _ = sender.send(AppMessage::InboxDismissed { ids: dismissed });
            }
            Err(error) => log::warn!("could not dismiss suggestions: {error}"),
        }
        let _ = sender.send(AppMessage::InboxChanged);
        repaint.request_repaint();
    });
}

/// Put dismissed suggestions back in front of the user.
pub fn reopen(app: &mut RetroJunkApp, ids: Vec<i64>, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        match retro_junk_backend::ops::inbox::reopen(&db_path, &ids) {
            Ok(outcome) => {
                if !outcome.is_complete() {
                    log::info!(
                        "reopened {}; {} superseded by newer reviews, {} no longer exist",
                        outcome.reopened.len(),
                        outcome.superseded.len(),
                        outcome.missing.len()
                    );
                }
            }
            Err(error) => log::warn!("could not reopen suggestions: {error}"),
        }
        let _ = sender.send(AppMessage::InboxChanged);
        repaint.request_repaint();
    });
}

/// Record a durable ignore rule and close the reviews it covers.
pub fn ignore(app: &mut RetroJunkApp, pattern: String, note: String, ctx: &egui::Context) {
    let exec = match crate::backend::convergence::exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Ignore files", error);
            return;
        }
    };
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        match retro_junk_backend::ignore_playables(&exec, &pattern, &note) {
            Ok(outcome) => log::info!(
                "ignoring '{}' from now on; closed {} open review(s)",
                outcome.rule.pattern,
                outcome.dismissed
            ),
            Err(error) => log::warn!("could not ignore '{pattern}': {error}"),
        }
        let _ = sender.send(AppMessage::InboxChanged);
        repaint.request_repaint();
    });
}

/// Revoke an ignore rule; the next sweep files those files again.
pub fn unignore(app: &mut RetroJunkApp, pattern: String, ctx: &egui::Context) {
    let exec = match crate::backend::convergence::exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Revoke ignore rule", error);
            return;
        }
    };
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        if let Err(error) = retro_junk_backend::unignore_playables(&exec, &pattern) {
            log::warn!("could not revoke '{pattern}': {error}");
        }
        let _ = sender.send(AppMessage::InboxChanged);
        repaint.request_repaint();
    });
}

/// Stop tracking a failed incoming package.
///
/// The file itself is untouched, so a watcher that still sees it in the drop
/// folder will observe it again — this clears the row, it does not decide
/// anything about the package.
pub fn forget_package(app: &mut RetroJunkApp, path: String, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        if let Err(error) = retro_junk_backend::ops::inbox::forget_package(&db_path, &path) {
            log::warn!("could not forget package {path}: {error}");
        }
        let _ = sender.send(AppMessage::InboxChanged);
        repaint.request_repaint();
    });
}

/// Send a failed package back through pre-processing and run that pass now.
///
/// The daemon would pick a re-queued package up on its next tick, but the
/// GUI should not require a daemon to answer a button press: the backend runs
/// the same `process_pending_incoming` the daemon runs, under the same policy.
pub fn retry_package(app: &mut RetroJunkApp, package: &IncomingPackage, ctx: &egui::Context) {
    let exec = match crate::backend::convergence::exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Retry package", error);
            return;
        }
    };
    let path = package.path.clone();
    let label = std::path::Path::new(&path)
        .file_name()
        .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned());
    crate::backend::worker::spawn_background_op(
        app,
        format!("Re-processing {label}"),
        OperationKind::ArchiveImport,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress = crate::backend::worker::forward_phases(op_id, sender.clone());
            let result = retro_junk_backend::ops::inbox::retry_package(
                &exec,
                &path,
                &OpCtx::new(&cancel, &progress),
            );
            let _ = sender.send(AppMessage::InboxChanged);
            crate::backend::worker::deliver_result(&sender, result, |result| {
                AppMessage::ArchiveOperationComplete { result }
            })
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}
