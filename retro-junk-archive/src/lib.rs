//! Preservation-master storage and playable-derivative workflows.
//!
//! Portable manifests are authoritative. Database rows and playable outputs
//! are deliberately rebuildable projections of the archive.

pub mod adopt;
pub mod assets;
pub mod collection;
pub mod evidence;
pub mod generation;
pub mod ignore;
pub mod index;
pub mod ingest;
pub mod layout;
pub mod lock;
pub mod manifest;
pub mod marks;
pub mod presence;
pub mod profile;
pub mod redumper;
pub mod sidecar;
pub mod verify;

pub use adopt::{
    ADOPTION_TOOL, AdoptError, OrphanedPlayable, claimed_playable_paths, current_build_evidence,
    current_release_builds, index_by_size, locate_by_content, orphaned_playables, record_adoption,
    search_directories, superseded_release_builds, write_build_evidence,
};
pub use evidence::{
    dump_catalog_attempted, dump_catalog_evidence, dump_catalog_verified, dump_has_current_evidence,
};
pub use generation::{
    advance_projection_generation, projection_generation, projection_generation_path,
};
pub use ignore::{IgnoreRule, IgnoreRules, ignore_directory, load_rules, remove_rule, write_rule};
pub use index::{
    ArchiveIndexSnapshot, IndexedBuild, IndexedCarrier, IndexedDump, IndexedPhysicalCopy,
    IndexedPhysicalCopyFile, IndexedRelease, IndexedReleaseFile, IndexedVerification, scan_archive,
    scan_archive_cancellable, scan_archive_release,
};
pub use ingest::{
    FileDigests, IngestError, IngestPlan, IngestProgress, IngestRequest, execute_ingest,
    hash_file_digests, plan_ingest,
};
pub use layout::{
    ArchiveLayout, ROOT_MANIFEST_FILE, normalize_relative_path, root_manifest_path, safe_file_stem,
    slugify,
};
pub use lock::{ArchiveLock, ArchiveLockError};
pub use manifest::*;
pub use marks::{
    CollectionMark, MarkError, MarkIndex, MarkKind, MarkedContent, load_marks, marks_directory,
    remove_mark, write_mark,
};
pub use presence::{
    RepresentationPresence, archived_files_presence, playable_presence, preservation_presence,
    resolve_playable, resolve_playable_path,
};
pub use profile::{CollectionProfile, WatchBackend, collection_root_for};
pub use redumper::{Redumper, RedumperAudit, RedumperError, RedumperWorkspace};
pub use sidecar::{SidecarError, SidecarRecord};
pub use verify::{IntegrityFailure, IntegrityReport, sha256_file, verify_dump_integrity};

/// Current portable archive-manifest schema.
pub const MANIFEST_SCHEMA_VERSION: u32 = 2;

#[cfg(test)]
#[path = "tests/evidence_tests.rs"]
mod evidence_tests;
#[cfg(test)]
#[path = "tests/archive_tests.rs"]
mod tests;
pub use assets::{
    AddedReleaseFile, NewPhysicalCopyFile, NewReleaseFile, SupportingFileError,
    add_physical_copy_file, add_release_file, add_release_files,
};
pub use collection::{
    CollectionError, ExpectedSourceFile, IngestedCarrierDump, NewCarrierDump, RedumperPackageError,
    bind_carrier_to_catalog, ingest_new_carrier_dump, initialize_archive,
    regional_physical_platform, set_platform_playable_default,
    upgrade_legacy_regional_physical_platforms, validate_redumper_package,
};
