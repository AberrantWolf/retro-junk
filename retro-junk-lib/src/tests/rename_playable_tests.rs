//! The repair for playables built under an older naming rule: the file, its
//! artwork, and the playlist that lists it all move together, and the archive
//! records where it went.

use super::*;
use std::sync::atomic::AtomicBool;

fn archive_with_playable(temp: &Path, playable_name: &str) -> (PathBuf, PathBuf, String) {
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
            source_package: retro_junk_archive::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();

    // A built playable sitting at a name an older rule produced.
    let system_dir = playable_root.join("psx");
    std::fs::create_dir_all(&system_dir).unwrap();
    let playable = system_dir.join(playable_name);
    std::fs::write(&playable, b"playable bytes").unwrap();
    let digests =
        retro_junk_archive::hash_file_digests(&playable, &AtomicBool::new(false)).unwrap();
    let child = retro_junk_archive::RepresentationId::new();
    retro_junk_archive::write_build_evidence(
        &ingested.dump_directory,
        &retro_junk_archive::BuildEvidence {
            schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
            build_id: retro_junk_archive::BuildId::new(),
            parent_representation_id: ingested.dump.representation_id,
            child_representation_id: child,
            performed_at: "2026-01-01T00:00:00Z".to_owned(),
            input_manifest_sha256: String::new(),
            recipe_version: 1,
            format: retro_junk_archive::RepresentationFormat::Chd,
            relative_output_path: format!("psx/{playable_name}"),
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
    (archive, playable_root, child.to_string())
}

#[test]
fn renaming_moves_the_file_its_artwork_and_the_playlist_that_lists_it() {
    let temp = tempfile::tempdir().unwrap();
    let (archive, playable_root, representation_id) =
        archive_with_playable(temp.path(), "Game (USA) (Track 1).chd");

    // Artwork is named after the playable's stem, so it must follow.
    let media_root = temp.path().join("media");
    let covers = media_root.join("psx").join("covers");
    std::fs::create_dir_all(&covers).unwrap();
    std::fs::write(covers.join("Game (USA) (Track 1).png"), b"art").unwrap();

    // A playlist in the same directory names the old file.
    let playlist = playable_root.join("psx").join("Game.m3u");
    std::fs::write(&playlist, "Game (USA) (Track 1).chd\n").unwrap();

    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let report = rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &playable_root,
        representation_id: &representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: Some(&media_root),
    })
    .unwrap();

    assert_eq!(report.to, "psx/Game (USA).chd");
    assert!(playable_root.join("psx/Game (USA).chd").is_file());
    assert!(!playable_root.join("psx/Game (USA) (Track 1).chd").exists());
    assert_eq!(report.media_renamed, 1);
    assert!(covers.join("Game (USA).png").is_file());
    assert_eq!(report.playlists_updated, 1);
    assert_eq!(
        std::fs::read_to_string(&playlist).unwrap().trim(),
        "Game (USA).chd"
    );
}

#[test]
fn the_archive_records_where_the_file_went_without_losing_where_it_was() {
    let temp = tempfile::tempdir().unwrap();
    let (archive, playable_root, representation_id) =
        archive_with_playable(temp.path(), "Game (USA) (Track 1).chd");
    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &playable_root,
        representation_id: &representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: None,
    })
    .unwrap();

    let rescanned = retro_junk_archive::scan_archive(&archive).unwrap();
    let dump = &rescanned.releases[0].physical_copies[0].carriers[0].dumps[0];
    // Evidence is append-only history: both records survive, and the newest
    // names the current location.
    assert_eq!(dump.builds.len(), 2);
    let newest = dump
        .builds
        .iter()
        .max_by(|left, right| left.evidence.performed_at.cmp(&right.evidence.performed_at))
        .unwrap();
    assert_eq!(newest.evidence.relative_output_path, "psx/Game (USA).chd");
    // Same file, same identity — so the projection updates the representation
    // rather than inventing a second one beside it.
    assert_eq!(
        newest.evidence.child_representation_id.to_string(),
        representation_id
    );
}

/// A rename must stay inside the build lineage it found.
///
/// Lineage is `(parent representation, format)`, and a representation row is
/// the current state of one file. Writing the *dump's* format instead of the
/// playable's started a second lineage, so both the old and the new record
/// stayed live and each projected a representation row under the same id —
/// which the projection rejects with a unique-constraint failure, taking the
/// whole reconcile down with it.
#[test]
fn renaming_supersedes_the_old_record_instead_of_starting_a_second_lineage() {
    let temp = tempfile::tempdir().unwrap();
    let (archive, playable_root, representation_id) =
        archive_with_playable(temp.path(), "Game (USA) (Track 1).chd");
    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let before = retro_junk_archive::current_build_evidence(
        &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0],
    );
    assert_eq!(before.len(), 1);
    let original_format = before[0].format.clone();

    rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &playable_root,
        representation_id: &representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: None,
    })
    .unwrap();

    let rescanned = retro_junk_archive::scan_archive(&archive).unwrap();
    let dump = &rescanned.releases[0].physical_copies[0].carriers[0].dumps[0];
    let current = retro_junk_archive::current_build_evidence(dump);
    assert_eq!(
        current.len(),
        1,
        "the rename left two live records, which project two rows under one id"
    );
    assert_eq!(current[0].relative_output_path, "psx/Game (USA).chd");
    // The playable's own format, not the preservation master's.
    assert_eq!(current[0].format, original_format);
}

/// A rename says nothing about the bytes, so what was verified about them
/// must survive it. Re-deriving these downgraded a playable that had been
/// proven to reproduce its master.
#[test]
fn renaming_preserves_what_was_verified_about_the_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let (archive, playable_root, representation_id) =
        archive_with_playable(temp.path(), "Game (USA) (Track 1).chd");
    // Mark the existing build as fully verified, as a real one would be.
    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
    let mut verified = dump.builds[0].evidence.clone();
    verified.catalog_verified = true;
    verified.round_trip_verified = true;
    verified.build_id = retro_junk_archive::BuildId::new();
    verified.performed_at = "2026-02-01T00:00:00Z".to_owned();
    retro_junk_archive::write_build_evidence(&dump.directory, &verified).unwrap();

    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &playable_root,
        representation_id: &representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: None,
    })
    .unwrap();

    let rescanned = retro_junk_archive::scan_archive(&archive).unwrap();
    let current = retro_junk_archive::current_build_evidence(
        &rescanned.releases[0].physical_copies[0].carriers[0].dumps[0],
    );
    assert_eq!(current.len(), 1);
    assert!(current[0].catalog_verified);
    assert!(
        current[0].round_trip_verified,
        "the rename dropped a round-trip verification it had no business touching"
    );
}

#[test]
fn a_name_collision_stops_rather_than_destroying_the_other_file() {
    let temp = tempfile::tempdir().unwrap();
    let (archive, playable_root, representation_id) =
        archive_with_playable(temp.path(), "Game (USA) (Track 1).chd");
    let occupied = playable_root.join("psx").join("Game (USA).chd");
    std::fs::write(&occupied, b"someone else's bytes").unwrap();

    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let result = rename_playable(&RenamePlayableRequest {
        snapshot: &snapshot,
        playable_root: &playable_root,
        representation_id: &representation_id,
        canonical_file_name: "Game (USA).chd",
        media_root: None,
    });
    assert!(matches!(result, Err(RenamePlayableError::TargetExists(_))));
    assert_eq!(
        std::fs::read(&occupied).unwrap(),
        b"someone else's bytes",
        "the existing file was overwritten"
    );
}
