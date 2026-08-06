use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::*;
use crate::{
    ArchiveRootManifest, CarrierKind, CatalogBinding, RepresentationFormat, SourcePackageRecord,
    ingest_new_carrier_dump, initialize_archive, scan_archive,
};

/// An archive holding one carrier with one preservation dump, plus a playable
/// output at `playable/<relative>` recorded by build evidence.
struct Fixture {
    _temp: tempfile::TempDir,
    archive: PathBuf,
    playable: PathBuf,
}

impl Fixture {
    fn new(playable_relative: &str, playable_bytes: &[u8]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        let playable = temp.path().join("playable");
        initialize_archive(&archive, &ArchiveRootManifest::new("Adopt test")).unwrap();
        let source = temp.path().join("game.iso");
        std::fs::write(&source, b"preservation bytes").unwrap();
        ingest_new_carrier_dump(
            &archive,
            &source,
            crate::NewCarrierDump {
                platform_id: "saturn".to_owned(),
                title: "Tenchi Muyou! Ryououki Gokuraku CD-ROM".to_owned(),
                region: "Japan".to_owned(),
                revision: String::new(),
                variant: String::new(),
                owner_id: "default".to_owned(),
                physical_copy_label: String::new(),
                serial: String::new(),
                sequence_number: 1,
                carrier_label: String::new(),
                carrier_kind: CarrierKind::OpticalDisc,
                format: RepresentationFormat::Iso,
                catalog_binding: CatalogBinding::default(),
                join_release: None,
                source_package: SourcePackageRecord::default(),
                expected_files: Vec::new(),
                physical_copy_id: None,
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();

        let fixture = Self {
            _temp: temp,
            archive,
            playable,
        };
        fixture.write_playable(playable_relative, playable_bytes);
        let evidence = fixture.build_evidence(playable_relative, playable_bytes);
        write_build_evidence(&fixture.dump_directory(), &evidence).unwrap();
        fixture
    }

    fn dump_directory(&self) -> PathBuf {
        let snapshot = scan_archive(&self.archive).unwrap();
        snapshot.releases[0].physical_copies[0].carriers[0].dumps[0]
            .directory
            .clone()
    }

    fn write_playable(&self, relative: &str, bytes: &[u8]) {
        let path = self.playable.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn build_evidence(&self, relative: &str, bytes: &[u8]) -> BuildEvidence {
        let snapshot = scan_archive(&self.archive).unwrap();
        let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
        let (size, sha256) =
            crate::sha256_file(&self.playable.join(relative), &AtomicBool::new(false)).unwrap();
        assert_eq!(size, bytes.len() as u64);
        BuildEvidence {
            schema_version: crate::MANIFEST_SCHEMA_VERSION,
            build_id: BuildId::new(),
            parent_representation_id: dump.manifest.representation_id,
            child_representation_id: RepresentationId::new(),
            performed_at: chrono::Utc::now().to_rfc3339(),
            input_manifest_sha256: dump.manifest_sha256.clone(),
            recipe_version: 1,
            format: RepresentationFormat::Chd,
            relative_output_path: relative.to_owned(),
            output_sha256: sha256,
            output_size: size,
            catalog_verified: false,
            round_trip_verified: true,
            tool: None,
            omitted_features: Vec::new(),
            canonical_intermediate: None,
        }
    }

    fn snapshot(&self) -> crate::ArchiveIndexSnapshot {
        scan_archive(&self.archive).unwrap()
    }
}

fn rename(from: &Path, to: &Path) {
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::rename(from, to).unwrap();
}

/// The reported failure: a playable renamed outside the archive orphans its
/// build evidence, and nothing re-adopts it by content.
#[test]
fn a_renamed_playable_is_found_again_by_its_recorded_digest() {
    let fixture = Fixture::new(
        "saturn/Game (Japan) (Track 1).chd",
        b"the playable disc image",
    );
    assert!(
        orphaned_playables(
            &fixture.snapshot(),
            &fixture.playable,
            &saturn_system_directory
        )
        .is_empty()
    );

    rename(
        &fixture.playable.join("saturn/Game (Japan) (Track 1).chd"),
        &fixture.playable.join("saturn/Game (Japan).chd"),
    );

    let snapshot = fixture.snapshot();
    let orphans = orphaned_playables(&snapshot, &fixture.playable, &saturn_system_directory);
    assert_eq!(orphans.len(), 1);

    let by_size = index_by_size(std::slice::from_ref(&fixture.playable)).unwrap();
    let located = locate_by_content(
        &fixture.playable,
        &orphans[0],
        &by_size,
        &claimed_playable_paths(&snapshot),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(located.as_deref(), Some("saturn/Game (Japan).chd"));

    record_adoption(&orphans[0], "saturn/Game (Japan).chd").unwrap();
    // The adoption is durable: a fresh scan resolves by path, with no hashing.
    assert!(
        orphaned_playables(
            &fixture.snapshot(),
            &fixture.playable,
            &saturn_system_directory
        )
        .is_empty()
    );
}

/// Adoption must retire the old path, not add a second live representation
/// beside it — otherwise the release keeps deriving adoption work forever and
/// the projection carries a permanently missing playable.
#[test]
fn adoption_supersedes_the_evidence_that_named_the_old_path() {
    let fixture = Fixture::new("saturn/old.chd", b"the playable disc image");
    rename(
        &fixture.playable.join("saturn/old.chd"),
        &fixture.playable.join("saturn/new.chd"),
    );
    let snapshot = fixture.snapshot();
    let orphans = orphaned_playables(&snapshot, &fixture.playable, &saturn_system_directory);
    record_adoption(&orphans[0], "saturn/new.chd").unwrap();

    let snapshot = fixture.snapshot();
    let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
    // Both records are kept — the archive is append-only — but only the newer
    // one describes a file that should exist now.
    assert_eq!(dump.builds.len(), 2);
    let current = current_build_evidence(dump);
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].relative_output_path, "saturn/new.chd");
    // A new representation id is required: the projection keys rows by it and
    // reusing one would collide with the record still naming the old path.
    assert_ne!(
        current[0].child_representation_id,
        orphans[0].evidence.child_representation_id
    );
}

/// Two outputs with identical bytes must not both adopt the same file.
#[test]
fn one_file_cannot_satisfy_two_orphans() {
    let fixture = Fixture::new("saturn/disc-1.chd", b"identical disc bytes");
    // A second build of the same dump, different format so it is its own
    // lineage, writing byte-identical output.
    fixture.write_playable("saturn/disc-2.chd", b"identical disc bytes");
    let mut second = fixture.build_evidence("saturn/disc-2.chd", b"identical disc bytes");
    second.format = RepresentationFormat::Iso;
    write_build_evidence(&fixture.dump_directory(), &second).unwrap();

    // Both move; only one destination survives.
    std::fs::remove_file(fixture.playable.join("saturn/disc-2.chd")).unwrap();
    rename(
        &fixture.playable.join("saturn/disc-1.chd"),
        &fixture.playable.join("saturn/only-one.chd"),
    );

    let snapshot = fixture.snapshot();
    let orphans = orphaned_playables(&snapshot, &fixture.playable, &saturn_system_directory);
    assert_eq!(orphans.len(), 2);
    let by_size = index_by_size(std::slice::from_ref(&fixture.playable)).unwrap();
    let mut claimed = claimed_playable_paths(&snapshot);
    let mut located = Vec::new();
    for orphan in &orphans {
        if let Some(path) = locate_by_content(
            &fixture.playable,
            orphan,
            &by_size,
            &claimed,
            &AtomicBool::new(false),
        )
        .unwrap()
        {
            claimed.insert(path.clone());
            located.push(path);
        }
    }
    assert_eq!(located, vec!["saturn/only-one.chd".to_owned()]);
}

/// A file that merely happens to be the same size is not the output. Size is
/// the search filter; the digest is the decision.
#[test]
fn a_same_size_impostor_is_not_adopted() {
    let fixture = Fixture::new("saturn/game.chd", b"the playable disc image");
    std::fs::remove_file(fixture.playable.join("saturn/game.chd")).unwrap();
    fixture.write_playable("saturn/other.chd", b"THE PLAYABLE DISC IMAGE");

    let snapshot = fixture.snapshot();
    let orphans = orphaned_playables(&snapshot, &fixture.playable, &saturn_system_directory);
    let by_size = index_by_size(std::slice::from_ref(&fixture.playable)).unwrap();
    assert!(
        locate_by_content(
            &fixture.playable,
            &orphans[0],
            &by_size,
            &claimed_playable_paths(&snapshot),
            &AtomicBool::new(false),
        )
        .unwrap()
        .is_none()
    );
}

/// A scoped repair must not stat the whole playable library: on a network
/// share that is thousands of round trips per release. It searches the system
/// directories the orphans' recorded paths name.
#[test]
fn a_scoped_search_is_bounded_to_the_systems_the_orphans_name() {
    let fixture = Fixture::new("saturn/game.chd", b"the playable disc image");
    fixture.write_playable("psx/unrelated.chd", b"a different game entirely");
    std::fs::rename(
        fixture.playable.join("saturn/game.chd"),
        fixture.playable.join("saturn/game (Japan).chd"),
    )
    .unwrap();

    let snapshot = fixture.snapshot();
    let orphans = orphaned_playables(&snapshot, &fixture.playable, &saturn_system_directory);
    let directories = search_directories(&fixture.playable, &orphans);
    assert_eq!(directories, vec![fixture.playable.join("saturn")]);

    // Bounded, but still enough to find the rename.
    let by_size = index_by_size(&directories).unwrap();
    assert_eq!(
        locate_by_content(
            &fixture.playable,
            &orphans[0],
            &by_size,
            &claimed_playable_paths(&snapshot),
            &AtomicBool::new(false),
        )
        .unwrap()
        .as_deref(),
        Some("saturn/game (Japan).chd")
    );
}

/// Overlapping search directories would index one file twice and let a second
/// orphan adopt a path the first already claimed.
#[test]
fn nested_search_directories_are_walked_once() {
    let fixture = Fixture::new("saturn/game.chd", b"the playable disc image");
    let index =
        index_by_size(&[fixture.playable.clone(), fixture.playable.join("saturn")]).unwrap();
    let entries: usize = index.values().map(Vec::len).sum();
    assert_eq!(entries, 1);
}

/// Evidence written before outputs were filed under a platform directory
/// records a bare file name. The projection has always followed those into the
/// frontend's system directory and reported them present — but the orphan scan
/// checked the recorded path alone, so on the reference archive 248 outputs
/// were simultaneously "present" to one and "missing" to the other, and a real
/// adoption run would have appended a redundant evidence record for every one.
/// Both now resolve through `resolve_playable`.
#[test]
fn a_bare_recorded_path_resolved_by_the_projection_is_not_an_orphan() {
    let fixture = Fixture::new("saturn/game.chd", b"the playable disc image");
    // Rewrite the evidence to the pre-platform-directory shape: a bare name,
    // while the file itself sits under the system directory.
    let mut evidence = fixture.build_evidence("saturn/game.chd", b"the playable disc image");
    evidence.relative_output_path = "game.chd".to_owned();
    write_build_evidence(&fixture.dump_directory(), &evidence).unwrap();

    let snapshot = fixture.snapshot();
    assert!(
        orphaned_playables(&snapshot, &fixture.playable, &saturn_system_directory).is_empty(),
        "a file the projection already resolves must not be re-adopted"
    );
    // And the shared resolution reports where it actually is.
    let current =
        current_build_evidence(&snapshot.releases[0].physical_copies[0].carriers[0].dumps[0]);
    let (path, presence) = crate::presence::resolve_playable(
        &fixture.playable,
        &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0].manifest_sha256,
        current[0],
        "saturn",
    );
    assert_eq!(path, "saturn/game.chd");
    assert_eq!(presence, crate::RepresentationPresence::Present);
}

/// The fixture's outputs live under `saturn/`; the frontend directory mapping
/// is the projection's business, so tests supply their own.
fn saturn_system_directory(_platform_id: &str, _region: &str) -> String {
    "saturn".to_owned()
}
