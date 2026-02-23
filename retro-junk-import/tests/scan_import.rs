use retro_junk_catalog::types::*;
use retro_junk_db::*;

fn setup_db_with_media() -> rusqlite::Connection {
    let conn = open_memory().unwrap();
    let platform = CatalogPlatform {
        id: "nes".to_string(),
        display_name: "Nintendo Entertainment System".to_string(),
        short_name: "NES".to_string(),
        manufacturer: "Nintendo".to_string(),
        generation: Some(3),
        media_type: MediaType::Cartridge,
        release_year: Some(1985),
        description: None,
        core_platform: Some("Nes".to_string()),
        regions: vec![],
        relationships: vec![],
    };
    upsert_platform(&conn, &platform).unwrap();

    insert_work(&conn, "nes:super-mario-bros", "Super Mario Bros.").unwrap();
    let release = Release {
        id: "nes:super-mario-bros:nes:usa".to_string(),
        work_id: "nes:super-mario-bros".to_string(),
        platform_id: "nes".to_string(),
        region: "usa".to_string(),
        title: "Super Mario Bros.".to_string(),
        alt_title: None,
        publisher_id: None,
        developer_id: None,
        release_date: None,
        game_serial: None,
        genre: None,
        players: None,
        rating: None,
        description: None,
        screenscraper_id: None,
        scraper_not_found: false,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_release(&conn, &release).unwrap();

    let media = Media {
        id: "m1".to_string(),
        release_id: "nes:super-mario-bros:nes:usa".to_string(),
        media_serial: None,
        disc_number: None,
        disc_label: None,
        revision: None,
        status: MediaStatus::Verified,
        dat_name: Some("Super Mario Bros. (USA).nes".to_string()),
        dat_source: Some("no-intro".to_string()),
        file_size: Some(40976),
        crc32: Some("d445f698".to_string()),
        sha1: Some("ea343f4e445a9050d4b4fbac2c77d0693b1d0922".to_string()),
        md5: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_media(&conn, &media).unwrap();

    conn
}

#[test]
fn collection_entry_lifecycle() {
    let conn = setup_db_with_media();

    // No collection entries initially
    let entries = list_collection(&conn, Some("nes"), None).unwrap();
    assert_eq!(entries.len(), 0);

    // Add to collection
    let entry = CollectionEntry {
        id: 0,
        media_id: "m1".to_string(),
        user_id: "default".to_string(),
        owned: true,
        condition: None,
        notes: None,
        date_acquired: None,
        rom_path: Some("/roms/nes/Super Mario Bros. (USA).nes".to_string()),
        verified_at: Some("2024-01-15T00:00:00Z".to_string()),
    };
    upsert_collection_entry(&conn, &entry).unwrap();

    // Should now appear in collection
    let entries = list_collection(&conn, Some("nes"), None).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].title, "Super Mario Bros.");
    assert_eq!(entries[0].platform_id, "nes");
    assert!(entries[0].owned);

    // find_collection_entry should find it
    let found = find_collection_entry(&conn, "m1", "default").unwrap();
    assert!(found.is_some());
    assert_eq!(
        found.unwrap().rom_path.as_deref(),
        Some("/roms/nes/Super Mario Bros. (USA).nes")
    );

    // Different user should not find it
    let not_found = find_collection_entry(&conn, "m1", "other_user").unwrap();
    assert!(not_found.is_none());
}

#[test]
fn collection_counts_by_platform_works() {
    let conn = setup_db_with_media();

    // Add a second platform with media
    let ps1 = CatalogPlatform {
        id: "ps1".to_string(),
        display_name: "PlayStation".to_string(),
        short_name: "PS1".to_string(),
        manufacturer: "Sony".to_string(),
        generation: Some(5),
        media_type: MediaType::Disc,
        release_year: Some(1994),
        description: None,
        core_platform: Some("Ps1".to_string()),
        regions: vec![],
        relationships: vec![],
    };
    upsert_platform(&conn, &ps1).unwrap();
    insert_work(&conn, "ps1:ff7", "Final Fantasy VII").unwrap();
    let r2 = Release {
        id: "ps1:ff7:ps1:usa".to_string(),
        work_id: "ps1:ff7".to_string(),
        platform_id: "ps1".to_string(),
        region: "usa".to_string(),
        title: "Final Fantasy VII".to_string(),
        alt_title: None,
        publisher_id: None,
        developer_id: None,
        release_date: None,
        game_serial: None,
        genre: None,
        players: None,
        rating: None,
        description: None,
        screenscraper_id: None,
        scraper_not_found: false,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_release(&conn, &r2).unwrap();
    let m2 = Media {
        id: "m2".to_string(),
        release_id: "ps1:ff7:ps1:usa".to_string(),
        media_serial: None,
        disc_number: Some(1),
        disc_label: None,
        revision: None,
        status: MediaStatus::Verified,
        dat_name: Some("Final Fantasy VII (USA) (Disc 1).bin".to_string()),
        dat_source: Some("redump".to_string()),
        file_size: Some(700000000),
        crc32: Some("aabb0001".to_string()),
        sha1: None,
        md5: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_media(&conn, &m2).unwrap();

    // Add collection entries for both platforms
    upsert_collection_entry(
        &conn,
        &CollectionEntry {
            id: 0,
            media_id: "m1".to_string(),
            user_id: "default".to_string(),
            owned: true,
            condition: None,
            notes: None,
            date_acquired: None,
            rom_path: None,
            verified_at: None,
        },
    )
    .unwrap();
    upsert_collection_entry(
        &conn,
        &CollectionEntry {
            id: 0,
            media_id: "m2".to_string(),
            user_id: "default".to_string(),
            owned: true,
            condition: None,
            notes: None,
            date_acquired: None,
            rom_path: None,
            verified_at: None,
        },
    )
    .unwrap();

    let counts = collection_counts_by_platform(&conn).unwrap();
    assert_eq!(counts.len(), 2);

    let nes_count = counts.iter().find(|(p, _)| p == "nes").map(|(_, c)| *c);
    let ps1_count = counts.iter().find(|(p, _)| p == "ps1").map(|(_, c)| *c);
    assert_eq!(nes_count, Some(1));
    assert_eq!(ps1_count, Some(1));
}

#[test]
fn collection_upsert_is_idempotent() {
    let conn = setup_db_with_media();

    let entry = CollectionEntry {
        id: 0,
        media_id: "m1".to_string(),
        user_id: "default".to_string(),
        owned: true,
        condition: None,
        notes: None,
        date_acquired: None,
        rom_path: Some("/roms/nes/smb.nes".to_string()),
        verified_at: None,
    };

    // Insert twice should not error
    upsert_collection_entry(&conn, &entry).unwrap();
    upsert_collection_entry(&conn, &entry).unwrap();

    let entries = list_collection(&conn, Some("nes"), None).unwrap();
    assert_eq!(entries.len(), 1);
}

#[test]
fn list_collection_without_platform_filter() {
    let conn = setup_db_with_media();

    upsert_collection_entry(
        &conn,
        &CollectionEntry {
            id: 0,
            media_id: "m1".to_string(),
            user_id: "default".to_string(),
            owned: true,
            condition: None,
            notes: None,
            date_acquired: None,
            rom_path: None,
            verified_at: None,
        },
    )
    .unwrap();

    // No platform filter — should return all
    let entries = list_collection(&conn, None, None).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].title, "Super Mario Bros.");
}

#[test]
fn catalog_stats_includes_collection() {
    let conn = setup_db_with_media();

    // Before adding to collection
    let stats = catalog_stats(&conn).unwrap();
    assert_eq!(stats.collection_owned, 0);

    // Add to collection
    upsert_collection_entry(
        &conn,
        &CollectionEntry {
            id: 0,
            media_id: "m1".to_string(),
            user_id: "default".to_string(),
            owned: true,
            condition: None,
            notes: None,
            date_acquired: None,
            rom_path: None,
            verified_at: None,
        },
    )
    .unwrap();

    let stats = catalog_stats(&conn).unwrap();
    assert_eq!(stats.collection_owned, 1);
}
