use retro_junk_db::*;

fn make_entry(name: &str, status: &str) -> LibraryEntryRow {
    LibraryEntryRow {
        display_name: name.to_string(),
        game_entry_json: format!(r#"{{"SingleFile":"/roms/{name}"}}"#),
        status: status.to_string(),
        tag: None,
        crc32: Some("aabbccdd".to_string()),
        sha1: None,
        md5: None,
        data_size: Some(1024),
        dat_game_name: None,
        dat_rom_name: None,
        dat_match_method: None,
        region_override: None,
        cover_title: None,
        screen_title: None,
        identification_json: None,
        disc_identifications_json: None,
        broken_references_json: None,
        ambiguous_candidates_json: None,
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
        root_id,
        "NES",
        "nes",
        "/roms/nes",
        "abc123",
        Some(500),
        &entries,
    )
    .unwrap();

    // Load consoles
    let consoles = load_consoles_for_root(&conn, root_id).unwrap();
    assert_eq!(consoles.len(), 1);
    assert_eq!(consoles[0].platform, "NES");
    assert_eq!(consoles[0].folder_name, "nes");
    assert_eq!(consoles[0].dat_game_count, Some(500));
    assert_eq!(consoles[0].id, console_id);

    // Load entries
    let loaded = load_entries_for_console(&conn, console_id).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].display_name, "game1.nes");
    assert_eq!(loaded[0].status, "matched");
    assert_eq!(loaded[0].crc32.as_deref(), Some("aabbccdd"));
    assert_eq!(loaded[1].display_name, "game2.nes");
}

#[test]
fn save_console_bulk_replaces_entries() {
    let conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();

    let entries_v1 = vec![make_entry("old.nes", "unknown")];
    let console_id = save_console_bulk(
        &conn,
        root_id,
        "NES",
        "nes",
        "/roms/nes",
        "fp1",
        None,
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
        root_id,
        "NES",
        "nes",
        "/roms/nes",
        "fp2",
        Some(100),
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
        root_id,
        "NES",
        "nes",
        "/roms/nes",
        "fp",
        None,
        &[make_entry("game.nes", "unknown")],
    )
    .unwrap();

    // Upsert the same entry with updated status
    let mut updated = make_entry("game.nes", "matched");
    updated.tag = Some("homebrew".to_string());
    upsert_entry(&conn, console_id, &updated).unwrap();

    let loaded = load_entries_for_console(&conn, console_id).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].status, "matched");
    assert_eq!(loaded[0].tag.as_deref(), Some("homebrew"));
}

#[test]
fn upsert_entries_batch() {
    let conn = open_memory().unwrap();
    let root_id = upsert_library_root(&conn, "/roms").unwrap();
    let console_id = save_console_bulk(
        &conn,
        root_id,
        "NES",
        "nes",
        "/roms/nes",
        "fp",
        None,
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
        root_id,
        "NES",
        "nes",
        "/roms/nes",
        "fp",
        None,
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
        root1,
        "NES",
        "nes",
        "/roms1/nes",
        "fp1",
        None,
        &[make_entry("game1.nes", "matched")],
    )
    .unwrap();
    save_console_bulk(
        &conn,
        root2,
        "SNES",
        "snes",
        "/roms2/snes",
        "fp2",
        None,
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
