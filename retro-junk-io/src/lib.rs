//! Network-friendly local staging and single-pass content hashing.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use sha1::Sha1;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const COPY_BUFFER_SIZE: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentDigests {
    pub size: u64,
    pub crc32: String,
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedFile {
    pub original_path: PathBuf,
    pub local_path: PathBuf,
    pub relative_path: String,
    pub digests: ContentDigests,
}

#[derive(Debug, Clone)]
pub struct PreparedPackage {
    lease: StagingLease,
    pub original_source: PathBuf,
    pub local_source: PathBuf,
    pub files: Vec<PreparedFile>,
    pub total_bytes: u64,
}

impl PreparedPackage {
    #[must_use]
    pub fn lease(&self) -> &StagingLease {
        &self.lease
    }
}

#[derive(Debug, Clone)]
pub struct StagingLease(Arc<StagingLeaseInner>);

impl StagingLease {
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.0.directory
    }
}

#[derive(Debug)]
struct StagingLeaseInner {
    directory: PathBuf,
}

impl Drop for StagingLeaseInner {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.directory)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log_cleanup_failure(&self.directory, &error);
        }
    }
}

fn log_cleanup_failure(path: &Path, error: &std::io::Error) {
    log::warn!(
        "failed to clean local staging directory {}: {error}",
        path.display()
    );
}

#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error("source is not a regular file or directory: {0}")]
    InvalidSource(String),
    #[error("symbolic links are not accepted in staged packages: {0}")]
    SymbolicLink(String),
    #[error("source changed while it was being staged: {0}")]
    SourceChanged(String),
    #[error("staging was cancelled")]
    Cancelled,
    #[error("unsafe relative source path: {0}")]
    UnsafePath(String),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceSnapshot {
    size: u64,
    modified: Option<SystemTime>,
}

/// Return the device-local default workspace for one collection profile.
#[must_use]
pub fn default_profile_workspace(profile_id: Uuid) -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("retro-junk/workspaces")
        .join(profile_id.to_string())
}

/// Return a device-local workspace for operations not associated with a
/// configured collection profile.
#[must_use]
pub fn default_transient_workspace() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("retro-junk/workspaces/transient")
}

/// Copy a file or directory package into a disposable local lease while
/// calculating every preservation digest in the same source read.
pub fn stage_package(
    source: &Path,
    workspace_root: &Path,
    cancel: &AtomicBool,
    mut on_bytes: impl FnMut(u64),
) -> Result<PreparedPackage, StageError> {
    let metadata = checked_metadata(source)?;
    let lease_directory = workspace_root
        .join("staging")
        .join(format!("stage-{}", Uuid::now_v7()));
    let package_directory = lease_directory.join("package");
    std::fs::create_dir_all(&package_directory).map_err(|source| StageError::Io {
        path: package_directory.display().to_string(),
        source,
    })?;
    let lease = StagingLease(Arc::new(StagingLeaseInner {
        directory: lease_directory,
    }));

    let paths = if metadata.is_file() {
        vec![(
            source.to_path_buf(),
            safe_relative(Path::new(source.file_name().unwrap_or_default()))?,
        )]
    } else if metadata.is_dir() {
        let mut paths = Vec::new();
        collect_files(source, source, &mut paths)?;
        paths
    } else {
        return Err(StageError::InvalidSource(source.display().to_string()));
    };

    let mut files = Vec::with_capacity(paths.len());
    let mut total_bytes = 0_u64;
    for (original_path, relative_path) in paths {
        if cancel.load(Ordering::Relaxed) {
            return Err(StageError::Cancelled);
        }
        let local_path = package_directory.join(&relative_path);
        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| StageError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let partial = local_path.with_extension(format!(
            "{}.partial",
            local_path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
        ));
        let digests = copy_and_hash(&original_path, &partial, cancel, &mut on_bytes)?;
        std::fs::rename(&partial, &local_path).map_err(|source| StageError::Io {
            path: local_path.display().to_string(),
            source,
        })?;
        total_bytes = total_bytes.saturating_add(digests.size);
        files.push(PreparedFile {
            original_path,
            local_path,
            relative_path: relative_path.to_string_lossy().replace('\\', "/"),
            digests,
        });
    }
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let local_source = if metadata.is_file() {
        files
            .first()
            .map(|file| file.local_path.clone())
            .unwrap_or(package_directory)
    } else {
        package_directory
    };
    Ok(PreparedPackage {
        lease,
        original_source: source.to_path_buf(),
        local_source,
        files,
        total_bytes,
    })
}

fn copy_and_hash(
    source_path: &Path,
    target_path: &Path,
    cancel: &AtomicBool,
    on_bytes: &mut impl FnMut(u64),
) -> Result<ContentDigests, StageError> {
    let before = snapshot(source_path)?;
    let mut source = File::open(source_path).map_err(|source| StageError::Io {
        path: source_path.display().to_string(),
        source,
    })?;
    let mut target = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(target_path)
        .map_err(|source| StageError::Io {
            path: target_path.display().to_string(),
            source,
        })?;
    let mut crc32 = crc32fast::Hasher::new();
    let mut md5 = md5::Context::new();
    let mut sha1 = Sha1::new();
    let mut sha256 = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; COPY_BUFFER_SIZE];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(StageError::Cancelled);
        }
        let count = source.read(&mut buffer).map_err(|source| StageError::Io {
            path: source_path.display().to_string(),
            source,
        })?;
        if count == 0 {
            break;
        }
        target
            .write_all(&buffer[..count])
            .map_err(|source| StageError::Io {
                path: target_path.display().to_string(),
                source,
            })?;
        size = size.saturating_add(count as u64);
        crc32.update(&buffer[..count]);
        md5.consume(&buffer[..count]);
        sha1.update(&buffer[..count]);
        sha256.update(&buffer[..count]);
        on_bytes(count as u64);
    }
    target.flush().map_err(|source| StageError::Io {
        path: target_path.display().to_string(),
        source,
    })?;
    if before != snapshot(source_path)? || before.size != size {
        return Err(StageError::SourceChanged(source_path.display().to_string()));
    }
    Ok(ContentDigests {
        size,
        crc32: format!("{:08x}", crc32.finalize()),
        md5: format!("{:x}", md5.compute()),
        sha1: format!("{:x}", sha1.finalize()),
        sha256: format!("{:x}", sha256.finalize()),
    })
}

fn collect_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), StageError> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|source| StageError::Io {
            path: directory.display().to_string(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| StageError::Io {
            path: directory.display().to_string(),
            source,
        })?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let metadata = checked_metadata(&path)?;
        if metadata.is_dir() {
            collect_files(root, &path, output)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| StageError::UnsafePath(path.display().to_string()))?;
            let relative = safe_relative(relative)?;
            output.push((path, relative));
        }
    }
    Ok(())
}

fn checked_metadata(path: &Path) -> Result<std::fs::Metadata, StageError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| StageError::Io {
        path: path.display().to_string(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(StageError::SymbolicLink(path.display().to_string()));
    }
    Ok(metadata)
}

fn snapshot(path: &Path) -> Result<SourceSnapshot, StageError> {
    let metadata = checked_metadata(path)?;
    Ok(SourceSnapshot {
        size: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn safe_relative(path: &Path) -> Result<PathBuf, StageError> {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => output.push(value),
            _ => return Err(StageError::UnsafePath(path.display().to_string())),
        }
    }
    if output.as_os_str().is_empty() {
        return Err(StageError::UnsafePath(path.display().to_string()));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_a_package_and_removes_it_when_the_last_lease_drops() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("game.bin"), b"network bytes").unwrap();
        std::fs::write(source.join("game.cue"), b"FILE game.bin BINARY").unwrap();
        let mut copied = 0;
        let package = stage_package(
            &source,
            &temp.path().join("work"),
            &AtomicBool::new(false),
            |count| copied += count,
        )
        .unwrap();
        assert_eq!(package.files.len(), 2);
        assert_eq!(copied, package.total_bytes);
        assert_eq!(
            std::fs::read(package.local_source.join("game.bin")).unwrap(),
            b"network bytes"
        );
        let lease_directory = package.lease().directory().to_path_buf();
        let clone = package.lease().clone();
        drop(package);
        assert!(lease_directory.exists());
        drop(clone);
        assert!(!lease_directory.exists());
    }

    #[test]
    fn stages_all_digests_in_one_copy_pass() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("game.rom");
        std::fs::write(&source, b"abc").unwrap();
        let package = stage_package(
            &source,
            &temp.path().join("work"),
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let digests = &package.files[0].digests;
        assert_eq!(digests.size, 3);
        assert_eq!(digests.crc32, "352441c2");
        assert_eq!(digests.md5, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(digests.sha1, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            digests.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
