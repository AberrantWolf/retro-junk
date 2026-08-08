//! The single execution path for convergence actions.
//!
//! GUI queue buttons, CLI `sync`, and the daemon all call
//! [`execute_actions`]: take the archive lock and the database connections
//! once, then per item claim → dispatch to the shared implementations in
//! `retro_junk_lib::archive_ops` → optional reconcile → release the claim.
//! [`execute_action`] is the same thing with one item in it, so there is one
//! protocol rather than a batch path and a single path that could drift.
//!
//! The executor adds coordination only — never logic; the orchestrations own
//! behavior, evidence, and idempotence.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use retro_junk_db::convergence::{ActionKind, ProposedAction, WorkTarget};
use retro_junk_db::work::{ClaimOutcome, HeldClaim};
use retro_junk_io::{PhaseProgressFn, ProgressUnit};
use retro_junk_lib::archive_ops::{
    ArchiveOpsError, FrontendRoots, IdentifyCarriersRequest, IdentifySelection,
    ReleaseBuildRequest, build_release_playable, identify_archived_carriers,
    verify_archive_integrity, verify_catalog_files,
};

/// External tool locations, resolved once by the caller (settings or flags).
#[derive(Debug, Clone, Default)]
pub struct ToolPaths {
    pub chdman: PathBuf,
    pub redumper: PathBuf,
    pub dolphin_tool: PathBuf,
}

/// What a scrape may do, resolved once by the caller from policy.
///
/// Same precedent as [`ToolPaths`]: an external resource the executor needs
/// but does not decide. Derivation needs `expected_assets` too, so it is the
/// one definition of "what artwork a release owes".
#[derive(Debug, Clone)]
pub struct ScrapeSettings {
    /// Asset types a release is expected to hold.
    pub expected_assets: retro_junk_frontend::AssetSelection,
    /// Refuse to publish a filename-tier match unattended. A filename match
    /// is a guess, and the archive is where guesses become durable.
    pub only_when_unambiguous: bool,
    /// Stop a run when the daily request budget falls to this many
    /// remaining. `0` disables the reserve.
    pub daily_request_reserve: u32,
}

impl Default for ScrapeSettings {
    fn default() -> Self {
        Self {
            expected_assets: retro_junk_frontend::AssetSelection::default(),
            only_when_unambiguous: true,
            daily_request_reserve: 0,
        }
    }
}

/// How to wait for the whole-archive lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockEtiquette {
    /// Interactive callers wait (cancel-aware) — a human clicked, the work
    /// happens as soon as the lock frees.
    InteractiveWait,
    /// The daemon never contends with a human: on `Busy` it skips and
    /// retries next tick.
    DaemonFailFast,
}

/// When the `SQLite` projection is refreshed after archive mutations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileMode {
    /// Immediately after each action (GUI single actions — keeps the
    /// incremental-refresh path current).
    PerAction,
    /// The caller reconciles once after its batch (worker runs — one
    /// archive scan per pass instead of per action).
    AtBatchEnd,
}

/// The archive as this run last saw it, walked at most once until something
/// changes it.
///
/// Walking the archive is the most expensive thing a short action does, and
/// most actions need it only to find the one release they are about. The
/// projection stage dispatches an action per release, so a library of three
/// hundred releases used to pay for three hundred complete walks of whatever
/// the archive is stored on — for a USB or network volume, almost the entire
/// run.
///
/// So the walk happens once and every action reads the same result. Anything
/// that changes the archive throws it away ([`ExecContext::archive_changed`]),
/// because a scan that outlived a mutation would hand the next action a tree
/// that no longer exists.
///
/// Cloning shares the same cache rather than starting an empty one, so a
/// context copied for a nested operation still reads the walk its parent
/// already paid for. A copy that genuinely wants a fresh view assigns
/// `ArchiveScan::default()`.
#[derive(Default, Clone)]
pub struct ArchiveScan(Arc<Mutex<Option<Arc<ScannedArchive>>>>);

/// One walk of the archive, plus the lookup that makes finding a release in it
/// constant rather than a search.
///
/// Kept together on purpose: the positions are indices into this snapshot's own
/// release list, so they cannot outlive it or be applied to another one. Before
/// this, every projection action scanned the whole release list comparing
/// stringified ids — a full pass over a library did that once per release, so
/// the comparisons grew with the square of the collection.
pub struct ScannedArchive {
    pub snapshot: Arc<retro_junk_archive::ArchiveIndexSnapshot>,
    positions: std::collections::HashMap<String, usize>,
}

impl ScannedArchive {
    fn index(snapshot: Arc<retro_junk_archive::ArchiveIndexSnapshot>) -> Self {
        let positions = snapshot
            .releases
            .iter()
            .enumerate()
            .map(|(position, release)| (release.manifest.archive_release_id.to_string(), position))
            .collect();
        Self {
            snapshot,
            positions,
        }
    }

    /// The release with this archive id, if the archive still holds it.
    #[must_use]
    pub fn release(&self, archive_release_id: &str) -> Option<&retro_junk_archive::IndexedRelease> {
        self.positions
            .get(archive_release_id)
            .and_then(|position| self.snapshot.releases.get(*position))
    }
}

/// Everything an execution needs besides the action itself.
#[derive(Clone)]
pub struct ExecContext {
    pub profile: retro_junk_archive::CollectionProfile,
    pub db_path: PathBuf,
    pub tools: ToolPaths,
    pub scrape: ScrapeSettings,
    pub roots: FrontendRoots,
    pub analyzers: Arc<retro_junk_lib::AnalysisContext>,
    /// Claim owner identity, e.g. `"host:1234:daemon"`.
    pub owner: String,
    pub lock: LockEtiquette,
    pub reconcile: ReconcileMode,
    /// Shared scan for this run. Build it with `ArchiveScan::default()`; it
    /// fills itself the first time an action asks. See [`ArchiveScan`].
    pub archive: ArchiveScan,
}

impl ExecContext {
    /// Standard owner string for this process.
    #[must_use]
    pub fn owner_string(role: &str) -> String {
        let host = std::env::var("HOSTNAME")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "local".to_owned());
        format!("{host}:{}:{role}", std::process::id())
    }

    /// The archive, walking it only if this run has not already.
    ///
    /// Every caller gets the same snapshot, so asking twice is free and no
    /// action has to thread one down from its caller to avoid a second walk.
    pub fn archive(&self) -> Result<Arc<retro_junk_archive::ArchiveIndexSnapshot>, WorkError> {
        Ok(Arc::clone(&self.scanned_archive()?.snapshot))
    }

    /// The same walk, with the release lookup built alongside it.
    pub fn scanned_archive(&self) -> Result<Arc<ScannedArchive>, WorkError> {
        let mut held = self
            .archive
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(scanned) = held.as_ref() {
            return Ok(Arc::clone(scanned));
        }
        let snapshot = Arc::new(
            retro_junk_archive::scan_archive(&self.profile.archive_root).map_err(WorkError::msg)?,
        );
        let scanned = Arc::new(ScannedArchive::index(snapshot));
        *held = Some(Arc::clone(&scanned));
        Ok(scanned)
    }

    /// A scan taken now, whatever this run last saw, which then becomes the
    /// shared one.
    ///
    /// For refreshing the projection, which is the act of writing down what is
    /// on disk *at this moment*. That happens straight after the action that
    /// changed the archive and before the run has marked the old scan stale, so
    /// accepting a cached answer there would record the archive as it used to
    /// be. Handing the fresh scan back to the cache means the next action does
    /// not walk the tree again for it.
    pub fn rescan_archive(
        &self,
    ) -> Result<Arc<retro_junk_archive::ArchiveIndexSnapshot>, WorkError> {
        self.archive_changed();
        self.archive()
    }

    /// Forget the scan: the archive on disk is no longer what it described.
    pub fn archive_changed(&self) {
        *self
            .archive
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

/// Does running this leave the archive different from how a scan last found
/// it?
///
/// Only the two projection kinds do not: they write into the frontend's media
/// tree and its gamelists, which are downstream of the archive rather than
/// part of it. Everything else appends evidence, publishes a dump, or renames
/// an output.
///
/// Written as "everything except", so a kind added later is treated as
/// changing the archive until someone decides otherwise. The failure from
/// guessing the other way — an action reading a tree that has moved on — is
/// silent, and this one only costs a re-scan.
pub(crate) const fn changes_the_archive(kind: ActionKind) -> bool {
    !matches!(kind, ActionKind::ProjectAssets | ActionKind::SyncGamelist)
}

/// Whether a completed action still owes a whole-snapshot reconcile.
/// Scraping changes the archive, but its handler projects the exact release
/// file delta before it returns.
pub(crate) const fn requires_full_reconcile(kind: ActionKind) -> bool {
    changes_the_archive(kind) && !matches!(kind, ActionKind::Scrape)
}

/// What happened to one action.
#[derive(Debug)]
pub enum ActionOutcome {
    /// Work ran to completion; `outputs` lists any files a build produced
    /// (empty for verification/projection actions).
    Completed {
        outputs: Vec<PathBuf>,
    },
    /// Another owner holds a fresh claim — surfaced, not an error.
    ClaimHeld(HeldClaim),
    /// The archive lock was busy and etiquette said don't wait.
    ArchiveBusy,
    /// The action arrived blocked (policy/completeness); nothing ran.
    Blocked(String),
    /// The work ran and did not succeed. Distinct from [`Self::Blocked`],
    /// which never started: one variant meant both, so the same outcome was
    /// counted as blocked in one place and failed in another.
    Failed(String),
    Cancelled,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkError {
    #[error("{0}")]
    Message(String),
    /// Somebody asked for this to stop. Its own variant because classifying
    /// cancellation by comparing an error's text to a literal turned any
    /// rewording of that message into a silent reclassification as failure.
    #[error("operation cancelled")]
    Cancelled,
    #[error(transparent)]
    Db(#[from] retro_junk_db::operations::OperationError),
    #[error(transparent)]
    Library(#[from] retro_junk_db::library::LibraryError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl WorkError {
    pub(crate) fn msg(error: impl std::fmt::Display) -> Self {
        Self::Message(error.to_string())
    }
}

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Execute one derived action end-to-end.
///
/// The one-action form of [`execute_actions`]; everything a single action pays
/// for — a database connection, the whole-archive lock, the walk behind the
/// shared scan — it pays for alone.
pub fn execute_action(
    ctx: &ExecContext,
    action: &ProposedAction,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<ActionOutcome, WorkError> {
    let mut outcomes = execute_actions(ctx, std::slice::from_ref(action), progress, cancelled)?;
    Ok(outcomes.pop().unwrap_or(ActionOutcome::Cancelled))
}

/// Execute a batch of derived actions end-to-end, one outcome per input.
///
/// A queue of work is [1..N] things done once, not one thing done N times.
/// Everything the batch can share, it acquires once: the database connection,
/// the connection the claim heartbeat beats on, the whole-archive lock, and the
/// walk behind the shared scan. Only the claim is per action, because it is the
/// claim that says which *item* is being worked on and lets another process
/// pick up the ones this batch has not reached.
///
/// Holding the archive lock for the batch is not only fewer file operations: it
/// is what makes the shared scan sound. Taking and dropping the lock between
/// items leaves a gap in which another process can change the archive, and the
/// cached walk would then describe a tree that has moved on.
///
/// A failure or a held claim on one item never ends the batch — the failure
/// verdict is recorded on that target (`work_errors`), its claim released, and
/// the next item runs. An `Err` return means the coordination machinery itself
/// failed, not the work.
pub fn execute_actions(
    ctx: &ExecContext,
    actions: &[ProposedAction],
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<Vec<ActionOutcome>, WorkError> {
    if actions.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    // Its own connection because the claim beat happens inside a progress
    // callback, while the dispatch it is reporting on holds the other one
    // mutably.
    let heartbeat = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;

    let archive_lock = match acquire_for_batch(ctx, progress, cancelled)? {
        BatchLock::Held(lock) => lock,
        // The archive belongs to someone else for now, so none of this batch
        // can run; the caller retries or the daemon picks it up next tick.
        BatchLock::Busy => return Ok(repeated(actions.len(), || ActionOutcome::ArchiveBusy)),
        BatchLock::Cancelled => return Ok(repeated(actions.len(), || ActionOutcome::Cancelled)),
    };

    let mut outcomes = Vec::with_capacity(actions.len());
    for (position, action) in actions.iter().enumerate() {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            outcomes.push(ActionOutcome::Cancelled);
            continue;
        }
        if let Some(reason) = &action.blocked {
            outcomes.push(ActionOutcome::Blocked(reason.to_string()));
            continue;
        }
        // Say where in the queue this is. Each action reports progress for its
        // own single target — an audit says "dump 1 of 1" — so without the
        // queue position a run over forty discs looks exactly like one disc
        // being reworked over and over.
        let placed = |phase: &str, unit: ProgressUnit, current: u64, total: u64| {
            progress(
                &format!("[{}/{}] {phase}", position + 1, actions.len()),
                unit,
                current,
                total,
            );
        };
        placed(&action.label, ProgressUnit::Items, 0, 0);
        outcomes.push(claim_and_dispatch(
            ctx, action, &mut conn, &heartbeat, &placed, cancelled,
        )?);
    }
    drop(archive_lock);
    Ok(outcomes)
}

/// The same verdict for every item, for the cases where nothing in the batch
/// could run at all.
fn repeated(count: usize, outcome: impl Fn() -> ActionOutcome) -> Vec<ActionOutcome> {
    (0..count).map(|_| outcome()).collect()
}

/// What acquiring the batch's archive lock produced.
enum BatchLock {
    Held(retro_junk_archive::ArchiveLock),
    Busy,
    Cancelled,
}

/// Take the whole-archive lock for the batch, honoring the caller's etiquette.
///
/// Safe to hold across a batch because nothing a dispatch reaches takes it
/// again: the one shared implementation that would — publishing scraped media —
/// is told the executor already holds it (`crate::scrape`, and
/// `retro_junk_scraper::session`'s `acquire_lock`). The lock is not re-entrant,
/// so a new nested acquisition would fail fast in the daemon and spin forever
/// in an interactive run.
fn acquire_for_batch(
    ctx: &ExecContext,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<BatchLock, WorkError> {
    match ctx.lock {
        LockEtiquette::DaemonFailFast => {
            match retro_junk_archive::ArchiveLock::acquire(&ctx.profile.archive_root) {
                Ok(lock) => Ok(BatchLock::Held(lock)),
                // Yield to whoever holds it.
                Err(retro_junk_archive::ArchiveLockError::Busy(_)) => Ok(BatchLock::Busy),
                Err(error) => Err(WorkError::msg(error)),
            }
        }
        LockEtiquette::InteractiveWait => {
            progress("Waiting for the archive lock", ProgressUnit::Items, 0, 0);
            match retro_junk_archive::ArchiveLock::acquire_wait(
                &ctx.profile.archive_root,
                cancelled,
            ) {
                Ok(Some(lock)) => Ok(BatchLock::Held(lock)),
                Ok(None) => Ok(BatchLock::Cancelled),
                Err(error) => Err(WorkError::msg(error)),
            }
        }
    }
}

/// Claim one item, run it, and release the claim with a verdict — on every
/// path, including the ones that return early.
fn claim_and_dispatch(
    ctx: &ExecContext,
    action: &ProposedAction,
    conn: &mut retro_junk_db::Connection,
    heartbeat: &retro_junk_db::Connection,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<ActionOutcome, WorkError> {
    let kind = action.kind.as_str();
    let (target_kind, target_id) = (action.target.kind(), action.target.id());
    if !retro_junk_db::work::try_claim(conn, kind, target_kind, target_id, &ctx.owner)? {
        let held = retro_junk_db::work::held_claim(conn, kind, target_kind, target_id)?;
        return Ok(ActionOutcome::ClaimHeld(held.unwrap_or(HeldClaim {
            owner: "unknown".to_owned(),
            since: String::new(),
        })));
    }

    // Piggy-back the claim heartbeat on progress callbacks: one wrapper, no
    // per-handler timers.
    let last_beat = std::cell::Cell::new(Instant::now());
    let beating_progress = |phase: &str, unit: ProgressUnit, current: u64, total: u64| {
        if last_beat.get().elapsed() >= HEARTBEAT_INTERVAL {
            last_beat.set(Instant::now());
            let _ = retro_junk_db::work::refresh_claim(
                heartbeat,
                kind,
                target_kind,
                target_id,
                &ctx.owner,
            );
        }
        progress(phase, unit, current, total);
    };

    if changes_the_archive(action.kind) {
        retro_junk_archive::advance_projection_generation(&ctx.profile.archive_root)
            .map_err(WorkError::msg)?;
    }
    let result = dispatch(ctx, action, conn, &beating_progress, cancelled);
    // Whatever the outcome: a kind that writes into the archive may have got
    // partway before failing, so the shared scan is stale either way.
    if changes_the_archive(action.kind) {
        ctx.archive_changed();
    }

    let (outcome, verdict) = match result {
        Ok(outputs) => (ActionOutcome::Completed { outputs }, ClaimOutcome::Success),
        Err(WorkError::Cancelled) => (ActionOutcome::Cancelled, ClaimOutcome::Cancelled),
        Err(error) => {
            log::warn!("{}: {} failed: {error}", action.label, action.kind.as_str());
            let message = error.to_string();
            (
                ActionOutcome::Failed(message.clone()),
                ClaimOutcome::Failed { error: message },
            )
        }
    };
    retro_junk_db::work::release_claim(conn, kind, target_kind, target_id, &ctx.owner, &verdict)?;
    Ok(outcome)
}

/// What resolved a forced rebuild.
#[derive(Debug)]
pub enum ForceRebuildOutcome {
    /// A file already at the target location was proven to be this
    /// release's own derivative and adopted — nothing was rebuilt.
    Adopted(String),
    /// A fresh playable was actually built.
    Built(Vec<PathBuf>),
}

/// Force a release's playable representation into a good state, regardless
/// of whether convergence currently reads it as satisfied.
///
/// Adoption runs first, unconditionally: it is always safe (it only links an
/// existing file to evidence by matching content, never writes over
/// anything) and resolves both a moved output (evidence exists, wrong path)
/// and a never-built carrier whose file already sits at the canonical spot
/// (proven by matching the carrier's verified track digests). Only once
/// adoption has had its chance and the release still genuinely needs a
/// build does this force one via [`retro_junk_db::convergence::forced_build_action`]
/// — going straight to a forced build first, without trying adoption, is
/// exactly what makes a forced build collide with a file adoption would
/// have recognized and linked instead.
pub fn force_rebuild_playable(
    ctx: &ExecContext,
    archive_release_id: &str,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<ForceRebuildOutcome, WorkError> {
    let profile_id = ctx.profile.profile_id.to_string();
    let adopt_action = ProposedAction {
        kind: ActionKind::AdoptPlayable,
        target: WorkTarget::Release(archive_release_id.to_owned()),
        profile_id: profile_id.clone(),
        platform_id: String::new(),
        playable_platform_id: String::new(),
        label: archive_release_id.to_owned(),
        blocked: None,
        build: None,
    };
    // A release with nothing findable by content reports as a failure here
    // (the same message a plain "retry adopt" would show); that is not
    // fatal to forcing — it means the build step below still has to run.
    let _ = execute_action(ctx, &adopt_action, progress, cancelled);
    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(WorkError::Cancelled);
    }

    // The adopt dispatch only reconciles under `ReconcileMode::PerAction`; a
    // forced rebuild needs the release's real post-adoption state regardless
    // of the caller's reconcile mode, so it reconciles explicitly here.
    let snapshot = ctx.rescan_archive()?;
    let mut conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &ctx.profile.playable_root,
        &ctx.profile.workspace_root,
    )
    .map_err(WorkError::msg)?;

    let still_needs_playable =
        retro_junk_db::library::release_needs_playable(&conn, &profile_id, archive_release_id)?;
    let Some(action) = retro_junk_db::convergence::forced_build_action(&conn, archive_release_id)?
    else {
        return Err(WorkError::Message(format!(
            "no archived release {archive_release_id} to rebuild"
        )));
    };
    drop(conn);
    if !still_needs_playable {
        return Ok(ForceRebuildOutcome::Adopted(action.label));
    }
    if let Some(reason) = &action.blocked {
        return Err(WorkError::Message(format!("{}: {reason}", action.label)));
    }
    match execute_action(ctx, &action, progress, cancelled)? {
        ActionOutcome::Completed { outputs } => Ok(ForceRebuildOutcome::Built(outputs)),
        ActionOutcome::Blocked(message) | ActionOutcome::Failed(message) => {
            let hint = if message.contains("already exists") {
                " — a file is already at that path but adoption could not confirm it belongs to \
                 this release (its content doesn't match, or the carrier isn't catalog-verified \
                 yet); verify the carrier, or move the file aside, then try again"
            } else {
                ""
            };
            Err(WorkError::Message(format!(
                "{}: {message}{hint}",
                action.label
            )))
        }
        ActionOutcome::ClaimHeld(held) => Err(WorkError::Message(format!(
            "{} is already being handled by {} (since {})",
            action.label, held.owner, held.since
        ))),
        ActionOutcome::ArchiveBusy => Err(WorkError::Message(format!(
            "the archive is busy; retry {}",
            action.label
        ))),
        ActionOutcome::Cancelled => Err(WorkError::Cancelled),
    }
}

// The per-kind dispatch table is deliberately one function: it is the
// single map from derived action kinds to shared implementations.
#[allow(clippy::too_many_lines)]
fn dispatch(
    ctx: &ExecContext,
    action: &ProposedAction,
    conn: &mut retro_junk_db::Connection,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<Vec<PathBuf>, WorkError> {
    let ops_err = |error: ArchiveOpsError| match error {
        ArchiveOpsError::Cancelled => WorkError::Cancelled,
        other => WorkError::msg(other),
    };
    match action.kind {
        ActionKind::VerifyIntegrity => {
            let snapshot = ctx.archive()?;
            let dump_id = dump_target(&action.target)?;
            let report = verify_archive_integrity(&snapshot, Some(dump_id), progress, cancelled)
                .map_err(ops_err)?;
            if report.failed > 0 {
                let detail = report
                    .failures
                    .iter()
                    .map(|(_, reason)| reason.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(WorkError::Message(format!(
                    "integrity verification failed: {detail}"
                )));
            }
            reconcile_if_per_action(ctx, conn)?;
            Ok(Vec::new())
        }
        ActionKind::VerifyCatalog => {
            let snapshot = ctx.archive()?;
            let dump_id = dump_target(&action.target)?;
            let report = verify_catalog_files(
                &snapshot,
                conn,
                &ctx.analyzers,
                Some(dump_id),
                progress,
                cancelled,
            )
            .map_err(ops_err)?;
            if report.identified == 0 {
                return Err(WorkError::Message(if report.ambiguous > 0 {
                    "catalog match is ambiguous; review required".to_owned()
                } else {
                    "no complete catalog match".to_owned()
                }));
            }
            reconcile_if_per_action(ctx, conn)?;
            Ok(Vec::new())
        }
        ActionKind::AuditRedumper => {
            let snapshot = ctx.archive()?;
            let dump_id = dump_target(&action.target)?;
            let report = identify_archived_carriers(
                &IdentifyCarriersRequest {
                    snapshot: &snapshot,
                    selection: IdentifySelection::All,
                    only_dump: Some(dump_id),
                    redumper_path: &ctx.tools.redumper,
                    workspace_root: &ctx.profile.processing_workspace_root(),
                },
                conn,
                progress,
                cancelled,
            )
            .map_err(ops_err)?;
            if report.identified == 0 {
                return Err(WorkError::Message(match () {
                    () if report.failed > 0 => "raw master could not be reproduced".to_owned(),
                    () if report.ambiguous > 0 => {
                        "catalog match is ambiguous; review required".to_owned()
                    }
                    () => "no complete catalog match".to_owned(),
                }));
            }
            reconcile_if_per_action(ctx, conn)?;
            Ok(Vec::new())
        }
        ActionKind::AdoptPlayable => {
            let snapshot = ctx.archive()?;
            let release_id = release_target(&action.target)?;
            let adoption = retro_junk_lib::archive_ops::AdoptionRequest {
                snapshot: &snapshot,
                playable_root: &ctx.profile.playable_root,
                only_release: Some(release_id),
                dry_run: false,
            };
            let mut report =
                retro_junk_lib::archive_ops::adopt_moved_playables(&adoption, progress, cancelled)
                    .map_err(ops_err)?;
            // A file the pipeline never built cannot have moved, so the two
            // passes never compete for the same file: one searches for a
            // recorded output digest, the other proves a derivative from the
            // carrier's verified track set.
            let unbuilt = retro_junk_lib::archive_ops::adopt_unbuilt_playables(
                &adoption, conn, progress, cancelled,
            )
            .map_err(ops_err)?;
            report.orphaned += unbuilt.orphaned;
            report.adopted.extend(unbuilt.adopted);
            if !report.unresolved.is_empty() && report.adopted.is_empty() {
                // Nothing found by content: the bytes really are gone, so the
                // build stage owes this release a rebuild. Report it rather
                // than claiming success, but the message says what it is.
                return Err(WorkError::Message(format!(
                    "{} playable output(s) are missing and nothing in their system directories matches the recorded content",
                    report.unresolved.len()
                )));
            }
            reconcile_if_per_action(ctx, conn)?;
            Ok(Vec::new())
        }
        ActionKind::BuildPlayable => {
            let gap = action
                .build
                .as_ref()
                .ok_or_else(|| WorkError::Message("build action lacks its gap".to_owned()))?;
            let format: retro_junk_archive::RepresentationFormat = gap
                .preferred_format
                .as_deref()
                .unwrap_or_default()
                .parse()
                .map_err(WorkError::Message)?;
            let outcome = build_release_playable(
                &ReleaseBuildRequest {
                    gap,
                    archive_root: ctx.profile.archive_root.clone(),
                    workspace_root: ctx.profile.processing_workspace_root(),
                    roots: ctx.roots.clone(),
                    format,
                    playable_platform_id: action.playable_platform_id.clone(),
                    chdman_path: ctx.tools.chdman.clone(),
                    redumper_path: ctx.tools.redumper.clone(),
                    dolphin_tool_path: ctx.tools.dolphin_tool.clone(),
                    options: std::collections::BTreeMap::new(),
                    project_assets: true,
                    update_gamelist: true,
                },
                conn,
                progress,
                cancelled,
            )
            .map_err(ops_err)?;
            if ctx.reconcile == ReconcileMode::PerAction {
                retro_junk_db::reconcile_archive_snapshot(
                    conn,
                    &outcome.snapshot,
                    &ctx.profile.playable_root,
                    &ctx.profile.workspace_root,
                )
                .map_err(WorkError::msg)?;
            }
            let mut outputs = outcome.built;
            if let Some(playlist) = outcome.playlist {
                outputs.push(playlist);
            }
            Ok(outputs)
        }
        ActionKind::Scrape => {
            let report =
                crate::scrape::scrape_release_artwork(ctx, action, conn, progress, cancelled)?;
            if let Some(weak) = &report.needs_review {
                // Not a failure: the match exists but is too weak to make
                // durable unattended, so it becomes a reviewable card rather
                // than an error that would back the release off for hours.
                crate::suggestions::open_scrape_suggestion(conn, action, weak)?;
                progress(
                    "Filed for review: only a filename match",
                    ProgressUnit::Items,
                    1,
                    1,
                );
            }
            if !report.changed_releases.is_empty() {
                let releases = report
                    .changed_releases
                    .iter()
                    .map(|release_id| {
                        retro_junk_archive::scan_archive_release(
                            &ctx.profile.archive_root,
                            *release_id,
                        )
                        .map_err(WorkError::msg)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                retro_junk_db::reconcile_archive_supporting_files(
                    conn,
                    &ctx.profile.archive_root,
                    &releases,
                )
                .map_err(WorkError::msg)?;
            }
            Ok(Vec::new())
        }
        ActionKind::ProjectAssets => {
            let scanned = ctx.scanned_archive()?;
            let release_id = release_target(&action.target)?;
            let release = indexed_release(&scanned, release_id)?;
            // The folder the release's own file is in, which is the folder its
            // gamelist entry is written to — asking the naming rule instead put
            // an off-folder release's artwork somewhere its entry never
            // pointed.
            let directory = retro_junk_lib::playable_location::release_publish_directory(
                release,
                &ctx.roots.playable_root,
            );
            retro_junk_lib::archive_assets::project_release_assets(
                release,
                &ctx.roots.media_root.join(&directory),
                &retro_junk_lib::archive_assets::release_media_stems(release),
                cancelled,
            )
            .map_err(WorkError::msg)?;
            retro_junk_db::projection_state::record_projection(
                conn,
                retro_junk_db::projection_state::ProjectionOf::assets(release_id),
            )?;
            Ok(Vec::new())
        }
        ActionKind::SyncGamelist => {
            let scanned = ctx.scanned_archive()?;
            let (profile_id, directory) = console_target(&action.target)?;
            // Every release the archive holds, filtered inside to the ones that
            // publish into this folder — one file read, one file written, for
            // however many games are listed in it.
            let releases = scanned.snapshot.releases.iter().collect::<Vec<_>>();
            retro_junk_lib::archive_assets::sync_esde_gamelist_for_console(
                &releases,
                directory,
                &ctx.roots.playable_root,
                &ctx.roots.metadata_root,
                &ctx.roots.media_root,
            )
            .map_err(WorkError::msg)?;
            retro_junk_db::projection_state::record_projection(
                conn,
                retro_junk_db::projection_state::ProjectionOf::gamelist(profile_id, directory),
            )?;
            Ok(Vec::new())
        }
        _ => Err(WorkError::Message(format!(
            "no executor for action kind {kind}",
            kind = action.kind.as_str()
        ))),
    }
}

fn dump_target(target: &WorkTarget) -> Result<&str, WorkError> {
    match target {
        WorkTarget::Dump(id) => Ok(id),
        other => Err(WorkError::Message(format!(
            "expected a dump target, got {}",
            other.kind()
        ))),
    }
}

fn release_target(target: &WorkTarget) -> Result<&str, WorkError> {
    match target {
        WorkTarget::Release(id) => Ok(id),
        other => Err(WorkError::Message(format!(
            "expected a release target, got {}",
            other.kind()
        ))),
    }
}

fn console_target(target: &WorkTarget) -> Result<(&str, &str), WorkError> {
    target.console_parts().ok_or_else(|| {
        WorkError::Message(format!("expected a console target, got {}", target.kind()))
    })
}

fn indexed_release<'a>(
    scanned: &'a ScannedArchive,
    release_id: &str,
) -> Result<&'a retro_junk_archive::IndexedRelease, WorkError> {
    scanned.release(release_id).ok_or_else(|| {
        WorkError::Message(format!(
            "archive release {release_id} is no longer present in the archive"
        ))
    })
}

/// Scan + reconcile after evidence-appending actions when the caller wants
/// immediate projection freshness.
fn reconcile_if_per_action(
    ctx: &ExecContext,
    conn: &mut retro_junk_db::Connection,
) -> Result<(), WorkError> {
    if ctx.reconcile == ReconcileMode::PerAction {
        let snapshot = ctx.rescan_archive()?;
        retro_junk_db::reconcile_archive_snapshot(
            conn,
            &snapshot,
            &ctx.profile.playable_root,
            &ctx.profile.workspace_root,
        )
        .map_err(WorkError::msg)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/executor_tests.rs"]
mod executor_tests;
