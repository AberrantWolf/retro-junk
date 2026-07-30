//! Starting, stopping, and reporting on the convergence daemon.
//!
//! The daemon is a CLI subcommand by design — one install, one config, one
//! set of credentials — so the GUI launches `retro-junk daemon start` rather
//! than growing a second daemon implementation inside the app process. What
//! the GUI adds is a place for its output to go (the CLI expects launchd,
//! systemd, or a terminal to capture stdout; a GUI launch has none of those)
//! and a status readout built from the same PID file and heartbeat
//! `retro-junk daemon status` reads.

use std::path::PathBuf;

use retro_junk_work::daemon::{daemon_log_path, process_alive, read_pid_file};

/// What the Settings section shows about the daemon.
pub enum DaemonStatus {
    /// No PID file: nothing has been started, or it exited cleanly.
    NotRunning,
    /// A PID file naming a process that is gone — a crash, not a shutdown.
    Stale(i32),
    Running {
        pid: i32,
        /// Heartbeat timestamp from `runtime_state`, if the daemon has
        /// reached its first tick.
        heartbeat: Option<String>,
    },
}

/// Read the daemon's current state.
#[must_use]
pub fn status(connection: Option<&retro_junk_db::Connection>) -> DaemonStatus {
    let Some(pid) = read_pid_file() else {
        return DaemonStatus::NotRunning;
    };
    if !process_alive(pid) {
        return DaemonStatus::Stale(pid);
    }
    let heartbeat = connection
        .and_then(|connection| retro_junk_db::work::read_runtime_state(connection).ok())
        .and_then(|runtime| runtime.daemon_heartbeat_at);
    DaemonStatus::Running { pid, heartbeat }
}

/// Locate the `retro-junk` CLI: beside this executable first (how a
/// packaged build ships), then whatever is on `PATH`.
#[must_use]
pub fn cli_path() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "retro-junk.exe"
    } else {
        "retro-junk"
    };
    if let Some(sibling) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        && sibling.is_file()
    {
        return Some(sibling);
    }
    // `PATH` lookup is left to the OS by returning the bare name.
    which_on_path(name)
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// Launch the daemon as a detached child, with its output captured to
/// [`daemon_log_path`].
pub fn start() -> Result<(), String> {
    let cli = cli_path().ok_or_else(|| {
        "could not find the retro-junk CLI; install it alongside the app or on PATH".to_owned()
    })?;
    let log_path = daemon_log_path();
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let log = std::fs::File::create(&log_path).map_err(|error| error.to_string())?;
    let errors = log.try_clone().map_err(|error| error.to_string())?;
    std::process::Command::new(&cli)
        .arg("daemon")
        .arg("start")
        .arg("--foreground")
        .stdout(log)
        .stderr(errors)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", cli.display()))?;
    // Deliberately not waited on: the child outlives this process, which is
    // the point — quitting the GUI should not stop a running convergence.
    Ok(())
}

/// Signal the daemon to stop. Returns the PID that was signalled.
pub fn stop() -> Result<i32, String> {
    let pid = read_pid_file().ok_or_else(|| "the daemon is not running".to_owned())?;
    if !process_alive(pid) {
        retro_junk_work::daemon::remove_pid_file();
        return Err(format!("daemon pid {pid} was already gone"));
    }
    let cli = cli_path()
        .ok_or_else(|| "could not find the retro-junk CLI to stop the daemon".to_owned())?;
    let status = std::process::Command::new(&cli)
        .arg("daemon")
        .arg("stop")
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(pid)
    } else {
        Err(format!("could not stop daemon pid {pid}"))
    }
}

/// Last `lines` lines of the captured daemon output.
///
/// Only the tail of the file is read: a long-running daemon's log is
/// unbounded, and Settings only ever shows the end of it.
#[must_use]
pub fn log_tail(lines: usize) -> Vec<String> {
    tail_of(&daemon_log_path(), lines)
}

fn tail_of(path: &std::path::Path, lines: usize) -> Vec<String> {
    use std::io::{Read as _, Seek as _, SeekFrom};

    const TAIL_BYTES: u64 = 16 * 1024;
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let length = file.metadata().map_or(0, |meta| meta.len());
    let offset = length.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    // Seeking lands mid-character and mid-line; a lossy decode plus dropping
    // the first line keeps the tail honest without reading the whole file.
    let text = String::from_utf8_lossy(&bytes);
    let mut all: Vec<&str> = text.lines().collect();
    if offset > 0 && !all.is_empty() {
        all.remove(0);
    }
    all.iter()
        .skip(all.len().saturating_sub(lines))
        .map(|line| (*line).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    /// A tail read seeks into the middle of the file, so the first line it
    /// sees is usually a fragment. Emitting it would put half a log line at
    /// the top of the panel every time the log grew past the window.
    #[test]
    fn a_tail_read_drops_the_partial_first_line() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("daemon.log");
        // Comfortably past the 16 KiB window so a seek is guaranteed.
        let mut filler = String::new();
        for index in 0..2000 {
            use std::fmt::Write as _;
            let _ = writeln!(filler, "line {index} padding padding padding padding");
        }
        std::fs::write(&path, format!("{filler}last one\n")).expect("write log");

        let lines = super::tail_of(&path, 5);
        assert_eq!(lines.last().map(String::as_str), Some("last one"));
        assert!(
            lines
                .iter()
                .all(|line| line.starts_with("line ") || line == "last one"),
            "a truncated fragment leaked into the tail: {lines:?}"
        );
    }

    #[test]
    fn a_short_log_is_returned_whole() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("daemon.log");
        std::fs::write(&path, "first\nsecond\n").expect("write log");
        assert_eq!(super::tail_of(&path, 10), vec!["first", "second"]);
    }

    #[test]
    fn a_missing_log_is_empty_rather_than_an_error() {
        assert!(super::tail_of(std::path::Path::new("/nonexistent/daemon.log"), 5).is_empty());
    }
}
