use std::sync::atomic::AtomicBool;

use crate::{
    ArchiveLayout, ArchiveRootManifest, CarrierId, DumpManifest, IngestRequest, NewCarrierDump,
    RepresentationFormat, VerificationEvidence, VerificationId, VerificationKind,
    VerificationOutcome, execute_ingest, ingest_new_carrier_dump, initialize_archive,
    normalize_relative_path, plan_ingest, read_toml, scan_archive,
    upgrade_legacy_regional_physical_platforms, verify_dump_integrity, write_json_new,
};

#[test]
fn rejects_parent_traversal() {
    assert!(normalize_relative_path(std::path::Path::new("../escape")).is_err());
}

#[test]
fn archive_lock_is_exclusive_and_released_on_drop() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    assert!(crate::ArchiveLock::acquire(&root).is_err());
    initialize_archive(&root, &ArchiveRootManifest::new("Lock test")).unwrap();
    let lock = crate::ArchiveLock::acquire(&root).unwrap();
    assert!(matches!(
        crate::ArchiveLock::acquire(&root),
        Err(crate::ArchiveLockError::Busy(_))
    ));
    drop(lock);
    crate::ArchiveLock::acquire(&root).unwrap();
}

#[test]
fn archive_lock_waits_for_the_current_writer() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    initialize_archive(&root, &ArchiveRootManifest::new("Wait test")).unwrap();
    let held = crate::ArchiveLock::acquire(&root).unwrap();
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let worker_root = root.clone();
    let worker_cancel = cancel.clone();
    let waiter = std::thread::spawn(move || {
        crate::ArchiveLock::acquire_wait(&worker_root, &worker_cancel)
            .unwrap()
            .is_some()
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(!waiter.is_finished());
    drop(held);
    assert!(waiter.join().unwrap());
}

#[test]
fn low_level_ingest_rejects_multiple_redumper_images() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    let source = temp.path().join("combined-redumper");
    initialize_archive(&archive, &ArchiveRootManifest::new("Boundary test")).unwrap();
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("disc1.scram"), b"disc one").unwrap();
    std::fs::write(source.join("disc2.scram"), b"disc two").unwrap();

    let result = ingest_new_carrier_dump(
        &archive,
        &source,
        NewCarrierDump {
            platform_id: "ps1".to_owned(),
            title: "Two Disc Game".to_owned(),
            region: "japan".to_owned(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: String::new(),
            sequence_number: 1,
            carrier_label: String::new(),
            carrier_kind: crate::CarrierKind::OpticalDisc,
            format: RepresentationFormat::RedumperRaw,
            catalog_binding: crate::CatalogBinding::default(),
            source_package: crate::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    );

    let error = result.unwrap_err().to_string();
    assert!(error.contains("disc1, disc2"));
    assert!(error.contains("separate subdirectory"));
    assert_eq!(scan_archive(&archive).unwrap().releases.len(), 0);
}

#[test]
fn legacy_japanese_nes_release_is_moved_to_famicom_without_recopying_dump() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    let mut root_manifest = ArchiveRootManifest::new("Famicom migration");
    root_manifest.applied_migrations.clear();
    initialize_archive(&archive, &root_manifest).unwrap();
    let rom = temp.path().join("game.nes");
    std::fs::write(&rom, b"preserved bytes").unwrap();
    let imported = ingest_new_carrier_dump(
        &archive,
        &rom,
        NewCarrierDump {
            platform_id: "nes".to_owned(),
            title: "Japanese Game".to_owned(),
            region: "Japan".to_owned(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: String::new(),
            sequence_number: 0,
            carrier_label: String::new(),
            carrier_kind: crate::CarrierKind::Cartridge,
            format: RepresentationFormat::Rom,
            catalog_binding: crate::CatalogBinding::default(),
            source_package: crate::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let old_release_directory = imported
        .dump_directory
        .ancestors()
        .find(|path| path.join("release.toml").is_file())
        .unwrap()
        .to_path_buf();

    assert_eq!(
        upgrade_legacy_regional_physical_platforms(&archive).unwrap(),
        1
    );
    assert!(!old_release_directory.exists());
    let snapshot = scan_archive(&archive).unwrap();
    assert_eq!(snapshot.releases[0].manifest.platform_id, "famicom");
    assert!(
        snapshot.releases[0]
            .directory
            .starts_with(archive.join("famicom"))
    );
    let raw = &snapshot.releases[0].physical_copies[0].carriers[0].dumps[0]
        .directory
        .join("raw/game.nes");
    assert_eq!(std::fs::read(raw).unwrap(), b"preserved bytes");
    assert_eq!(
        upgrade_legacy_regional_physical_platforms(&archive).unwrap(),
        0
    );
}

#[test]
fn legacy_japanese_saturn_release_is_moved_to_saturnjp_without_recopying_dump() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    let mut root_manifest = ArchiveRootManifest::new("Japanese Saturn migration");
    root_manifest.applied_migrations.clear();
    initialize_archive(&archive, &root_manifest).unwrap();
    let disc = temp.path().join("game.iso");
    std::fs::write(&disc, b"preserved saturn bytes").unwrap();
    let imported = ingest_new_carrier_dump(
        &archive,
        &disc,
        NewCarrierDump {
            platform_id: "saturn".to_owned(),
            title: "Japanese Saturn Game".to_owned(),
            region: "Japan".to_owned(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: String::new(),
            sequence_number: 1,
            carrier_label: String::new(),
            carrier_kind: crate::CarrierKind::OpticalDisc,
            format: RepresentationFormat::Iso,
            catalog_binding: crate::CatalogBinding::default(),
            source_package: crate::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let old_release_directory = imported
        .dump_directory
        .ancestors()
        .find(|path| path.join("release.toml").is_file())
        .unwrap()
        .to_path_buf();

    assert_eq!(
        upgrade_legacy_regional_physical_platforms(&archive).unwrap(),
        1
    );
    assert!(!old_release_directory.exists());
    let snapshot = scan_archive(&archive).unwrap();
    assert_eq!(snapshot.releases[0].manifest.platform_id, "saturnjp");
    assert!(
        snapshot.releases[0]
            .directory
            .starts_with(archive.join("saturnjp"))
    );
    let raw = snapshot.releases[0].physical_copies[0].carriers[0].dumps[0]
        .directory
        .join("raw/game.iso");
    assert_eq!(std::fs::read(raw).unwrap(), b"preserved saturn bytes");
}

#[test]
fn initialization_upgrades_an_empty_schema_one_prototype_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    std::fs::create_dir_all(root.join(".retro-junk")).unwrap();
    let mut manifest = ArchiveRootManifest::new("Prototype");
    manifest.schema_version = 1;
    std::fs::write(
        root.join("retro-junk-archive.toml"),
        toml::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    initialize_archive(&root, &manifest).unwrap();
    let upgraded: ArchiveRootManifest = read_toml(&root.join("retro-junk-archive.toml")).unwrap();
    assert_eq!(upgraded.schema_version, crate::MANIFEST_SCHEMA_VERSION);
    assert_eq!(upgraded.profile_id, manifest.profile_id);
}

#[test]
fn portable_hierarchy_and_append_only_evidence_are_scannable() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    let root = ArchiveRootManifest::new("Test Collection");
    initialize_archive(&archive, &root).unwrap();
    let source = temp.path().join("game.nes");
    std::fs::write(&source, b"rom bytes").unwrap();
    let ingested = ingest_new_carrier_dump(
        &archive,
        &source,
        NewCarrierDump {
            platform_id: "nes".to_owned(),
            title: "Test Game".to_owned(),
            region: "usa".to_owned(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: "boxed copy".to_owned(),
            serial: "NES-TG-USA".to_owned(),
            sequence_number: 0,
            carrier_label: "cartridge".to_owned(),
            carrier_kind: crate::CarrierKind::Cartridge,
            format: RepresentationFormat::Rom,
            catalog_binding: crate::CatalogBinding::default(),
            source_package: crate::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let verification_id = VerificationId::new();
    let evidence_dir = ingested.dump_directory.join("evidence");
    std::fs::create_dir(&evidence_dir).unwrap();
    write_json_new(
        &evidence_dir.join(format!("verification-{verification_id}.json")),
        &VerificationEvidence {
            schema_version: crate::MANIFEST_SCHEMA_VERSION,
            verification_id,
            representation_id: ingested.dump.representation_id,
            performed_at: "2026-07-21T00:00:00Z".to_owned(),
            input_manifest_sha256: "manifest-hash".to_owned(),
            kind: VerificationKind::Integrity,
            outcome: VerificationOutcome::Verified,
            tool: None,
            catalog: None,
            tracks: Vec::new(),
            detail: "verified".to_owned(),
        },
    )
    .unwrap();

    let snapshot = scan_archive(&archive).unwrap();
    assert_eq!(snapshot.releases.len(), 1);
    assert_eq!(
        snapshot.releases[0].physical_copies[0].carriers[0].dumps[0]
            .verifications
            .len(),
        1
    );
}

#[test]
fn ingest_copies_hashes_publishes_and_verifies() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("disc.scram"), b"preservation bytes").unwrap();
    std::fs::write(source.join("disc.fulltoc"), b"toc").unwrap();

    let destination = temp.path().join("archive/dump-1");
    let plan = plan_ingest(&source, &destination).unwrap();
    let manifest = DumpManifest::new(CarrierId::new(), RepresentationFormat::RedumperRaw);
    let result = execute_ingest(
        IngestRequest { plan, manifest },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();

    assert_eq!(
        std::fs::read(source.join("disc.scram")).unwrap(),
        b"preservation bytes"
    );
    let persisted: DumpManifest = read_toml(&destination.join("dump.toml")).unwrap();
    assert_eq!(persisted, result);
    assert_eq!(persisted.files.len(), 2);
    assert!(
        persisted.files.iter().all(|file| {
            !file.crc32.is_empty() && !file.md5.is_empty() && !file.sha1.is_empty()
        })
    );
    assert!(
        verify_dump_integrity(&destination, &persisted, &AtomicBool::new(false))
            .unwrap()
            .is_verified()
    );
    std::fs::write(destination.join("raw/unrecorded.txt"), b"extra").unwrap();
    let report = verify_dump_integrity(&destination, &persisted, &AtomicBool::new(false)).unwrap();
    assert!(!report.is_verified());
    assert!(
        report
            .failures
            .iter()
            .any(|failure| failure.path == "unrecorded.txt")
    );
}

#[test]
fn ingest_rejects_bytes_that_differ_from_precomputed_staging_digests() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("game.rom");
    std::fs::write(&source, b"changed bytes").unwrap();
    let destination = temp.path().join("archive/dump-1");
    let mut plan = plan_ingest(&source, &destination).unwrap();
    plan.files[0].expected_digests = Some(crate::FileDigests {
        size: 13,
        crc32: "00000000".to_owned(),
        md5: "wrong".to_owned(),
        sha1: "wrong".to_owned(),
        sha256: "wrong".to_owned(),
    });
    let manifest = DumpManifest::new(CarrierId::new(), RepresentationFormat::Rom);
    assert!(matches!(
        execute_ingest(
            IngestRequest { plan, manifest },
            &AtomicBool::new(false),
            |_| {},
        ),
        Err(crate::IngestError::CopyMismatch(_))
    ));
    assert!(!destination.exists());
}

#[test]
fn presence_is_distinct_from_integrity_and_build_freshness() {
    let temp = tempfile::tempdir().unwrap();
    let dump = temp.path().join("dump");
    std::fs::create_dir_all(dump.join("raw")).unwrap();
    std::fs::write(dump.join("raw/game.rom"), b"game").unwrap();
    let mut manifest = DumpManifest::new(CarrierId::new(), RepresentationFormat::Rom);
    manifest.files.push(crate::ArchivedFile {
        path: "game.rom".to_owned(),
        size: 4,
        crc32: String::new(),
        md5: String::new(),
        sha1: String::new(),
        sha256: "not-read-for-presence".to_owned(),
    });
    assert_eq!(
        crate::preservation_presence(&dump, &manifest),
        crate::RepresentationPresence::Present
    );
    manifest.files[0].size = 5;
    assert_eq!(
        crate::preservation_presence(&dump, &manifest),
        crate::RepresentationPresence::Modified
    );

    let output = temp.path().join("playable/game.chd");
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    std::fs::write(&output, b"chd").unwrap();
    let evidence = crate::BuildEvidence {
        schema_version: crate::MANIFEST_SCHEMA_VERSION,
        build_id: crate::BuildId::new(),
        parent_representation_id: manifest.representation_id,
        child_representation_id: crate::RepresentationId::new(),
        performed_at: "2026-07-21T00:00:00Z".to_owned(),
        input_manifest_sha256: "old".to_owned(),
        recipe_version: 1,
        format: RepresentationFormat::Chd,
        relative_output_path: "game.chd".to_owned(),
        output_sha256: String::new(),
        output_size: 3,
        catalog_verified: false,
        round_trip_verified: false,
        tool: None,
        omitted_features: Vec::new(),
        canonical_intermediate: None,
    };
    assert_eq!(
        crate::playable_presence(&temp.path().join("playable"), "new", &evidence),
        crate::RepresentationPresence::Stale
    );
}

#[test]
fn archive_layout_groups_release_physical_copy_and_carrier() {
    let temp = tempfile::tempdir().unwrap();
    let layout = ArchiveLayout::new(temp.path());
    let release = layout.release_dir(
        "ps1",
        "Final Fantasy VII",
        "usa",
        "",
        crate::ArchiveReleaseId::new(),
    );
    let physical_copy = ArchiveLayout::physical_copy_dir(&release, 1);
    let carrier = ArchiveLayout::carrier_dir(&physical_copy, "SCUS-94163", 1);
    let second_carrier = ArchiveLayout::carrier_dir(&physical_copy, "SCUS-94163", 2);
    assert!(
        carrier
            .to_string_lossy()
            .contains("ps1/final-fantasy-vii-usa/physical-copies/copy-01")
    );
    assert!(carrier.to_string_lossy().ends_with("carriers/scus-94163"));
    assert!(
        second_carrier
            .to_string_lossy()
            .ends_with("carriers/scus-94163-disc-2")
    );
}

#[test]
fn platform_playable_default_is_portable_replaceable_and_clearable() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = crate::ArchiveRootManifest::new("Policy test");
    crate::initialize_archive(temp.path(), &manifest).unwrap();
    let policy = crate::DesiredPlayablePolicy {
        format: RepresentationFormat::Chd,
        retain_canonical_intermediate: true,
        allow_unverified: false,
        options: std::collections::BTreeMap::default(),
    };
    let updated = crate::set_platform_playable_default(temp.path(), "psx", Some(policy)).unwrap();
    assert_eq!(updated.platform_defaults.len(), 1);
    assert_eq!(
        updated.platform_defaults[0].policy.format,
        RepresentationFormat::Chd
    );

    let cleared = crate::set_platform_playable_default(temp.path(), "PSX", None).unwrap();
    assert!(cleared.platform_defaults.is_empty());
    let reread: crate::ArchiveRootManifest =
        crate::read_toml(&temp.path().join("retro-junk-archive.toml")).unwrap();
    assert!(reread.platform_defaults.is_empty());
}

#[cfg(unix)]
#[test]
fn redumper_audit_uses_disposable_copy_and_parses_track_records() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let raw = temp.path().join("raw");
    let work = temp.path().join("work");
    std::fs::create_dir(&raw).unwrap();
    std::fs::write(raw.join("disc.scram"), b"raw master").unwrap();
    std::fs::write(raw.join("disc.state"), b"state").unwrap();
    std::fs::write(raw.join("disc.toc"), b"toc").unwrap();
    let tool = temp.path().join("redumper");
    std::fs::write(
        &tool,
        r#"#!/bin/sh
if [ "$1" = "--help" ]; then echo "redumper build test"; exit 0; fi
if [ "$1" = "split" ]; then
  printf 'FILE "disc (Track 01).bin" BINARY\n' > disc.cue
  printf 'track' > 'disc (Track 01).bin'
fi
echo '<rom name="disc (Track 01).bin" size="5" crc="AABBCCDD" md5="0011" sha1="11223344" />'
"#,
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&tool).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&tool, permissions).unwrap();

    let redumper = crate::Redumper::detect(&tool).unwrap();
    let audit = redumper
        .audit(&raw, &work, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(audit.tracks.len(), 1);
    assert_eq!(audit.tracks[0].sha1, "11223344");
    assert!(!raw.join("disc.cue").exists());
    assert_eq!(std::fs::read_dir(&work).unwrap().count(), 0);
    let mut phases = Vec::new();
    let prepared = redumper
        .prepare_with_phase_progress(
            &raw,
            &work,
            &AtomicBool::new(false),
            |phase, current, total| phases.push((phase.to_owned(), current, total)),
        )
        .unwrap();
    assert!(
        phases
            .iter()
            .any(|(phase, _, total)| { phase == "Copying Redumper source files" && *total > 0 })
    );
    assert!(phases.iter().any(|(phase, current, total)| {
        phase == "Running Redumper split" && *current == 0 && *total == 0
    }));
    assert!(phases.iter().any(|(phase, current, total)| {
        phase == "Running Redumper hash" && *current == 0 && *total == 0
    }));
    let retained = temp.path().join("intermediate");
    let files = prepared
        .retain_intermediate(&retained, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(files.len(), 2);
    assert!(retained.join("raw/disc.cue").is_file());
    assert!(retained.join("raw/disc (Track 01).bin").is_file());
}

#[test]
fn release_assets_are_copied_and_indexed_as_authoritative_originals() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    initialize_archive(&archive, &ArchiveRootManifest::new("Assets")).unwrap();
    let rom = temp.path().join("game.rom");
    std::fs::write(&rom, b"rom").unwrap();
    let ingested = ingest_new_carrier_dump(
        &archive,
        &rom,
        NewCarrierDump {
            platform_id: "nes".to_owned(),
            title: "Asset Game".to_owned(),
            region: String::new(),
            revision: String::new(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: String::new(),
            sequence_number: 0,
            carrier_label: String::new(),
            carrier_kind: crate::CarrierKind::Cartridge,
            format: RepresentationFormat::Rom,
            catalog_binding: crate::CatalogBinding::default(),
            source_package: crate::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let cover = temp.path().join("cover.png");
    std::fs::write(&cover, b"original pixels").unwrap();
    let request = crate::NewReleaseFile {
        release_id: ingested.release.archive_release_id,
        source_file: &cover,
        category: crate::ReleaseFileCategory::Artwork,
        asset_type: "box-front",
        source: "screenscraper",
        source_url: "https://example.invalid/cover",
        caption: "front cover",
    };
    crate::add_release_file(&archive, request, &AtomicBool::new(false)).unwrap();
    let duplicate =
        crate::add_release_files(&archive, &[request], &AtomicBool::new(false)).unwrap();
    assert!(!duplicate[0].added);
    let receipt = temp.path().join("receipt.txt");
    std::fs::write(&receipt, b"provenance").unwrap();
    crate::add_physical_copy_file(
        &archive,
        crate::NewPhysicalCopyFile {
            physical_copy_id: ingested.physical_copy.physical_copy_id,
            source_file: &receipt,
            category: crate::PhysicalCopyFileCategory::Provenance,
            asset_type: "receipt",
            source: "user",
            caption: "purchase receipt",
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    let snapshot = scan_archive(&archive).unwrap();
    assert_eq!(snapshot.releases[0].supporting_files.len(), 1);
    assert_eq!(
        snapshot.releases[0].physical_copies[0]
            .supporting_files
            .len(),
        1
    );
    assert_eq!(
        snapshot.releases[0].supporting_files[0].manifest.file.size,
        15
    );
    assert_eq!(std::fs::read(&cover).unwrap(), b"original pixels");
}

#[test]
fn carrier_binding_replaces_a_single_identity_and_generalizes_a_mixed_parent() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive");
    initialize_archive(&archive, &ArchiveRootManifest::new("Binding")).unwrap();
    let disc = temp.path().join("disc.bin");
    std::fs::write(&disc, b"disc").unwrap();
    let imported = ingest_new_carrier_dump(
        &archive,
        &disc,
        NewCarrierDump {
            platform_id: "saturn".to_owned(),
            title: "Game".to_owned(),
            region: "japan".to_owned(),
            revision: "mastering-a".to_owned(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: "GS-0000".to_owned(),
            sequence_number: 1,
            carrier_label: String::new(),
            carrier_kind: crate::CarrierKind::OpticalDisc,
            format: RepresentationFormat::Rom,
            catalog_binding: crate::CatalogBinding {
                catalog_work_id: "work".to_owned(),
                catalog_release_id: "release-a".to_owned(),
                catalog_media_id: "media-a".to_owned(),
                ..Default::default()
            },
            source_package: crate::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: None,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let release_path = imported
        .dump_directory
        .ancestors()
        .find(|path| path.join("release.toml").is_file())
        .unwrap()
        .join("release.toml");
    let carrier_path = imported
        .dump_directory
        .ancestors()
        .find(|path| path.join("carrier.toml").is_file())
        .unwrap()
        .join("carrier.toml");
    assert!(
        !crate::bind_carrier_to_catalog(
            &release_path,
            &carrier_path,
            &crate::CatalogBinding {
                catalog_work_id: "work".to_owned(),
                catalog_release_id: "release-b".to_owned(),
                catalog_media_id: "media-b".to_owned(),
                ..Default::default()
            },
        )
        .unwrap()
    );
    let rebound = scan_archive(&archive).unwrap();
    assert_eq!(
        rebound.releases[0]
            .manifest
            .catalog_binding
            .catalog_release_id,
        "release-b"
    );

    let second_disc = temp.path().join("disc-2.bin");
    std::fs::write(&second_disc, b"disc two").unwrap();
    let second = ingest_new_carrier_dump(
        &archive,
        &second_disc,
        NewCarrierDump {
            platform_id: "saturn".to_owned(),
            title: "Game".to_owned(),
            region: "japan".to_owned(),
            revision: "mastering-a".to_owned(),
            variant: String::new(),
            owner_id: "default".to_owned(),
            physical_copy_label: String::new(),
            serial: "GS-0000".to_owned(),
            sequence_number: 2,
            carrier_label: String::new(),
            carrier_kind: crate::CarrierKind::OpticalDisc,
            format: RepresentationFormat::Rom,
            catalog_binding: crate::CatalogBinding::default(),
            source_package: crate::SourcePackageRecord::default(),
            expected_files: Vec::new(),
            physical_copy_id: Some(imported.physical_copy.physical_copy_id),
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let second_carrier_path = second
        .dump_directory
        .ancestors()
        .find(|path| path.join("carrier.toml").is_file())
        .unwrap()
        .join("carrier.toml");
    assert!(
        crate::bind_carrier_to_catalog(
            &release_path,
            &second_carrier_path,
            &crate::CatalogBinding {
                catalog_work_id: "work".to_owned(),
                catalog_release_id: "release-c".to_owned(),
                catalog_media_id: "media-c".to_owned(),
                ..Default::default()
            },
        )
        .unwrap()
    );

    let snapshot = scan_archive(&archive).unwrap();
    assert_eq!(
        snapshot.releases[0]
            .manifest
            .catalog_binding
            .catalog_work_id,
        "work"
    );
    assert!(
        snapshot.releases[0]
            .manifest
            .catalog_binding
            .catalog_release_id
            .is_empty()
    );
    let carrier = snapshot.releases[0].physical_copies[0]
        .carriers
        .iter()
        .find(|carrier| carrier.manifest.sequence_number == 2)
        .unwrap();
    assert_eq!(
        carrier.manifest.catalog_binding.catalog_release_id,
        "release-c"
    );
    assert_eq!(carrier.manifest.catalog_binding.catalog_media_id, "media-c");
}
