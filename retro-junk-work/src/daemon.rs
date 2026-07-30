//! The convergence daemon: watch, derive, execute, repeat.
//!
//! Foreground-only by design — launchd/systemd/tmux own daemonization. The
//! loop is: reconcile when something changed, pre-process incoming
//! packages, run one stage-ordered convergence pass, heartbeat, then block
//! on filesystem events for a tick. Policy edits apply within one tick via
//! an mtime check; SIGINT/SIGTERM cancel cooperatively.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use retro_junk_db::convergence::Scope;

use crate::executor::{ExecContext, WorkError};
use crate::incoming::{note_incoming_event, process_pending_incoming};
use crate::policy::AutomationPolicy;
use crate::watch::{DirectoryWatcher, WatchEvent};
use crate::worker::{RunMode, run_once};

/// Default event-wait tick.
pub const DEFAULT_TICK: Duration = Duration::from_secs(30);

/// PID-file location shared by `daemon start` and `daemon stop`.
#[must_use]
pub fn daemon_pid_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("retro-junk")
        .join("daemon.pid")
}

/// Write this process's PID; errors are fatal (stop needs the file).
pub fn write_pid_file() -> std::io::Result<()> {
    let path = daemon_pid_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, std::process::id().to_string())
}

pub fn remove_pid_file() {
    let _ = std::fs::remove_file(daemon_pid_path());
}

/// Read the recorded daemon PID, if any.
#[must_use]
pub fn read_pid_file() -> Option<i32> {
    std::fs::read_to_string(daemon_pid_path())
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Run the daemon loop until cancelled. `ctx` must use
/// `LockEtiquette::DaemonFailFast` so interactive users always win the
/// archive lock.
// The loop is one protocol: policy reload, reconcile, incoming, converge,
// heartbeat, wait. Splitting it would hide the ordering the module doc
// promises.
#[allow(clippy::too_many_lines)]
pub fn run(
    ctx: &ExecContext,
    tick: Duration,
    cancelled: &Arc<AtomicBool>,
) -> Result<(), WorkError> {
    let conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    retro_junk_db::work::daemon_started(&conn, i64::from(std::process::id() as i32))?;
    drop(conn);

    let policy_path = retro_junk_lib::settings::settings_path();
    let mut policy = AutomationPolicy::load_from(&policy_path);
    let mut policy_mtime = file_mtime(&policy_path);

    // Watch the incoming drop folders and the playable root. The archive
    // tree is deliberately unwatched: it changes only through the tool.
    // Roots are canonicalized so backend-reported event paths (which are
    // canonical on macOS) classify against them by prefix.
    let incoming_roots: Vec<PathBuf> = ctx
        .profile
        .incoming_roots
        .iter()
        .filter(|root| root.is_dir())
        .map(|root| canonical(root))
        .collect();
    let playable_root = canonical(&ctx.profile.playable_root);
    let mut roots = incoming_roots.clone();
    if playable_root.is_dir() {
        roots.push(playable_root.clone());
    }
    let watching =
        !roots.is_empty() && ctx.profile.watch_backend != retro_junk_archive::WatchBackend::Off;
    // Keep a sender alive when not watching so the event channel never
    // disconnects and the loop just ticks.
    let (idle_tx, idle_rx) = std::sync::mpsc::channel::<WatchEvent>();
    let mut _watcher: Option<DirectoryWatcher> = None;
    let events = if watching {
        let (watcher, events) =
            DirectoryWatcher::start(&roots, ctx.profile.watch_backend).map_err(WorkError::msg)?;
        _watcher = Some(watcher);
        drop(idle_tx);
        events
    } else {
        std::mem::forget(idle_tx);
        idle_rx
    };
    log::info!(
        "daemon: watching {} root(s), tick {}s",
        roots.len(),
        tick.as_secs()
    );

    let playable_extensions: std::collections::HashSet<String> = ctx
        .analyzers
        .consoles()
        .flat_map(|console| console.analyzer.file_extensions())
        .map(|extension| extension.to_ascii_lowercase())
        .collect();
    let mut needs_reconcile = true;
    let mut force_projections = true;
    let mut last_deep = Instant::now();
    while !cancelled.load(Ordering::Relaxed) {
        // Policy edits apply within one tick.
        let current_mtime = file_mtime(&policy_path);
        if current_mtime != policy_mtime {
            policy_mtime = current_mtime;
            policy = AutomationPolicy::load_from(&policy_path);
            log::info!("daemon: automation policy reloaded");
        }

        if needs_reconcile {
            if let Err(error) = reconcile(ctx) {
                log::warn!("daemon: projection refresh failed: {error}");
            }
            needs_reconcile = false;
        }

        match process_pending_incoming(ctx, &policy, cancelled) {
            Ok(stats) => {
                if stats.imported > 0 {
                    needs_reconcile = true;
                }
                if stats.imported + stats.suggested + stats.errored > 0 {
                    log::info!(
                        "daemon: incoming — {} imported, {} suggested, {} errored",
                        stats.imported,
                        stats.suggested,
                        stats.errored
                    );
                }
            }
            Err(error) => log::warn!("daemon: incoming pass failed: {error}"),
        }

        let projections = if force_projections {
            crate::worker::ProjectionPass::Always
        } else {
            crate::worker::ProjectionPass::OnlyAfterMutation
        };
        force_projections = false;
        match run_once(
            ctx,
            &policy,
            &Scope::Profile(ctx.profile.profile_id.to_string()),
            RunMode::Daemon,
            projections,
            None,
            None,
            &|phase, current, total| {
                if total > 0 {
                    log::debug!("{phase}: {current}/{total}");
                } else {
                    log::debug!("{phase}");
                }
            },
            cancelled,
        ) {
            Ok(stats) => {
                if stats.completed + stats.failed > 0 {
                    log::info!(
                        "daemon: pass — {} completed, {} failed, {} blocked, {} backoff",
                        stats.completed,
                        stats.failed,
                        stats.blocked,
                        stats.skipped_backoff
                    );
                }
            }
            Err(error) => log::warn!("daemon: convergence pass failed: {error}"),
        }

        let conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
        retro_junk_db::work::daemon_heartbeat(&conn)?;
        drop(conn);

        if policy.deep_rescan_hours > 0
            && last_deep.elapsed()
                >= Duration::from_secs(u64::from(policy.deep_rescan_hours) * 3600)
        {
            log::info!("daemon: periodic deep rescan");
            needs_reconcile = true;
            force_projections = true;
            last_deep = Instant::now();
        }

        match events.recv_timeout(tick) {
            Ok(event) => {
                apply_event(
                    ctx,
                    &incoming_roots,
                    &playable_root,
                    &playable_extensions,
                    &event,
                    &mut needs_reconcile,
                )?;
                while let Ok(next) = events.try_recv() {
                    apply_event(
                        ctx,
                        &incoming_roots,
                        &playable_root,
                        &playable_extensions,
                        &next,
                        &mut needs_reconcile,
                    )?;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                log::warn!("daemon: watcher stopped; continuing on tick only");
                std::thread::sleep(tick);
            }
        }
    }
    log::info!("daemon: stopping");
    Ok(())
}

fn reconcile(ctx: &ExecContext) -> Result<(), WorkError> {
    let snapshot =
        retro_junk_archive::scan_archive(&ctx.profile.archive_root).map_err(WorkError::msg)?;
    let mut conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &ctx.profile.playable_root,
        &ctx.profile.workspace_root,
    )
    .map_err(WorkError::msg)?;
    Ok(())
}

fn canonical(path: &std::path::Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn apply_event(
    ctx: &ExecContext,
    incoming_roots: &[PathBuf],
    playable_root: &std::path::Path,
    playable_extensions: &std::collections::HashSet<String>,
    event: &WatchEvent,
    needs_reconcile: &mut bool,
) -> Result<(), WorkError> {
    let mut conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    let profile_id = ctx.profile.profile_id.to_string();
    // Renames touch both endpoints; everything else just its path.
    let mut touched: Vec<&std::path::Path> = vec![event.path()];
    if let WatchEvent::FileRenamed { from, .. } = event {
        touched.push(from);
    }
    for path in touched {
        if let Some(root) = incoming_roots.iter().find(|root| path.starts_with(root)) {
            note_incoming_event(&mut conn, &profile_id, root, path)?;
            continue;
        }
        if path.starts_with(playable_root) {
            let relevant = matches!(event, WatchEvent::DirAdded(_) | WatchEvent::DirRemoved(_))
                || path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|ext| playable_extensions.contains(&ext.to_ascii_lowercase()));
            if !relevant {
                continue;
            }
            // The playable tree changed under us: presence states and the
            // library projection are stale. Mark the console and refresh.
            *needs_reconcile = true;
            let root = ctx.profile.playable_root.to_string_lossy();
            if let Some(console_id) =
                retro_junk_db::library::console_for_path(&conn, &root, &path.to_string_lossy())?
            {
                let _ = retro_junk_db::mark_console_stale(&mut conn, console_id);
            }
        }
    }
    Ok(())
}

fn file_mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

// ── Signal handling ────────────────────────────────────────────────────────
// Hand-rolled FFI to `signal(2)`: no extra crates, and the handler only
// flips an AtomicBool — async-signal-safe by construction.

#[cfg(unix)]
pub fn install_signal_handlers(cancel: &Arc<AtomicBool>) {
    use std::os::raw::c_int;
    use std::sync::OnceLock;

    static CANCEL: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    const SIGINT: c_int = 2;
    const SIGTERM: c_int = 15;

    unsafe extern "C" fn trampoline(_signum: c_int) {
        if let Some(cancel) = CANCEL.get() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    unsafe extern "C" {
        fn signal(signum: c_int, handler: usize) -> usize;
    }

    let _ = CANCEL.set(Arc::clone(cancel));
    // SAFETY: installing an async-signal-safe handler that only stores to
    // an atomic.
    unsafe {
        signal(SIGINT, trampoline as *const () as usize);
        signal(SIGTERM, trampoline as *const () as usize);
    }
}

#[cfg(not(unix))]
pub fn install_signal_handlers(_cancel: &Arc<AtomicBool>) {
    // Ctrl-C handling on non-Unix hosts arrives with a platform backend.
}

/// Whether a recorded PID refers to a live process (same-host check).
/// Unknown liveness (non-Unix hosts) reads as "not running" so daemon
/// commands report honestly rather than claiming a daemon exists.
#[must_use]
pub fn process_alive(pid: i32) -> bool {
    retro_junk_io::process_alive(pid).unwrap_or(false)
}
