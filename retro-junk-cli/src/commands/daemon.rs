//! `retro-junk daemon start|stop|status`.
//!
//! `start` runs in the foreground — launchd/systemd/tmux own backgrounding —
//! wiring the watcher and the convergence loop over the active collection
//! profile. `stop` signals the recorded PID and waits for a clean exit;
//! `status` reports liveness plus the shared convergence summary.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use retro_junk_backend::daemon;
use retro_junk_backend::{ExecContext, LockEtiquette, ReconcileMode, ToolPaths};

use crate::CliError;
use crate::cli_types::DaemonAction;

pub(crate) fn run_daemon(action: DaemonAction) -> Result<(), CliError> {
    match action {
        DaemonAction::Start {
            profile,
            db,
            foreground,
            tick,
            chdman,
            redumper,
            dolphin_tool,
        } => run_start(
            profile.as_deref(),
            db,
            foreground,
            tick,
            chdman,
            redumper,
            dolphin_tool,
        ),
        DaemonAction::Stop => run_stop(),
        DaemonAction::Status { db } => {
            match daemon::read_pid_file() {
                Some(pid) if daemon::process_alive(pid) => {
                    log::info!("Daemon process: running (pid {pid})");
                }
                Some(pid) => log::warn!("Daemon process: stale PID file (pid {pid} is gone)"),
                None => log::info!("Daemon process: not running"),
            }
            crate::commands::sync::run_status(crate::cli_types::StatusArgs {
                profile: None,
                archive_root: None,
                playable_root: None,
                db,
            })
        }
    }
}

fn run_start(
    profile: Option<&str>,
    db: Option<PathBuf>,
    foreground: bool,
    tick: Option<u64>,
    chdman: Option<PathBuf>,
    redumper: Option<PathBuf>,
    dolphin_tool: Option<PathBuf>,
) -> Result<(), CliError> {
    if !foreground {
        return Err(CliError::config(
            "the daemon runs in the foreground; pass --foreground, and use \
             launchd/systemd/tmux for backgrounding",
        ));
    }
    let profile = retro_junk_backend::profiles::resolve_profile(profile).ok_or_else(|| {
        CliError::config("no collection profile found; configure one in the GUI or settings.toml")
    })?;
    if !retro_junk_archive::root_manifest_path(&profile.archive_root).is_file() {
        return Err(CliError::config(format!(
            "profile archive root {} is not an initialized archive",
            profile.archive_root.display()
        )));
    }
    let db_path = match db {
        Some(path) => path,
        None => retro_junk_lib::settings::ensure_catalog_database_location()?,
    };
    let general = retro_junk_backend::profiles::load_general();
    let roots = retro_junk_backend::profiles::frontend_roots(&profile.playable_root, None, None);
    let ctx = ExecContext {
        profile,
        db_path,
        tools: ToolPaths {
            chdman: chdman.unwrap_or_else(|| PathBuf::from(general.chdman_path.trim())),
            redumper: redumper.unwrap_or_default(),
            dolphin_tool: dolphin_tool.unwrap_or_default(),
        },
        scrape: retro_junk_backend::AutomationPolicy::load().scrape_settings(),
        roots,
        analyzers: Arc::new(retro_junk_lib::create_default_context()),
        owner: ExecContext::owner_string("daemon"),
        lock: LockEtiquette::DaemonFailFast,
        reconcile: ReconcileMode::AtBatchEnd,
    };
    daemon::write_pid_file()?;
    let cancelled = Arc::new(AtomicBool::new(false));
    daemon::install_signal_handlers(&cancelled);
    log::info!(
        "Daemon started for profile {} (pid {})",
        ctx.profile.display_name,
        std::process::id()
    );
    let tick = tick.map_or(daemon::DEFAULT_TICK, std::time::Duration::from_secs);
    let result = daemon::run(&ctx, tick, &cancelled);
    daemon::remove_pid_file();
    result.map_err(|error| CliError::other(error.to_string()))
}

fn run_stop() -> Result<(), CliError> {
    let Some(pid) = daemon::read_pid_file() else {
        return Err(CliError::other(
            "no daemon PID file found; is the daemon running?",
        ));
    };
    if !daemon::process_alive(pid) {
        daemon::remove_pid_file();
        log::info!("Daemon pid {pid} was already gone; removed the stale PID file");
        return Ok(());
    }
    let status = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()?;
    if !status.success() {
        return Err(CliError::other(format!(
            "could not signal daemon pid {pid}"
        )));
    }
    // The daemon removes its PID file on clean exit.
    for _ in 0..50 {
        if !daemon::daemon_pid_path().is_file() {
            log::info!("Daemon stopped");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    log::warn!("Daemon pid {pid} has not exited yet; it may be mid-action");
    Ok(())
}
