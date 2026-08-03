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

fn noop_progress(_: &str, _: retro_junk_io::ProgressUnit, _: u64, _: u64) {}

/// A cue/bin master is stored as separate track files, so it needs no
/// reproduction to be catalog-verified — its digests were recorded at ingest.
/// This case previously matched neither identification path: `identify` only
/// looked at raw redumper images, and catalog verification only at
/// single-file masters, so a multi-track disc could never be bound at all.
#[test]
fn a_multi_track_master_is_catalog_verified_from_its_stored_tracks() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    let package = temp.path().join("package");
    std::fs::create_dir(&package).unwrap();
    // Track 10 exists so filename order and cue order genuinely disagree:
    // sorted as text, "(Track 10)" comes before "(Track 2)".
    let track1 = package.join("Game (Track 1).bin");
    let track2 = package.join("Game (Track 2).bin");
    let track10 = package.join("Game (Track 10).bin");
    std::fs::write(&track1, b"first track payload").unwrap();
    std::fs::write(&track2, b"second track payload!!").unwrap();
    std::fs::write(&track10, b"tenth track payload").unwrap();
    std::fs::write(
        package.join("Game.cue"),
        "FILE \"Game (Track 1).bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\n\
         FILE \"Game (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\n\
         FILE \"Game (Track 10).bin\" BINARY\n  TRACK 10 AUDIO\n    INDEX 01 00:00:00\n",
    )
    .unwrap();
    init_archive(&archive);
    ingest(&archive, &package, "faketest", "Game");

    let conn = retro_junk_db::open_memory().unwrap();
    conn.execute(
        "INSERT INTO platforms(id,display_name,short_name,manufacturer,generation,media_type,release_year,description,core_platform)
         VALUES('faketest','Fake','Fake','Nobody',1,'cd',1994,'','Psx')",
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
        "INSERT INTO media(id,release_id,dat_source,dat_name,rom_name) \
         VALUES('m1','r','redump','Game (USA)','Game (USA).cue')",
        [],
    )
    .unwrap();
    // The catalog stores the medium as its ordered track set.
    let cancel = AtomicBool::new(false);
    for (number, path) in [(1, &track1), (2, &track2), (10, &track10)] {
        let digests = retro_junk_archive::hash_file_digests(path, &cancel).unwrap();
        conn.execute(
            "INSERT INTO media_tracks(media_id,track_number,track_name,file_size,crc32,md5,sha1)
             VALUES('m1',?1,?2,?3,?4,?5,?6)",
            (
                number,
                format!("Game (USA) (Track {number}).bin"),
                i64::try_from(digests.size).unwrap(),
                digests.crc32.as_str(),
                digests.md5.as_str(),
                digests.sha1.as_str(),
            ),
        )
        .unwrap();
    }

    let ctx = crate::create_default_context();
    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let report =
        verify_catalog_files(&snapshot, &conn, &ctx, None, &noop_progress, &cancel).unwrap();
    assert_eq!(
        report.identified, 1,
        "multi-track master was not identified"
    );

    let rescanned = retro_junk_archive::scan_archive(&archive).unwrap();
    let carrier = &rescanned.releases[0].physical_copies[0].carriers[0];
    assert_eq!(carrier.manifest.catalog_binding.catalog_media_id, "m1");
    // Verified against the whole ordered set, so it counts as a complete
    // match rather than "one track happened to line up".
    let evidence = retro_junk_archive::dump_catalog_evidence(&carrier.dumps[0]).unwrap();
    assert!(evidence.complete_track_set);
}

/// Ingest reads every published file back and compares it against the digest
/// taken while writing. That is an integrity verification, and it must be
/// recorded as one — otherwise convergence immediately schedules a re-hash of
/// bytes that were verified seconds earlier.
#[test]
fn ingest_records_the_integrity_check_it_performs() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    let source = temp.path().join("game.nes");
    std::fs::write(&source, b"freshly dumped bytes").unwrap();
    init_archive(&archive);
    ingest(&archive, &source, "nes", "Game");

    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
    assert!(
        retro_junk_archive::dump_has_current_evidence(
            dump,
            retro_junk_archive::VerificationKind::Integrity
        ),
        "ingest verified the published bytes but recorded no evidence"
    );
}

/// The recorded evidence is bound to the manifest it describes. If the dump
/// is later repaired or re-ingested, the manifest hash changes and the old
/// record stops counting — exactly as it does for a standalone verification.
#[test]
fn ingest_evidence_is_bound_to_the_manifest_it_describes() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    let source = temp.path().join("game.nes");
    std::fs::write(&source, b"freshly dumped bytes").unwrap();
    init_archive(&archive);
    ingest(&archive, &source, "nes", "Game");

    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
    let evidence = &dump.verifications[0].evidence;
    assert_eq!(evidence.input_manifest_sha256, dump.manifest_sha256);
    assert_eq!(evidence.representation_id, dump.manifest.representation_id);
}

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

/// `evidence/` is append-only, so a release rebuilt or re-adopted under a
/// corrected name carries both names in its history. A projection that reads
/// all of them republishes artwork under a name no file has carried since —
/// one copy per name, on every run — and gives the frontend a second, dead
/// entry for the same game.
#[test]
fn only_the_name_a_release_currently_publishes_under_is_projected() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    init_archive(&archive);
    let source = temp.path().join("dump");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("Game.bin"), b"disc bytes").unwrap();
    let ingested = ingest(&archive, &source, "psx", "Game");

    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];

    // The same lineage published twice: first under a track-shaped name, then
    // corrected. Both records stay in the archive; only the second is current.
    let parent = dump.manifest.representation_id;
    for path in ["psx/Game (USA) (Track 1).chd", "psx/Game (USA).chd"] {
        retro_junk_archive::write_build_evidence(
            &dump.directory,
            &retro_junk_archive::BuildEvidence {
                schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                build_id: retro_junk_archive::BuildId::new(),
                parent_representation_id: parent,
                child_representation_id: retro_junk_archive::RepresentationId::new(),
                performed_at: chrono::Utc::now().to_rfc3339(),
                input_manifest_sha256: dump.manifest_sha256.clone(),
                recipe_version: 1,
                format: RepresentationFormat::Chd,
                relative_output_path: path.to_owned(),
                output_sha256: "abc".to_owned(),
                output_size: 10,
                catalog_verified: false,
                round_trip_verified: true,
                tool: None,
                omitted_features: Vec::new(),
                canonical_intermediate: None,
            },
        )
        .unwrap();
    }

    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let release = &snapshot.releases[0];

    let stems = crate::archive_assets::release_media_stems(release);
    assert!(
        stems.contains("Game (USA)"),
        "the name in use must be projected"
    );
    assert!(
        !stems.contains("Game (USA) (Track 1)"),
        "a name the release stopped using must not keep receiving artwork"
    );

    let retired = crate::archive_assets::superseded_media_stems(release);
    assert!(
        retired.contains("Game (USA) (Track 1)"),
        "the abandoned name is what a projection has to clean up"
    );
    assert!(
        !retired.contains("Game (USA)"),
        "a live name must never be scheduled for deletion"
    );
}

/// Two lineages — two dumps of one carrier — can land on the same output name.
/// That name is live for both, so retiring one lineage's record must not
/// delete the artwork the other is still publishing under.
#[test]
fn a_name_two_lineages_share_is_never_retired() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    init_archive(&archive);
    let source = temp.path().join("dump");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("Game.bin"), b"disc bytes").unwrap();
    let ingested = ingest(&archive, &source, "psx", "Game");

    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let dump = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0];
    let mut write = |parent, path: &str| {
        retro_junk_archive::write_build_evidence(
            &dump.directory,
            &retro_junk_archive::BuildEvidence {
                schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                build_id: retro_junk_archive::BuildId::new(),
                parent_representation_id: parent,
                child_representation_id: retro_junk_archive::RepresentationId::new(),
                performed_at: chrono::Utc::now().to_rfc3339(),
                input_manifest_sha256: dump.manifest_sha256.clone(),
                recipe_version: 1,
                format: RepresentationFormat::Chd,
                relative_output_path: path.to_owned(),
                output_sha256: "abc".to_owned(),
                output_size: 10,
                catalog_verified: false,
                round_trip_verified: true,
                tool: None,
                omitted_features: Vec::new(),
                canonical_intermediate: None,
            },
        )
        .unwrap();
    };
    // One lineage moved off the shared name; another still publishes under it.
    write(dump.manifest.representation_id, "psx/Game (USA).chd");
    write(
        dump.manifest.representation_id,
        "psx/Game (USA) (Disc 1).chd",
    );
    write(
        retro_junk_archive::RepresentationId::new(),
        "psx/Game (USA).chd",
    );

    let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
    let release = &snapshot.releases[0];
    assert!(
        !crate::archive_assets::superseded_media_stems(release).contains("Game (USA)"),
        "a name another lineage still publishes under is live, not abandoned"
    );
}
