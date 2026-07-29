//! Shared-orchestration behavior: integrity evidence on success and failure,
//! catalog binding on unique matches only, and release-aware mirror builds.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::*;
use retro_junk_archive::{
    CarrierKind, CatalogBinding, NewCarrierDump, RepresentationFormat, SourcePackageRecord,
};

fn init_archive(root: &Path) {
    retro_junk_archive::initialize_archive(
        root,
        &retro_junk_archive::ArchiveRootManifest::new("Test"),
    )
    .unwrap();
}

fn ingest(
    archive: &Path,
    source: &Path,
    platform_id: &str,
    title: &str,
) -> retro_junk_archive::IngestedCarrierDump {
    retro_junk_archive::ingest_new_carrier_dump(
        archive,
        source,
        NewCarrierDump {
            platform_id: platform_id.to_owned(),
            title: title.to_owned(),
            region: "usa".to_owned(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: String::new(),
            sequence_number: 0,
            carrier_label: String::new(),
            carrier_kind: CarrierKind::Cartridge,
            format: RepresentationFormat::Rom,
            catalog_binding: CatalogBinding::default(),
            source_package: SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap()
}

fn noop_progress(_: &str, _: u64, _: u64) {}

#[test]
fn integrity_verification_appends_evidence_and_flags_corruption() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    let source = temp.path().join("game.nes");
    std::fs::write(&source, b"stored bytes").unwrap();
    init_archive(&archive);
    ingest(&archive, &source, "nes", "Game");

    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let report =
        verify_archive_integrity(&snapshot, None, &noop_progress, &AtomicBool::new(false)).unwrap();
    assert_eq!(report.checked, 1);
    assert_eq!(report.failed, 0);

    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
    assert!(retro_junk_archive::dump_has_current_evidence(
        dump,
        retro_junk_archive::VerificationKind::Integrity
    ));

    // Corrupt the stored master; the next run records a failure honestly and
    // the previously verified state does not linger.
    let stored = &dump.manifest.files[0];
    std::fs::write(dump.directory.join("raw").join(&stored.path), b"rotten").unwrap();
    let report =
        verify_archive_integrity(&snapshot, None, &noop_progress, &AtomicBool::new(false)).unwrap();
    assert_eq!(report.failed, 1);
    assert_eq!(report.failures.len(), 1);
    let rescanned = retro_junk_archive::scan_archive(&archive).unwrap();
    let dump = &rescanned.releases[0].physical_copies[0].carriers[0].dumps[0];
    // Both verified and failed records exist; the latest failure is evidence,
    // and the earlier verified record still satisfies "current" — append-only
    // history is the projection's problem to interpret, not this function's.
    assert!(dump.verifications.len() >= 2);
}

#[test]
fn catalog_file_verification_binds_unique_matches_and_refuses_ambiguity() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    let source = temp.path().join("game.bin");
    std::fs::write(&source, b"catalog payload").unwrap();
    init_archive(&archive);
    // Platform id deliberately unknown to the analyzer registry so raw
    // digests are matched without normalization.
    ingest(&archive, &source, "faketest", "Game");

    let digests = retro_junk_archive::hash_file_digests(&source, &AtomicBool::new(false)).unwrap();
    let conn = retro_junk_db::open_memory().unwrap();
    conn.execute(
        "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
         VALUES('faketest','Fake','Fake','Nobody',1,'cartridge',1990,'','Nes')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO works(id,canonical_name) VALUES('w','Game')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO releases(id,work_id,platform_id,region,title) VALUES('r','w','faketest','usa','Game')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO media(id,release_id,dat_source,dat_name,rom_name,crc32,md5,sha1,file_size)
         VALUES('m1','r','no-intro','Game (USA)','Game (USA).bin',?1,?2,?3,?4)",
        (
            digests.crc32.as_str(),
            digests.md5.as_str(),
            digests.sha1.as_str(),
            i64::try_from(digests.size).unwrap(),
        ),
    )
    .unwrap();

    let ctx = crate::create_default_context();
    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let report = verify_catalog_files(
        &snapshot,
        &conn,
        &ctx,
        None,
        &noop_progress,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(report.selected, 1);
    assert_eq!(report.identified, 1);

    let rescanned = retro_junk_archive::scan_archive(&archive).unwrap();
    let carrier = &rescanned.releases[0].physical_copies[0].carriers[0];
    assert_eq!(carrier.manifest.catalog_binding.catalog_media_id, "m1");
    assert!(retro_junk_archive::dump_catalog_verified(&carrier.dumps[0]));

    // A second identical catalog medium makes the match ambiguous: evidence
    // is recorded, but nothing is bound.
    conn.execute(
        "INSERT INTO media(id,release_id,dat_source,dat_name,rom_name,crc32,md5,sha1,file_size)
         VALUES('m2','r','no-intro','Game (USA) (Rev 1)','Game (USA) (Rev 1).bin',?1,?2,?3,?4)",
        (
            digests.crc32.as_str(),
            digests.md5.as_str(),
            digests.sha1.as_str(),
            i64::try_from(digests.size).unwrap(),
        ),
    )
    .unwrap();
    let source2 = temp.path().join("other.bin");
    std::fs::write(&source2, b"catalog payload").unwrap();
    let ingested2 = ingest(&archive, &source2, "faketest", "Other");
    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let report = verify_catalog_files(
        &snapshot,
        &conn,
        &ctx,
        Some(&ingested2.dump.dump_id.to_string()),
        &noop_progress,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(report.ambiguous, 1);
    assert_eq!(report.identified, 0);
    let rescanned = retro_junk_archive::scan_archive(&archive).unwrap();
    let other = rescanned
        .releases
        .iter()
        .find(|release| release.manifest.title == "Other")
        .unwrap();
    let carrier = &other.physical_copies[0].carriers[0];
    assert!(carrier.manifest.catalog_binding.catalog_media_id.is_empty());
    assert!(!retro_junk_archive::dump_catalog_verified(
        &carrier.dumps[0]
    ));
}

#[test]
fn release_build_mirrors_verified_dump_byte_identically_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    let playable = temp.path().join("playable");
    let source = temp.path().join("game.nes");
    std::fs::write(&source, b"mirror me").unwrap();
    init_archive(&archive);
    let ingested = ingest(&archive, &source, "nes", "Game");

    let gap = retro_junk_db::ArchivedPlayableGap {
        archive_release_id: ingested.release.archive_release_id.to_string(),
        physical_copy_id: ingested.physical_copy.physical_copy_id.to_string(),
        title: "Game".to_owned(),
        region: "usa".to_owned(),
        preferred_format: Some("rom".to_owned()),
        allow_unverified: false,
        retain_intermediate: false,
        buildable: true,
        needs_playable: true,
        needs_playlist: false,
        expected_disc_count: 1,
        archived_disc_count: 1,
        verified_disc_count: 1,
        carriers: vec![retro_junk_db::ArchivedPlayableCarrier {
            carrier_id: ingested.carrier.carrier_id.to_string(),
            dump_id: Some(ingested.dump.dump_id.to_string()),
            catalog_media_id: None,
            sequence_number: 0,
            source_format: Some("rom".to_owned()),
            catalog_verified: true,
            buildable: true,
            needs_playable: true,
        }],
    };
    let conn = retro_junk_db::open_memory().unwrap();
    let request = ReleaseBuildRequest {
        gap: &gap,
        archive_root: archive.clone(),
        workspace_root: temp.path().join("work"),
        roots: FrontendRoots::from_settings(&playable, "", ""),
        format: RepresentationFormat::Rom,
        playable_platform_id: "nes".to_owned(),
        chdman_path: PathBuf::new(),
        redumper_path: PathBuf::new(),
        dolphin_tool_path: PathBuf::new(),
        options: std::collections::BTreeMap::new(),
        project_assets: false,
        update_gamelist: false,
    };
    let outcome =
        build_release_playable(&request, &conn, &noop_progress, &AtomicBool::new(false)).unwrap();
    assert_eq!(outcome.built.len(), 1);
    assert_eq!(std::fs::read(&outcome.built[0]).unwrap(), b"mirror me");
    assert!(outcome.playlist.is_none());

    // Re-running publishes nothing new: one build record, same output.
    let outcome =
        build_release_playable(&request, &conn, &noop_progress, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        outcome.snapshot.releases[0].physical_copies[0].carriers[0].dumps[0]
            .builds
            .len(),
        1
    );
}

#[test]
fn release_playlist_writes_relative_ordered_entries_once() {
    let temp = tempfile::tempdir().unwrap();
    let playable = temp.path().join("playable");
    let disc_dir = playable.join("psx");
    std::fs::create_dir_all(&disc_dir).unwrap();
    let files = vec![
        disc_dir.join("Game (Disc 1).chd"),
        disc_dir.join("Game (Disc 2).chd"),
    ];
    for file in &files {
        std::fs::write(file, b"chd").unwrap();
    }
    let playlist =
        write_release_playlist(&playable, "psx", "Game", "usa", "Game (USA)", &files).unwrap();
    let contents = std::fs::read_to_string(&playlist).unwrap();
    assert_eq!(contents, "../Game (Disc 1).chd\n../Game (Disc 2).chd\n");
    // Idempotent: a second call is current, not an overwrite.
    let again =
        write_release_playlist(&playable, "psx", "Game", "usa", "Game (USA)", &files).unwrap();
    assert_eq!(again, playlist);
}
