//! Same-host process liveness probing.

/// Whether a recorded PID refers to a live process on this host, or `None`
/// when liveness cannot be probed (non-Unix platforms, or a PID that does not
/// address a single process). Callers own the conservative interpretation of
/// `None`.
///
/// `kill(pid, 0)` performs existence and permission checks without delivering
/// a signal. `EPERM` means the process exists but belongs to another user, so
/// it still counts as alive.
#[cfg(unix)]
#[must_use]
pub fn process_alive(pid: i32) -> Option<bool> {
    if pid <= 0 {
        // Zero and negative values address process groups, not a process.
        return None;
    }
    // SAFETY: signal 0 performs error checking only; nothing is delivered.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return Some(true);
    }
    Some(std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}

#[cfg(not(unix))]
#[must_use]
pub fn process_alive(_pid: i32) -> Option<bool> {
    None
}
