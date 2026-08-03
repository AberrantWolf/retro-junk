//! Unit tests for state.rs: entry lookups, the CHD-compression completion
//! handler (D4), and the multi-disc file refresh (D5).

use std::collections::HashSet;

use retro_junk_dat::{FileHashes, MatchMethod};
use retro_junk_lib::RomIdentification;
use retro_junk_lib::scanner::GameEntry;

use super::*;

#[test]
fn database_stale_consoles_auto_scan_even_when_preference_is_disabled() {
    assert!(should_queue_auto_scan(ScanStatus::NotScanned, false, true));
    assert!(!should_queue_auto_scan(
        ScanStatus::NotScanned,
        false,
        false
    ));
    assert!(!should_queue_auto_scan(ScanStatus::Scanned, true, true));
}

#[test]
fn catalog_region_slugs_match_header_regions_case_insensitively() {
    assert!(regions_match_dat(&[Region::Usa], "usa"));
    assert!(regions_match_dat(&[Region::Japan], "Japan"));
    assert!(!regions_match_dat(&[Region::Europe], "usa"));
    assert!(regions_match_dat(&[Region::Europe], "world"));
}
use crate::test_support::{test_console, test_entry};

fn catalog_candidate(rom_name: &str, crc32: &str, sha1: &str) -> retro_junk_db::CatalogMediaMatch {
    retro_junk_db::CatalogMediaMatch {
        media: retro_junk_catalog::types::Media {
            id: rom_name.to_string(),
            release_id: "super-mario-64:n64:usa".to_string(),
            media_serial: "NSME".to_string(),
            disc_number: 0,
            disc_label: String::new(),
            revision: String::new(),
            status: retro_junk_catalog::types::MediaStatus::Verified,
            tag: None,
            dat_name: "Super Mario 64 (USA)".to_string(),
            rom_name: rom_name.to_string(),
            dat_source: "no-intro".to_string(),
            file_size: 8_388_608,
            crc32: crc32.to_string(),
            sha1: sha1.to_string(),
            md5: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        },
        platform_id: "n64".to_string(),
        region: "usa".to_string(),
        release_revision: String::new(),
        release_title: "Super Mario 64".to_string(),
        cover_title: String::new(),
        screen_title: String::new(),
    }
}

#[test]
fn unique_serial_is_likely_before_hashing_and_only_matching_hash_upgrades_it() {
    let candidates = vec![catalog_candidate(
        "Super Mario 64 (USA).v64",
        "42c43204",
        "1002dd7b56aa0a59a9103f1fb3d57d6b161f8da7",
    )];
    let local_hashes = FileHashes {
        crc32: "3ce60709".to_string(),
        sha1: Some("9bef1128717f958171a4afac3ed78ee2bb4e86ce".to_string()),
        md5: None,
        data_size: 8_388_608,
        warnings: Vec::new(),
    };

    let mut entry = test_entry(GameEntry::SingleFile(PathBuf::from("game.z64")));
    let mut identification = RomIdentification::new();
    identification.serial_number = "NUS-NSME-USA".into();
    entry.identification = Some(identification);
    apply_catalog_resolution(&mut entry, &candidates);
    assert_eq!(entry.status, EntryStatus::LikelyMatched);
    assert_eq!(
        entry.dat_match.as_ref().unwrap().method,
        MatchMethod::Serial
    );

    entry.hashes = Some(local_hashes);
    apply_catalog_resolution(&mut entry, &candidates);
    assert_eq!(entry.status, EntryStatus::LikelyMatched);
    assert_eq!(
        entry.dat_match.as_ref().unwrap().method,
        MatchMethod::Serial
    );

    entry.hashes = Some(FileHashes {
        crc32: "42c43204".to_string(),
        sha1: Some("1002dd7b56aa0a59a9103f1fb3d57d6b161f8da7".to_string()),
        md5: None,
        data_size: 8_388_608,
        warnings: Vec::new(),
    });
    apply_catalog_resolution(&mut entry, &candidates);
    assert_eq!(entry.status, EntryStatus::Matched);
    assert_eq!(entry.dat_match.unwrap().method, MatchMethod::Crc32);
}

#[test]
fn matching_data_track_does_not_verify_an_incomplete_disc() {
    let candidates = vec![catalog_candidate(
        "Game (USA) (Track 1).bin",
        "42c43204",
        "1002dd7b56aa0a59a9103f1fb3d57d6b161f8da7",
    )];
    let mut entry = test_entry(GameEntry::SingleFile(PathBuf::from("game.cue")));
    entry.hashes = Some(FileHashes {
        crc32: "42c43204".into(),
        sha1: Some("1002dd7b56aa0a59a9103f1fb3d57d6b161f8da7".into()),
        md5: None,
        data_size: 8_388_608,
        warnings: vec!["Incomplete disc: DAT Track 2 is missing".into()],
    });
    entry.disc_verification = DiscVerification::Incomplete;

    apply_catalog_resolution(&mut entry, &candidates);

    assert_eq!(entry.status, EntryStatus::LikelyMatched);
    assert_eq!(entry.dat_match.unwrap().method, MatchMethod::Crc32);
}

#[test]
fn complete_disc_track_set_permits_verified_status() {
    let candidates = vec![catalog_candidate(
        "Game (USA) (Track 1).bin",
        "42c43204",
        "1002dd7b56aa0a59a9103f1fb3d57d6b161f8da7",
    )];
    let mut entry = test_entry(GameEntry::SingleFile(PathBuf::from("game.cue")));
    entry.hashes = Some(FileHashes {
        crc32: "42c43204".into(),
        sha1: Some("1002dd7b56aa0a59a9103f1fb3d57d6b161f8da7".into()),
        md5: None,
        data_size: 8_388_608,
        warnings: Vec::new(),
    });
    entry.disc_verification = DiscVerification::Complete;

    apply_catalog_resolution(&mut entry, &candidates);

    assert_eq!(entry.status, EntryStatus::Matched);
}

#[test]
fn duplicate_serial_candidates_select_the_hash_verified_byte_order() {
    let candidates = vec![
        catalog_candidate(
            "Super Mario 64 (USA).z64",
            "3ce60709",
            "9bef1128717f958171a4afac3ed78ee2bb4e86ce",
        ),
        catalog_candidate(
            "Super Mario 64 (USA).v64",
            "42c43204",
            "1002dd7b56aa0a59a9103f1fb3d57d6b161f8da7",
        ),
    ];
    let local_hashes = FileHashes {
        crc32: "3ce60709".to_string(),
        sha1: Some("9bef1128717f958171a4afac3ed78ee2bb4e86ce".to_string()),
        md5: None,
        data_size: 8_388_608,
        warnings: Vec::new(),
    };

    let mut entry = test_entry(GameEntry::SingleFile(PathBuf::from("game.z64")));
    entry.identification = Some(RomIdentification::new());
    entry.hashes = Some(local_hashes);

    apply_catalog_resolution(&mut entry, &candidates);

    let matched = entry.dat_match.unwrap();
    assert_eq!(matched.rom_name, "Super Mario 64 (USA).z64");
    assert_eq!(matched.method, MatchMethod::Crc32);
    assert_eq!(entry.status, EntryStatus::Matched);
}

#[test]
fn nds_header_revision_resolves_shared_serial_without_claiming_verification() {
    let mut original = catalog_candidate("Partners in Time (USA).nds", "11111111", "");
    original.platform_id = "nds".into();
    original.media.dat_name = "Mario & Luigi - Partners in Time (USA)".into();
    original.release_revision = String::new();

    let mut revision = catalog_candidate("Partners in Time (USA) (Rev 1).nds", "22222222", "");
    revision.platform_id = "nds".into();
    revision.media.dat_name = "Mario & Luigi - Partners in Time (USA) (Rev 1)".into();
    revision.release_revision = "Rev 1".into();

    let mut identification = RomIdentification::new();
    identification.serial_number = "NTR-ARME".into();
    identification.version = "v1".into();
    identification.regions = vec![Region::Usa];
    identification.file_size = 57_506_248; // Valid trimmed size; neither DAT size matches.

    let mut entry = test_entry(GameEntry::SingleFile(PathBuf::from("game.nds")));
    entry.identification = Some(identification);

    apply_catalog_resolution(&mut entry, &[original, revision]);

    assert_eq!(
        entry.dat_match.as_ref().unwrap().game_name,
        "Mario & Luigi - Partners in Time (USA) (Rev 1)"
    );
    assert_eq!(entry.dat_match.unwrap().method, MatchMethod::Serial);
    assert_eq!(entry.status, EntryStatus::LikelyMatched);
}

#[test]
fn completed_hash_batch_persists_after_its_console_projection_is_evicted() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("catalog.db");
    let (entry_id, source_revision) = {
        let mut conn = retro_junk_db::open_database(&db_path).unwrap();
        let root = retro_junk_db::upsert_library_root(&conn, "/roms").unwrap();
        let console_id = retro_junk_db::ensure_library_console(
            &conn,
            &retro_junk_db::LibraryConsoleDescriptor {
                root_id: root,
                platform: "psx".into(),
                folder_name: "psx".into(),
                folder_path: "/roms/psx".into(),
            },
        )
        .unwrap();
        let row = retro_junk_db::LibraryEntryRow {
            display_name: "game.bin".into(),
            game_entry_json: r#"{"SingleFile":"/roms/psx/game.bin"}"#.into(),
            status: "unknown".into(),
            tag: String::new(),
            crc32: String::new(),
            sha1: String::new(),
            md5: String::new(),
            data_size: 0,
            hash_warnings_json: None,
            disc_verification: "not_applicable".into(),
            dat_game_name: String::new(),
            dat_rom_name: String::new(),
            dat_match_method: String::new(),
            region_override: String::new(),
            cover_title: String::new(),
            screen_title: String::new(),
            identification_json: None,
            disc_identifications_json: None,
            broken_references_json: None,
            ambiguous_candidates_json: None,
            cue_compat_issues_json: None,
        };
        let scanned = retro_junk_db::ScannedLibraryEntry {
            entry_key: retro_junk_db::file_source_key(Path::new("game.bin")).unwrap(),
            source_fingerprint: "source-v1".into(),
            row,
        };
        let token = retro_junk_db::begin_console_scan(&conn, console_id).unwrap();
        retro_junk_db::reconcile_console_scan(&mut conn, token, "folder-v1", &[scanned]).unwrap();
        let detail = retro_junk_db::load_entry_details_for_console(&conn, console_id)
            .unwrap()
            .remove(0);
        (detail.id, detail.source_revision)
    };

    let mut snapshot = test_entry(GameEntry::SingleFile(PathBuf::from("/roms/psx/game.bin")));
    snapshot.id = Some(entry_id);
    snapshot.source_revision = source_revision;
    let hashes = FileHashes {
        crc32: "42c43204".into(),
        sha1: Some("1002dd7b56aa0a59a9103f1fb3d57d6b161f8da7".into()),
        md5: None,
        data_size: 8_388_608,
        warnings: Vec::new(),
    };
    let mut candidate = catalog_candidate(
        "Game (USA) (Track 1).bin",
        "42c43204",
        "1002dd7b56aa0a59a9103f1fb3d57d6b161f8da7",
    );
    candidate.platform_id = "psx".into();
    candidate.media.dat_name = "Game (USA)".into();

    let ctx = dummy_ctx();
    let mut app = crate::app::RetroJunkApp::with_parts(
        &ctx,
        crate::settings::AppSettings::default(),
        None,
        Some(db_path.clone()),
    );
    app.library_store =
        Some(crate::backend::library_store::LibraryStore::start(db_path.clone()).unwrap());
    // The console descriptor remains, but navigation has evicted its rich rows.
    app.browser.consoles.push(test_console("psx", Vec::new()));

    handle_message(
        &mut app,
        AppMessage::EntryHashBatchComplete {
            folder_name: "psx".into(),
            entry: Box::new(snapshot),
            results: vec![EntryHashResult {
                disc_path: None,
                hashes,
                catalog_matches: vec![candidate],
                disc_verification: DiscVerification::NotApplicable,
            }],
        },
        &ctx,
    );

    let reply = app
        .library_store
        .as_ref()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert!(matches!(
        reply.payload,
        Ok(crate::backend::library_store::LibraryStoreValue::ChangeSet(
            _
        ))
    ));
    let conn = retro_junk_db::open_database(&db_path).unwrap();
    let persisted = retro_junk_db::load_entry_detail(&conn, entry_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.row.status, "matched");
    assert_eq!(persisted.row.crc32, "42c43204");
    assert_eq!(persisted.row.dat_match_method, "crc32");
}

#[test]
fn multi_disc_hash_batch_applies_every_disc_before_aggregating_status() {
    let disc1 = PathBuf::from("/roms/psx/Game (Disc 1).bin");
    let disc2 = PathBuf::from("/roms/psx/Game (Disc 2).bin");
    let mut entry = test_entry(GameEntry::MultiDisc {
        name: "Game.m3u".into(),
        files: vec![disc1.clone(), disc2.clone()],
    });
    entry.disc_identifications = Some(vec![
        DiscIdentification {
            path: disc1.clone(),
            identification: RomIdentification::new(),
            hashes: None,
            dat_match: None,
            ambiguous_candidates: Vec::new(),
            disc_verification: DiscVerification::NotApplicable,
        },
        DiscIdentification {
            path: disc2.clone(),
            identification: RomIdentification::new(),
            hashes: None,
            dat_match: None,
            ambiguous_candidates: Vec::new(),
            disc_verification: DiscVerification::NotApplicable,
        },
    ]);

    let make_result = |path: PathBuf, disc: u8| {
        let crc = format!("0000000{disc}");
        let mut candidate = catalog_candidate(&format!("Game (Disc {disc}).bin"), &crc, "");
        candidate.media.id = format!("game-disc-{disc}");
        candidate.media.dat_name = format!("Game (USA) (Disc {disc})");
        candidate.media.file_size = 64;
        EntryHashResult {
            disc_path: Some(path),
            hashes: FileHashes {
                crc32: crc,
                sha1: None,
                md5: None,
                data_size: 64,
                warnings: Vec::new(),
            },
            catalog_matches: vec![candidate],
            disc_verification: DiscVerification::NotApplicable,
        }
    };
    apply_entry_hash_results(&mut entry, &[make_result(disc1, 1), make_result(disc2, 2)]);

    let discs = entry.disc_identifications.as_ref().unwrap();
    assert!(discs.iter().all(|disc| disc.hashes.is_some()));
    assert!(discs.iter().all(|disc| {
        disc.dat_match
            .as_ref()
            .is_some_and(|matched| matched.method == MatchMethod::Crc32)
    }));
    assert_eq!(entry.status, EntryStatus::Matched);
    assert_eq!(entry.dat_match.unwrap().game_name, "Game (USA)");
}

// -- find_entry_by_file_mut (D4) --

#[test]
fn find_entry_by_file_mut_matches_single_file_entry() {
    let path = PathBuf::from("/roms/psx/Game.cue");
    let mut console = test_console("psx", vec![test_entry(GameEntry::SingleFile(path.clone()))]);

    let found = console.find_entry_by_file_mut(&path);
    assert!(found.is_some());
}

#[test]
fn find_entry_by_file_mut_matches_any_file_of_multidisc_entry() {
    let disc1 = PathBuf::from("/roms/psx/Game (Disc 1).chd");
    let disc2 = PathBuf::from("/roms/psx/Game (Disc 2).chd");
    let mut console = test_console(
        "psx",
        vec![test_entry(GameEntry::MultiDisc {
            name: "Game.m3u".to_string(),
            files: vec![disc1.clone(), disc2.clone()],
        })],
    );

    // Matching the second disc (not just the first) must still find the entry.
    assert!(console.find_entry_by_file_mut(&disc2).is_some());
    assert!(console.find_entry_by_file_mut(&disc1).is_some());
}

#[test]
fn find_entry_by_file_mut_returns_none_for_unknown_file() {
    let mut console = test_console(
        "psx",
        vec![test_entry(GameEntry::SingleFile(PathBuf::from(
            "/roms/psx/Game.cue",
        )))],
    );
    assert!(
        console
            .find_entry_by_file_mut(Path::new("/roms/psx/Other.cue"))
            .is_none()
    );
}

// -- refresh_multidisc_files (D5) --

#[test]
fn refresh_multidisc_files_remaps_via_playlist_with_claim_tracking() {
    let dir = tempfile::TempDir::new().unwrap();
    // Canonicalize so expectations match the resolved paths the refresh
    // produces (macOS tempdirs live behind the /var → /private/var symlink).
    let canonical = dir.path().canonicalize().unwrap();
    let folder = canonical.as_path();

    // Two discs, remapped by the playlist to new .chd names.
    std::fs::write(folder.join("D1.chd"), "").unwrap();
    std::fs::write(folder.join("D2.chd"), "").unwrap();
    // Stray leftovers from a failed delete — must not appear in `files`.
    std::fs::write(folder.join("D1.sbi"), "").unwrap();
    std::fs::write(folder.join("D1.bin"), "").unwrap();
    std::fs::write(folder.join("Game.m3u"), "D1.chd\nD2.chd\n").unwrap();

    let mut entry = test_entry(GameEntry::MultiDisc {
        name: "Game.m3u".to_string(),
        files: vec![folder.join("D1.bin"), folder.join("D2.bin")],
    });
    entry.disc_identifications = Some(vec![
        DiscIdentification {
            path: folder.join("D1.bin"),
            identification: RomIdentification::new(),
            hashes: None,
            dat_match: None,
            ambiguous_candidates: Vec::new(),
            disc_verification: DiscVerification::NotApplicable,
        },
        DiscIdentification {
            path: folder.join("D2.bin"),
            identification: RomIdentification::new(),
            hashes: None,
            dat_match: None,
            ambiguous_candidates: Vec::new(),
            disc_verification: DiscVerification::NotApplicable,
        },
    ]);

    let extensions: HashSet<String> = ["chd", "bin", "sbi"]
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    refresh_multidisc_files(&mut entry, folder, &extensions);

    let GameEntry::MultiDisc { files, .. } = &entry.game_entry else {
        panic!("expected MultiDisc");
    };
    // Only the two chds — the .sbi/.bin stragglers are excluded because the
    // playlist (not the fallback scan) drives collection.
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|p| p.extension().unwrap() == "chd"));

    let discs = entry.disc_identifications.as_ref().unwrap();
    assert_eq!(discs[0].path, folder.join("D1.chd"));
    assert_eq!(discs[1].path, folder.join("D2.chd"));
}

#[test]
fn refresh_multidisc_files_does_not_wipe_entry_on_empty_result() {
    let dir = tempfile::TempDir::new().unwrap();
    let folder = dir.path(); // empty directory, no matching files

    let original_files = vec![folder.join("D1.chd"), folder.join("D2.chd")];
    let mut entry = test_entry(GameEntry::MultiDisc {
        name: "Game.m3u".to_string(),
        files: original_files.clone(),
    });

    let extensions: HashSet<String> = ["chd"]
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    refresh_multidisc_files(&mut entry, folder, &extensions);

    let GameEntry::MultiDisc { files, .. } = &entry.game_entry else {
        panic!("expected MultiDisc");
    };
    assert_eq!(
        files, &original_files,
        "an empty collection result must not wipe the entry's file list"
    );
}

#[test]
fn refresh_multidisc_files_claim_tracking_avoids_double_assignment() {
    let dir = tempfile::TempDir::new().unwrap();
    let folder = dir.path();

    // Only one new file, but two discs whose old stems don't match anything
    // in the new set — neither should both grab the same file.
    std::fs::write(folder.join("Only.chd"), "").unwrap();

    let mut entry = test_entry(GameEntry::MultiDisc {
        name: "Game.m3u".to_string(),
        files: vec![folder.join("Old1.bin"), folder.join("Old2.bin")],
    });
    entry.disc_identifications = Some(vec![
        DiscIdentification {
            path: folder.join("Old1.bin"),
            identification: RomIdentification::new(),
            hashes: None,
            dat_match: None,
            ambiguous_candidates: Vec::new(),
            disc_verification: DiscVerification::NotApplicable,
        },
        DiscIdentification {
            path: folder.join("Old2.bin"),
            identification: RomIdentification::new(),
            hashes: None,
            dat_match: None,
            ambiguous_candidates: Vec::new(),
            disc_verification: DiscVerification::NotApplicable,
        },
    ]);

    let extensions: HashSet<String> = ["chd"]
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    refresh_multidisc_files(&mut entry, folder, &extensions);

    let discs = entry.disc_identifications.as_ref().unwrap();
    // Neither stem ("Old1"/"Old2") matches "Only", so both keep their stale
    // path rather than both collapsing onto "Only.chd".
    assert_eq!(discs[0].path, folder.join("Old1.bin"));
    assert_eq!(discs[1].path, folder.join("Old2.bin"));
}

// -- ChdCompressPromptReady handler (D1) --

#[test]
fn chd_compress_prompt_ready_stores_the_prompt() {
    let mut app = crate::app::RetroJunkApp::with_parts(
        &dummy_ctx(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );
    assert!(app.ui_state.chd_compress_prompt.is_none());

    let prompt = ChdCompressPrompt {
        folder_name: "psx".to_string(),
        items: Vec::new(),
        skipped: Vec::new(),
        chdman: Err(retro_junk_lib::chd_convert::ChdmanUnavailable {
            reason: "not found".to_string(),
        }),
        delete_sources: false,
    };

    handle_message(
        &mut app,
        AppMessage::ChdCompressPromptReady { prompt },
        &dummy_ctx(),
    );

    let stored = app
        .ui_state
        .chd_compress_prompt
        .as_ref()
        .expect("prompt must be stored");
    assert_eq!(stored.folder_name, "psx");
}

// -- ChdCompressComplete handler (D4) --

fn dummy_ctx() -> egui::Context {
    egui::Context::default()
}

#[test]
fn operation_phase_replaces_completed_byte_progress_with_matching_status() {
    let mut app = crate::app::RetroJunkApp::with_parts(
        &dummy_ctx(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );
    let mut operation = BackgroundOperation::new(
        42,
        "Computing hashes".into(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        OperationKind::Hash,
        "ps1".into(),
        ProgressDisplay::Bytes,
    );
    operation.progress_current = 100;
    operation.progress_total = 100;
    app.operations.push(operation);

    handle_message(
        &mut app,
        AppMessage::OperationPhase {
            op_id: 42,
            description: "Matching 20 hashed file(s) against the catalog".into(),
            display: ProgressDisplay::Count,
            current: 0,
            total: 0,
        },
        &dummy_ctx(),
    );

    let operation = &app.operations[0];
    assert_eq!(
        operation.description,
        "Matching 20 hashed file(s) against the catalog"
    );
    assert_eq!(operation.display, ProgressDisplay::Count);
    assert_eq!(operation.progress_total, 0);
}

#[test]
fn durable_job_results_are_not_discarded_on_root_navigation() {
    assert!(
        !AppMessage::EntryAnalysisSnapshotsComplete {
            folder_name: "nds".into(),
            entries: Vec::new(),
        }
        .is_root_scoped()
    );
    assert!(
        !AppMessage::ScanSnapshotPrepared {
            folder_name: "nds".into(),
            console_id: Some(retro_junk_db::LibraryConsoleId(7)),
            result: Err("test".into()),
        }
        .is_root_scoped()
    );
    assert!(
        AppMessage::ScanProjectionInfo {
            folder_name: "nds".into(),
            loose_disc_files: Vec::new(),
            fingerprint: crate::fingerprint::FolderFingerprint {
                name_hash: "projection-only".into(),
            },
        }
        .is_root_scoped()
    );
}

#[test]
fn chd_compress_complete_does_not_patch_loaded_entry_projection() {
    let mut app = crate::app::RetroJunkApp::with_parts(
        &dummy_ctx(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );
    let input = PathBuf::from("/roms/psx/Game.cue");
    let output = PathBuf::from("/roms/psx/Game.chd");
    let mut entry = test_entry(GameEntry::SingleFile(input.clone()));
    entry.hashes = Some(retro_junk_dat::FileHashes {
        crc32: "deadbeef".to_string(),
        md5: None,
        sha1: None,
        data_size: 1,
        warnings: Vec::new(),
    });
    entry.broken_references = Some(Vec::new());
    entry.cue_compat_issues = Some(Vec::new());
    app.browser.consoles.push(test_console("psx", vec![entry]));

    let job = retro_junk_lib::chd_convert::CompressionJob {
        input: input.clone(),
        media: retro_junk_core::ChdMedia::Cd,
        output: output.clone(),
        source_files: vec![input.clone()],
        input_bytes: 100,
    };

    let results = vec![ChdCompressResult {
        input_name: "Game.cue".to_string(),
        job,
        outcome: ChdCompressOutcome::Compressed {
            input_bytes: 100,
            output_bytes: 40,
            tracks: 1,
            sources_deleted: true,
            delete_failures: Vec::new(),
        },
    }];

    handle_message(
        &mut app,
        AppMessage::ChdCompressComplete {
            folder_name: "psx".to_string(),
            rescan_target: None,
            results,
        },
        &dummy_ctx(),
    );

    let console = &app.browser.consoles[0];
    assert_eq!(console.entries.len(), 1);
    let GameEntry::SingleFile(ref path) = console.entries[0].game_entry else {
        panic!("expected SingleFile");
    };
    assert_eq!(path, &input, "completion must not patch UI-owned state");
    assert!(console.entries[0].hashes.is_some());
    assert!(console.entries[0].broken_references.is_some());
    assert!(console.entries[0].cue_compat_issues.is_some());
}

#[test]
fn chd_compress_complete_without_source_deletion_leaves_projection_untouched() {
    let mut app = crate::app::RetroJunkApp::with_parts(
        &dummy_ctx(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );
    let input = PathBuf::from("/roms/psx/Game.cue");
    let mut entry = test_entry(GameEntry::SingleFile(input.clone()));
    entry.hashes = Some(retro_junk_dat::FileHashes {
        crc32: "deadbeef".to_string(),
        md5: None,
        sha1: None,
        data_size: 1,
        warnings: Vec::new(),
    });
    let mut console = test_console("psx", vec![entry]);
    console.fingerprint = Some(crate::fingerprint::FolderFingerprint {
        name_hash: "stale".to_string(),
    });
    app.browser.consoles.push(console);

    let job = retro_junk_lib::chd_convert::CompressionJob {
        input: input.clone(),
        media: retro_junk_core::ChdMedia::Cd,
        output: PathBuf::from("/roms/psx/Game.chd"),
        source_files: vec![input.clone()],
        input_bytes: 100,
    };

    let results = vec![ChdCompressResult {
        input_name: "Game.cue".to_string(),
        job,
        outcome: ChdCompressOutcome::Compressed {
            input_bytes: 100,
            output_bytes: 40,
            tracks: 1,
            sources_deleted: false,
            delete_failures: Vec::new(),
        },
    }];

    handle_message(
        &mut app,
        AppMessage::ChdCompressComplete {
            folder_name: "psx".to_string(),
            rescan_target: None,
            results,
        },
        &dummy_ctx(),
    );

    let console = &app.browser.consoles[0];
    // Entry untouched: still SingleFile at the original cue path, hashes kept.
    let GameEntry::SingleFile(ref path) = console.entries[0].game_entry else {
        panic!("expected SingleFile");
    };
    assert_eq!(path, &input);
    assert!(console.entries[0].hashes.is_some());
    assert_eq!(console.fingerprint.as_ref().unwrap().name_hash, "stale");
}

#[test]
fn chd_compress_complete_leaves_ghost_cleanup_to_durable_rescan() {
    let mut app = crate::app::RetroJunkApp::with_parts(
        &dummy_ctx(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );
    let cue = PathBuf::from("/roms/psx/Game.cue");
    let track_bin = PathBuf::from("/roms/psx/Game (Track 2).bin");
    let output = PathBuf::from("/roms/psx/Game.chd");

    // The scanner's stem-only dedup can leave a lone SingleFile "ghost"
    // entry for a track bin alongside the real cue entry.
    let cue_entry = test_entry(GameEntry::SingleFile(cue.clone()));
    let ghost_entry = test_entry(GameEntry::SingleFile(track_bin.clone()));
    app.browser
        .consoles
        .push(test_console("psx", vec![cue_entry, ghost_entry]));

    let job = retro_junk_lib::chd_convert::CompressionJob {
        input: cue.clone(),
        media: retro_junk_core::ChdMedia::Cd,
        output: output.clone(),
        source_files: vec![cue.clone(), track_bin.clone()],
        input_bytes: 100,
    };

    let results = vec![ChdCompressResult {
        input_name: "Game.cue".to_string(),
        job,
        outcome: ChdCompressOutcome::Compressed {
            input_bytes: 100,
            output_bytes: 40,
            tracks: 2,
            sources_deleted: true,
            delete_failures: Vec::new(),
        },
    }];

    handle_message(
        &mut app,
        AppMessage::ChdCompressComplete {
            folder_name: "psx".to_string(),
            rescan_target: None,
            results,
        },
        &dummy_ctx(),
    );

    let console = &app.browser.consoles[0];
    assert_eq!(console.entries.len(), 2);
    let GameEntry::SingleFile(ref path) = console.entries[0].game_entry else {
        panic!("expected SingleFile");
    };
    assert_eq!(path, &cue);
}
