//! Thin dispatch to `retro_junk_backend::ops::convergence`. Scheduling,
//! progress forwarding, and message delivery only — running actions,
//! converging a scope, and reading the backlog all live in the backend, on
//! the same claim → archive-lock → shared-implementation path the CLI and
//! daemon take. A build started from a row badge, from the context menu,
//! from `retro-junk sync`, or by the daemon is literally the same work.

use std::path::PathBuf;

use retro_junk_backend::ops::OpCtx;
use retro_junk_db::convergence::{ActionKind, ProposedAction, Scope};

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

pub use retro_junk_backend::ops::convergence::{Backlog, kind_label};

/// Build the executor context from the app's settings and active profile.
pub fn exec_context(app: &RetroJunkApp) -> Result<retro_junk_backend::ExecContext, String> {
    let profile = app
        .settings
        .library
        .active_profile()
        .cloned()
        .ok_or_else(|| "No active collection profile".to_owned())?;
    let db_path = app
        .db_path
        .clone()
        .ok_or_else(|| "Catalog database is unavailable".to_owned())?;
    Ok(retro_junk_backend::ExecContext {
        roots: app.settings.frontend_roots(&profile.playable_root),
        profile,
        db_path,
        tools: retro_junk_backend::ToolPaths {
            chdman: PathBuf::from(app.settings.general.chdman_path.trim()),
            redumper: PathBuf::new(),
            dolphin_tool: PathBuf::new(),
        },
        scrape: retro_junk_backend::AutomationPolicy::load().scrape_settings(),
        analyzers: app.context.clone(),
        owner: retro_junk_backend::ExecContext::owner_string("gui"),
        // A human clicked: wait for the lock rather than fail fast, and keep
        // the projection current so the row updates as soon as it finishes.
        lock: retro_junk_backend::LockEtiquette::InteractiveWait,
        reconcile: retro_junk_backend::ReconcileMode::PerAction,
    })
}

/// Run one derived action in the background, reporting through the activity
/// bar and completing on the shared archive-refresh path.
pub fn run_action(
    app: &mut RetroJunkApp,
    action: ProposedAction,
    description: String,
    ctx: &egui::Context,
) {
    let exec = match exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Archive action", error);
            return;
        }
    };
    crate::backend::worker::spawn_background_op(
        app,
        description,
        OperationKind::Other,
        // One scope string for every archive action keeps the existing
        // "more archive work queued" deferral in `PlayableBuildComplete`
        // working for badge-driven runs too.
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress = crate::backend::worker::forward_phases(op_id, sender.clone());
            let result = retro_junk_backend::ops::convergence::run_action(
                &exec,
                &action,
                &OpCtx::new(&cancel, &progress),
            );
            let _ = sender.send(AppMessage::PlayableBuildComplete { op_id, result });
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Run every currently derivable action in `scope`, stage by stage — a click
/// is consent, so this is `retro-junk sync` with the GUI as the caller.
pub fn run_scope(app: &mut RetroJunkApp, scope: Scope, ctx: &egui::Context) {
    let exec = match exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Convergence", error);
            return;
        }
    };
    crate::backend::worker::spawn_background_op(
        app,
        "Converging the archive".to_owned(),
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress = crate::backend::worker::forward_phases(op_id, sender.clone());
            let result = retro_junk_backend::ops::convergence::run_scope(
                exec,
                &scope,
                &OpCtx::new(&cancel, &progress),
            );
            let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Re-run one action kind for one archive release — the badge popover's
/// "run this again".
pub fn run_release_kind(
    app: &mut RetroJunkApp,
    archive_release_id: String,
    kind: ActionKind,
    label: String,
    ctx: &egui::Context,
) {
    let exec = match exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Archive action", error);
            return;
        }
    };
    crate::backend::worker::spawn_background_op(
        app,
        format!("{} for {label}", kind_label(kind)),
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress = crate::backend::worker::forward_phases(op_id, sender.clone());
            let result = retro_junk_backend::ops::convergence::run_release_kind(
                &exec,
                archive_release_id,
                kind,
                &label,
                &OpCtx::new(&cancel, &progress),
            );
            let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Force a release's playable representation into a good state — the
/// archive context menu's "Force Rebuild Playable". Adoption is tried before
/// forcing a build; the CLI's `rebuild-playable` goes through the identical
/// backend function, so "force" means the same thing from either surface.
pub fn force_rebuild_playable(
    app: &mut RetroJunkApp,
    archive_release_id: String,
    label: String,
    ctx: &egui::Context,
) {
    let exec = match exec_context(app) {
        Ok(exec) => exec,
        Err(error) => {
            app.push_error("Force rebuild", error);
            return;
        }
    };
    crate::backend::worker::spawn_background_op(
        app,
        format!("Force rebuilding {label}"),
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress = crate::backend::worker::forward_phases(op_id, sender.clone());
            let result = retro_junk_backend::ops::convergence::force_rebuild(
                &exec,
                &archive_release_id,
                &label,
                &OpCtx::new(&cancel, &progress),
            );
            let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Keep the loaded backlog pointed at what the user is looking at.
///
/// The scope narrows to the selected console's archive platform when the
/// page carries archived releases to name it; otherwise it stays at the
/// whole profile. Deriving profile-wide and filtering afterwards would
/// re-derive on every console change for no benefit — `summarize_convergence`
/// takes the scope directly.
pub fn ensure_backlog_loaded(app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(profile_id) = app
        .settings
        .library
        .active_profile()
        .map(|profile| profile.profile_id.to_string())
    else {
        app.ui_state.backlog_scope = None;
        return;
    };
    let platform_id = app
        .browser
        .active_page
        .as_ref()
        .and_then(|page| page.archived_releases.first())
        .map(|release| release.summary.platform_id.clone());
    let scope = match platform_id {
        Some(platform_id) => Scope::Platform {
            profile_id,
            platform_id,
        },
        None => Scope::Profile(profile_id),
    };
    if app.ui_state.backlog_scope.as_ref() == Some(&scope) {
        return;
    }
    app.ui_state.backlog_scope = Some(scope.clone());
    load_backlog(app, scope, ctx);
}

/// Load the backlog for `scope` off the render thread.
pub fn load_backlog(app: &mut RetroJunkApp, scope: Scope, ctx: &egui::Context) {
    let Some(db_path) = app.db_path.clone() else {
        return;
    };
    if app.ui_state.backlog_loading {
        return;
    }
    app.ui_state.backlog_loading = true;
    let sender = app.message_tx.clone();
    let repaint = ctx.clone();
    std::thread::spawn(move || {
        let result = retro_junk_backend::ops::convergence::load_backlog(&db_path, &scope);
        let _ = sender.send(AppMessage::BacklogReady { scope, result });
        repaint.request_repaint();
    });
}
