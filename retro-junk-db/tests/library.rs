use retro_junk_db::*;

fn make_entry(name: &str, status: &str) -> LibraryEntryRow {
    LibraryEntryRow {
        display_name: name.to_string(),
        game_entry_json: format!(r#"{{"SingleFile":"/roms/{name}"}}"#),
        status: status.to_string(),
        tag: String::new(),
        crc32: "aabbccdd".to_string(),
        sha1: String::new(),
        md5: String::new(),
        data_size: 1024,
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
fn save_and_load_console_bulk() {
    let conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();

    let entries = vec![
        make_entry("game1.nes", "matched"),
        make_entry("game2.nes", "unknown"),
    ];

    let console_id = save_console_bulk(
        &conn,
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
    assert_eq!(consoles[0].dat_game_count, 500);
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
fn save_console_bulk_replaces_entries() {
    let conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();

    let entries_v1 = vec![make_entry("old.nes", "unknown")];
    let console_id = save_console_bulk(
        &conn,
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
    let console_id2 = save_console_bulk(
        &conn,
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
fn upsert_single_entry() {
    let conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();
    let console_id = save_console_bulk(
        &conn,
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

    // Upsert the same entry with updated status
    let mut updated = make_entry("game.nes", "matched");
    updated.tag = "homebrew".to_string();
    upsert_entry(&conn, console_id, &updated).unwrap();

    let loaded = load_entries_for_console(&conn, console_id).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].status, "matched");
    assert_eq!(loaded[0].tag, "homebrew");
}

#[test]
fn upsert_entries_batch() {
    let conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();
    let console_id = save_console_bulk(
        &conn,
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

    // Batch upsert
    let updates = vec![
        make_entry("game1.nes", "matched"),
        make_entry("game2.nes", "matched"),
    ];
    upsert_entries(&conn, console_id, &updates).unwrap();

    let loaded = load_entries_for_console(&conn, console_id).unwrap();
    assert!(loaded.iter().all(|e| e.status == "matched"));
}

#[test]
fn delete_root_cascades() {
    let conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();
    save_console_bulk(
        &conn,
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
    let conn = open_memory().unwrap();
    let root1 = upsert_library_root(&conn, "/roms1").unwrap();
    let root2 = upsert_library_root(&conn, "/roms2").unwrap();

    save_console_bulk(
        &conn,
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
    save_console_bulk(
        &conn,
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
