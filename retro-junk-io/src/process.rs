//! Same-host process liveness probing and host identity.

/// A stable name for this machine, for records that processes on *other*
/// machines may read (a lock file on a network share, for instance). A PID is
/// only meaningful next to the host that issued it, so such records must
/// carry both. `None` when the platform offers no way to ask.
///
/// Whitespace is folded to `-` so the value stays a single token in
/// space-separated records.
#[must_use]
pub fn local_host_id() -> Option<&'static str> {
    static HOST: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    HOST.get_or_init(read_host_name).as_deref()
}

#[cfg(unix)]
fn read_host_name() -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: gethostname writes a NUL-terminated name into the buffer it is
    // given, never past its stated length.
    if unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0 {
        return None;
    }
    let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    let name = String::from_utf8_lossy(&buffer[..end])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    (!name.is_empty()).then_some(name)
}

#[cfg(not(unix))]
fn read_host_name() -> Option<String> {
    None
}

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
