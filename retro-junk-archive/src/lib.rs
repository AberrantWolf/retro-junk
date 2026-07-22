//! Preservation-master storage and playable-derivative workflows.
//!
//! Portable manifests are authoritative. Database rows and playable outputs
//! are deliberately rebuildable projections of the archive.

pub mod assets;
pub mod collection;
pub mod index;
pub mod ingest;
pub mod layout;
pub mod lock;
pub mod manifest;
pub mod presence;
pub mod profile;
pub mod redumper;
pub mod verify;

pub use index::{
    ArchiveIndexSnapshot, IndexedBuild, IndexedCarrier, IndexedDump, IndexedPhysicalCopy,
    IndexedPhysicalCopyFile, IndexedRelease, IndexedReleaseFile, IndexedVerification, scan_archive,
};
pub use ingest::{
    FileDigests, IngestError, IngestPlan, IngestProgress, IngestRequest, execute_ingest,
    hash_file_digests, plan_ingest,
};
pub use layout::{ArchiveLayout, normalize_relative_path, slugify};
pub use lock::{ArchiveLock, ArchiveLockError};
pub use manifest::*;
pub use presence::{
    RepresentationPresence, archived_files_presence, playable_presence, preservation_presence,
};
pub use profile::CollectionProfile;
pub use redumper::{Redumper, RedumperAudit, RedumperError, RedumperWorkspace};
pub use verify::{IntegrityFailure, IntegrityReport, sha256_file, verify_dump_integrity};

/// Current portable archive-manifest schema.
pub const MANIFEST_SCHEMA_VERSION: u32 = 2;

#[cfg(test)]
#[path = "tests/archive_tests.rs"]
mod tests;
pub use assets::{
    NewPhysicalCopyFile, NewReleaseFile, SupportingFileError, add_physical_copy_file,
    add_release_file,
};
pub use collection::{
    CollectionError, IngestedCarrierDump, NewCarrierDump, ingest_new_carrier_dump,
    initialize_archive, upgrade_legacy_regional_physical_platforms,
};
