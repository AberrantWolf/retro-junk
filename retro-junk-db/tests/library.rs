use retro_junk_db::*;

#[allow(dead_code)]
struct ConsoleRecord<'a> {
    root_id: LibraryRootId,
    platform: &'a str,
    folder_name: &'a str,
    folder_path: &'a str,
    fingerprint_hash: &'a str,
    dat_game_count: i64,
}

fn make_entry(name: &str, status: &str) -> LibraryEntryRow {
    LibraryEntryRow {
        display_name: name.to_string(),
        game_entry_json: format!(r#"{{"SingleFile":"{name}"}}"#),
        status: status.to_string(),
        tag: String::new(),
        crc32: "aabbccdd".to_string(),
        sha1: String::new(),
        md5: String::new(),
        data_size: 1024,
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
    }
}

fn reconcile_test_console(
    conn: &mut Connection,
    console: &ConsoleRecord<'_>,
    entries: &[LibraryEntryRow],
) -> Result<LibraryConsoleId, LibraryError> {
    let id = ensure_library_console(
        conn,
        &LibraryConsoleDescriptor {
            root_id: console.root_id,
            platform: console.platform.to_owned(),
            folder_name: console.folder_name.to_owned(),
            folder_path: console.folder_path.to_owned(),
        },
    )?;
    let scanned: Vec<_> = entries
        .iter()
        .map(|row| {
            Ok(ScannedLibraryEntry {
                entry_key: source_key_from_game_entry_json(
                    &row.game_entry_json,
                    std::path::Path::new(console.folder_path),
                )?,
                source_fingerprint: String::new(),
                row: row.clone(),
            })
        })
        .collect::<Result<_, LibraryError>>()?;
    let token = begin_console_scan(conn, id)?;
    reconcile_console_scan(conn, token, console.fingerprint_hash, &scanned)?;
    for detail in load_entry_details_for_console(conn, id)? {
        let Some(source) = entries
            .iter()
            .find(|row| row.display_name == detail.row.display_name)
        else {
            continue;
        };
        apply_entry_analysis(
            conn,
            detail.id,
            detail.source_revision,
            &EntryAnalysisUpdate {
                status: source.status.clone(),
                crc32: source.crc32.clone(),
                sha1: source.sha1.clone(),
                md5: source.md5.clone(),
                data_size: source.data_size,
                hash_warnings_json: source.hash_warnings_json.clone(),
                disc_verification: source.disc_verification.clone(),
                dat_game_name: source.dat_game_name.clone(),
                dat_rom_name: source.dat_rom_name.clone(),
                dat_match_method: source.dat_match_method.clone(),
                cover_title: source.cover_title.clone(),
                screen_title: source.screen_title.clone(),
                identification_json: source.identification_json.clone(),
                disc_identifications_json: source.disc_identifications_json.clone(),
                broken_references_json: source.broken_references_json.clone(),
                ambiguous_candidates_json: source.ambiguous_candidates_json.clone(),
                cue_compat_issues_json: source.cue_compat_issues_json.clone(),
            },
        )?;
        set_entry_tag(
            conn,
            detail.id,
            (!source.tag.is_empty()).then_some(source.tag.as_str()),
        )?;
    }
    Ok(id)
}

#[test]
fn upsert_and_load_library_root() {
    let conn = open_memory().unwrap();
    let id1 = upsert_library_root(&conn, "/roms").unwrap();
    let id2 = upsert_library_root(&conn, "/roms").unwrap();
    assert_eq!(id1, id2, "upsert should return same id for same path");

    let found = get_library_root_id(&conn, "/roms").unwrap();
    assert_eq!(found, Some(id1));

    let missing = get_library_root_id(&conn, "/other").unwrap();
    assert_eq!(missing, None);
}

#[test]
fn reconcile_and_load_console() {
    let mut conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();

    let entries = vec![
        make_entry("game1.nes", "matched"),
        make_entry("game2.nes", "unknown"),
    ];

    let console_id = reconcile_test_console(
        &mut conn,
        &ConsoleRecord {
            root_id,
            platform: "NES",
            folder_name: "nes",
            folder_path: "/roms/nes",
            fingerprint_hash: "abc123",
            dat_game_count: 500,
        },
        &entries,
    )
    .unwrap();

    // Load consoles
    let consoles = load_consoles_for_root(&conn, root_id).unwrap();
    assert_eq!(consoles.len(), 1);
    assert_eq!(consoles[0].platform, "NES");
    assert_eq!(consoles[0].folder_name, "nes");
    assert_eq!(consoles[0].id, console_id);

    // Load entries
    let loaded = load_entries_for_console(&conn, console_id).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].display_name, "game1.nes");
    assert_eq!(loaded[0].status, "matched");
    assert_eq!(loaded[0].crc32, "aabbccdd");
    // Unset text fields round-trip as empty strings; unset JSON blobs stay None.
    assert_eq!(loaded[0].sha1, "");
    assert_eq!(loaded[0].tag, "");
    assert!(loaded[0].identification_json.is_none());
    assert_eq!(loaded[1].display_name, "game2.nes");
}

#[test]
fn reconciliation_replaces_absent_entries() {
    let mut conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();

    let entries_v1 = vec![make_entry("old.nes", "unknown")];
    let console_id = reconcile_test_console(
        &mut conn,
        &ConsoleRecord {
            root_id,
            platform: "NES",
            folder_name: "nes",
            folder_path: "/roms/nes",
            fingerprint_hash: "fp1",
            dat_game_count: 0,
        },
        &entries_v1,
    )
    .unwrap();

    // Replace with new entries
    let entries_v2 = vec![
        make_entry("new1.nes", "matched"),
        make_entry("new2.nes", "matched"),
    ];
    let console_id2 = reconcile_test_console(
        &mut conn,
        &ConsoleRecord {
            root_id,
            platform: "NES",
            folder_name: "nes",
            folder_path: "/roms/nes",
            fingerprint_hash: "fp2",
            dat_game_count: 100,
        },
        &entries_v2,
    )
    .unwrap();

    assert_eq!(console_id, console_id2, "same console should be reused");

    let loaded = load_entries_for_console(&conn, console_id).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].display_name, "new1.nes");
}

#[test]
fn reconciliation_updates_single_entry() {
    let mut conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();
    let console_id = reconcile_test_console(
        &mut conn,
        &ConsoleRecord {
            root_id,
            platform: "NES",
            folder_name: "nes",
            folder_path: "/roms/nes",
            fingerprint_hash: "fp",
            dat_game_count: 0,
        },
        &[make_entry("game.nes", "unknown")],
    )
    .unwrap();

    // Reconcile the same source with updated derived state.
    let mut updated = make_entry("game.nes", "matched");
    updated.tag = "homebrew".to_string();
    reconcile_test_console(
        &mut conn,
        &ConsoleRecord {
            root_id,
            platform: "NES",
            folder_name: "nes",
            folder_path: "/roms/nes",
            fingerprint_hash: "fp2",
            dat_game_count: 0,
        },
        &[updated],
    )
    .unwrap();

    let loaded = load_entries_for_console(&conn, console_id).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].status, "matched");
    assert_eq!(loaded[0].tag, "homebrew");
}

#[test]
fn reconciliation_updates_entry_batch() {
    let mut conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();
    let console_id = reconcile_test_console(
        &mut conn,
        &ConsoleRecord {
            root_id,
            platform: "NES",
            folder_name: "nes",
            folder_path: "/roms/nes",
            fingerprint_hash: "fp",
            dat_game_count: 0,
        },
        &[
            make_entry("game1.nes", "unknown"),
            make_entry("game2.nes", "unknown"),
        ],
    )
    .unwrap();

    // Reconcile a batch with updated derived state.
    let updates = vec![
        make_entry("game1.nes", "matched"),
        make_entry("game2.nes", "matched"),
    ];
    reconcile_test_console(
        &mut conn,
        &ConsoleRecord {
            root_id,
            platform: "NES",
            folder_name: "nes",
            folder_path: "/roms/nes",
            fingerprint_hash: "fp2",
            dat_game_count: 0,
        },
        &updates,
    )
    .unwrap();

    let loaded = load_entries_for_console(&conn, console_id).unwrap();
    assert!(loaded.iter().all(|e| e.status == "matched"));
}

#[test]
fn delete_root_cascades() {
    let mut conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();
    reconcile_test_console(
        &mut conn,
        &ConsoleRecord {
            root_id,
            platform: "NES",
            folder_name: "nes",
            folder_path: "/roms/nes",
            fingerprint_hash: "fp",
            dat_game_count: 0,
        },
        &[make_entry("game.nes", "unknown")],
    )
    .unwrap();

    delete_library_root(&conn, root_id).unwrap();

    assert_eq!(get_library_root_id(&conn, "/roms").unwrap(), None);
    let consoles = load_consoles_for_root(&conn, root_id).unwrap();
    assert!(consoles.is_empty());
}

#[test]
fn multiple_roots_independent() {
    let mut conn = open_memory().unwrap();
    let root1 = upsert_library_root(&conn, "/roms1").unwrap();
    let root2 = upsert_library_root(&conn, "/roms2").unwrap();

    reconcile_test_console(
        &mut conn,
        &ConsoleRecord {
            root_id: root1,
            platform: "NES",
            folder_name: "nes",
            folder_path: "/roms1/nes",
            fingerprint_hash: "fp1",
            dat_game_count: 0,
        },
        &[make_entry("game1.nes", "matched")],
    )
    .unwrap();
    reconcile_test_console(
        &mut conn,
        &ConsoleRecord {
            root_id: root2,
            platform: "SNES",
            folder_name: "snes",
            folder_path: "/roms2/snes",
            fingerprint_hash: "fp2",
            dat_game_count: 0,
        },
        &[make_entry("game2.sfc", "unknown")],
    )
    .unwrap();

    let c1 = load_consoles_for_root(&conn, root1).unwrap();
    let c2 = load_consoles_for_root(&conn, root2).unwrap();
    assert_eq!(c1.len(), 1);
    assert_eq!(c2.len(), 1);
    assert_eq!(c1[0].platform, "NES");
    assert_eq!(c2[0].platform, "SNES");

    // Deleting root1 doesn't affect root2
    delete_library_root(&conn, root1).unwrap();
    let c2_after = load_consoles_for_root(&conn, root2).unwrap();
    assert_eq!(c2_after.len(), 1);
}

// ── Scrape identity strength ───────────────────────────────────────────────

/// Automation gates on this: a serial or a complete hash triple identifies a
/// release, a bare filename is a guess. Getting the order wrong would either
/// publish guesses into the archive unattended or refuse to scrape releases
/// that are perfectly well identified.
#[test]
fn identity_strength_ranks_serial_above_hashes_above_filename() {
    use retro_junk_db::library::{ArchivedScrapeIdentity, ScrapeIdentityTier};

    let base = ArchivedScrapeIdentity {
        filename: "Game (USA).chd".to_owned(),
        file_size: 1,
        serial: String::new(),
        crc32: String::new(),
        md5: String::new(),
        sha1: String::new(),
    };

    assert_eq!(base.tier(), ScrapeIdentityTier::Filename);
    assert_eq!(
        ArchivedScrapeIdentity {
            crc32: "a".to_owned(),
            md5: "b".to_owned(),
            sha1: "c".to_owned(),
            ..base.clone()
        }
        .tier(),
        ScrapeIdentityTier::Hashes
    );
    assert_eq!(
        ArchivedScrapeIdentity {
            serial: "SLUS-00067".to_owned(),
            ..base.clone()
        }
        .tier(),
        ScrapeIdentityTier::Serial
    );
    assert_eq!(
        ArchivedScrapeIdentity {
            filename: String::new(),
            ..base.clone()
        }
        .tier(),
        ScrapeIdentityTier::None
    );
    assert!(ScrapeIdentityTier::Filename < ScrapeIdentityTier::Hashes);
    assert!(ScrapeIdentityTier::Hashes < ScrapeIdentityTier::Serial);
}

/// A partial hash set is not a hash match: `ScreenScraper`'s hash tier needs
/// the whole triple, so two of three is really just a filename.
#[test]
fn a_partial_hash_set_does_not_count_as_a_hash_identity() {
    use retro_junk_db::library::{ArchivedScrapeIdentity, ScrapeIdentityTier};

    let partial = ArchivedScrapeIdentity {
        filename: "Game (USA).chd".to_owned(),
        file_size: 1,
        serial: String::new(),
        crc32: "a".to_owned(),
        md5: "b".to_owned(),
        sha1: String::new(),
    };

    assert_eq!(partial.tier(), ScrapeIdentityTier::Filename);
}
