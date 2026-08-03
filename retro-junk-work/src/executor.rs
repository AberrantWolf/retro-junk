//! The single execution path for convergence actions.
//!
//! GUI queue buttons, CLI `sync`, and the daemon all call
//! [`execute_action`]: claim → archive lock → dispatch to the shared
//! implementations in `retro_junk_lib::archive_ops` → optional reconcile →
//! release claim. The executor adds coordination only — never logic; the
//! orchestrations own behavior, evidence, and idempotence.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
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

/// Everything an execution needs besides the action itself.
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
    Cancelled,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkError {
    #[error("{0}")]
    Message(String),
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
/// The failure verdict is recorded on the target (`work_errors`) and the
/// claim released in every path; an `Err` return means the coordination
/// machinery itself failed, not the work.
// Claim → lock-etiquette → dispatch → release is one protocol; splitting it
// would scatter the release-on-every-path guarantee.
#[allow(clippy::too_many_lines)]
pub fn execute_action(
    ctx: &ExecContext,
    action: &ProposedAction,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<ActionOutcome, WorkError> {
    if let Some(reason) = &action.blocked {
        return Ok(ActionOutcome::Blocked(reason.to_string()));
    }
    let mut conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    let kind = action.kind.as_str();
    let (target_kind, target_id) = (action.target.kind(), action.target.id());
    if !retro_junk_db::work::try_claim(&conn, kind, target_kind, target_id, &ctx.owner)? {
        let held = retro_junk_db::work::held_claim(&conn, kind, target_kind, target_id)?;
        return Ok(ActionOutcome::ClaimHeld(held.unwrap_or(HeldClaim {
            owner: "unknown".to_owned(),
            since: String::new(),
        })));
    }

    // Whole-archive lock, held for this one action.
    let archive_lock = match ctx.lock {
        LockEtiquette::DaemonFailFast => {
            match retro_junk_archive::ArchiveLock::acquire(&ctx.profile.archive_root) {
                Ok(lock) => Some(lock),
                Err(retro_junk_archive::ArchiveLockError::Busy(_)) => {
                    // Yield to whoever holds it; drop our claim untouched.
                    retro_junk_db::work::release_claim(
                        &mut conn,
                        kind,
                        target_kind,
                        target_id,
                        &ctx.owner,
                        &ClaimOutcome::Cancelled,
                    )?;
                    return Ok(ActionOutcome::ArchiveBusy);
                }
                Err(error) => {
                    let outcome = ClaimOutcome::Failed {
                        error: error.to_string(),
                    };
                    retro_junk_db::work::release_claim(
                        &mut conn,
                        kind,
                        target_kind,
                        target_id,
                        &ctx.owner,
                        &outcome,
                    )?;
                    return Err(WorkError::msg(error));
                }
            }
        }
        LockEtiquette::InteractiveWait => {
            progress("Waiting for the archive lock", ProgressUnit::Items, 0, 0);
            match retro_junk_archive::ArchiveLock::acquire_wait(
                &ctx.profile.archive_root,
                cancelled,
            ) {
                Ok(Some(lock)) => Some(lock),
                Ok(None) => {
                    retro_junk_db::work::release_claim(
                        &mut conn,
                        kind,
                        target_kind,
                        target_id,
                        &ctx.owner,
                        &ClaimOutcome::Cancelled,
                    )?;
                    return Ok(ActionOutcome::Cancelled);
                }
                Err(error) => {
                    let outcome = ClaimOutcome::Failed {
                        error: error.to_string(),
                    };
                    retro_junk_db::work::release_claim(
                        &mut conn,
                        kind,
                        target_kind,
                        target_id,
                        &ctx.owner,
                        &outcome,
                    )?;
                    return Err(WorkError::msg(error));
                }
            }
        }
    };

    // Piggy-back the claim heartbeat on progress callbacks: one wrapper, no
    // per-handler timers. A dedicated connection avoids fighting the
    // dispatch borrow.
    let heartbeat_conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    let last_beat = std::cell::Cell::new(Instant::now());
    let owner = ctx.owner.clone();
    let (beat_kind, beat_target_kind, beat_target_id) = (
        kind.to_owned(),
        target_kind.to_owned(),
        target_id.to_owned(),
    );
    let beating_progress = move |phase: &str, unit: ProgressUnit, current: u64, total: u64| {
        if last_beat.get().elapsed() >= HEARTBEAT_INTERVAL {
            last_beat.set(Instant::now());
            let _ = retro_junk_db::work::refresh_claim(
                &heartbeat_conn,
                &beat_kind,
                &beat_target_kind,
                &beat_target_id,
                &owner,
            );
        }
        progress(phase, unit, current, total);
    };

    let result = dispatch(ctx, action, &mut conn, &beating_progress, cancelled);
    drop(archive_lock);

    let (outcome, verdict) = match result {
        Ok(outputs) => (ActionOutcome::Completed { outputs }, ClaimOutcome::Success),
        Err(WorkError::Message(message)) if message == "operation cancelled" => {
            (ActionOutcome::Cancelled, ClaimOutcome::Cancelled)
        }
        Err(error) => {
            log::warn!("{}: {} failed: {error}", action.label, action.kind.as_str());
            let message = error.to_string();
            (
                ActionOutcome::Blocked(message.clone()),
                ClaimOutcome::Failed { error: message },
            )
        }
    };
    retro_junk_db::work::release_claim(
        &mut conn,
        kind,
        target_kind,
        target_id,
        &ctx.owner,
        &verdict,
    )?;
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
        return Err(WorkError::Message("operation cancelled".to_owned()));
    }

    // The adopt dispatch only reconciles under `ReconcileMode::PerAction`; a
    // forced rebuild needs the release's real post-adoption state regardless
    // of the caller's reconcile mode, so it reconciles explicitly here.
    let snapshot = scan(ctx)?;
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
        ActionOutcome::Blocked(message) => {
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
        ActionOutcome::Cancelled => Err(WorkError::Message(format!(
            "rebuilding {} was cancelled",
            action.label
        ))),
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
        ArchiveOpsError::Cancelled => WorkError::Message("operation cancelled".to_owned()),
        other => WorkError::msg(other),
    };
    match action.kind {
        ActionKind::VerifyIntegrity => {
            let snapshot = scan(ctx)?;
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
            let snapshot = scan(ctx)?;
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
            let snapshot = scan(ctx)?;
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
            let snapshot = scan(ctx)?;
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
            if ctx.reconcile == ReconcileMode::PerAction && report.published > 0 {
                reconcile_if_per_action(ctx, conn)?;
            }
            Ok(Vec::new())
        }
        ActionKind::ProjectAssets => {
            let snapshot = scan(ctx)?;
            let release = indexed_release(&snapshot, &action.target)?;
            let media_directory = ctx.roots.media_root.join(&action.playable_platform_id);
            retro_junk_lib::archive_assets::project_release_assets(
                release,
                &media_directory,
                &retro_junk_lib::archive_assets::release_media_stems(release),
                cancelled,
            )
            .map_err(WorkError::msg)?;
            Ok(Vec::new())
        }
        ActionKind::SyncGamelist => {
            let snapshot = scan(ctx)?;
            let release = indexed_release(&snapshot, &action.target)?;
            retro_junk_lib::archive_assets::sync_esde_gamelist_for_release(
                release,
                &ctx.roots.playable_root,
                &ctx.roots.metadata_root,
                &ctx.roots.media_root,
            )
            .map_err(WorkError::msg)?;
            Ok(Vec::new())
        }
        _ => Err(WorkError::Message(format!(
            "no executor for action kind {kind}",
            kind = action.kind.as_str()
        ))),
    }
}

fn scan(ctx: &ExecContext) -> Result<retro_junk_archive::ArchiveIndexSnapshot, WorkError> {
    retro_junk_archive::scan_archive(&ctx.profile.archive_root).map_err(WorkError::msg)
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

fn indexed_release<'a>(
    snapshot: &'a retro_junk_archive::ArchiveIndexSnapshot,
    target: &WorkTarget,
) -> Result<&'a retro_junk_archive::IndexedRelease, WorkError> {
    let WorkTarget::Release(release_id) = target else {
        return Err(WorkError::Message(format!(
            "expected a release target, got {}",
            target.kind()
        )));
    };
    snapshot
        .releases
        .iter()
        .find(|release| release.manifest.archive_release_id.to_string() == *release_id)
        .ok_or_else(|| {
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
        let snapshot = scan(ctx)?;
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
