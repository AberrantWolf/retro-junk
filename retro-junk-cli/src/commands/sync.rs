//! `retro-junk sync` and `retro-junk status`: run and report convergence.
//!
//! `sync` reconciles the projection from the authoritative manifests, then
//! derives and executes every pending action through the shared executor —
//! the same path the GUI queue buttons and the daemon use. `--dry-run`
//! prints the derived plan (blocked actions included) and exits non-zero
//! when anything is blocked, preserving the old `archive build --dry-run`
//! planning-failure contract.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use retro_junk_backend::{
    AutomationPolicy, ExecContext, LockEtiquette, ReconcileMode, RunMode, ToolPaths, run_once,
};
use retro_junk_db::convergence::{ActionKind, Scope, derive_convergence, summarize_convergence};

use crate::CliError;
use crate::cli_types::{RebuildPlayableArgs, RenamePlayablesArgs, StatusArgs, SyncArgs};

/// Resolve the target profile: `--profile` selector, explicit roots, or the
/// active settings profile.
pub(crate) fn resolve_target(
    profile: Option<&str>,
    archive_root: Option<PathBuf>,
    playable_root: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
) -> Result<retro_junk_archive::CollectionProfile, CliError> {
    if let Some(archive_root) = archive_root {
        // Fail loudly on an uninitialized root: explicit roots are a direct
        // instruction, so silently minting an identity would be wrong here.
        let _: retro_junk_archive::ArchiveRootManifest =
            retro_junk_archive::read_toml(&retro_junk_archive::root_manifest_path(&archive_root))
                .map_err(|error| CliError::other(error.to_string()))?;
        let playable_root = playable_root
            .ok_or_else(|| CliError::config("--playable-root is required with --archive-root"))?;
        let workspace_root =
            workspace_root.unwrap_or_else(|| archive_root.join(".retro-junk").join("work"));
        let mut target =
            retro_junk_archive::CollectionProfile::for_roots(archive_root, playable_root);
        target.workspace_root = workspace_root;
        target.network_mode = false;
        return Ok(target);
    }
    retro_junk_backend::profiles::resolve_profile(profile).ok_or_else(|| {
        CliError::config(
            "no collection profile found; pass --profile, or --archive-root with \
             --playable-root",
        )
    })
}

pub(crate) fn exec_context(
    profile: retro_junk_archive::CollectionProfile,
    db: Option<PathBuf>,
    tools: ToolPaths,
    media_root: Option<PathBuf>,
    metadata_root: Option<PathBuf>,
    ctx: &Arc<retro_junk_lib::AnalysisContext>,
) -> Result<ExecContext, CliError> {
    let db_path = match db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    let roots = retro_junk_backend::profiles::frontend_roots(
        &profile.playable_root,
        media_root,
        metadata_root,
    );
    Ok(ExecContext {
        profile,
        db_path,
        tools,
        scrape: AutomationPolicy::load().scrape_settings(),
        roots,
        analyzers: Arc::clone(ctx),
        owner: ExecContext::owner_string("cli"),
        lock: LockEtiquette::InteractiveWait,
        reconcile: ReconcileMode::AtBatchEnd,
        archive: retro_junk_backend::ArchiveScan::default(),
    })
}

/// Reconcile the projection from the manifests so derivation sees the
/// archive as it is on disk right now.
pub(crate) fn reconcile_projection(exec: &ExecContext) -> Result<(), CliError> {
    let snapshot = retro_junk_archive::scan_archive(&exec.profile.archive_root)
        .map_err(|error| CliError::other(error.to_string()))?;
    let mut conn = retro_junk_db::open_database(&exec.db_path)
        .map_err(|error| CliError::database(error.to_string()))?;
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &exec.profile.playable_root,
        &exec.profile.workspace_root,
    )
    .map_err(|error| CliError::database(error.to_string()))?;
    Ok(())
}

pub(crate) fn run_sync(args: SyncArgs) -> Result<(), CliError> {
    let ctx = Arc::new(retro_junk_lib::create_default_context());
    let only = args
        .only
        .iter()
        .map(|value| value.parse::<ActionKind>().map_err(CliError::other))
        .collect::<Result<Vec<_>, _>>()?;
    let only = (!only.is_empty()).then_some(only);
    let profile = resolve_target(
        args.profile.as_deref(),
        args.archive_root,
        args.playable_root,
        args.workspace_root,
    )?;
    let scope = scope_for(&profile, args.platform.as_deref(), args.release.as_deref());
    let exec = exec_context(
        profile,
        args.db,
        ToolPaths {
            chdman: args.chdman.unwrap_or_default(),
            redumper: args.redumper.unwrap_or_default(),
            dolphin_tool: args.dolphin_tool.unwrap_or_default(),
        },
        args.media_root,
        args.metadata_root,
        &ctx,
    )?;
    reconcile_projection(&exec)?;
    let conn = retro_junk_db::open_database(&exec.db_path)
        .map_err(|error| CliError::database(error.to_string()))?;

    if args.dry_run {
        let actions = derive_convergence(&conn, &scope, &exec.scrape.expected_assets)
            .map_err(|error| CliError::database(error.to_string()))?;
        let mut blocked = 0_usize;
        for action in &actions {
            if only
                .as_deref()
                .is_some_and(|kinds| !kinds.contains(&action.kind))
            {
                continue;
            }
            match &action.blocked {
                Some(reason) => {
                    blocked += 1;
                    log::warn!(
                        "{:<16} {} — blocked: {reason}",
                        action.kind.as_str(),
                        action.label
                    );
                }
                None => log::info!("{:<16} {}", action.kind.as_str(), action.label),
            }
        }
        if blocked > 0 {
            return Err(CliError::other(format!(
                "{blocked} pending action(s) cannot run unattended"
            )));
        }
        return Ok(());
    }

    drop(conn);
    // The executor checks cancellation at operation boundaries, so Ctrl-C
    // stops between actions with everything completed so far persisted.
    let cancelled = Arc::new(AtomicBool::new(false));
    retro_junk_backend::daemon::install_signal_handlers(&cancelled);
    let policy = AutomationPolicy::load();
    let stats = run_once(
        &exec,
        &policy,
        &scope,
        RunMode::Explicit,
        retro_junk_backend::ProjectionPass::Always,
        only.as_deref(),
        args.limit,
        &crate::commands::archive::log_progress,
        &cancelled,
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    log::info!(
        "Sync finished: {} completed, {} failed, {} blocked, {} busy",
        stats.completed,
        stats.failed,
        stats.blocked,
        stats.skipped_busy
    );
    if stats.failed > 0 {
        Err(CliError::other(format!(
            "{} action(s) failed; details are recorded per target",
            stats.failed
        )))
    } else {
        Ok(())
    }
}

/// Force a release's playable representation into a good state, bypassing
/// the "already satisfied" check `sync`'s normal derivation applies.
///
/// A release whose evidence points at bytes that moved, were regenerated,
/// or were adopted against a file that turned out not to be there reads as
/// satisfied off of stale caches and never reaches `sync`'s derivation at
/// all. `--dry-run` previews via `forced_build_action` alone (adoption is
/// never a preview — it only ever links a file to evidence by matching
/// content, so there is nothing unsafe about just running it); a real run
/// goes through `retro_junk_backend::force_rebuild_playable`, the identical
/// function the GUI's "Force Rebuild Playable" uses, so "force" means the
/// same thing from either surface.
pub(crate) fn run_rebuild_playable(args: RebuildPlayableArgs) -> Result<(), CliError> {
    let ctx = Arc::new(retro_junk_lib::create_default_context());
    let profile = resolve_target(
        args.profile.as_deref(),
        args.archive_root,
        args.playable_root,
        args.workspace_root,
    )?;
    let exec = exec_context(
        profile,
        args.db,
        ToolPaths {
            chdman: args.chdman.unwrap_or_default(),
            redumper: args.redumper.unwrap_or_default(),
            dolphin_tool: args.dolphin_tool.unwrap_or_default(),
        },
        None,
        None,
        &ctx,
    )?;
    reconcile_projection(&exec)?;
    if args.dry_run {
        let conn = retro_junk_db::open_database(&exec.db_path)
            .map_err(|error| CliError::database(error.to_string()))?;
        let action = retro_junk_db::convergence::forced_build_action(&conn, &args.release_id)
            .map_err(|error| CliError::database(error.to_string()))?
            .ok_or_else(|| {
                CliError::other(format!(
                    "no archived release {} to rebuild",
                    args.release_id
                ))
            })?;
        match &action.blocked {
            Some(reason) => log::warn!("{} — blocked: {reason}", action.label),
            None => log::info!("Would rebuild: {}", action.label),
        }
        return Ok(());
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    retro_junk_backend::daemon::install_signal_handlers(&cancelled);
    let outcome = retro_junk_backend::force_rebuild_playable(
        &exec,
        &args.release_id,
        &crate::commands::archive::log_progress,
        &cancelled,
    )
    .map_err(|error| CliError::other(error.to_string()))?;
    match outcome {
        retro_junk_backend::ForceRebuildOutcome::Adopted(label) => {
            log::info!("{label} was already there — adopted it");
            Ok(())
        }
        retro_junk_backend::ForceRebuildOutcome::Built(outputs) => {
            for output in &outputs {
                log::info!("Rebuilt {}", output.display());
            }
            Ok(())
        }
    }
}

/// Rename built playables whose name the catalog disagrees with.
///
/// The list comes from the same completion fold the GUI renders, so a dry run
/// here shows exactly the renames the Details panel offers.
pub(crate) fn run_rename_playables(args: RenamePlayablesArgs) -> Result<(), CliError> {
    let profile = resolve_target(
        args.profile.as_deref(),
        args.archive_root,
        args.playable_root,
        None,
    )?;
    let db_path = match args.db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    let expected_assets = AutomationPolicy::load().scrape_selection();
    if args.dry_run {
        let conn = retro_junk_db::open_database(&db_path)
            .map_err(|error| CliError::database(error.to_string()))?;
        let stale = retro_junk_backend::ops::rename_playable::stale_playable_names(
            &conn,
            &profile.profile_id.to_string(),
            &expected_assets,
            args.release_id.as_deref(),
        )
        .map_err(CliError::other)?;
        if stale.is_empty() {
            log::info!("Every built playable already has its catalog name");
            return Ok(());
        }
        for (current, canonical) in &stale {
            log::info!("{current}  ->  {canonical}");
        }
        log::info!("{} playable(s) would be renamed", stale.len());
        return Ok(());
    }
    let media_root =
        retro_junk_backend::profiles::frontend_roots(&profile.playable_root, args.media_root, None)
            .media_root;
    let cancelled = Arc::new(AtomicBool::new(false));
    retro_junk_backend::daemon::install_signal_handlers(&cancelled);
    let report = retro_junk_backend::ops::rename_playable::rename_stale_playables(
        &profile,
        &db_path,
        Some(&media_root),
        &expected_assets,
        args.release_id.as_deref(),
        &retro_junk_backend::ops::OpCtx::new(&cancelled, &crate::commands::archive::log_progress),
    )
    .map_err(CliError::other)?;
    for renamed in &report.renamed {
        log::info!("{}  ->  {}", renamed.from, renamed.to);
    }
    for failure in &report.failures {
        log::warn!("{failure}");
    }
    log::info!("Renamed {} playable(s)", report.renamed.len());
    if report.failures.is_empty() {
        Ok(())
    } else {
        Err(CliError::other(format!(
            "{} playable(s) could not be renamed",
            report.failures.len()
        )))
    }
}

pub(crate) fn run_status(args: StatusArgs) -> Result<(), CliError> {
    let profile = resolve_target(
        args.profile.as_deref(),
        args.archive_root,
        args.playable_root,
        None,
    )?;
    let db_path = match args.db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    let conn = retro_junk_db::open_database(&db_path)
        .map_err(|error| CliError::database(error.to_string()))?;
    let scope = Scope::Profile(profile.profile_id.to_string());
    let expected = AutomationPolicy::load().scrape_selection();
    let summary = summarize_convergence(&conn, &scope, &expected)
        .map_err(|error| CliError::database(error.to_string()))?;
    log::info!("Profile: {} ({})", profile.display_name, profile.profile_id);
    for (kind, counts) in &summary.per_kind {
        log::info!(
            "{:<16} done {:>5}  pending {:>4}  unresolved {:>3}  blocked {:>3}  errored {:>3}  running {:>2}",
            kind.as_str(),
            counts.done,
            counts.pending,
            counts.unresolved,
            counts.blocked,
            counts.errored,
            counts.running
        );
    }
    log::info!("Open suggestions: {}", summary.open_suggestions);
    let runtime = retro_junk_db::work::read_runtime_state(&conn)
        .map_err(|error| CliError::database(error.to_string()))?;
    match (&runtime.daemon_pid, &runtime.daemon_heartbeat_at) {
        (Some(pid), Some(heartbeat)) => {
            log::info!("Daemon: pid {pid}, last heartbeat {heartbeat}");
        }
        _ => log::info!("Daemon: not running"),
    }
    log::info!("Change tick: {}", runtime.dirty_tick);
    Ok(())
}

fn scope_for(
    profile: &retro_junk_archive::CollectionProfile,
    platform: Option<&str>,
    release: Option<&str>,
) -> Scope {
    if let Some(release) = release {
        return Scope::Release {
            archive_release_id: release.to_owned(),
        };
    }
    if let Some(platform) = platform {
        return Scope::Platform {
            profile_id: profile.profile_id.to_string(),
            platform_id: platform.to_owned(),
        };
    }
    Scope::Profile(profile.profile_id.to_string())
}
