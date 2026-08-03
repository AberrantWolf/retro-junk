//! Convergence commands: run derived actions, converge a scope, and read the
//! backlog — the same executor path no matter which frontend asked.
//!
//! Every function here goes through the shared claim → archive-lock →
//! shared-implementation machinery in [`crate::executor`] and
//! [`crate::worker`]; this module adds no behaviour of its own beyond turning
//! outcomes into the summary strings frontends show.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use retro_junk_db::convergence::{
    ActionKind, BlockedReason, ConvergenceSummary, ProposedAction, Scope,
};
use retro_junk_db::work::WorkError;

use super::OpCtx;
use crate::executor::{ExecContext, ReconcileMode};
use crate::policy::AutomationPolicy;

/// The backlog for one scope: per-kind counts plus every open error and
/// blocked action, grouped by the archive release each belongs to, loaded
/// together so a refresh is one background pass rather than several.
#[derive(Default)]
pub struct Backlog {
    pub summary: ConvergenceSummary,
    pub errors: BTreeMap<String, Vec<(ActionKind, WorkError)>>,
    pub blocked: BTreeMap<String, Vec<(ActionKind, BlockedReason)>>,
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

    /// Actions derived for one archive release that cannot run unattended
    /// right now, with why — the worker skips these before the executor ever
    /// sees them, so this is the only place their reason is visible.
    #[must_use]
    pub fn release_blocked(&self, archive_release_id: &str) -> &[(ActionKind, BlockedReason)] {
        self.blocked
            .get(archive_release_id)
            .map_or(&[][..], Vec::as_slice)
    }
}

/// Load the backlog for `scope`: the per-kind summary, the open errors, and
/// the blocked actions, in one database pass.
///
/// Derivation is pure SQL over the projection, but the projection lives on
/// whatever filesystem the catalog database does, so callers should not run
/// this on a render thread.
pub fn load_backlog(db_path: &Path, scope: &Scope) -> Result<Backlog, String> {
    let expected = AutomationPolicy::load().scrape_selection();
    let connection = retro_junk_db::open_database(db_path).map_err(|error| error.to_string())?;
    let summary = retro_junk_db::convergence::summarize_convergence(&connection, scope, &expected)
        .map_err(|error| error.to_string())?;
    let errors = retro_junk_db::convergence::errors_by_release(&connection)
        .map_err(|error| error.to_string())?;
    let blocked = retro_junk_db::convergence::blocked_by_release(&connection, scope, &expected)
        .map_err(|error| error.to_string())?;
    Ok(Backlog {
        summary,
        errors,
        blocked,
    })
}

/// Run one derived action through the shared executor, turning every
/// non-completed outcome into the sentence a frontend shows. On success the
/// returned path is the primary output the action produced, when it has one.
pub fn run_action(
    exec: &ExecContext,
    action: &ProposedAction,
    ctx: &OpCtx,
) -> Result<Option<PathBuf>, String> {
    match crate::executor::execute_action(exec, action, ctx.progress, ctx.cancel) {
        Ok(crate::ActionOutcome::Completed { mut outputs }) => Ok(outputs.pop()),
        Ok(crate::ActionOutcome::ClaimHeld(held)) => Err(format!(
            "{} is already being handled by {} (since {})",
            action.label, held.owner, held.since
        )),
        Ok(crate::ActionOutcome::ArchiveBusy) => Err(format!(
            "the archive is busy; {} will be retried",
            action.label
        )),
        Ok(crate::ActionOutcome::Blocked(reason)) => Err(reason),
        Ok(crate::ActionOutcome::Cancelled) => Err(format!("{} was cancelled", action.label)),
        Err(error) => Err(error.to_string()),
    }
}

/// Run every currently derivable action in `scope`, stage by stage.
///
/// This is `retro-junk sync` with a frontend as the caller: same `run_once`,
/// same `RunMode::Explicit` (an explicit request is consent, so policy gates
/// and error backoff do not apply), same executor underneath. Reconciling
/// once at the end rather than per action keeps a large run to one archive
/// scan.
pub fn run_scope(exec: ExecContext, scope: &Scope, ctx: &OpCtx) -> Result<String, String> {
    let exec = ExecContext {
        reconcile: ReconcileMode::AtBatchEnd,
        ..exec
    };
    let policy = AutomationPolicy::load();
    crate::run_once(
        &exec,
        &policy,
        scope,
        crate::RunMode::Explicit,
        crate::ProjectionPass::Always,
        None,
        None,
        ctx.progress,
        ctx.cancel,
    )
    .map(|stats| {
        format!(
            "Converged: {} completed, {} failed, {} blocked, {} busy",
            stats.completed, stats.failed, stats.blocked, stats.skipped_busy
        )
    })
    .map_err(|error| error.to_string())
}

/// Re-run one action kind for one archive release.
///
/// Derivation, not construction: asking `run_once` for a release-scoped run
/// restricted to one kind means the caller never has to know that integrity
/// verification targets a dump while a build targets the release.
pub fn run_release_kind(
    exec: &ExecContext,
    archive_release_id: String,
    kind: ActionKind,
    label: &str,
    ctx: &OpCtx,
) -> Result<String, String> {
    let policy = AutomationPolicy::load();
    let scope = Scope::Release { archive_release_id };
    crate::run_once(
        exec,
        &policy,
        &scope,
        crate::RunMode::Explicit,
        crate::ProjectionPass::Always,
        Some(&[kind]),
        None,
        ctx.progress,
        ctx.cancel,
    )
    .map(|stats| {
        // `completed == 0` is not one outcome — it also covers a run that
        // failed or that never reached the executor at all, and those need
        // to say so, not read as "nothing was pending" beside a dot that is
        // visibly red or gray.
        if stats.completed > 0 {
            format!("{} finished for {label}", kind_label(kind))
        } else if stats.failed > 0 {
            format!(
                "{} failed for {label} — see the evidence dot for why",
                kind_label(kind)
            )
        } else if stats.blocked > 0 {
            format!(
                "{} is blocked for {label} — see the evidence dot for why",
                kind_label(kind)
            )
        } else if stats.skipped_busy > 0 {
            format!("{label} is already being worked on")
        } else {
            format!("Nothing to do: {label} is already current")
        }
    })
    .map_err(|error| error.to_string())
}

/// Force a release's playable representation into a good state, whether or
/// not convergence currently believes anything is owed.
///
/// Unlike [`run_release_kind`], this does not derive from the projection: a
/// release whose evidence points at bytes that moved, were regenerated, or
/// were adopted against a file that turned out not to be there reads as
/// satisfied and never reaches derivation at all. This routes straight to
/// [`crate::force_rebuild_playable`], which tries adoption before forcing a
/// build — every frontend's "force" means the same thing.
pub fn force_rebuild(
    exec: &ExecContext,
    archive_release_id: &str,
    label: &str,
    ctx: &OpCtx,
) -> Result<String, String> {
    match crate::force_rebuild_playable(exec, archive_release_id, ctx.progress, ctx.cancel) {
        Ok(crate::ForceRebuildOutcome::Adopted(found_label)) => {
            Ok(format!("{found_label} was already there — adopted it"))
        }
        Ok(crate::ForceRebuildOutcome::Built(_)) => Ok(format!("Rebuilt {label}")),
        Err(error) => Err(error.to_string()),
    }
}

/// Human label for a backlog chip.
#[must_use]
pub fn kind_label(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::VerifyIntegrity => "integrity",
        ActionKind::VerifyCatalog => "catalog",
        ActionKind::AuditRedumper => "raw audit",
        ActionKind::AdoptPlayable => "moved",
        ActionKind::BuildPlayable => "playable",
        ActionKind::Scrape => "scrape",
        ActionKind::ProjectAssets => "artwork",
        ActionKind::SyncGamelist => "gamelist",
        _ => "other",
    }
}
