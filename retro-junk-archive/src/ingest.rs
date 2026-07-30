use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::layout::normalize_relative_path;
use crate::manifest::{ArchivedFile, DumpManifest, ManifestError, write_toml_atomic};

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("source does not exist or is not a regular file/directory: {0}")]
    InvalidSource(String),
    #[error("symbolic links are not accepted into preservation masters: {0}")]
    SymbolicLink(String),
    #[error("archive target already exists: {0}")]
    TargetExists(String),
    #[error("ingest was cancelled")]
    Cancelled,
    #[error("source changed or staged copy did not match: {0}")]
    CopyMismatch(String),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error(transparent)]
    Layout(#[from] crate::layout::LayoutError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

#[derive(Debug, Clone)]
pub struct IngestPlan {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub files: Vec<PlannedFile>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct PlannedFile {
    pub source: PathBuf,
    pub relative_path: String,
    pub size: u64,
    pub expected_digests: Option<FileDigests>,
}

pub struct IngestRequest {
    pub plan: IngestPlan,
    pub manifest: DumpManifest,
    /// Re-read and hash every published file after the copy pass.
    ///
    /// The copy pass already digests the source stream and compares it with
    /// the planned digests, so this second full read mostly guards against
    /// storage lying about completed writes. On network mounts it doubles
    /// the transfer per package; automation policy exposes it as
    /// `verify_published_bytes` (default off), with the daemon's background
    /// integrity verification supplying the deferred read-back instead.
    pub verify_published_bytes: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct IngestProgress {
    pub copied_bytes: u64,
    pub total_bytes: u64,
}

pub fn plan_ingest(source: &Path, destination: &Path) -> Result<IngestPlan, IngestError> {
    if destination.exists() {
        return Err(IngestError::TargetExists(destination.display().to_string()));
    }
    let metadata = std::fs::symlink_metadata(source).map_err(|source_error| IngestError::Io {
        path: source.display().to_string(),
        source: source_error,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(IngestError::SymbolicLink(source.display().to_string()));
    }
    let mut files = Vec::new();
    if metadata.is_file() {
        let name = source
            .file_name()
            .ok_or_else(|| IngestError::InvalidSource(source.display().to_string()))?;
        files.push(PlannedFile {
            source: source.to_path_buf(),
            relative_path: normalize_relative_path(Path::new(name))?,
            size: metadata.len(),
            expected_digests: None,
        });
    } else if metadata.is_dir() {
        collect_files(source, source, &mut files)?;
    } else {
        return Err(IngestError::InvalidSource(source.display().to_string()));
    }
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let total_bytes = files.iter().map(|file| file.size).sum();
    Ok(IngestPlan {
        source: source.to_path_buf(),
        destination: destination.to_path_buf(),
        files,
        total_bytes,
    })
}

fn collect_files(
    root: &Path,
    dir: &Path,
    output: &mut Vec<PlannedFile>,
) -> Result<(), IngestError> {
    let mut entries = std::fs::read_dir(dir)
        .map_err(|source| IngestError::Io {
            path: dir.display().to_string(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| IngestError::Io {
            path: dir.display().to_string(),
            source,
        })?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|source| IngestError::Io {
            path: path.display().to_string(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(IngestError::SymbolicLink(path.display().to_string()));
        }
        if metadata.is_dir() {
            collect_files(root, &path, output)?;
        } else if metadata.is_file() && !retro_junk_io::is_noise_path(&path) {
            // Host-filesystem sidecars beside a dump are not part of it; a
            // package staged from exFAT or SMB must archive the same bytes as
            // one staged from APFS.
            let relative = path
                .strip_prefix(root)
                .map_err(|_| IngestError::InvalidSource(path.display().to_string()))?;
            let relative_path = normalize_relative_path(relative)?;
            output.push(PlannedFile {
                source: path,
                relative_path,
                size: metadata.len(),
                expected_digests: None,
            });
        }
    }
    Ok(())
}

pub fn execute_ingest(
    mut request: IngestRequest,
    cancel: &AtomicBool,
    on_progress: impl Fn(IngestProgress),
) -> Result<DumpManifest, IngestError> {
    let destination = request.plan.destination.clone();
    if destination.exists() {
        return Err(IngestError::TargetExists(destination.display().to_string()));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| IngestError::InvalidSource(destination.display().to_string()))?;
    std::fs::create_dir_all(parent).map_err(|source| IngestError::Io {
        path: parent.display().to_string(),
        source,
    })?;
    let name = destination
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("dump");
    let staging = parent.join(format!(".{name}.staging-{}", request.manifest.dump_id));
    if staging.exists() {
        return Err(IngestError::TargetExists(staging.display().to_string()));
    }
    std::fs::create_dir(&staging).map_err(|source| IngestError::Io {
        path: staging.display().to_string(),
        source,
    })?;
    let result = ingest_into_staging(&mut request, &staging, cancel, on_progress);
    match result {
        Ok(()) => {
            std::fs::rename(&staging, &destination).map_err(|source| IngestError::Io {
                path: destination.display().to_string(),
                source,
            })?;
            // The rename published the dump. Flushing the parent directory
            // only makes that durable sooner, so a filesystem that refuses it
            // must not turn a committed package into a rolled-back import.
            if let Err(error) = crate::manifest::sync_directory(parent) {
                log::warn!(
                    "published {} but could not flush its parent directory: {error}",
                    destination.display()
                );
            }
            Ok(request.manifest)
        }
        Err(error) => {
            if let Err(cleanup) = std::fs::remove_dir_all(&staging) {
                log::warn!(
                    "failed to clean ingest staging {}: {cleanup}",
                    staging.display()
                );
            }
            Err(error)
        }
    }
}

fn ingest_into_staging(
    request: &mut IngestRequest,
    staging: &Path,
    cancel: &AtomicBool,
    on_progress: impl Fn(IngestProgress),
) -> Result<(), IngestError> {
    let raw = staging.join("raw");
    std::fs::create_dir(&raw).map_err(|source| IngestError::Io {
        path: raw.display().to_string(),
        source,
    })?;
    let mut copied_bytes = 0_u64;
    let mut archived = Vec::with_capacity(request.plan.files.len());
    for planned in &request.plan.files {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let target = raw.join(&planned.relative_path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|source| IngestError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        let source_hashes = copy_hashing(&planned.source, &target, cancel, |bytes| {
            copied_bytes += bytes;
            on_progress(IngestProgress {
                copied_bytes,
                total_bytes: request.plan.total_bytes,
            });
        })?;
        if source_hashes.size != planned.size
            || planned
                .expected_digests
                .as_ref()
                .is_some_and(|expected| expected != &source_hashes)
        {
            return Err(IngestError::CopyMismatch(planned.relative_path.clone()));
        }
        if request.verify_published_bytes {
            let (target_size, target_sha256) = hash_file(&target, cancel)?;
            if target_size != source_hashes.size || target_sha256 != source_hashes.sha256 {
                return Err(IngestError::CopyMismatch(planned.relative_path.clone()));
            }
        }
        archived.push(ArchivedFile {
            path: planned.relative_path.clone(),
            size: source_hashes.size,
            crc32: source_hashes.crc32,
            md5: source_hashes.md5,
            sha1: source_hashes.sha1,
            sha256: source_hashes.sha256,
        });
    }
    request.manifest.files = archived;
    write_toml_atomic(&staging.join("dump.toml"), &request.manifest)?;
    Ok(())
}

fn copy_hashing(
    source_path: &Path,
    target_path: &Path,
    cancel: &AtomicBool,
    mut on_bytes: impl FnMut(u64),
) -> Result<FileDigests, IngestError> {
    let mut source = File::open(source_path).map_err(|source| IngestError::Io {
        path: source_path.display().to_string(),
        source,
    })?;
    let mut target = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target_path)
        .map_err(|source| IngestError::Io {
            path: target_path.display().to_string(),
            source,
        })?;
    let mut sha256 = Sha256::new();
    let mut sha1 = Sha1::new();
    let mut md5 = md5::Context::new();
    let mut crc32 = crc32fast::Hasher::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; 8 * 1024 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let count = source.read(&mut buffer).map_err(|source| IngestError::Io {
            path: source_path.display().to_string(),
            source,
        })?;
        if count == 0 {
            break;
        }
        target
            .write_all(&buffer[..count])
            .map_err(|source| IngestError::Io {
                path: target_path.display().to_string(),
                source,
            })?;
        size = size.saturating_add(count as u64);
        sha256.update(&buffer[..count]);
        sha1.update(&buffer[..count]);
        md5.consume(&buffer[..count]);
        crc32.update(&buffer[..count]);
        on_bytes(count as u64);
    }
    target.sync_all().map_err(|source| IngestError::Io {
        path: target_path.display().to_string(),
        source,
    })?;
    Ok(FileDigests {
        size,
        crc32: format!("{:08x}", crc32.finalize()),
        md5: format!("{:x}", md5.compute()),
        sha1: format!("{:x}", sha1.finalize()),
        sha256: format!("{:x}", sha256.finalize()),
    })
}

pub(crate) fn hash_file(path: &Path, cancel: &AtomicBool) -> Result<(u64, String), IngestError> {
    let mut file = File::open(path).map_err(|source| IngestError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut sha256 = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; 8 * 1024 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let count = file.read(&mut buffer).map_err(|source| IngestError::Io {
            path: path.display().to_string(),
            source,
        })?;
        if count == 0 {
            break;
        }
        size = size.saturating_add(count as u64);
        sha256.update(&buffer[..count]);
    }
    Ok((size, format!("{:x}", sha256.finalize())))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDigests {
    pub size: u64,
    pub crc32: String,
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}

pub fn hash_file_digests(path: &Path, cancel: &AtomicBool) -> Result<FileDigests, IngestError> {
    let mut file = File::open(path).map_err(|source| IngestError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut sha256 = Sha256::new();
    let mut sha1 = Sha1::new();
    let mut md5 = md5::Context::new();
    let mut crc32 = crc32fast::Hasher::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; 8 * 1024 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let count = file.read(&mut buffer).map_err(|source| IngestError::Io {
            path: path.display().to_string(),
            source,
        })?;
        if count == 0 {
            break;
        }
        size += count as u64;
        sha256.update(&buffer[..count]);
        sha1.update(&buffer[..count]);
        md5.consume(&buffer[..count]);
        crc32.update(&buffer[..count]);
    }
    Ok(FileDigests {
        size,
        crc32: format!("{:08x}", crc32.finalize()),
        md5: format!("{:x}", md5.compute()),
        sha1: format!("{:x}", sha1.finalize()),
        sha256: format!("{:x}", sha256.finalize()),
    })
}
