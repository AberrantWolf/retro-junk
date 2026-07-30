use std::path::{Path, PathBuf};

use crate::manifest::{
    ArchiveRootManifest, BuildEvidence, CarrierManifest, DumpManifest, ManifestError,
    PhysicalCopyFileManifest, PhysicalCopyManifest, ReleaseFileManifest, ReleaseManifest,
    VerificationEvidence, read_build_json, read_toml_with_digest, read_verification_json,
};

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("archive root manifest is missing: {0}")]
    MissingRootManifest(String),
}

#[derive(Debug, Clone)]
pub struct ArchiveIndexSnapshot {
    pub root: PathBuf,
    pub manifest: ArchiveRootManifest,
    pub manifest_sha256: String,
    pub releases: Vec<IndexedRelease>,
}

#[derive(Debug, Clone)]
pub struct IndexedRelease {
    pub directory: PathBuf,
    pub manifest: ReleaseManifest,
    pub manifest_sha256: String,
    pub physical_copies: Vec<IndexedPhysicalCopy>,
    pub supporting_files: Vec<IndexedReleaseFile>,
}

#[derive(Debug, Clone)]
pub struct IndexedReleaseFile {
    pub directory: PathBuf,
    pub manifest: ReleaseFileManifest,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone)]
pub struct IndexedPhysicalCopy {
    pub directory: PathBuf,
    pub manifest: PhysicalCopyManifest,
    pub manifest_sha256: String,
    pub carriers: Vec<IndexedCarrier>,
    pub supporting_files: Vec<IndexedPhysicalCopyFile>,
}

#[derive(Debug, Clone)]
pub struct IndexedPhysicalCopyFile {
    pub directory: PathBuf,
    pub manifest: PhysicalCopyFileManifest,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone)]
pub struct IndexedCarrier {
    pub directory: PathBuf,
    pub manifest: CarrierManifest,
    pub manifest_sha256: String,
    pub dumps: Vec<IndexedDump>,
}

#[derive(Debug, Clone)]
pub struct IndexedDump {
    pub directory: PathBuf,
    pub manifest: DumpManifest,
    pub manifest_sha256: String,
    pub verifications: Vec<IndexedVerification>,
    pub builds: Vec<IndexedBuild>,
}

#[derive(Debug, Clone)]
pub struct IndexedVerification {
    pub path: PathBuf,
    pub evidence: VerificationEvidence,
}

#[derive(Debug, Clone)]
pub struct IndexedBuild {
    pub path: PathBuf,
    pub evidence: BuildEvidence,
}

pub fn scan_archive(root: &Path) -> Result<ArchiveIndexSnapshot, IndexError> {
    let root_path = crate::layout::root_manifest_path(root);
    if !root_path.is_file() {
        return Err(IndexError::MissingRootManifest(
            root_path.display().to_string(),
        ));
    }
    let (manifest, manifest_sha256) = read_toml_with_digest(&root_path)?;
    let mut release_directories = Vec::new();
    for platform_dir in child_directories(root)? {
        if platform_dir.file_name().and_then(|v| v.to_str()) == Some(".retro-junk") {
            continue;
        }
        for release_dir in child_directories(&platform_dir)? {
            if release_dir.join("release.toml").is_file() {
                release_directories.push(release_dir);
            }
        }
    }
    let mut releases = scan_releases(release_directories)?;
    releases.sort_by(|a, b| a.directory.cmp(&b.directory));
    Ok(ArchiveIndexSnapshot {
        root: root.to_path_buf(),
        manifest,
        manifest_sha256,
        releases,
    })
}

/// Read every release subtree, several at a time.
///
/// A whole-archive scan is thousands of small manifest reads, and on a network
/// share nearly all of that time is round-trip latency rather than transfer or
/// parse work — so overlapping the reads is what makes it fast. Releases are
/// independent and read-only here, and the caller sorts the result, so
/// concurrency changes only the wall clock.
fn scan_releases(directories: Vec<PathBuf>) -> Result<Vec<IndexedRelease>, IndexError> {
    if directories.len() < 2 {
        return directories
            .into_iter()
            .map(|directory| {
                let manifest_path = directory.join("release.toml");
                scan_release(&directory, &manifest_path)
            })
            .collect();
    }
    // More workers than cores on purpose: these threads are waiting on the
    // server, not computing.
    let workers = directories.len().min(16);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut collected = Vec::with_capacity(directories.len());
    let mut failure = None;
    std::thread::scope(|scope| {
        let directories = &directories;
        let next = &next;
        let handles = (0..workers)
            .map(|_| {
                scope.spawn(move || {
                    let mut scanned = Vec::new();
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(directory) = directories.get(index) else {
                            return Ok(scanned);
                        };
                        let manifest_path = directory.join("release.toml");
                        scanned.push(scan_release(directory, &manifest_path)?);
                    }
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            match handle.join() {
                Ok(Ok(scanned)) => collected.extend(scanned),
                Ok(Err(error)) => failure = failure.take().or(Some(error)),
                Err(_) => {
                    failure = failure.take().or(Some(IndexError::Io {
                        path: "archive scan".to_owned(),
                        source: std::io::Error::other("archive scan worker panicked"),
                    }));
                }
            }
        }
    });
    match failure {
        Some(error) => Err(error),
        None => Ok(collected),
    }
}

fn scan_release(directory: &Path, manifest_path: &Path) -> Result<IndexedRelease, IndexError> {
    let (manifest, manifest_sha256) = read_toml_with_digest(manifest_path)?;
    let mut physical_copies = Vec::new();
    let copies_dir = directory.join("physical-copies");
    if copies_dir.is_dir() {
        for copy_dir in child_directories(&copies_dir)? {
            let copy_manifest = copy_dir.join("physical-copy.toml");
            if copy_manifest.is_file() {
                physical_copies.push(scan_physical_copy(&copy_dir, &copy_manifest)?);
            }
        }
    }
    physical_copies.sort_by_key(|copy| copy.manifest.copy_number);
    let mut supporting_files = Vec::new();
    for category in ["artwork", "videos", "documents", "metadata"] {
        collect_release_files(&directory.join(category), &mut supporting_files)?;
    }
    supporting_files.sort_by(|a, b| a.directory.cmp(&b.directory));
    Ok(IndexedRelease {
        directory: directory.to_path_buf(),
        manifest,
        manifest_sha256,
        physical_copies,
        supporting_files,
    })
}

fn collect_release_files(
    directory: &Path,
    output: &mut Vec<IndexedReleaseFile>,
) -> Result<(), IndexError> {
    if !directory.is_dir() {
        return Ok(());
    }
    let manifest_path = directory.join("supporting-file.toml");
    if manifest_path.is_file() {
        let (manifest, manifest_sha256) = read_toml_with_digest(&manifest_path)?;
        output.push(IndexedReleaseFile {
            directory: directory.to_path_buf(),
            manifest,
            manifest_sha256,
        });
        return Ok(());
    }
    for child in child_directories(directory)? {
        collect_release_files(&child, output)?;
    }
    Ok(())
}

fn scan_physical_copy(
    directory: &Path,
    manifest_path: &Path,
) -> Result<IndexedPhysicalCopy, IndexError> {
    let (manifest, manifest_sha256) = read_toml_with_digest(manifest_path)?;
    let mut carriers = Vec::new();
    let carriers_dir = directory.join("carriers");
    if carriers_dir.is_dir() {
        for child in child_directories(&carriers_dir)? {
            let carrier_manifest = child.join("carrier.toml");
            if carrier_manifest.is_file() {
                carriers.push(scan_carrier(&child, &carrier_manifest)?);
            }
        }
    }
    carriers.sort_by(|a, b| a.directory.cmp(&b.directory));
    let mut supporting_files = Vec::new();
    for category in ["photos", "provenance", "documents"] {
        collect_physical_copy_files(&directory.join(category), &mut supporting_files)?;
    }
    supporting_files.sort_by(|a, b| a.directory.cmp(&b.directory));
    Ok(IndexedPhysicalCopy {
        directory: directory.to_path_buf(),
        manifest,
        manifest_sha256,
        carriers,
        supporting_files,
    })
}

fn collect_physical_copy_files(
    directory: &Path,
    output: &mut Vec<IndexedPhysicalCopyFile>,
) -> Result<(), IndexError> {
    if !directory.is_dir() {
        return Ok(());
    }
    let manifest_path = directory.join("supporting-file.toml");
    if manifest_path.is_file() {
        let (manifest, manifest_sha256) = read_toml_with_digest(&manifest_path)?;
        output.push(IndexedPhysicalCopyFile {
            directory: directory.to_path_buf(),
            manifest,
            manifest_sha256,
        });
        return Ok(());
    }
    for child in child_directories(directory)? {
        collect_physical_copy_files(&child, output)?;
    }
    Ok(())
}

fn scan_carrier(directory: &Path, manifest_path: &Path) -> Result<IndexedCarrier, IndexError> {
    let (manifest, manifest_sha256) = read_toml_with_digest(manifest_path)?;
    let mut dumps = Vec::new();
    let dumps_dir = directory.join("dumps");
    if dumps_dir.is_dir() {
        for child in child_directories(&dumps_dir)? {
            let dump_manifest = child.join("dump.toml");
            if dump_manifest.is_file() {
                let (manifest, manifest_sha256) = read_toml_with_digest(&dump_manifest)?;
                let (verifications, builds) = scan_evidence(&child)?;
                dumps.push(IndexedDump {
                    directory: child,
                    manifest,
                    manifest_sha256,
                    verifications,
                    builds,
                });
            }
        }
    }
    dumps.sort_by(|a, b| a.directory.cmp(&b.directory));
    Ok(IndexedCarrier {
        directory: directory.to_path_buf(),
        manifest,
        manifest_sha256,
        dumps,
    })
}

fn scan_evidence(
    dump_directory: &Path,
) -> Result<(Vec<IndexedVerification>, Vec<IndexedBuild>), IndexError> {
    let evidence_directory = dump_directory.join("evidence");
    if !evidence_directory.is_dir() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut verifications = Vec::new();
    let mut builds = Vec::new();
    let mut paths = std::fs::read_dir(&evidence_directory)
        .map_err(|source| IndexError::Io {
            path: evidence_directory.display().to_string(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let is_json = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
        if name.starts_with("verification-") && is_json {
            verifications.push(IndexedVerification {
                evidence: read_verification_json(&path)?,
                path,
            });
        } else if name.starts_with("build-") && is_json {
            builds.push(IndexedBuild {
                evidence: read_build_json(&path)?,
                path,
            });
        }
    }
    Ok((verifications, builds))
}

fn child_directories(path: &Path) -> Result<Vec<PathBuf>, IndexError> {
    let mut directories = std::fs::read_dir(path)
        .map_err(|source| IndexError::Io {
            path: path.display().to_string(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|child| child.is_dir())
        .collect::<Vec<_>>();
    directories.sort();
    Ok(directories)
}
