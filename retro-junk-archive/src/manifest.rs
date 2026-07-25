use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::MANIFEST_SCHEMA_VERSION;

pub const REGIONAL_PHYSICAL_PLATFORM_MIGRATION: &str = "regional-physical-platform-v2";

macro_rules! archive_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl std::str::FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse().map(Self)
            }
        }
    };
}

archive_id!(ArchiveProfileId);
archive_id!(ArchiveReleaseId);
archive_id!(PhysicalCopyId);
archive_id!(CarrierId);
archive_id!(DumpId);
archive_id!(RepresentationId);
archive_id!(VerificationId);
archive_id!(BuildId);
archive_id!(SupportingFileId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveRootManifest {
    pub schema_version: u32,
    pub profile_id: ArchiveProfileId,
    pub display_name: String,
    #[serde(default)]
    pub platform_defaults: Vec<PlatformPlayableDefault>,
    #[serde(default)]
    pub applied_migrations: Vec<String>,
}

impl ArchiveRootManifest {
    #[must_use]
    pub fn new(display_name: impl Into<String>) -> Self {
        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            profile_id: ArchiveProfileId::new(),
            display_name: display_name.into(),
            platform_defaults: Vec::new(),
            applied_migrations: vec![REGIONAL_PHYSICAL_PLATFORM_MIGRATION.to_owned()],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformPlayableDefault {
    pub platform_id: String,
    pub policy: DesiredPlayablePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub archive_release_id: ArchiveReleaseId,
    pub platform_id: String,
    pub title: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub variant: String,
    #[serde(default)]
    pub catalog_binding: CatalogBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalCopyManifest {
    pub schema_version: u32,
    pub physical_copy_id: PhysicalCopyId,
    pub archive_release_id: ArchiveReleaseId,
    pub copy_number: u32,
    #[serde(default = "default_owner")]
    pub owner_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub condition: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub date_acquired: String,
    #[serde(default)]
    pub provenance: String,
}

fn default_owner() -> String {
    "default".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarrierManifest {
    pub schema_version: u32,
    pub carrier_id: CarrierId,
    pub physical_copy_id: PhysicalCopyId,
    pub kind: CarrierKind,
    #[serde(default)]
    pub serial: String,
    #[serde(default)]
    pub sequence_number: u32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub catalog_binding: CatalogBinding,
    #[serde(default)]
    pub playable_policy: Option<DesiredPlayablePolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "type", content = "name")]
pub enum CarrierKind {
    OpticalDisc,
    Cartridge,
    Card,
    Tape,
    FloppyDisk,
    #[default]
    Unknown,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CatalogBinding {
    /// Stable work-level identity shared by compatible releases/masterings.
    ///
    /// A logical owned release may contain individually verified carriers
    /// whose catalog release IDs differ. In that case the release manifest
    /// keeps this work ID while `catalog_release_id` remains empty; each
    /// carrier retains its exact release/media binding.
    #[serde(default)]
    pub catalog_work_id: String,
    #[serde(default)]
    pub catalog_release_id: String,
    #[serde(default)]
    pub catalog_media_id: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub dat_name: String,
    #[serde(default)]
    pub source_version: String,
    #[serde(default)]
    pub serials: Vec<String>,
    #[serde(default)]
    pub expected_tracks: Vec<TrackDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackDigest {
    pub number: u32,
    pub size: u64,
    #[serde(default)]
    pub crc32: String,
    #[serde(default)]
    pub md5: String,
    #[serde(default)]
    pub sha1: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DumpManifest {
    pub schema_version: u32,
    pub dump_id: DumpId,
    pub carrier_id: CarrierId,
    pub representation_id: RepresentationId,
    pub format: RepresentationFormat,
    pub captured_at: String,
    pub ingested_at: String,
    #[serde(default)]
    pub source_package: SourcePackageRecord,
    #[serde(default)]
    pub tool: Option<ToolRecord>,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub captured_features: Vec<CapturedFeature>,
    pub files: Vec<ArchivedFile>,
}

impl DumpManifest {
    #[must_use]
    pub fn new(carrier_id: CarrierId, format: RepresentationFormat) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            dump_id: DumpId::new(),
            carrier_id,
            representation_id: RepresentationId::new(),
            format,
            captured_at: now.clone(),
            ingested_at: now,
            source_package: SourcePackageRecord::default(),
            tool: None,
            command: Vec::new(),
            captured_features: Vec::new(),
            files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SourcePackageRecord {
    #[serde(default)]
    pub original_name: String,
    #[serde(default)]
    pub package_sha256: String,
    #[serde(default)]
    pub observed_captured_at: String,
    #[serde(default)]
    pub timestamp_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchivedFile {
    /// Path relative to the dump's `raw/` directory.
    pub path: String,
    pub size: u64,
    #[serde(default)]
    pub crc32: String,
    #[serde(default)]
    pub md5: String,
    #[serde(default)]
    pub sha1: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRecord {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub build: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepresentationRole {
    PreservationMaster,
    CanonicalIntermediate,
    Playable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "name")]
pub enum RepresentationFormat {
    RedumperRaw,
    Rom,
    CueBin,
    Iso,
    Chd,
    Rvz,
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturedFeature {
    MainChannel,
    RawSubcode,
    LeadIn,
    LeadOut,
    ErrorMap,
    FullToc,
    DriveCache,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredPlayablePolicy {
    pub format: RepresentationFormat,
    #[serde(default)]
    pub retain_canonical_intermediate: bool,
    #[serde(default)]
    pub allow_unverified: bool,
    #[serde(default)]
    pub options: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub schema_version: u32,
    pub verification_id: VerificationId,
    pub representation_id: RepresentationId,
    pub performed_at: String,
    pub input_manifest_sha256: String,
    pub kind: VerificationKind,
    pub outcome: VerificationOutcome,
    #[serde(default)]
    pub tool: Option<ToolRecord>,
    #[serde(default)]
    pub catalog: Option<CatalogEvidence>,
    #[serde(default)]
    pub tracks: Vec<TrackVerification>,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationKind {
    Integrity,
    Reproduction,
    Catalog,
    RoundTrip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    Verified,
    Unmatched,
    Ambiguous,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEvidence {
    pub source: String,
    pub system: String,
    pub version: String,
    pub game: String,
    pub complete_track_set: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackVerification {
    pub number: u32,
    pub size: u64,
    pub expected_sha1: String,
    pub actual_sha1: String,
    pub matched: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildEvidence {
    pub schema_version: u32,
    pub build_id: BuildId,
    pub parent_representation_id: RepresentationId,
    pub child_representation_id: RepresentationId,
    pub performed_at: String,
    pub input_manifest_sha256: String,
    pub recipe_version: u32,
    pub format: RepresentationFormat,
    pub relative_output_path: String,
    pub output_sha256: String,
    pub output_size: u64,
    pub catalog_verified: bool,
    pub round_trip_verified: bool,
    #[serde(default)]
    pub tool: Option<ToolRecord>,
    #[serde(default)]
    pub omitted_features: Vec<CapturedFeature>,
    #[serde(default)]
    pub canonical_intermediate: Option<CanonicalIntermediateEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalIntermediateEvidence {
    pub representation_id: RepresentationId,
    pub format: RepresentationFormat,
    /// Path relative to the dump directory; files live below its `raw/` child.
    pub relative_path: String,
    pub files: Vec<ArchivedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseFileManifest {
    pub schema_version: u32,
    pub supporting_file_id: SupportingFileId,
    pub archive_release_id: ArchiveReleaseId,
    pub category: ReleaseFileCategory,
    pub asset_type: String,
    pub captured_at: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub caption: String,
    pub file: ArchivedFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalCopyFileManifest {
    pub schema_version: u32,
    pub supporting_file_id: SupportingFileId,
    pub physical_copy_id: PhysicalCopyId,
    pub category: PhysicalCopyFileCategory,
    pub asset_type: String,
    pub captured_at: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub caption: String,
    pub file: ArchivedFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseFileCategory {
    Artwork,
    Video,
    Document,
    Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalCopyFileCategory {
    Photo,
    Provenance,
    Document,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid TOML manifest {path}: {source}")]
    TomlDecode {
        path: String,
        source: toml::de::Error,
    },
    #[error("could not encode TOML manifest: {0}")]
    TomlEncode(#[from] toml::ser::Error),
    #[error("could not encode JSON evidence: {0}")]
    JsonEncode(#[from] serde_json::Error),
    #[error("invalid JSON evidence {path}: {source}")]
    JsonDecode {
        path: String,
        source: serde_json::Error,
    },
    #[error("unsupported manifest schema {found}; this build supports {supported}")]
    UnsupportedSchema { found: u32, supported: u32 },
    #[error("evidence file already exists: {0}")]
    EvidenceExists(String),
}

pub trait VersionedManifest {
    fn schema_version(&self) -> u32;
}

macro_rules! versioned {
    ($($ty:ty),+ $(,)?) => {$ (
        impl VersionedManifest for $ty {
            fn schema_version(&self) -> u32 { self.schema_version }
        }
    )+ };
}

versioned!(
    ArchiveRootManifest,
    ReleaseManifest,
    PhysicalCopyManifest,
    CarrierManifest,
    DumpManifest,
    ReleaseFileManifest,
    PhysicalCopyFileManifest,
);

pub fn read_toml<T: DeserializeOwned + VersionedManifest>(path: &Path) -> Result<T, ManifestError> {
    let contents = std::fs::read_to_string(path).map_err(|source| ManifestError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let manifest: T = toml::from_str(&contents).map_err(|source| ManifestError::TomlDecode {
        path: path.display().to_string(),
        source,
    })?;
    if manifest.schema_version() != MANIFEST_SCHEMA_VERSION {
        return Err(ManifestError::UnsupportedSchema {
            found: manifest.schema_version(),
            supported: MANIFEST_SCHEMA_VERSION,
        });
    }
    Ok(manifest)
}

pub fn write_toml_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), ManifestError> {
    use std::io::Write;
    let contents = toml::to_string_pretty(value)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("manifest");
    let temp = parent.join(format!(".{filename}.{}.tmp", Uuid::now_v7()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|source| ManifestError::Io {
            path: temp.display().to_string(),
            source,
        })?;
    file.write_all(contents.as_bytes())
        .map_err(|source| ManifestError::Io {
            path: temp.display().to_string(),
            source,
        })?;
    file.sync_all().map_err(|source| ManifestError::Io {
        path: temp.display().to_string(),
        source,
    })?;
    std::fs::rename(&temp, path).map_err(|source| ManifestError::Io {
        path: path.display().to_string(),
        source,
    })?;
    sync_directory(parent)
}

pub fn write_json_new<T: Serialize>(path: &Path, value: &T) -> Result<(), ManifestError> {
    use std::io::Write;
    let contents = serde_json::to_vec_pretty(value)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                ManifestError::EvidenceExists(path.display().to_string())
            } else {
                ManifestError::Io {
                    path: path.display().to_string(),
                    source,
                }
            }
        })?;
    file.write_all(&contents)
        .map_err(|source| ManifestError::Io {
            path: path.display().to_string(),
            source,
        })?;
    file.sync_all().map_err(|source| ManifestError::Io {
        path: path.display().to_string(),
        source,
    })?;
    sync_directory(path.parent().unwrap_or_else(|| Path::new(".")))
}

pub(crate) fn sync_directory(directory: &Path) -> Result<(), ManifestError> {
    let handle = std::fs::File::open(directory).map_err(|source| ManifestError::Io {
        path: directory.display().to_string(),
        source,
    })?;
    handle.sync_all().map_err(|source| ManifestError::Io {
        path: directory.display().to_string(),
        source,
    })
}

pub fn read_verification_json(path: &Path) -> Result<VerificationEvidence, ManifestError> {
    let value: VerificationEvidence = read_json(path)?;
    validate_evidence_schema(value.schema_version)?;
    Ok(value)
}

pub fn read_build_json(path: &Path) -> Result<BuildEvidence, ManifestError> {
    let value: BuildEvidence = read_json(path)?;
    validate_evidence_schema(value.schema_version)?;
    Ok(value)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, ManifestError> {
    let contents = std::fs::read(path).map_err(|source| ManifestError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_slice(&contents).map_err(|source| ManifestError::JsonDecode {
        path: path.display().to_string(),
        source,
    })
}

fn validate_evidence_schema(found: u32) -> Result<(), ManifestError> {
    if found == MANIFEST_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(ManifestError::UnsupportedSchema {
            found,
            supported: MANIFEST_SCHEMA_VERSION,
        })
    }
}
