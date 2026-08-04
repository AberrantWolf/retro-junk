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

/// Regression: a crashed holder's existence-lock (old binaries, or the
/// no-advisory-lock fallback) used to wedge the archive for 24 hours on
/// macOS. A demonstrably dead PID must be reclaimed immediately.
#[cfg(unix)]
#[test]
fn archive_lock_reclaims_a_lock_left_by_a_dead_process() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    initialize_archive(&root, &ArchiveRootManifest::new("Dead holder test")).unwrap();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    child.wait().unwrap();
    let dead_pid = child.id();
    std::fs::write(
        root.join(".retro-junk/archive.lock"),
        format!("pid={dead_pid} started_at=2026-07-29T12:28:08.272960+00:00\n"),
    )
    .unwrap();
    crate::ArchiveLock::acquire(&root).unwrap();
}

/// A live existence-holder (a process that acquired on a filesystem without
/// advisory locks, or an older binary) must not have its lock stolen just
/// because the OS lock on the file is free.
#[cfg(unix)]
#[test]
fn archive_lock_respects_a_live_existence_holder() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    initialize_archive(&root, &ArchiveRootManifest::new("Live holder test")).unwrap();
    std::fs::write(
        root.join(".retro-junk/archive.lock"),
        format!(
            "pid={} started_at=2026-07-30T00:00:00+00:00\n",
            std::process::id()
        ),
    )
    .unwrap();
    assert!(matches!(
        crate::ArchiveLock::acquire(&root),
        Err(crate::ArchiveLockError::Busy(_))
    ));
}

/// A network share is shared: another machine's lock record must never be
/// PID-probed here. Its PID numbers mean nothing on this host, and a
/// dead-looking foreign holder may be very much alive — deleting its lock
/// opens the archive to two concurrent writers, the exact corruption the
/// lock exists to prevent. Only the conservative age window may reclaim it.
#[cfg(unix)]
#[test]
fn archive_lock_never_pid_probes_another_hosts_record() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    initialize_archive(&root, &ArchiveRootManifest::new("Foreign holder test")).unwrap();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    child.wait().unwrap();
    let dead_pid = child.id();
    std::fs::write(
        root.join(".retro-junk/archive.lock"),
        format!("pid={dead_pid} host=some-other-machine started_at=2026-07-29T12:00:00+00:00\n"),
    )
    .unwrap();
    assert!(matches!(
        crate::ArchiveLock::acquire(&root),
        Err(crate::ArchiveLockError::Busy(_))
    ));
}

/// The complement: a record that names this host reclaims immediately on a
/// demonstrably dead PID, exactly like an unattributed local record.
#[cfg(unix)]
#[test]
fn archive_lock_reclaims_this_hosts_dead_holder_by_name() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    initialize_archive(&root, &ArchiveRootManifest::new("Named dead holder test")).unwrap();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    child.wait().unwrap();
    let dead_pid = child.id();
    let host = retro_junk_io::local_host_id().expect("unix hosts can name themselves");
    std::fs::write(
        root.join(".retro-junk/archive.lock"),
        format!("pid={dead_pid} host={host} started_at=2026-07-29T12:00:00+00:00\n"),
    )
    .unwrap();
    crate::ArchiveLock::acquire(&root).unwrap();
}

/// Diagnostics left behind by an OS-mode holder that crashed (its lock was
/// released by the kernel) must never block a new acquisition.
#[test]
fn archive_lock_ignores_crash_leftover_os_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    initialize_archive(&root, &ArchiveRootManifest::new("Leftover test")).unwrap();
    std::fs::write(
        root.join(".retro-junk/archive.lock"),
        "mode=os pid=999999999 started_at=2026-07-29T12:00:00+00:00\n",
    )
    .unwrap();
    crate::ArchiveLock::acquire(&root).unwrap();
}

/// An empty lock file records no claim, but it may be a legacy acquisition
/// mid-write: it must only count as stale once old enough to rule that out.
#[test]
fn empty_lock_file_is_stale_only_after_the_write_window() {
    let temp = tempfile::tempdir().unwrap();
    let lock_path = temp.path().join("archive.lock");
    std::fs::write(&lock_path, "").unwrap();
    assert!(!crate::lock::lock_is_stale(&lock_path));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&lock_path)
        .unwrap();
    let backdated = std::time::SystemTime::now() - std::time::Duration::from_mins(2);
    file.set_times(std::fs::FileTimes::new().set_modified(backdated))
        .unwrap();
    assert!(crate::lock::lock_is_stale(&lock_path));
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
    // Ingest already recorded its own read-back verification here, so this
    // record is appended alongside it.
    let evidence_dir = ingested.dump_directory.join("evidence");
    std::fs::create_dir_all(&evidence_dir).unwrap();
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
    // Evidence is append-only: ingest's own record plus the one written
    // above, both scanned back.
    assert_eq!(
        snapshot.releases[0].physical_copies[0].carriers[0].dumps[0]
            .verifications
            .len(),
        2
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
        IngestRequest {
            plan,
            manifest,
            verify_published_bytes: true,
        },
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
            IngestRequest {
                plan,
                manifest,
                verify_published_bytes: true,
            },
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
        .prepare_with_progress(
            &raw,
            &work,
            &AtomicBool::new(false),
            |phase, unit, current, total| phases.push((phase.to_owned(), unit, current, total)),
        )
        .unwrap();
    // The copy is what makes a disc audit slow, so it has to report bytes: a
    // caller that cannot tell bytes from item counts renders "0 B / 1 B".
    assert!(phases.iter().any(|(phase, unit, _, total)| {
        phase == crate::redumper::COPY_PHASE
            && *unit == retro_junk_io::ProgressUnit::Bytes
            && *total > 0
    }));
    assert!(phases.iter().any(|(phase, _, current, total)| {
        phase == "Running Redumper split" && *current == 0 && *total == 0
    }));
    assert!(phases.iter().any(|(phase, _, current, total)| {
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

/// A network share that refuses to flush a directory (macOS smbfs answers
/// every directory `fsync` with `ENOTSUP`) must not make the archive
/// unwritable — but a filesystem that ran out of room still must fail.
#[test]
#[cfg(unix)]
fn a_filesystem_that_cannot_flush_directories_is_not_a_write_failure() {
    let unsupported = [
        libc::ENOTSUP,
        libc::EOPNOTSUPP,
        libc::EINVAL,
        libc::EBADF,
        libc::EISDIR,
        libc::EPERM,
        libc::EACCES,
    ];
    for code in unsupported {
        assert!(
            crate::manifest::directory_sync_unsupported(&std::io::Error::from_raw_os_error(code)),
            "errno {code} means the directory cannot be flushed, not that the write was lost"
        );
    }
    for code in [libc::ENOSPC, libc::EIO, libc::EDQUOT] {
        assert!(
            !crate::manifest::directory_sync_unsupported(&std::io::Error::from_raw_os_error(code)),
            "errno {code} is a real write failure"
        );
    }
}

/// Indexing reads each manifest once and digests the bytes it parsed. The
/// recorded digest must still be the digest of the file on disk — evidence
/// currency compares them.
#[test]
fn a_scanned_manifest_digest_matches_the_file_on_disk() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    initialize_archive(&root, &ArchiveRootManifest::new("Digest")).unwrap();
    let source = temp.path().join("game.gb");
    std::fs::write(&source, b"game").unwrap();
    ingest_new_carrier_dump(
        &root,
        &source,
        NewCarrierDump {
            platform_id: "gb".to_owned(),
            title: "Game".to_owned(),
            region: "japan".to_owned(),
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

    let snapshot = scan_archive(&root).unwrap();
    let release = &snapshot.releases[0];
    let on_disk =
        |path: &std::path::Path| crate::sha256_file(path, &AtomicBool::new(false)).unwrap().1;
    assert_eq!(
        release.manifest_sha256,
        on_disk(&release.directory.join("release.toml"))
    );
    let copy = &release.physical_copies[0];
    assert_eq!(
        copy.manifest_sha256,
        on_disk(&copy.directory.join("physical-copy.toml"))
    );
    let carrier = &copy.carriers[0];
    assert_eq!(
        carrier.manifest_sha256,
        on_disk(&carrier.directory.join("carrier.toml"))
    );
    let dump = &carrier.dumps[0];
    assert_eq!(
        dump.manifest_sha256,
        on_disk(&dump.directory.join("dump.toml"))
    );
    assert_eq!(
        snapshot.manifest_sha256,
        on_disk(&root.join("retro-junk-archive.toml"))
    );
}

/// An archive mirrored onto exFAT or SMB arrives with an `AppleDouble` sidecar
/// beside every dump file. Those are host metadata, not dump content: counting
/// them would report the whole mirror as corrupt, and ingesting them would
/// bake this device's filesystem into a preservation manifest.
#[test]
fn host_filesystem_sidecars_are_neither_ingested_nor_verified_as_content() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("disc.scram"), b"preservation bytes").unwrap();
    std::fs::write(source.join("._disc.scram"), b"apple double").unwrap();
    std::fs::write(source.join(".DS_Store"), b"finder").unwrap();

    let destination = temp.path().join("archive/dump-1");
    let plan = plan_ingest(&source, &destination).unwrap();
    let manifest = DumpManifest::new(CarrierId::new(), RepresentationFormat::RedumperRaw);
    let persisted = execute_ingest(
        IngestRequest {
            plan,
            manifest,
            verify_published_bytes: true,
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();

    assert_eq!(
        persisted
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["disc.scram"]
    );

    // Sidecars appearing after ingest, as a copy onto exFAT would create.
    std::fs::write(destination.join("raw/._disc.scram"), b"apple double").unwrap();
    std::fs::write(destination.join("raw/.DS_Store"), b"finder").unwrap();
    let report = verify_dump_integrity(&destination, &persisted, &AtomicBool::new(false)).unwrap();

    assert!(report.is_verified(), "unexpected failures: {report:?}");

    // A genuinely unrecorded content file is still a failure.
    std::fs::write(destination.join("raw/unrecorded.bin"), b"extra").unwrap();
    let report = verify_dump_integrity(&destination, &persisted, &AtomicBool::new(false)).unwrap();
    assert!(
        report
            .failures
            .iter()
            .any(|failure| failure.path == "unrecorded.bin")
    );
}

#[test]
fn image_name_discovery_ignores_apple_double_sidecars() {
    // `._disc.scram` sorts ahead of `disc.scram` and carries the same
    // extension, so naming the image from the first match splits a 4 KiB
    // resource fork and fails with "unable to establish base LBA".
    let temp = tempfile::tempdir().unwrap();
    let raw = temp.path().join("raw");
    std::fs::create_dir(&raw).unwrap();
    std::fs::write(raw.join("._disc.scram"), b"\x00\x05\x16\x07resource").unwrap();
    std::fs::write(raw.join("disc.scram"), b"scrambled sectors").unwrap();

    assert_eq!(crate::redumper::find_image_name(&raw).unwrap(), "disc");
}

#[test]
fn image_name_discovery_reports_a_raw_set_that_is_only_sidecars() {
    let temp = tempfile::tempdir().unwrap();
    let raw = temp.path().join("raw");
    std::fs::create_dir(&raw).unwrap();
    std::fs::write(raw.join("._disc.scram"), b"\x00\x05\x16\x07resource").unwrap();

    assert!(matches!(
        crate::redumper::find_image_name(&raw),
        Err(crate::RedumperError::MissingRawImage(_))
    ));
}

/// A busy-lock message has to be readable without doing timezone arithmetic.
/// The record stores RFC 3339 UTC, and printing it raw made a lock taken 34
/// minutes earlier read as "held since 02:06 this morning" to a reader nine
/// hours ahead of UTC — indistinguishable from a wedged archive.
#[test]
fn a_busy_lock_reports_how_long_it_has_been_held_not_a_utc_timestamp() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("archive");
    initialize_archive(&root, &ArchiveRootManifest::new("Busy message")).unwrap();
    let started = chrono::Utc::now() - chrono::Duration::minutes(95);
    std::fs::write(
        root.join(".retro-junk/archive.lock"),
        format!(
            "pid={} started_at={}\n",
            std::process::id(),
            started.to_rfc3339()
        ),
    )
    .unwrap();

    let message = match crate::ArchiveLock::acquire(&root) {
        Err(crate::ArchiveLockError::Busy(details)) => details,
        Err(other) => panic!("expected a busy lock, got {other}"),
        Ok(_) => panic!("expected a busy lock, acquired it instead"),
    };
    assert!(
        message.contains("held 1h35m"),
        "elapsed time, not a timestamp: {message}"
    );
    assert!(
        message.contains(&format!("pid {}", std::process::id())),
        "and who holds it: {message}"
    );
    assert!(
        !message.contains("started_at"),
        "the raw record is what misled: {message}"
    );
}

/// A catalog name is written by people for people, and is used verbatim as a
/// playable's filename — but a name is not a filename. The reference archive
/// lives on exFAT, where a colon is illegal, so a title carrying one could not
/// be written at all; a slash would be worse, silently meaning a directory.
#[test]
fn a_catalog_name_is_made_safe_to_write_without_being_mangled() {
    use crate::safe_file_stem;

    // The overwhelmingly common case: nothing to do.
    assert_eq!(
        safe_file_stem("Castlevania - Symphony of the Night (USA)"),
        "Castlevania - Symphony of the Night (USA)"
    );

    // A colon reads as a subtitle break, which is what No-Intro and Redump
    // already write as " - ".
    assert_eq!(
        safe_file_stem("Harvest Moon: Boy Meets Girl (Japan)"),
        "Harvest Moon - Boy Meets Girl (Japan)"
    );

    // A separator must never survive: it would place the file somewhere else
    // entirely, or fail.
    assert_eq!(safe_file_stem("Either/Or (USA)"), "Either-Or (USA)");
    assert_eq!(safe_file_stem(r"Back\Slash (USA)"), "Back-Slash (USA)");

    // Windows drops trailing dots and spaces silently, which would make the
    // recorded path and the real one differ by something nobody can see.
    assert_eq!(safe_file_stem("Mr. Do! (USA)."), "Mr. Do! (USA)");
    assert_eq!(
        safe_file_stem("Trailing space (USA)   "),
        "Trailing space (USA)"
    );

    // Periods inside a name are ordinary and must be kept — `Dr. Mario` is not
    // an extension.
    assert_eq!(safe_file_stem("Dr. Mario (USA)"), "Dr. Mario (USA)");

    // Illegal characters are replaced, not deleted, so a name made only of
    // them still names something writable rather than collapsing to a shared
    // placeholder that two different titles would collide on.
    assert_eq!(safe_file_stem("///"), "---");

    // A name with nothing left after trimming has to produce a usable
    // filename rather than an empty one.
    assert_eq!(safe_file_stem("   "), "untitled");
    assert_eq!(safe_file_stem("\u{7}\u{1}"), "untitled");
}

/// A European PC Engine release stays under `pce` rather than being filed as
/// a TurboGrafx-16. NEC's European machine ran the US card library in
/// localized boxes, so no European-specific cards exist to file: No-Intro's
/// PC Engine set has no `(Europe)` dumps and Redump's PC Engine CD set has
/// none either. A European shelf of this console is imported Japanese
/// software, and sending it to `tg16` split one collection across two
/// folders on nothing but a region string. It also disagreed with the
/// playable projection, which has always sent the same release to
/// `pcengine`.
#[test]
fn european_pc_engine_is_not_filed_as_turbografx() {
    for region in ["Europe", "europe", "eur", " EUR "] {
        assert_eq!(
            crate::regional_physical_platform("pce", region),
            None,
            "region {region:?} must leave the release under pce"
        );
    }
    // North America is still the TurboGrafx-16, which is the whole reason
    // the regional split exists.
    for region in ["USA", "us", "Canada"] {
        assert_eq!(
            crate::regional_physical_platform("pce", region),
            Some("tg16")
        );
    }
    assert_eq!(crate::regional_physical_platform("pce", "Japan"), None);
}

/// The Mega Drive arm keeps Europe on purpose: unlike the PC Engine, Sega
/// really did press a distinct European library, so this test guards against
/// "simplifying" the two arms into one shared region list.
#[test]
fn european_genesis_still_goes_to_megadrive() {
    assert_eq!(
        crate::regional_physical_platform("genesis", "Europe"),
        Some("megadrive")
    );
}
