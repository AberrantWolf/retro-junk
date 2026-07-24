use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::{
    ArchiveReleaseId, ArchivedFile, MANIFEST_SCHEMA_VERSION, PhysicalCopyFileCategory,
    PhysicalCopyFileManifest, PhysicalCopyId, ReleaseFileCategory, ReleaseFileManifest,
    SupportingFileId, normalize_relative_path, sha256_file, slugify, write_toml_atomic,
};

#[derive(Debug, thiserror::Error)]
pub enum SupportingFileError {
    #[error("release {0} was not found in the archive")]
    ReleaseNotFound(ArchiveReleaseId),
    #[error("supporting-file source is not a regular file: {0}")]
    InvalidSource(String),
    #[error("asset destination already exists: {0}")]
    TargetExists(String),
    #[error(transparent)]
    Manifest(#[from] crate::ManifestError),
    #[error(transparent)]
    Index(#[from] crate::index::IndexError),
    #[error(transparent)]
    Ingest(#[from] crate::IngestError),
    #[error(transparent)]
    Layout(#[from] crate::layout::LayoutError),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

#[derive(Clone, Copy)]
pub struct NewReleaseFile<'a> {
    pub release_id: ArchiveReleaseId,
    pub source_file: &'a Path,
    pub category: ReleaseFileCategory,
    pub asset_type: &'a str,
    pub source: &'a str,
    pub source_url: &'a str,
    pub caption: &'a str,
}

#[derive(Clone, Copy)]
pub struct NewPhysicalCopyFile<'a> {
    pub physical_copy_id: PhysicalCopyId,
    pub source_file: &'a Path,
    pub category: PhysicalCopyFileCategory,
    pub asset_type: &'a str,
    pub source: &'a str,
    pub caption: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedReleaseFile {
    pub destination: PathBuf,
    pub added: bool,
}

pub fn add_physical_copy_file(
    archive_root: &Path,
    request: NewPhysicalCopyFile<'_>,
    cancel: &AtomicBool,
) -> Result<PathBuf, SupportingFileError> {
    if !request.source_file.is_file() {
        return Err(SupportingFileError::InvalidSource(
            request.source_file.display().to_string(),
        ));
    }
    let snapshot = crate::scan_archive(archive_root)?;
    let physical_copy = snapshot
        .releases
        .into_iter()
        .flat_map(|release| release.physical_copies)
        .find(|copy| copy.manifest.physical_copy_id == request.physical_copy_id)
        .ok_or(SupportingFileError::InvalidSource(format!(
            "physical copy {} was not found",
            request.physical_copy_id
        )))?;
    let supporting_file_id = SupportingFileId::new();
    let category = physical_copy_category_path(request.category);
    let destination = physical_copy
        .directory
        .join(category)
        .join(slugify(request.asset_type))
        .join(format!("file-{supporting_file_id}"));
    let parent = destination
        .parent()
        .expect("supporting-file destination has parent");
    std::fs::create_dir_all(parent).map_err(|source| SupportingFileError::Io {
        path: parent.display().to_string(),
        source,
    })?;
    let staging = parent.join(format!(".file-{supporting_file_id}.staging"));
    let original = staging.join("original");
    std::fs::create_dir_all(&original).map_err(|source| SupportingFileError::Io {
        path: original.display().to_string(),
        source,
    })?;
    let result = (|| {
        let filename = request.source_file.file_name().ok_or_else(|| {
            SupportingFileError::InvalidSource(request.source_file.display().to_string())
        })?;
        let target = original.join(filename);
        std::fs::copy(request.source_file, &target).map_err(|source| SupportingFileError::Io {
            path: target.display().to_string(),
            source,
        })?;
        let (source_size, source_hash) = sha256_file(request.source_file, cancel)?;
        let (target_size, target_hash) = sha256_file(&target, cancel)?;
        if source_size != target_size || source_hash != target_hash {
            return Err(crate::IngestError::CopyMismatch(
                request.source_file.display().to_string(),
            )
            .into());
        }
        let manifest = PhysicalCopyFileManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            supporting_file_id,
            physical_copy_id: request.physical_copy_id,
            category: request.category,
            asset_type: request.asset_type.to_owned(),
            captured_at: chrono::Utc::now().to_rfc3339(),
            source: request.source.to_owned(),
            caption: request.caption.to_owned(),
            file: ArchivedFile {
                path: normalize_relative_path(Path::new("original").join(filename).as_path())?,
                size: target_size,
                crc32: String::new(),
                md5: String::new(),
                sha1: String::new(),
                sha256: target_hash,
            },
        };
        write_toml_atomic(&staging.join("supporting-file.toml"), &manifest)?;
        std::fs::rename(&staging, &destination).map_err(|source| SupportingFileError::Io {
            path: destination.display().to_string(),
            source,
        })?;
        Ok(destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

pub fn add_release_file(
    archive_root: &Path,
    request: NewReleaseFile<'_>,
    cancel: &AtomicBool,
) -> Result<PathBuf, SupportingFileError> {
    add_release_files(archive_root, &[request], cancel).map(|mut files| {
        files
            .pop()
            .expect("one release-file request produces one result")
            .destination
    })
}

/// Add several release-level supporting files with one archive index scan.
///
/// A file whose SHA-256, category, and asset type already exist on the release
/// is returned with `added == false`. This makes adopting frontend artwork a
/// cheap, idempotent operation while preserving the archive copy as authority.
pub fn add_release_files(
    archive_root: &Path,
    requests: &[NewReleaseFile<'_>],
    cancel: &AtomicBool,
) -> Result<Vec<AddedReleaseFile>, SupportingFileError> {
    if requests.is_empty() {
        return Ok(Vec::new());
    }
    for request in requests {
        if !request.source_file.is_file() {
            return Err(SupportingFileError::InvalidSource(
                request.source_file.display().to_string(),
            ));
        }
    }
    let snapshot = crate::scan_archive(archive_root)?;
    let mut release_directories = HashMap::new();
    let mut existing = HashMap::<(ArchiveReleaseId, &'static str, String, String), PathBuf>::new();
    for release in snapshot.releases {
        let release_id = release.manifest.archive_release_id;
        for file in release.supporting_files {
            existing.insert(
                (
                    release_id,
                    release_category_path(file.manifest.category),
                    file.manifest.asset_type.trim().to_ascii_lowercase(),
                    file.manifest.file.sha256.clone(),
                ),
                file.directory,
            );
        }
        release_directories.insert(release_id, release.directory);
    }

    let mut output = Vec::with_capacity(requests.len());
    for request in requests {
        let (source_size, source_hash) = sha256_file(request.source_file, cancel)?;
        let key = (
            request.release_id,
            release_category_path(request.category),
            request.asset_type.trim().to_ascii_lowercase(),
            source_hash.clone(),
        );
        if let Some(destination) = existing.get(&key) {
            output.push(AddedReleaseFile {
                destination: destination.clone(),
                added: false,
            });
            continue;
        }
        let release_directory = release_directories
            .get(&request.release_id)
            .ok_or(SupportingFileError::ReleaseNotFound(request.release_id))?;
        let destination = add_release_file_at(
            release_directory,
            request,
            source_size,
            &source_hash,
            cancel,
        )?;
        existing.insert(key, destination.clone());
        output.push(AddedReleaseFile {
            destination,
            added: true,
        });
    }
    Ok(output)
}

fn add_release_file_at(
    release_directory: &Path,
    request: &NewReleaseFile<'_>,
    source_size: u64,
    source_hash: &str,
    cancel: &AtomicBool,
) -> Result<PathBuf, SupportingFileError> {
    let release_id = request.release_id;
    let supporting_file_id = SupportingFileId::new();
    let destination = release_directory
        .join(release_category_path(request.category))
        .join(slugify(request.asset_type))
        .join(format!("file-{supporting_file_id}"));
    let parent = destination
        .parent()
        .expect("supporting-file destination has parent");
    std::fs::create_dir_all(parent).map_err(|source| SupportingFileError::Io {
        path: parent.display().to_string(),
        source,
    })?;
    let staging = parent.join(format!(".file-{supporting_file_id}.staging"));
    if destination.exists() || staging.exists() {
        return Err(SupportingFileError::TargetExists(
            destination.display().to_string(),
        ));
    }
    let original = staging.join("original");
    std::fs::create_dir_all(&original).map_err(|source| SupportingFileError::Io {
        path: original.display().to_string(),
        source,
    })?;
    let filename = request.source_file.file_name().ok_or_else(|| {
        SupportingFileError::InvalidSource(request.source_file.display().to_string())
    })?;
    let target = original.join(filename);
    let result = (|| {
        std::fs::copy(request.source_file, &target).map_err(|source| SupportingFileError::Io {
            path: target.display().to_string(),
            source,
        })?;
        let (target_size, target_hash) = sha256_file(&target, cancel)?;
        if source_size != target_size || source_hash != target_hash {
            return Err(crate::IngestError::CopyMismatch(
                request.source_file.display().to_string(),
            )
            .into());
        }
        let relative = normalize_relative_path(Path::new("original").join(filename).as_path())?;
        let manifest = ReleaseFileManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            supporting_file_id,
            archive_release_id: release_id,
            category: request.category,
            asset_type: request.asset_type.to_owned(),
            captured_at: chrono::Utc::now().to_rfc3339(),
            source: request.source.to_owned(),
            source_url: request.source_url.to_owned(),
            caption: request.caption.to_owned(),
            file: ArchivedFile {
                path: relative,
                size: target_size,
                crc32: String::new(),
                md5: String::new(),
                sha1: String::new(),
                sha256: target_hash,
            },
        };
        write_toml_atomic(&staging.join("supporting-file.toml"), &manifest)?;
        std::fs::rename(&staging, &destination).map_err(|source| SupportingFileError::Io {
            path: destination.display().to_string(),
            source,
        })?;
        Ok(destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

const fn release_category_path(category: ReleaseFileCategory) -> &'static str {
    match category {
        ReleaseFileCategory::Artwork => "artwork",
        ReleaseFileCategory::Video => "videos",
        ReleaseFileCategory::Document => "documents",
        ReleaseFileCategory::Metadata => "metadata",
    }
}

const fn physical_copy_category_path(category: PhysicalCopyFileCategory) -> &'static str {
    match category {
        PhysicalCopyFileCategory::Photo => "photos",
        PhysicalCopyFileCategory::Provenance => "provenance",
        PhysicalCopyFileCategory::Document => "documents",
    }
}
