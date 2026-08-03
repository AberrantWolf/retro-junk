//! Stage-ordered convergence runs.
//!
//! One `run_once` drains everything currently derivable: a verification
//! pass first (so unverified dumps become buildable without a human), a
//! re-derive, the build pass, another re-derive, then the projection pass.
//! Verify-then-build is a single unattended run — the automation-first
//! reversal of the old "blocked: unverified" wall.

use std::sync::atomic::AtomicBool;

use retro_junk_db::convergence::{ActionKind, Scope, derive_convergence};
use retro_junk_io::{PhaseProgressFn, ProgressUnit};

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
            ActionOutcome::Completed { .. } => self.completed += 1,
            ActionOutcome::ClaimHeld(_) | ActionOutcome::ArchiveBusy => self.skipped_busy += 1,
            ActionOutcome::Blocked(_) => self.blocked += 1,
            ActionOutcome::Cancelled => self.cancelled += 1,
        }
    }
}

/// The verification, build, scrape, and projection stages, in order.
///
/// Scraping is its own stage rather than part of the projection one: the
/// projection stage is gated on "did this run change anything", and artwork a
/// release has never had is pending work regardless of whether a build just
/// happened. Running it before projections also means freshly fetched art
/// reaches the frontend in the same pass, via the stage-boundary re-derive.
const STAGES: &[&[ActionKind]] = &[
    &[
        ActionKind::VerifyIntegrity,
        ActionKind::VerifyCatalog,
        ActionKind::AuditRedumper,
    ],
    // Its own stage before builds, and the stage boundary reconciles: a
    // playable that merely moved is re-adopted and stops looking like a gap,
    // so the build stage never rebuilds a file the library already holds.
    &[ActionKind::AdoptPlayable],
    &[ActionKind::BuildPlayable],
    &[ActionKind::Scrape],
    &[ActionKind::ProjectAssets, ActionKind::SyncGamelist],
];

/// Behavior differences between explicit runs and the daemon loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// `sync` / GUI: run whatever was asked, ignore policy gates and error
    /// backoff — an explicit invocation is consent.
    Explicit,
    /// The daemon: honor [`daemon_may_run`] and back off targets that errored
    /// recently.
    Daemon,
}

/// Whether the unattended daemon may run this kind under `policy`.
///
/// Each gate asks permission to *produce* something the user might not want
/// appearing on its own: `auto_verify` re-reads archived bytes, `auto_build`
/// writes derivatives into the playable library, `auto_scrape` spends a daily
/// external quota.
///
/// [`ActionKind::AdoptPlayable`] is ungated because it produces nothing. It
/// corrects the archive's record of where a file it already built now lives,
/// which is bookkeeping about the present, not new work. Gating it meant a
/// rename outside the archive orphaned the playable until someone intervened,
/// and the release advertised a gap that did not exist.
///
/// An unrecognized kind is refused: a new kind must state its own gate before
/// the daemon will run it unattended.
#[must_use]
pub fn daemon_may_run(kind: ActionKind, policy: &AutomationPolicy) -> bool {
    match kind {
        ActionKind::VerifyIntegrity | ActionKind::VerifyCatalog | ActionKind::AuditRedumper => {
            policy.auto_verify
        }
        ActionKind::AdoptPlayable => true,
        ActionKind::BuildPlayable | ActionKind::ProjectAssets | ActionKind::SyncGamelist => {
            policy.auto_build
        }
        ActionKind::Scrape => policy.auto_scrape,
        _ => false,
    }
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
    progress: &PhaseProgressFn<'_>,
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
        if is_projection_stage && projections == ProjectionPass::OnlyAfterMutation && !any_completed
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
        let actions = derive_convergence(&conn, scope, &ctx.scrape.expected_assets)?
            .into_iter()
            .filter(|action| stage.contains(&action.kind))
            .collect::<Vec<_>>();
        let queued = actions.len();
        for (position, action) in actions.into_iter().enumerate() {
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
                if !daemon_may_run(action.kind, policy) {
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
            // Say where in the queue this is. Each action reports progress for
            // its own single target — an audit says "dump 1 of 1" — so without
            // the queue position a run over forty discs looks exactly like one
            // disc being reworked over and over.
            let queue_position = format!("[{}/{queued}] ", position + 1);
            let placed = |phase: &str, unit: ProgressUnit, current: u64, total: u64| {
                progress(&format!("{queue_position}{phase}"), unit, current, total);
            };
            placed(&action.label, ProgressUnit::Items, 0, 0);
            let outcome = execute_action(ctx, &action, &placed, cancelled)?;
            if matches!(outcome, ActionOutcome::Completed { .. }) {
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
    progress: &PhaseProgressFn<'_>,
) -> Result<(), WorkError> {
    progress(
        "Refreshing the archive projection",
        ProgressUnit::Items,
        0,
        0,
    );
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
