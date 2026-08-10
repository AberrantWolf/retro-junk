use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

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
    #[error("archive scan cancelled")]
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct ArchiveIndexSnapshot {
    pub root: PathBuf,
    pub manifest: ArchiveRootManifest,
    pub manifest_sha256: String,
    /// Digest of every authoritative metadata file represented by this scan.
    pub source_fingerprint: String,
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
    scan_archive_inner(root, None)
}

/// Scan the archive while honoring cancellation between release subtrees.
pub fn scan_archive_cancellable(
    root: &Path,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<ArchiveIndexSnapshot, IndexError> {
    scan_archive_inner(root, Some(cancel))
}

/// Fingerprint the authoritative metadata tree without parsing it or touching
/// preservation/playable payloads.
///
/// The generation marker is a fast hint for mutations made through this
/// application. It cannot observe a person correcting a TOML file by hand, so
/// projection freshness also compares this content digest. Relative paths are
/// included, making additions, removals and renames observable as well as
/// edits. Hidden maintenance/backup directories are deliberately excluded.
pub fn projection_source_fingerprint(root: &Path) -> Result<String, IndexError> {
    let mut paths = Vec::new();
    collect_projection_sources(root, &mut paths)?;
    paths.sort();

    let mut tree = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    for path in paths {
        let relative = path.strip_prefix(root).unwrap_or(&path);
        tree.update(relative.to_string_lossy().as_bytes());
        tree.update([0]);

        let mut file = std::fs::File::open(&path).map_err(|source| IndexError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let mut contents = Sha256::new();
        loop {
            let read =
                std::io::Read::read(&mut file, &mut buffer).map_err(|source| IndexError::Io {
                    path: path.display().to_string(),
                    source,
                })?;
            if read == 0 {
                break;
            }
            contents.update(&buffer[..read]);
        }
        tree.update(contents.finalize());
    }
    Ok(format!("{:x}", tree.finalize()))
}

fn collect_projection_sources(
    directory: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), IndexError> {
    let entries = std::fs::read_dir(directory).map_err(|source| IndexError::Io {
        path: directory.display().to_string(),
        source,
    })?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| IndexError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if file_type.is_dir() {
            if name.starts_with('.') {
                continue;
            }
            collect_projection_sources(&path, output)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let is_manifest = matches!(
            name.as_ref(),
            crate::layout::ROOT_MANIFEST_FILE
                | "release.toml"
                | "physical-copy.toml"
                | "carrier.toml"
                | "dump.toml"
                | "supporting-file.toml"
        );
        let is_evidence = path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|parent| parent == "evidence")
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            && (name.starts_with("verification-") || name.starts_with("build-"));
        if is_manifest || is_evidence {
            output.push(path);
        }
    }
    Ok(())
}

fn scan_archive_inner(
    root: &Path,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<ArchiveIndexSnapshot, IndexError> {
    let root_path = crate::layout::root_manifest_path(root);
    if !root_path.is_file() {
        return Err(IndexError::MissingRootManifest(
            root_path.display().to_string(),
        ));
    }
    let (manifest, manifest_sha256) = read_toml_with_digest(&root_path)?;
    let mut release_directories = Vec::new();
    for platform_dir in child_directories(root)? {
        if cancel.is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Relaxed)) {
            return Err(IndexError::Cancelled);
        }
        if platform_dir.file_name().and_then(|v| v.to_str()) == Some(".retro-junk") {
            continue;
        }
        for release_dir in child_directories(&platform_dir)? {
            if release_dir.join("release.toml").is_file() {
                release_directories.push(release_dir);
            }
        }
    }
    let mut releases = scan_releases(release_directories, cancel)?;
    releases.sort_by(|a, b| a.directory.cmp(&b.directory));
    Ok(ArchiveIndexSnapshot {
        root: root.to_path_buf(),
        manifest,
        manifest_sha256,
        source_fingerprint: projection_source_fingerprint(root)?,
        releases,
    })
}

/// Read one release subtree without walking every other release in the
/// archive.
///
/// Known mutations already carry the release id they changed. Making those
/// callers construct a whole [`ArchiveIndexSnapshot`] turned a one-file
/// artwork publication into thousands of unrelated manifest reads.
pub fn scan_archive_release(
    root: &Path,
    release_id: crate::ArchiveReleaseId,
) -> Result<IndexedRelease, IndexError> {
    for platform_dir in child_directories(root)? {
        if platform_dir.file_name().and_then(|value| value.to_str()) == Some(".retro-junk") {
            continue;
        }
        for release_dir in child_directories(&platform_dir)? {
            let manifest_path = release_dir.join("release.toml");
            if !manifest_path.is_file() {
                continue;
            }
            let manifest: ReleaseManifest = crate::read_toml(&manifest_path)?;
            if manifest.archive_release_id == release_id {
                return scan_release(&release_dir, &manifest_path);
            }
        }
    }
    Err(IndexError::Io {
        path: root.display().to_string(),
        source: std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("archive release {release_id} was not found"),
        ),
    })
}

/// Read every release subtree, several at a time.
///
/// A whole-archive scan is thousands of small manifest reads, and on a network
/// share nearly all of that time is round-trip latency rather than transfer or
/// parse work — so overlapping the reads is what makes it fast. Releases are
/// independent and read-only here, and the caller sorts the result, so
/// concurrency changes only the wall clock.
fn scan_releases(
    directories: Vec<PathBuf>,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<IndexedRelease>, IndexError> {
    if directories.len() < 2 {
        return directories
            .into_iter()
            .map(|directory| {
                if cancel.is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Relaxed)) {
                    return Err(IndexError::Cancelled);
                }
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
                        if cancel
                            .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Relaxed))
                        {
                            return Err(IndexError::Cancelled);
                        }
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
