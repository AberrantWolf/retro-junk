//! The GUI's one entry point to the shared convergence machinery.
//!
//! Everything the GUI knows about pending work goes through here:
//! [`exec_context`] builds the executor context from settings once instead
//! of at every call site, [`run_action`] dispatches any derived
//! [`ProposedAction`] through `retro_junk_work::execute_action` — the same
//! claim → archive-lock → shared-implementation path the CLI and daemon
//! take — and [`load_backlog`] reads the backlog summary and open-error set
//! off the render thread.
//!
//! The executor is the only writer; this module adds no behaviour of its
//! own, so a build started from a row badge, from the context menu, from
//! `retro-junk sync`, or by the daemon is literally the same work.

use std::collections::BTreeMap;
use std::path::PathBuf;

use retro_junk_db::convergence::{ActionKind, ConvergenceSummary, ProposedAction, Scope};
use retro_junk_db::work::WorkError;

use crate::app::RetroJunkApp;
use crate::state::{AppMessage, OperationKind, ProgressDisplay};

/// The backlog for one scope: per-kind counts plus every open error grouped
/// by the archive release it belongs to, loaded together so a refresh is one
/// background pass rather than two.
#[derive(Default)]
pub struct Backlog {
    pub summary: ConvergenceSummary,
    pub errors: BTreeMap<String, Vec<(ActionKind, WorkError)>>,
}

impl Backlog {
    /// Open errors recorded against one archive release, verification
    /// failures on its dumps included.
    #[must_use]
    pub fn release_errors(&self, archive_release_id: &str) -> &[(ActionKind, WorkError)] {
        self.errors
            .get(archive_release_id)
            .map_or(&[][..], Vec::as_slice)
    }
}

/// Build the executor context from the app's settings and active profile.
pub fn exec_context(app: &RetroJunkApp) -> Result<retro_junk_work::ExecContext, String> {
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
    Ok(retro_junk_work::ExecContext {
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
        // A human clicked: wait for the lock rather than fail fast, and keep
        // the projection current so the row updates as soon as it finishes.
        lock: retro_junk_work::LockEtiquette::InteractiveWait,
        reconcile: retro_junk_work::ReconcileMode::PerAction,
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
            let progress_sender = sender.clone();
            let outcome = retro_junk_work::execute_action(
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
            let result = match outcome {
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
                    Err(format!("{} was cancelled", action.label))
                }
                Err(error) => Err(error.to_string()),
            };
            let _ = sender.send(AppMessage::PlayableBuildComplete { op_id, result });
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Run every currently derivable action in `scope`, stage by stage.
///
/// This is `retro-junk sync` with the GUI as the caller: same `run_once`,
/// same `RunMode::Explicit` (a click is consent, so policy gates and error
/// backoff do not apply), same executor underneath. Reconciling once at the
/// end rather than per action keeps a large run to one archive scan.
pub fn run_scope(app: &mut RetroJunkApp, scope: Scope, ctx: &egui::Context) {
    let exec = match exec_context(app) {
        Ok(exec) => retro_junk_work::ExecContext {
            reconcile: retro_junk_work::ReconcileMode::AtBatchEnd,
            ..exec
        },
        Err(error) => {
            app.push_error("Convergence", error);
            return;
        }
    };
    let policy = retro_junk_work::AutomationPolicy::load();
    crate::backend::worker::spawn_background_op(
        app,
        "Converging the archive".to_owned(),
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress_sender = sender.clone();
            let result = retro_junk_work::run_once(
                &exec,
                &policy,
                &scope,
                retro_junk_work::RunMode::Explicit,
                retro_junk_work::ProjectionPass::Always,
                None,
                None,
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
            let result = result
                .map(|stats| {
                    format!(
                        "Converged: {} completed, {} failed, {} blocked, {} busy",
                        stats.completed, stats.failed, stats.blocked, stats.skipped_busy
                    )
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Re-run one action kind for one archive release — the badge popover's
/// "run this again".
///
/// Derivation, not construction: asking `run_once` for a release-scoped run
/// restricted to one kind means the GUI never has to know that integrity
/// verification targets a dump while a build targets the release.
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
    let policy = retro_junk_work::AutomationPolicy::load();
    let scope = Scope::Release { archive_release_id };
    crate::backend::worker::spawn_background_op(
        app,
        format!("{} for {label}", kind_label(kind)),
        OperationKind::Other,
        "archive".to_owned(),
        ProgressDisplay::Count,
        move |op_id, cancel, sender| {
            let progress_sender = sender.clone();
            let result = retro_junk_work::run_once(
                &exec,
                &policy,
                &scope,
                retro_junk_work::RunMode::Explicit,
                retro_junk_work::ProjectionPass::Always,
                Some(&[kind]),
                None,
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
            let result = result
                .map(|stats| {
                    if stats.completed > 0 {
                        format!("{} finished for {label}", kind_label(kind))
                    } else {
                        format!("Nothing to do: {label} is already current")
                    }
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(AppMessage::ArchiveOperationComplete { op_id, result });
        },
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(20));
}

/// Load the backlog for `scope` off the render thread.
///
/// Derivation is pure SQL over the projection, but the projection lives on
/// whatever filesystem the catalog database does, so it never runs inline.
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
        let result = retro_junk_db::open_database(&db_path)
            .map_err(|error| error.to_string())
            .and_then(|connection| {
                let summary =
                    retro_junk_db::convergence::summarize_convergence(&connection, &scope)
                        .map_err(|error| error.to_string())?;
                let errors = retro_junk_db::convergence::errors_by_release(&connection)
                    .map_err(|error| error.to_string())?;
                Ok(Backlog { summary, errors })
            });
        let _ = sender.send(AppMessage::BacklogReady { result });
        repaint.request_repaint();
    });
}

/// Human label for a backlog chip.
#[must_use]
pub fn kind_label(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::VerifyIntegrity => "integrity",
        ActionKind::VerifyCatalog => "catalog",
        ActionKind::AuditRedumper => "raw audit",
        ActionKind::BuildPlayable => "playable",
        ActionKind::ProjectAssets => "artwork",
        ActionKind::SyncGamelist => "gamelist",
        _ => "other",
    }
}
