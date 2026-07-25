use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::{
    ArchiveLayout, ArchiveProfileId, ArchiveReleaseId, ArchiveRootManifest, CarrierId, CarrierKind,
    CarrierManifest, CatalogBinding, DumpManifest, IngestError, IngestProgress, IngestRequest,
    MANIFEST_SCHEMA_VERSION, PhysicalCopyId, PhysicalCopyManifest, ReleaseManifest,
    RepresentationFormat, execute_ingest, plan_ingest, read_toml, write_toml_atomic,
};

#[derive(Debug, thiserror::Error)]
pub enum CollectionError {
    #[error("archive root already belongs to profile {found}, not {expected}")]
    ProfileMismatch {
        found: ArchiveProfileId,
        expected: ArchiveProfileId,
    },
    #[error("archive root is not initialized: {0}")]
    NotInitialized(String),
    #[error("archive path already contains non-archive data: {0}")]
    NonEmptyRoot(String),
    #[error(
        "carrier matched catalog work {found}, but its archived parent is already bound to {expected}"
    )]
    CatalogWorkConflict { found: String, expected: String },
    #[error(transparent)]
    InvalidPackage(#[from] RedumperPackageError),
    #[error(transparent)]
    Manifest(#[from] crate::ManifestError),
    #[error(transparent)]
    Ingest(#[from] IngestError),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum RedumperPackageError {
    #[error("I/O error while inspecting Redumper package {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error(
        "{path} contains multiple Redumper images ({images}) — put each image's files in a separate subdirectory and import their parent directory"
    )]
    MultipleImages { path: String, images: String },
}

#[derive(Debug, Clone)]
pub struct NewCarrierDump {
    pub platform_id: String,
    pub title: String,
    pub region: String,
    pub revision: String,
    pub variant: String,
    pub owner_id: String,
    pub physical_copy_label: String,
    pub serial: String,
    pub sequence_number: u32,
    pub carrier_label: String,
    pub carrier_kind: CarrierKind,
    pub format: RepresentationFormat,
    pub catalog_binding: CatalogBinding,
    pub source_package: crate::SourcePackageRecord,
    /// Digests calculated while staging the source. When supplied, archive
    /// copy reads must reproduce them before publication.
    pub expected_files: Vec<ExpectedSourceFile>,
    pub physical_copy_id: Option<PhysicalCopyId>,
}

#[derive(Debug, Clone)]
pub struct ExpectedSourceFile {
    pub relative_path: String,
    pub digests: crate::FileDigests,
}

#[derive(Debug, Clone)]
pub struct IngestedCarrierDump {
    pub release: ReleaseManifest,
    pub physical_copy: PhysicalCopyManifest,
    pub carrier: CarrierManifest,
    pub dump: DumpManifest,
    pub dump_directory: PathBuf,
}

/// Reject a directory that combines raw capture files from multiple discs.
///
/// This is public so import planning can report the same boundary violation
/// before the caller commits to an ingest.
pub fn validate_redumper_package(source: &Path) -> Result<(), RedumperPackageError> {
    if !source.is_dir() {
        return Ok(());
    }
    let entries = std::fs::read_dir(source).map_err(|source_error| RedumperPackageError::Io {
        path: source.display().to_string(),
        source: source_error,
    })?;
    let mut image_names = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|source_error| RedumperPackageError::Io {
            path: source.display().to_string(),
            source: source_error,
        })?;
        let path = entry.path();
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if matches!(
            extension.to_ascii_lowercase().as_str(),
            "scram" | "scrap" | "sdram" | "sbram"
        ) && let Some(stem) = path.file_stem().and_then(|value| value.to_str())
        {
            image_names.insert(stem.to_owned());
        }
    }
    if image_names.len() <= 1 {
        return Ok(());
    }
    Err(RedumperPackageError::MultipleImages {
        path: source.display().to_string(),
        images: image_names.into_iter().collect::<Vec<_>>().join(", "),
    })
}

/// Apply an exact catalog identity to one carrier and keep its parent release
/// honest when the owned copy spans compatible mastering-specific releases.
///
/// A parent stays release-bound while all identified carriers agree. If an
/// incoming carrier names another release for the same work, the parent keeps
/// only the work identity; exact release and media IDs remain on each carrier.
pub fn bind_carrier_to_catalog(
    release_manifest_path: &Path,
    carrier_manifest_path: &Path,
    binding: &CatalogBinding,
) -> Result<bool, CollectionError> {
    let original_release: ReleaseManifest = read_toml(release_manifest_path)?;
    let mut release = original_release.clone();
    let mut carrier: CarrierManifest = read_toml(carrier_manifest_path)?;

    let existing_work = &release.catalog_binding.catalog_work_id;
    if !existing_work.is_empty()
        && !binding.catalog_work_id.is_empty()
        && existing_work != &binding.catalog_work_id
    {
        return Err(CollectionError::CatalogWorkConflict {
            found: binding.catalog_work_id.clone(),
            expected: existing_work.clone(),
        });
    }

    carrier.catalog_binding = binding.clone();
    let already_work_level = !release.catalog_binding.catalog_work_id.is_empty()
        && release.catalog_binding.catalog_release_id.is_empty();
    let mixed_catalog_releases = !release.catalog_binding.catalog_release_id.is_empty()
        && release.catalog_binding.catalog_release_id != binding.catalog_release_id
        && sibling_uses_catalog_release(
            release_manifest_path
                .parent()
                .unwrap_or_else(|| Path::new("")),
            carrier_manifest_path,
            &release.catalog_binding.catalog_release_id,
        )?;
    release
        .catalog_binding
        .catalog_work_id
        .clone_from(&binding.catalog_work_id);
    release.catalog_binding.source.clone_from(&binding.source);
    release
        .catalog_binding
        .dat_name
        .clone_from(&binding.dat_name);
    release
        .catalog_binding
        .source_version
        .clone_from(&binding.source_version);
    let work_level = already_work_level || mixed_catalog_releases;
    if work_level {
        release.catalog_binding.catalog_release_id.clear();
        release.catalog_binding.catalog_media_id.clear();
        release.catalog_binding.serials.clear();
        release.catalog_binding.expected_tracks.clear();
        release.revision.clear();
        release.variant.clear();
    } else {
        release
            .catalog_binding
            .catalog_release_id
            .clone_from(&binding.catalog_release_id);
        release
            .catalog_binding
            .catalog_media_id
            .clone_from(&binding.catalog_media_id);
    }

    write_toml_atomic(release_manifest_path, &release)?;
    if let Err(error) = write_toml_atomic(carrier_manifest_path, &carrier) {
        let _ = write_toml_atomic(release_manifest_path, &original_release);
        return Err(error.into());
    }
    Ok(work_level)
}

fn sibling_uses_catalog_release(
    release_directory: &Path,
    target_carrier_manifest: &Path,
    catalog_release_id: &str,
) -> Result<bool, CollectionError> {
    let copies_directory = release_directory.join("physical-copies");
    if !copies_directory.is_dir() {
        return Ok(false);
    }
    for copy in std::fs::read_dir(&copies_directory).map_err(|source| CollectionError::Io {
        path: copies_directory.display().to_string(),
        source,
    })? {
        let copy = copy.map_err(|source| CollectionError::Io {
            path: copies_directory.display().to_string(),
            source,
        })?;
        let carriers_directory = copy.path().join("carriers");
        if !carriers_directory.is_dir() {
            continue;
        }
        for carrier in
            std::fs::read_dir(&carriers_directory).map_err(|source| CollectionError::Io {
                path: carriers_directory.display().to_string(),
                source,
            })?
        {
            let carrier = carrier.map_err(|source| CollectionError::Io {
                path: carriers_directory.display().to_string(),
                source,
            })?;
            let manifest_path = carrier.path().join("carrier.toml");
            if manifest_path == target_carrier_manifest || !manifest_path.is_file() {
                continue;
            }
            let sibling: CarrierManifest = read_toml(&manifest_path)?;
            if sibling.catalog_binding.catalog_release_id == catalog_release_id {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Set or clear the archive-authoritative playable default for one platform.
pub fn set_platform_playable_default(
    root: &Path,
    platform_id: &str,
    policy: Option<crate::DesiredPlayablePolicy>,
) -> Result<ArchiveRootManifest, CollectionError> {
    let path = root.join("retro-junk-archive.toml");
    if !path.is_file() {
        return Err(CollectionError::NotInitialized(root.display().to_string()));
    }
    let mut manifest: ArchiveRootManifest = read_toml(&path)?;
    manifest
        .platform_defaults
        .retain(|default| !default.platform_id.eq_ignore_ascii_case(platform_id));
    if let Some(policy) = policy {
        manifest
            .platform_defaults
            .push(crate::PlatformPlayableDefault {
                platform_id: platform_id.to_owned(),
                policy,
            });
        manifest
            .platform_defaults
            .sort_by(|a, b| a.platform_id.cmp(&b.platform_id));
    }
    write_toml_atomic(&path, &manifest)?;
    Ok(manifest)
}

/// Creates a portable archive root. Existing roots are accepted only when
/// their manifest has the same profile identity.
pub fn initialize_archive(
    root: &Path,
    manifest: &ArchiveRootManifest,
) -> Result<(), CollectionError> {
    let root_manifest = root.join("retro-junk-archive.toml");
    if root_manifest.is_file() {
        let existing: ArchiveRootManifest = match read_toml(&root_manifest) {
            Ok(existing) => existing,
            Err(crate::ManifestError::UnsupportedSchema { found: 1, .. })
                if empty_prototype_archive(root)? =>
            {
                let contents = std::fs::read_to_string(&root_manifest).map_err(|source| {
                    CollectionError::Io {
                        path: root_manifest.display().to_string(),
                        source,
                    }
                })?;
                let mut existing: ArchiveRootManifest =
                    toml::from_str(&contents).map_err(|source| {
                        crate::ManifestError::TomlDecode {
                            path: root_manifest.display().to_string(),
                            source,
                        }
                    })?;
                existing.schema_version = MANIFEST_SCHEMA_VERSION;
                write_toml_atomic(&root_manifest, &existing)?;
                existing
            }
            Err(error) => return Err(error.into()),
        };
        if existing.profile_id != manifest.profile_id {
            return Err(CollectionError::ProfileMismatch {
                found: existing.profile_id,
                expected: manifest.profile_id,
            });
        }
        return Ok(());
    }
    if root.exists() {
        let mut entries = std::fs::read_dir(root).map_err(|source| CollectionError::Io {
            path: root.display().to_string(),
            source,
        })?;
        if entries.next().is_some() {
            return Err(CollectionError::NonEmptyRoot(root.display().to_string()));
        }
    }
    std::fs::create_dir_all(root.join(".retro-junk")).map_err(|source| CollectionError::Io {
        path: root.display().to_string(),
        source,
    })?;
    write_toml_atomic(&root_manifest, manifest)?;
    Ok(())
}

/// Upgrade 0.4 pre-release archives that used a combined catalog platform as
/// the physical platform. Directory renames remain on the same archive
/// filesystem; dump payloads are never recopied.
pub fn upgrade_legacy_regional_physical_platforms(root: &Path) -> Result<usize, CollectionError> {
    let root_manifest_path = root.join("retro-junk-archive.toml");
    let mut root_manifest: ArchiveRootManifest = read_toml(&root_manifest_path)?;
    if root_manifest
        .applied_migrations
        .iter()
        .any(|migration| migration == crate::REGIONAL_PHYSICAL_PLATFORM_MIGRATION)
    {
        return Ok(0);
    }
    let mut upgraded = 0;
    for platform_entry in std::fs::read_dir(root).map_err(|source| CollectionError::Io {
        path: root.display().to_string(),
        source,
    })? {
        let platform_entry = platform_entry.map_err(|source| CollectionError::Io {
            path: root.display().to_string(),
            source,
        })?;
        if !platform_entry
            .file_type()
            .map_err(|source| CollectionError::Io {
                path: platform_entry.path().display().to_string(),
                source,
            })?
            .is_dir()
        {
            continue;
        }
        let platform_directory = platform_entry.path();
        let platform_folder = platform_entry.file_name().to_string_lossy().into_owned();
        for release_entry in
            std::fs::read_dir(&platform_directory).map_err(|source| CollectionError::Io {
                path: platform_directory.display().to_string(),
                source,
            })?
        {
            let release_entry = release_entry.map_err(|source| CollectionError::Io {
                path: platform_directory.display().to_string(),
                source,
            })?;
            if !release_entry
                .file_type()
                .map_err(|source| CollectionError::Io {
                    path: release_entry.path().display().to_string(),
                    source,
                })?
                .is_dir()
            {
                continue;
            }
            let source_directory = release_entry.path();
            let source_manifest_path = source_directory.join("release.toml");
            if !source_manifest_path.is_file() {
                continue;
            }
            upgraded += usize::from(upgrade_legacy_regional_release(root, &source_directory)?);
        }
        if matches!(
            platform_folder.to_ascii_lowercase().as_str(),
            "nes" | "snes" | "genesis" | "pce" | "saturn"
        ) {
            let _ = std::fs::remove_dir(&platform_directory);
        }
    }
    root_manifest
        .applied_migrations
        .push(crate::REGIONAL_PHYSICAL_PLATFORM_MIGRATION.to_owned());
    write_toml_atomic(&root_manifest_path, &root_manifest)?;
    Ok(upgraded)
}

fn upgrade_legacy_regional_release(
    root: &Path,
    source_directory: &Path,
) -> Result<bool, CollectionError> {
    let mut manifest: ReleaseManifest = read_toml(&source_directory.join("release.toml"))?;
    let Some(target_platform) = regional_physical_platform(&manifest.platform_id, &manifest.region)
    else {
        return Ok(false);
    };
    if target_platform.eq_ignore_ascii_case(&manifest.platform_id) {
        return Ok(false);
    }
    manifest.platform_id.clear();
    manifest.platform_id.push_str(target_platform);
    let target_parent = root.join(target_platform);
    std::fs::create_dir_all(&target_parent).map_err(|source| CollectionError::Io {
        path: target_parent.display().to_string(),
        source,
    })?;
    let name = source_directory.file_name().unwrap_or_default();
    let mut target_directory = target_parent.join(name);
    if target_directory != source_directory && target_directory.exists() {
        target_directory = target_parent.join(format!(
            "{}--{}",
            name.to_string_lossy(),
            &manifest.archive_release_id.to_string()[..8]
        ));
    }
    if target_directory != source_directory {
        std::fs::rename(source_directory, &target_directory).map_err(|source| {
            CollectionError::Io {
                path: format!(
                    "{} -> {}",
                    source_directory.display(),
                    target_directory.display()
                ),
                source,
            }
        })?;
    }
    let target_manifest_path = target_directory.join("release.toml");
    if let Err(error) = write_toml_atomic(&target_manifest_path, &manifest) {
        if target_directory != source_directory {
            let _ = std::fs::rename(&target_directory, source_directory);
        }
        return Err(error.into());
    }
    Ok(true)
}

fn regional_physical_platform(platform: &str, region: &str) -> Option<&'static str> {
    let platform = platform.trim().to_ascii_lowercase();
    let region = region.trim().to_ascii_lowercase();
    match platform.as_str() {
        "nes" if matches!(region.as_str(), "japan" | "jp" | "jpn") => Some("famicom"),
        "snes" if matches!(region.as_str(), "japan" | "jp" | "jpn") => Some("super-famicom"),
        "genesis"
            if matches!(
                region.as_str(),
                "japan" | "jp" | "jpn" | "europe" | "eur" | "australia" | "brazil" | "asia"
            ) =>
        {
            Some("megadrive")
        }
        "pce" if matches!(region.as_str(), "usa" | "us" | "canada" | "europe" | "eur") => {
            Some("tg16")
        }
        "saturn" if matches!(region.as_str(), "japan" | "jp" | "jpn") => Some("saturnjp"),
        _ => None,
    }
}

fn empty_prototype_archive(root: &Path) -> Result<bool, CollectionError> {
    for entry in std::fs::read_dir(root).map_err(|source| CollectionError::Io {
        path: root.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| CollectionError::Io {
            path: root.display().to_string(),
            source,
        })?;
        let name = entry.file_name();
        if name != "retro-junk-archive.toml" && name != ".retro-junk" {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Ingests a new physical copy and carrier. The dump itself is published by
/// atomic rename only after every staged byte has been re-read and verified.
/// If ingest fails, newly-created hierarchy metadata is removed.
#[allow(clippy::too_many_lines)]
pub fn ingest_new_carrier_dump(
    archive_root: &Path,
    source: &Path,
    spec: NewCarrierDump,
    cancel: &AtomicBool,
    on_progress: impl Fn(IngestProgress),
) -> Result<IngestedCarrierDump, CollectionError> {
    if matches!(spec.format, RepresentationFormat::RedumperRaw) {
        validate_redumper_package(source)?;
    }
    let root_manifest = archive_root.join("retro-junk-archive.toml");
    if !root_manifest.is_file() {
        return Err(CollectionError::NotInitialized(
            archive_root.display().to_string(),
        ));
    }
    let _: ArchiveRootManifest = read_toml(&root_manifest)?;

    let proposed_release = ReleaseManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        archive_release_id: ArchiveReleaseId::new(),
        platform_id: spec.platform_id,
        title: spec.title,
        region: spec.region,
        revision: spec.revision,
        variant: spec.variant,
        catalog_binding: spec.catalog_binding.clone(),
    };
    let requested_physical_copy_id = spec.physical_copy_id;
    let existing_release = crate::scan_archive(archive_root).ok().and_then(|snapshot| {
        snapshot.releases.into_iter().find(|candidate| {
            let owns_requested_copy = requested_physical_copy_id.is_some_and(|requested| {
                candidate
                    .physical_copies
                    .iter()
                    .any(|copy| copy.manifest.physical_copy_id == requested)
            });
            let same_catalog_work = !proposed_release.catalog_binding.catalog_work_id.is_empty()
                && (candidate.manifest.catalog_binding.catalog_work_id
                    == proposed_release.catalog_binding.catalog_work_id
                    || candidate.physical_copies.iter().any(|copy| {
                        copy.carriers.iter().any(|carrier| {
                            carrier.manifest.catalog_binding.catalog_work_id
                                == proposed_release.catalog_binding.catalog_work_id
                        })
                    }));
            candidate.manifest.platform_id == proposed_release.platform_id
                && candidate.manifest.region == proposed_release.region
                && (owns_requested_copy
                    || same_catalog_work
                    || (candidate.manifest.title == proposed_release.title
                        && candidate.manifest.revision == proposed_release.revision
                        && candidate.manifest.variant == proposed_release.variant))
        })
    });
    let copy_number = existing_release.as_ref().map_or(1, |existing| {
        existing
            .physical_copies
            .iter()
            .map(|copy| copy.manifest.copy_number)
            .max()
            .unwrap_or(0)
            + 1
    });
    let requested_copy = spec.physical_copy_id.and_then(|requested| {
        existing_release.as_ref().and_then(|release| {
            release
                .physical_copies
                .iter()
                .find(|copy| copy.manifest.physical_copy_id == requested)
                .cloned()
        })
    });
    let (release, release_dir, created_release, updated_release) =
        if let Some(existing) = &existing_release {
            let mut manifest = existing.manifest.clone();
            let incoming = &proposed_release.catalog_binding;
            let same_work = !incoming.catalog_work_id.is_empty()
                && manifest.catalog_binding.catalog_work_id == incoming.catalog_work_id;
            let mixed_catalog_releases = same_work
                && !manifest.catalog_binding.catalog_release_id.is_empty()
                && manifest.catalog_binding.catalog_release_id != incoming.catalog_release_id;
            if mixed_catalog_releases {
                manifest
                    .catalog_binding
                    .catalog_work_id
                    .clone_from(&incoming.catalog_work_id);
                manifest.catalog_binding.catalog_release_id.clear();
                manifest.catalog_binding.catalog_media_id.clear();
                manifest.catalog_binding.serials.clear();
                manifest.catalog_binding.expected_tracks.clear();
                manifest.revision.clear();
                manifest.variant.clear();
            }
            let updated = manifest != existing.manifest;
            (manifest, existing.directory.clone(), false, updated)
        } else {
            let layout = ArchiveLayout::new(archive_root);
            let directory = layout.release_dir(
                &proposed_release.platform_id,
                &proposed_release.title,
                &proposed_release.region,
                &proposed_release.revision,
                proposed_release.archive_release_id,
            );
            (proposed_release, directory, true, false)
        };
    let (physical_copy, copy_dir, created_copy, existing_carriers) =
        if let Some(existing) = requested_copy {
            (
                existing.manifest,
                existing.directory,
                false,
                existing.carriers,
            )
        } else {
            let manifest = PhysicalCopyManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                physical_copy_id: PhysicalCopyId::new(),
                archive_release_id: release.archive_release_id,
                copy_number,
                owner_id: if spec.owner_id.trim().is_empty() {
                    "default".to_owned()
                } else {
                    spec.owner_id
                },
                label: spec.physical_copy_label,
                condition: String::new(),
                notes: String::new(),
                date_acquired: String::new(),
                provenance: String::new(),
            };
            let directory = ArchiveLayout::physical_copy_dir(&release_dir, copy_number);
            (manifest, directory, true, Vec::new())
        };
    let matching_carrier = existing_carriers.into_iter().find(|candidate| {
        (!spec.catalog_binding.catalog_media_id.is_empty()
            && candidate.manifest.catalog_binding.catalog_media_id
                == spec.catalog_binding.catalog_media_id)
            || (!spec.serial.is_empty()
                && candidate.manifest.serial.eq_ignore_ascii_case(&spec.serial)
                && candidate.manifest.sequence_number == spec.sequence_number)
    });
    let (carrier, carrier_dir, created_carrier) = if let Some(existing) = matching_carrier {
        (existing.manifest, existing.directory, false)
    } else {
        let manifest = CarrierManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            carrier_id: CarrierId::new(),
            physical_copy_id: physical_copy.physical_copy_id,
            kind: spec.carrier_kind,
            serial: spec.serial,
            sequence_number: spec.sequence_number,
            label: spec.carrier_label,
            catalog_binding: spec.catalog_binding,
            playable_policy: None,
        };
        let directory =
            ArchiveLayout::carrier_dir(&copy_dir, &manifest.serial, manifest.sequence_number);
        (manifest, directory, true)
    };
    let mut dump = DumpManifest::new(carrier.carrier_id, spec.format);
    dump.source_package = spec.source_package;

    let dump_dir = ArchiveLayout::dump_dir(&carrier_dir, &dump.captured_at, dump.dump_id);
    let mut plan = plan_ingest(source, &dump_dir)?;
    for planned in &mut plan.files {
        if let Some(expected) = spec
            .expected_files
            .iter()
            .find(|expected| expected.relative_path == planned.relative_path)
        {
            planned.expected_digests = Some(expected.digests.clone());
        }
    }
    if !spec.expected_files.is_empty()
        && (plan.files.len() != spec.expected_files.len()
            || plan
                .files
                .iter()
                .any(|planned| planned.expected_digests.is_none()))
    {
        return Err(IngestError::CopyMismatch(source.display().to_string()).into());
    }

    std::fs::create_dir_all(&carrier_dir).map_err(|source| CollectionError::Io {
        path: carrier_dir.display().to_string(),
        source,
    })?;
    let hierarchy_result = (|| {
        if created_release || updated_release {
            write_toml_atomic(&release_dir.join("release.toml"), &release)?;
        }
        if created_copy {
            write_toml_atomic(&copy_dir.join("physical-copy.toml"), &physical_copy)?;
        }
        if created_carrier {
            write_toml_atomic(&carrier_dir.join("carrier.toml"), &carrier)?;
        }
        Ok::<_, CollectionError>(())
    })();
    if let Err(error) = hierarchy_result {
        if created_release {
            let _ = std::fs::remove_dir_all(&release_dir);
        } else if updated_release {
            if let Some(existing) = &existing_release {
                let _ = write_toml_atomic(&release_dir.join("release.toml"), &existing.manifest);
            }
        } else if created_copy {
            let _ = std::fs::remove_dir_all(&copy_dir);
        } else if created_carrier {
            let _ = std::fs::remove_dir_all(&carrier_dir);
        }
        return Err(error);
    }

    let request = IngestRequest {
        plan,
        manifest: dump,
    };
    match execute_ingest(request, cancel, on_progress) {
        Ok(dump) => Ok(IngestedCarrierDump {
            release,
            physical_copy,
            carrier,
            dump,
            dump_directory: dump_dir,
        }),
        Err(error) => {
            if created_release {
                let _ = std::fs::remove_dir_all(&release_dir);
            } else if updated_release {
                if let Some(existing) = &existing_release {
                    let _ =
                        write_toml_atomic(&release_dir.join("release.toml"), &existing.manifest);
                }
            } else if created_copy {
                let _ = std::fs::remove_dir_all(&copy_dir);
            } else if created_carrier {
                let _ = std::fs::remove_dir_all(&carrier_dir);
            }
            Err(error.into())
        }
    }
}
