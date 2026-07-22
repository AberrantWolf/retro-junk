use std::io::Write;
use std::path::{Path, PathBuf};

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

/// Process-level guard for manifest/evidence mutations. A crashed process's
/// lock is reclaimed when its PID is demonstrably absent on Linux, or after a
/// conservative 24-hour age elsewhere.
pub struct ArchiveLock {
    path: PathBuf,
}

impl ArchiveLock {
    pub fn acquire(root: &Path) -> Result<Self, ArchiveLockError> {
        if !root.join("retro-junk-archive.toml").is_file() {
            return Err(ArchiveLockError::Uninitialized(root.display().to_string()));
        }
        let state = root.join(".retro-junk");
        std::fs::create_dir_all(&state).map_err(|source| ArchiveLockError::Io {
            path: state.display().to_string(),
            source,
        })?;
        let path = state.join("archive.lock");
        if path.exists() && lock_is_stale(&path) {
            std::fs::remove_file(&path).map_err(|source| ArchiveLockError::Io {
                path: path.display().to_string(),
                source,
            })?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|source| {
                if source.kind() == std::io::ErrorKind::AlreadyExists {
                    ArchiveLockError::Busy(
                        std::fs::read_to_string(&path)
                            .unwrap_or_else(|_| path.display().to_string()),
                    )
                } else {
                    ArchiveLockError::Io {
                        path: path.display().to_string(),
                        source,
                    }
                }
            })?;
        writeln!(
            file,
            "pid={} started_at={}",
            std::process::id(),
            chrono::Utc::now().to_rfc3339()
        )
        .map_err(|source| ArchiveLockError::Io {
            path: path.display().to_string(),
            source,
        })?;
        file.sync_all().map_err(|source| ArchiveLockError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Ok(Self { path })
    }
}

impl Drop for ArchiveLock {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.path) {
            log::warn!(
                "could not release archive lock {}: {error}",
                self.path.display()
            );
        }
    }
}

fn lock_is_stale(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let contents = std::fs::read_to_string(path).unwrap_or_default();
        if let Some(pid) = contents
            .split_whitespace()
            .find_map(|part| part.strip_prefix("pid="))
            .and_then(|value| value.parse::<u32>().ok())
        {
            return !Path::new("/proc").join(pid.to_string()).exists();
        }
    }
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
        .is_ok_and(|age| age > std::time::Duration::from_hours(24))
}
