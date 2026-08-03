use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, thiserror::Error)]
pub enum ArchiveLockError {
    #[error("archive root is not initialized: {0}")]
    Uninitialized(String),
    #[error("archive is already being modified ({0})")]
    Busy(String),
    #[error("could not manage archive lock {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

/// Process-level guard for manifest/evidence mutations.
///
/// Where the filesystem enforces OS advisory locks (verified per acquisition,
/// because macOS smbfs *accepts* `flock` while enforcing nothing), the lock is
/// an OS lock on `.retro-junk/archive.lock`: the kernel releases it the moment
/// the holding process exits, so a crash cannot leave the archive wedged. On
/// filesystems without enforcement — notably network mounts — the fallback is
/// the existence-based protocol, where a crashed holder is reclaimed once its
/// record names this host and the PID is demonstrably absent. A record from
/// another machine (a network share is shared) or one that cannot be
/// attributed is reclaimed only after a conservative 24-hour age.
pub struct ArchiveLock {
    backing: Backing,
    path: PathBuf,
}

enum Backing {
    /// Held OS lock. The lock file itself stays on disk (unlinking it would
    /// race a concurrent acquirer's open of the same path); its contents are
    /// diagnostics for the `Busy` message and are cleared on release.
    Os(File),
    /// Existence-based fallback; the file is unlinked on release.
    Legacy,
}

impl ArchiveLock {
    pub fn acquire(root: &Path) -> Result<Self, ArchiveLockError> {
        if !crate::layout::root_manifest_path(root).is_file() {
            return Err(ArchiveLockError::Uninitialized(root.display().to_string()));
        }
        let state = root.join(".retro-junk");
        std::fs::create_dir_all(&state).map_err(|source| ArchiveLockError::Io {
            path: state.display().to_string(),
            source,
        })?;
        let path = state.join("archive.lock");
        if !fs_enforces_os_locks(&state) {
            return Self::acquire_legacy(path);
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| ArchiveLockError::Io {
                path: path.display().to_string(),
                source,
            })?;
        match file.try_lock() {
            Ok(()) => {
                // The OS lock is ours, but a holder from an older binary (or
                // one that acquired before this filesystem supported locks)
                // may still own the file by existence; only reclaim its
                // recorded claim when stale.
                let contents = std::fs::read_to_string(&path).unwrap_or_default();
                if !contents.trim().is_empty()
                    && !contents.contains("mode=os")
                    && !lock_is_stale(&path)
                {
                    // Same formatter as the OS-lock branch: both describe one
                    // holder, and only one of them being readable is worse
                    // than neither.
                    return Err(ArchiveLockError::Busy(busy_details(&path)));
                }
                write_owner(&file, "os").map_err(|source| ArchiveLockError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
                Ok(Self {
                    backing: Backing::Os(file),
                    path,
                })
            }
            Err(TryLockError::WouldBlock) => Err(ArchiveLockError::Busy(busy_details(&path))),
            Err(TryLockError::Error(source))
                if source.kind() == std::io::ErrorKind::Unsupported =>
            {
                drop(file);
                Self::acquire_legacy(path)
            }
            Err(TryLockError::Error(source)) => Err(ArchiveLockError::Io {
                path: path.display().to_string(),
                source,
            }),
        }
    }

    /// Existence-based acquisition for filesystems without enforced OS locks.
    /// `create_new` is the atomic claim; a stale leftover is renamed aside
    /// first. Rename rather than remove, because several contenders can judge
    /// the same leftover stale at once: exactly one rename wins, so a loser
    /// cannot delete the lock a faster contender just created — with remove,
    /// that deletion let two of them acquire.
    fn acquire_legacy(path: PathBuf) -> Result<Self, ArchiveLockError> {
        if path.exists() && lock_is_stale(&path) {
            let aside = path.with_extension(format!("stale-{}", std::process::id()));
            match std::fs::rename(&path, &aside) {
                // A crash before this remove leaves an inert `.stale-*` file;
                // nothing reads it, and the lock path itself is already free.
                Ok(()) => {
                    let _ = std::fs::remove_file(&aside);
                }
                // Another contender reclaimed first; fall through and let
                // `create_new` decide who acquires.
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(ArchiveLockError::Io {
                        path: path.display().to_string(),
                        source,
                    });
                }
            }
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|source| {
                if source.kind() == std::io::ErrorKind::AlreadyExists {
                    ArchiveLockError::Busy(busy_details(&path))
                } else {
                    ArchiveLockError::Io {
                        path: path.display().to_string(),
                        source,
                    }
                }
            })?;
        write_owner(&file, "legacy").map_err(|source| ArchiveLockError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Ok(Self {
            backing: Backing::Legacy,
            path,
        })
    }

    /// Wait in FIFO-friendly polling intervals for the current archive writer.
    ///
    /// The lock file remains the cross-process authority; this helper prevents
    /// completed background work from being discarded merely because another
    /// in-process action is publishing at the same time.
    pub fn acquire_wait(
        root: &Path,
        cancel: &AtomicBool,
    ) -> Result<Option<Self>, ArchiveLockError> {
        let mut reported_wait = false;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            match Self::acquire(root) {
                Ok(lock) => return Ok(Some(lock)),
                Err(ArchiveLockError::Busy(_)) => {
                    if !reported_wait {
                        log::info!(
                            "Waiting for the current archive writer at {}",
                            root.display()
                        );
                        reported_wait = true;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for ArchiveLock {
    fn drop(&mut self) {
        match &self.backing {
            Backing::Os(file) => {
                // Clear the diagnostics; closing `file` releases the OS lock.
                let _ = file.set_len(0);
            }
            Backing::Legacy => {
                if let Err(error) = std::fs::remove_file(&self.path) {
                    log::warn!(
                        "could not release archive lock {}: {error}",
                        self.path.display()
                    );
                }
            }
        }
    }
}

/// Whether this filesystem actually enforces the OS advisory locks it
/// accepts. macOS smbfs reports `flock` success while enforcing nothing, so
/// success alone proves nothing: a second handle must be *refused*. Network
/// filesystems that emulate `flock` with per-process POSIX locks also fail
/// this probe and use the existence protocol — correct, just less precise
/// about crashes. The scratch file is per-process so concurrent probes cannot
/// disturb each other.
fn fs_enforces_os_locks(state: &Path) -> bool {
    let path = state.join(format!(".lock-probe-{}", std::process::id()));
    let enforced = (|| {
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .ok()?;
        held.try_lock().ok()?;
        let probe = OpenOptions::new().read(true).write(true).open(&path).ok()?;
        Some(matches!(probe.try_lock(), Err(TryLockError::WouldBlock)))
    })();
    let _ = std::fs::remove_file(&path);
    enforced.unwrap_or(false)
}

/// Record the holder in the lock file for `Busy` diagnostics and staleness
/// decisions. The host rides along because a network share is read by other
/// machines, and a PID means nothing without the host that issued it.
fn write_owner(mut file: &File, mode: &str) -> std::io::Result<()> {
    let host = retro_junk_io::local_host_id()
        .map(|host| format!(" host={host}"))
        .unwrap_or_default();
    file.set_len(0)?;
    writeln!(
        file,
        "mode={mode} pid={}{host} started_at={}",
        std::process::id(),
        chrono::Utc::now().to_rfc3339()
    )?;
    file.sync_all()
}

/// Current holder description for the `Busy` error, falling back to the lock
/// path when the contents cannot be read (or have not been written yet).
/// Describe the current holder in terms a reader can act on.
///
/// The record's `started_at` is RFC 3339 in UTC, and printing it raw invited
/// exactly one misreading: a lock taken 34 minutes earlier read as "held since
/// 02:06 this morning" to someone nine hours ahead of UTC, which looks like a
/// wedged archive rather than a job still running. Elapsed time needs no
/// timezone to interpret.
fn busy_details(path: &Path) -> String {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return path.display().to_string();
    };
    let contents = contents.trim();
    if contents.is_empty() {
        return path.display().to_string();
    }
    let field = |name: &str| {
        contents
            .split_whitespace()
            .find_map(|part| part.strip_prefix(name))
    };
    let held = field("started_at=")
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|started| {
            let elapsed = chrono::Utc::now().signed_duration_since(started);
            let minutes = elapsed.num_minutes().max(0);
            if minutes < 60 {
                format!("held {minutes}m")
            } else {
                format!("held {}h{}m", minutes / 60, minutes % 60)
            }
        });
    match (held, field("pid=")) {
        (Some(held), Some(pid)) => format!("{held} by pid {pid}"),
        (Some(held), None) => held,
        (None, _) => contents.to_owned(),
    }
}

/// Whether an existence-based lock no longer has a live owner.
///
/// An empty file records no claim — it is either an OS-mode release leftover
/// or a crash inside the legacy write window — but a just-created file may be
/// a legacy acquisition that has not written its owner yet, so only age rules
/// it out. A recorded PID is authoritative in both directions — but only when
/// the record can be attributed to *this* host: the legacy protocol runs
/// precisely on network shares, where the holder may be another machine whose
/// PIDs this one cannot probe (and whose PID numbers collide with local
/// ones). A record naming this host qualifies anywhere; a record with no host
/// (an older binary's) qualifies only on a local filesystem, where no other
/// machine could have written it. Everything else is reclaimed only by the
/// conservative 24-hour age.
pub(crate) fn lock_is_stale(path: &Path) -> bool {
    let contents = std::fs::read_to_string(path).unwrap_or_default();
    if contents.trim().is_empty() {
        return lock_age_exceeds(path, std::time::Duration::from_mins(1));
    }
    let field = |name: &str| {
        contents
            .split_whitespace()
            .find_map(|part| part.strip_prefix(name))
    };
    let probe_is_authoritative = match field("host=") {
        Some(recorded) => retro_junk_io::local_host_id() == Some(recorded),
        None => retro_junk_io::remote_mount_kind(path).is_none(),
    };
    if probe_is_authoritative
        && let Some(alive) = field("pid=")
            .and_then(|value| value.parse::<i32>().ok())
            .and_then(retro_junk_io::process_alive)
    {
        return !alive;
    }
    lock_age_exceeds(path, std::time::Duration::from_hours(24))
}

fn lock_age_exceeds(path: &Path, limit: std::time::Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
        .is_ok_and(|age| age > limit)
}
