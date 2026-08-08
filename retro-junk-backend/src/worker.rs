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

use crate::executor::{ActionOutcome, ExecContext, WorkError, execute_actions};
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
            ActionOutcome::Failed(_) => self.failed += 1,
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
    // Two stages, not one: a gamelist entry names the artwork files it found in
    // the media tree, so every asset has to be projected before any gamelist is
    // written. The stage boundary is what guarantees that ordering, and it only
    // applies between stages.
    &[ActionKind::ProjectAssets],
    &[ActionKind::SyncGamelist],
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
    let mut exhausted = false;
    for stage in STAGES {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let is_projection_stage =
            stage.contains(&ActionKind::ProjectAssets) || stage.contains(&ActionKind::SyncGamelist);
        if is_projection_stage && projections == ProjectionPass::OnlyAfterMutation && !any_completed
        {
            // Nothing changed this run: existing projections are current.
            continue;
        }
        // Re-derive at each stage boundary so verification results unlock
        // builds and fresh builds unlock projections within one run. The
        // derivation reads the projection, so a stage that mutated the
        // archive owes a reconcile before the next derivation sees it.
        if stage_mutated && ctx.reconcile == crate::executor::ReconcileMode::AtBatchEnd {
            reconcile(ctx, &mut conn, progress)?;
            stage_mutated = false;
        }
        // Everything this stage could do, narrowed to what will actually run
        // before anything is dispatched. Gating afterwards made the queue
        // counter describe a different set of work than the one executing —
        // `--only gamelist` reported "[3/600]" — and made the daemon pay a
        // claim and a lock cycle to discover it was not allowed to proceed.
        let mut queue = Vec::new();
        for action in derive_convergence(&conn, scope, &ctx.scrape.expected_assets)? {
            if !stage.contains(&action.kind) {
                continue;
            }
            if only.is_some_and(|kinds| !kinds.contains(&action.kind)) {
                continue;
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
            if let Some(limit) = limit
                && executed + queue.len() >= limit
            {
                exhausted = true;
                break;
            }
            queue.push(action);
        }
        executed += queue.len();
        // A limit reached exactly at the end of a stage is still reached: without
        // this, every remaining stage paid for a full derivation only to find its
        // first action was one too many.
        exhausted |= limit.is_some_and(|limit| executed >= limit);

        // One batch, so the connection, the whole-archive lock, and the walk
        // behind the shared scan are paid for once for the whole stage rather
        // than once per item.
        let outcomes = execute_actions(ctx, &queue, progress, cancelled)?;
        for (action, outcome) in queue.iter().zip(outcomes) {
            match &outcome {
                ActionOutcome::Completed { .. } => {
                    any_completed = true;
                    // Only work that changed the archive owes a reconcile.
                    // Keying this on "did anything complete" made a converged
                    // run pay for a whole extra archive walk to write down that
                    // the frontend tree, which is not part of the archive, had
                    // been refreshed.
                    stage_mutated |= crate::executor::changes_the_archive(action.kind);
                }
                ActionOutcome::Failed(reason) => log::warn!("{}: {reason}", action.label),
                _ => {}
            }
            stats.absorb(&outcome);
        }
        if exhausted {
            break;
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
    // Fresh by definition — a stage just changed the archive — and kept, so
    // the next stage's actions read this walk instead of paying for another.
    let snapshot = ctx.rescan_archive()?;
    retro_junk_db::reconcile_archive_snapshot(
        conn,
        &snapshot,
        &ctx.profile.playable_root,
        &ctx.profile.workspace_root,
    )
    .map_err(|error| WorkError::Message(error.to_string()))?;
    Ok(())
}
