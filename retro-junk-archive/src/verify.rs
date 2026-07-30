use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ingest::{IngestError, hash_file};
use crate::manifest::DumpManifest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityFailure {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityReport {
    pub checked_files: usize,
    pub checked_bytes: u64,
    pub failures: Vec<IntegrityFailure>,
}

impl IntegrityReport {
    #[must_use]
    pub fn is_verified(&self) -> bool {
        self.failures.is_empty()
    }
}

pub fn verify_dump_integrity(
    dump_dir: &Path,
    manifest: &DumpManifest,
    cancel: &AtomicBool,
) -> Result<IntegrityReport, IngestError> {
    let raw_dir = dump_dir.join("raw");
    let mut report = IntegrityReport {
        checked_files: 0,
        checked_bytes: 0,
        failures: Vec::new(),
    };
    let expected_paths = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut actual_paths = Vec::new();
    collect_raw_files(&raw_dir, &raw_dir, &mut actual_paths, cancel)?;
    for actual in actual_paths {
        if !expected_paths.contains(actual.as_str()) {
            report.failures.push(IntegrityFailure {
                path: actual,
                reason: "unexpected file is not recorded by the dump manifest".to_owned(),
            });
        }
    }
    for expected in &manifest.files {
        let path: PathBuf = raw_dir.join(&expected.path);
        match hash_file(&path, cancel) {
            Ok((size, sha256)) => {
                report.checked_files += 1;
                report.checked_bytes += size;
                if size != expected.size {
                    report.failures.push(IntegrityFailure {
                        path: expected.path.clone(),
                        reason: format!("size mismatch: expected {}, found {size}", expected.size),
                    });
                } else if sha256 != expected.sha256 {
                    report.failures.push(IntegrityFailure {
                        path: expected.path.clone(),
                        reason: "SHA-256 mismatch".to_owned(),
                    });
                }
            }
            Err(IngestError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                report.failures.push(IntegrityFailure {
                    path: expected.path.clone(),
                    reason: "file is not present on this device".to_owned(),
                });
            }
            Err(error) => return Err(error),
        }
    }
    Ok(report)
}

fn collect_raw_files(
    root: &Path,
    directory: &Path,
    output: &mut Vec<String>,
    cancel: &AtomicBool,
) -> Result<(), IngestError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(IngestError::Cancelled);
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(IngestError::Io {
                path: directory.display().to_string(),
                source,
            });
        }
    };
    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            return Err(IngestError::Cancelled);
        }
        let entry = entry.map_err(|source| IngestError::Io {
            path: directory.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| IngestError::Io {
            path: path.display().to_string(),
            source,
        })?;
        if file_type.is_dir() {
            collect_raw_files(root, &path, output, cancel)?;
        } else if !retro_junk_io::is_noise_path(&path) {
            // A sidecar the host filesystem wrote beside a dump file is not
            // dump content. Counting it would report every mirror of the
            // archive on exFAT or SMB as an integrity failure, and would bake
            // host metadata into the manifest on ingest.
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let normalized = crate::normalize_relative_path(relative)?;
            output.push(normalized);
        }
    }
    output.sort();
    Ok(())
}

pub fn sha256_file(path: &Path, cancel: &AtomicBool) -> Result<(u64, String), IngestError> {
    hash_file(path, cancel)
}
