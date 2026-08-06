//! A small archive holding one built playable, for tests about finding it.
//!
//! Shared rather than copied: the rename repair and the location resolver ask
//! the same question of the same shape of archive, and two hand-built
//! fixtures would drift until only one of them still described reality.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// What the fixture built, for tests that need to reach into it.
pub struct ArchivedPlayable {
    pub archive: PathBuf,
    pub playable_root: PathBuf,
    /// The built playable's representation id.
    pub representation_id: String,
}

/// One release with one built playable, both in the frontend's `psx` folder.
pub fn archive_with_playable(temp: &Path, playable_name: &str) -> ArchivedPlayable {
    archive_with_playable_at(temp, playable_name, "psx", "psx")
}

/// The same, where `recorded_directory` is the folder the build evidence names
/// and `actual_directory` is the folder the file is really in.
///
/// They differ whenever a release's archive platform folder is not the
/// frontend's system folder — `ps1` against `psx`, say. An empty
/// `recorded_directory` records a bare file name with no folder at all, which
/// is what evidence written before playables were filed by system folder
/// looks like.
pub fn archive_with_playable_at(
    temp: &Path,
    playable_name: &str,
    recorded_directory: &str,
    actual_directory: &str,
) -> ArchivedPlayable {
    let archive = temp.join("archive");
    let playable_root = temp.join("playable");
    retro_junk_archive::initialize_archive(
        &archive,
        &retro_junk_archive::ArchiveRootManifest::new("Collection"),
    )
    .unwrap();
    let source = temp.join("master.bin");
    std::fs::write(&source, b"master bytes").unwrap();
    let ingested = retro_junk_archive::ingest_new_carrier_dump(
        &archive,
        &source,
        retro_junk_archive::NewCarrierDump {
            platform_id: "psx".to_owned(),
            title: "Game".to_owned(),
            region: "usa".to_owned(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: String::new(),
            sequence_number: 0,
            carrier_label: String::new(),
            carrier_kind: retro_junk_archive::CarrierKind::OpticalDisc,
            format: retro_junk_archive::RepresentationFormat::CueBin,
            catalog_binding: retro_junk_archive::CatalogBinding::default(),
            join_release: None,
            source_package: retro_junk_archive::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();

    let system_dir = playable_root.join(actual_directory);
    std::fs::create_dir_all(&system_dir).unwrap();
    let playable = system_dir.join(playable_name);
    std::fs::write(&playable, b"playable bytes").unwrap();
    let digests =
        retro_junk_archive::hash_file_digests(&playable, &AtomicBool::new(false)).unwrap();
    // The evidence has to claim the dump it was actually built from, or every
    // reader calls the playable stale and stops looking at it.
    let manifest_sha256 = retro_junk_archive::scan_archive(&archive).unwrap().releases[0]
        .physical_copies[0]
        .carriers[0]
        .dumps[0]
        .manifest_sha256
        .clone();
    let child = retro_junk_archive::RepresentationId::new();
    retro_junk_archive::write_build_evidence(
        &ingested.dump_directory,
        &retro_junk_archive::BuildEvidence {
            schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
            build_id: retro_junk_archive::BuildId::new(),
            parent_representation_id: ingested.dump.representation_id,
            child_representation_id: child,
            performed_at: "2026-01-01T00:00:00Z".to_owned(),
            input_manifest_sha256: manifest_sha256,
            recipe_version: 1,
            format: retro_junk_archive::RepresentationFormat::Chd,
            relative_output_path: if recorded_directory.is_empty() {
                playable_name.to_owned()
            } else {
                format!("{recorded_directory}/{playable_name}")
            },
            output_sha256: digests.sha256,
            output_size: digests.size,
            catalog_verified: false,
            round_trip_verified: false,
            tool: None,
            omitted_features: Vec::new(),
            canonical_intermediate: None,
        },
    )
    .unwrap();
    ArchivedPlayable {
        archive,
        playable_root,
        representation_id: child.to_string(),
    }
}
