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
    let NewReleaseFile {
        release_id,
        source_file,
        category,
        asset_type,
        source,
        source_url,
        caption,
    } = request;
    if !source_file.is_file() {
        return Err(SupportingFileError::InvalidSource(
            source_file.display().to_string(),
        ));
    }
    let snapshot = crate::scan_archive(archive_root)?;
    let release = snapshot
        .releases
        .into_iter()
        .find(|release| release.manifest.archive_release_id == release_id)
        .ok_or(SupportingFileError::ReleaseNotFound(release_id))?;
    let supporting_file_id = SupportingFileId::new();
    let destination = release
        .directory
        .join(release_category_path(category))
        .join(slugify(asset_type))
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
    let filename = source_file
        .file_name()
        .ok_or_else(|| SupportingFileError::InvalidSource(source_file.display().to_string()))?;
    let target = original.join(filename);
    let result = (|| {
        std::fs::copy(source_file, &target).map_err(|source| SupportingFileError::Io {
            path: target.display().to_string(),
            source,
        })?;
        let (source_size, source_hash) = sha256_file(source_file, cancel)?;
        let (target_size, target_hash) = sha256_file(&target, cancel)?;
        if source_size != target_size || source_hash != target_hash {
            return Err(crate::IngestError::CopyMismatch(source_file.display().to_string()).into());
        }
        let relative = normalize_relative_path(Path::new("original").join(filename).as_path())?;
        let manifest = ReleaseFileManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            supporting_file_id,
            archive_release_id: release_id,
            category,
            asset_type: asset_type.to_owned(),
            captured_at: chrono::Utc::now().to_rfc3339(),
            source: source.to_owned(),
            source_url: source_url.to_owned(),
            caption: caption.to_owned(),
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
