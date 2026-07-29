//! Stage-ordered convergence runs.
//!
//! One `run_once` drains everything currently derivable: a verification
//! pass first (so unverified dumps become buildable without a human), a
//! re-derive, the build pass, another re-derive, then the projection pass.
//! Verify-then-build is a single unattended run — the automation-first
//! reversal of the old "blocked: unverified" wall.

use std::sync::atomic::AtomicBool;

use retro_junk_db::convergence::{ActionKind, Scope, derive_convergence};

use crate::executor::{ActionOutcome, ExecContext, WorkError, execute_action};
use crate::policy::AutomationPolicy;

/// Counters for one run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RunStats {
    pub completed: usize,
    pub failed: usize,
    /// Skipped because another owner held the claim or the archive lock.
    pub skipped_busy: usize,
    /// Skipped by automation policy (daemon mode only).
    pub skipped_policy: usize,
    /// Skipped due to a recent recorded error (daemon backoff).
    pub skipped_backoff: usize,
    pub blocked: usize,
    pub cancelled: usize,
}

impl RunStats {
    fn absorb(&mut self, outcome: &ActionOutcome) {
        match outcome {
            ActionOutcome::Completed => self.completed += 1,
            ActionOutcome::ClaimHeld(_) | ActionOutcome::ArchiveBusy => self.skipped_busy += 1,
            ActionOutcome::Blocked(_) => self.blocked += 1,
            ActionOutcome::Cancelled => self.cancelled += 1,
        }
    }
}

/// The verification, build, and projection stages, in order.
const STAGES: &[&[ActionKind]] = &[
    &[
        ActionKind::VerifyIntegrity,
        ActionKind::VerifyCatalog,
        ActionKind::AuditRedumper,
    ],
    &[ActionKind::BuildPlayable],
    &[ActionKind::ProjectAssets, ActionKind::SyncGamelist],
];

/// Behavior differences between explicit runs and the daemon loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// `sync` / GUI: run whatever was asked, ignore policy gates and error
    /// backoff — an explicit invocation is consent.
    Explicit,
    /// The daemon: honor `auto_verify`/`auto_build` and back off targets
    /// that errored recently.
    Daemon,
}

/// When the projection stage (assets + gamelists) runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionPass {
    /// Run projections every time (explicit runs, deep rescans).
    Always,
    /// Run projections only when this run completed earlier work — the
    /// steady-state daemon gate that keeps idle ticks from re-statting the
    /// frontend tree and rewriting gamelists.
    OnlyAfterMutation,
}

/// Run every currently derivable action in scope through the executor,
/// stage by stage with a re-derivation between stages.
///
/// `only` restricts to specific kinds (CLI `--only`); `limit` bounds the
/// number of executed actions.
// One loop, one gate sequence per action; the stage protocol reads as a
// single unit.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn run_once(
    ctx: &ExecContext,
    policy: &AutomationPolicy,
    scope: &Scope,
    mode: RunMode,
    projections: ProjectionPass,
    only: Option<&[ActionKind]>,
    limit: Option<usize>,
    progress: &dyn Fn(&str, u64, u64),
    cancelled: &AtomicBool,
) -> Result<RunStats, WorkError> {
    let mut stats = RunStats::default();
    let mut executed = 0_usize;
    let mut conn = retro_junk_db::open_database(&ctx.db_path)
        .map_err(|error| WorkError::Message(error.to_string()))?;
    let mut stage_mutated = false;
    let mut any_completed = false;
    for stage in STAGES {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let is_projection_stage = stage.contains(&ActionKind::ProjectAssets);
        if is_projection_stage
            && projections == ProjectionPass::OnlyAfterMutation
            && !any_completed
        {
            // Nothing changed this run: existing projections are current.
            break;
        }
        // Re-derive at each stage boundary so verification results unlock
        // builds and fresh builds unlock projections within one run. The
        // derivation reads the projection, so a stage that mutated the
        // archive owes a reconcile before the next derivation sees it.
        if stage_mutated && ctx.reconcile == crate::executor::ReconcileMode::AtBatchEnd {
            reconcile(ctx, &mut conn, progress)?;
            stage_mutated = false;
        }
        let actions = derive_convergence(&conn, scope)?;
        for action in actions
            .into_iter()
            .filter(|action| stage.contains(&action.kind))
        {
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                stats.cancelled += 1;
                break;
            }
            if let Some(kinds) = only
                && !kinds.contains(&action.kind)
            {
                continue;
            }
            if limit.is_some_and(|limit| executed >= limit) {
                break;
            }
            if mode == RunMode::Daemon {
                let allowed = match action.kind {
                    ActionKind::VerifyIntegrity
                    | ActionKind::VerifyCatalog
                    | ActionKind::AuditRedumper => policy.auto_verify,
                    ActionKind::BuildPlayable
                    | ActionKind::ProjectAssets
                    | ActionKind::SyncGamelist => policy.auto_build,
                    _ => false,
                };
                if !allowed {
                    stats.skipped_policy += 1;
                    continue;
                }
                if retro_junk_db::work::has_recent_error(
                    &conn,
                    action.kind.as_str(),
                    action.target.kind(),
                    action.target.id(),
                    retro_junk_db::work::ERROR_BACKOFF_HOURS,
                )? {
                    stats.skipped_backoff += 1;
                    continue;
                }
            }
            if action.blocked.is_some() {
                stats.blocked += 1;
                continue;
            }
            executed += 1;
            let outcome = execute_action(ctx, &action, progress, cancelled)?;
            if matches!(outcome, ActionOutcome::Completed) {
                stage_mutated = true;
                any_completed = true;
            } else if let ActionOutcome::Blocked(reason) = &outcome {
                log::warn!("{}: {}", action.label, reason);
                stats.failed += 1;
                continue;
            }
            stats.absorb(&outcome);
        }
    }
    // The final stage's mutations still owe one projection refresh.
    if stage_mutated && ctx.reconcile == crate::executor::ReconcileMode::AtBatchEnd {
        reconcile(ctx, &mut conn, progress)?;
    }
    Ok(stats)
}

fn reconcile(
    ctx: &ExecContext,
    conn: &mut retro_junk_db::Connection,
    progress: &dyn Fn(&str, u64, u64),
) -> Result<(), WorkError> {
    progress("Refreshing the archive projection", 0, 0);
    let snapshot = retro_junk_archive::scan_archive(&ctx.profile.archive_root)
        .map_err(|error| WorkError::Message(error.to_string()))?;
    retro_junk_db::reconcile_archive_snapshot(
        conn,
        &snapshot,
        &ctx.profile.playable_root,
        &ctx.profile.workspace_root,
    )
    .map_err(|error| WorkError::Message(error.to_string()))?;
    Ok(())
}
