use retro_junk_catalog::types::*;
use retro_junk_db::*;

fn test_platform() -> CatalogPlatform {
    CatalogPlatform {
        id: "nes".to_string(),
        display_name: "Nintendo Entertainment System".to_string(),
        short_name: "NES".to_string(),
        manufacturer: "Nintendo".to_string(),
        generation: Some(3),
        media_type: MediaType::Cartridge,
        release_year: Some(1985),
        description: None,
        core_platform: Some("Nes".to_string()),
        regions: vec![PlatformRegion {
            region: "usa".to_string(),
            release_date: Some("1985-10-18".to_string()),
        }],
        relationships: vec![],
    }
}

fn test_company() -> Company {
    Company {
        id: "nintendo".to_string(),
        name: "Nintendo Co., Ltd.".to_string(),
        country: Some("Japan".to_string()),
        aliases: vec!["Nintendo".to_string(), "Nintendo EAD".to_string()],
    }
}

#[test]
fn upsert_and_query_platform() {
    let conn = open_memory().unwrap();
    let platform = test_platform();
    upsert_platform(&conn, &platform).unwrap();

    let name: String = conn
        .query_row(
            "SELECT display_name FROM platforms WHERE id = 'nes'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(name, "Nintendo Entertainment System");

    let region_count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM platform_regions WHERE platform_id = 'nes'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(region_count, 1);
}

#[test]
fn upsert_platform_is_idempotent() {
    let conn = open_memory().unwrap();
    let platform = test_platform();
    upsert_platform(&conn, &platform).unwrap();
    upsert_platform(&conn, &platform).unwrap();

    let count: i32 = conn
        .query_row("SELECT COUNT(*) FROM platforms", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn upsert_and_find_company() {
    let conn = open_memory().unwrap();
    let company = test_company();
    upsert_company(&conn, &company).unwrap();

    let found = find_company_by_alias(&conn, "Nintendo EAD").unwrap();
    assert_eq!(found, Some("nintendo".to_string()));

    let not_found = find_company_by_alias(&conn, "Sega").unwrap();
    assert_eq!(not_found, None);
}

#[test]
fn work_crud() {
    let conn = open_memory().unwrap();
    insert_work(&conn, "smb1", "Super Mario Bros.").unwrap();

    let found = find_work_by_name(&conn, "Super Mario Bros.").unwrap();
    assert_eq!(found, Some("smb1".to_string()));

    retro_junk_db::operations::update_work_name(&conn, "smb1", "Super Mario Bros").unwrap();
    let found = find_work_by_name(&conn, "Super Mario Bros").unwrap();
    assert_eq!(found, Some("smb1".to_string()));
}

#[test]
fn release_upsert_and_find() {
    let conn = open_memory().unwrap();
    upsert_platform(&conn, &test_platform()).unwrap();
    insert_work(&conn, "smb1", "Super Mario Bros.").unwrap();

    let release = Release {
        id: "smb1-nes-usa".to_string(),
        work_id: "smb1".to_string(),
        platform_id: "nes".to_string(),
        region: "usa".to_string(),
        revision: String::new(),
        variant: String::new(),
        title: "Super Mario Bros.".to_string(),
        alt_title: None,
        publisher_id: None,
        developer_id: None,
        release_date: Some("1985-10-18".to_string()),
        game_serial: None,
        genre: Some("Platform".to_string()),
        players: Some("1-2".to_string()),
        rating: None,
        description: None,
        screen_title: None,
        cover_title: None,
        screenscraper_id: None,
        scraper_not_found: false,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_release(&conn, &release).unwrap();

    let found = find_release(&conn, "smb1", "nes", "usa", "", "").unwrap();
    assert!(found.is_some());
    let r = found.unwrap();
    assert_eq!(r.title, "Super Mario Bros.");
    assert_eq!(r.genre.as_deref(), Some("Platform"));
}

#[test]
fn media_upsert_and_find() {
    let conn = open_memory().unwrap();
    upsert_platform(&conn, &test_platform()).unwrap();
    insert_work(&conn, "smb1", "Super Mario Bros.").unwrap();

    let release = Release {
        id: "smb1-nes-usa".to_string(),
        work_id: "smb1".to_string(),
        platform_id: "nes".to_string(),
        region: "usa".to_string(),
        revision: String::new(),
        variant: String::new(),
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
        screen_title: None,
        cover_title: None,
        screenscraper_id: None,
        scraper_not_found: false,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_release(&conn, &release).unwrap();

    let media = Media {
        id: "smb1-nes-usa-v1".to_string(),
        release_id: "smb1-nes-usa".to_string(),
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

    let found = find_media_by_dat_name(&conn, "Super Mario Bros. (USA).nes").unwrap();
    assert!(found.is_some());
    let m = found.unwrap();
    assert_eq!(m.crc32.as_deref(), Some("d445f698"));
}

#[test]
fn disagreement_lifecycle() {
    let conn = open_memory().unwrap();
    let d = Disagreement {
        id: 0,
        entity_type: "release".to_string(),
        entity_id: "smb1-nes-usa".to_string(),
        field: "release_date".to_string(),
        source_a: "no-intro".to_string(),
        value_a: Some("1985-10-18".to_string()),
        source_b: "screenscraper".to_string(),
        value_b: Some("1985-09-13".to_string()),
        resolved: false,
        resolution: None,
        resolved_at: None,
        created_at: String::new(),
    };
    let id = insert_disagreement(&conn, &d).unwrap();
    assert!(id > 0);

    resolve_disagreement(&conn, id, "source_a").unwrap();

    let resolved: bool = conn
        .query_row(
            "SELECT resolved FROM disagreements WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(resolved);
}

#[test]
fn get_disagreement_returns_record() {
    let conn = open_memory().unwrap();
    let d = Disagreement {
        id: 0,
        entity_type: "release".to_string(),
        entity_id: "smb1-nes-usa".to_string(),
        field: "title".to_string(),
        source_a: "no-intro".to_string(),
        value_a: Some("Super Mario Bros.".to_string()),
        source_b: "screenscraper".to_string(),
        value_b: Some("Super Mario Brothers".to_string()),
        resolved: false,
        resolution: None,
        resolved_at: None,
        created_at: String::new(),
    };
    let id = insert_disagreement(&conn, &d).unwrap();

    let found = get_disagreement(&conn, id).unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.field, "title");
    assert_eq!(found.value_a.as_deref(), Some("Super Mario Bros."));

    // Not found
    let missing = get_disagreement(&conn, 9999).unwrap();
    assert!(missing.is_none());
}

#[test]
fn apply_disagreement_resolution_updates_entity() {
    let conn = open_memory().unwrap();
    let platform = test_platform();
    upsert_platform(&conn, &platform).unwrap();
    insert_work(&conn, "nes:smb", "Super Mario Bros.").unwrap();

    let release = Release {
        id: "nes:smb:nes:usa".to_string(),
        work_id: "nes:smb".to_string(),
        platform_id: "nes".to_string(),
        region: "usa".to_string(),
        revision: String::new(),
        variant: String::new(),
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
        screen_title: None,
        cover_title: None,
        screenscraper_id: None,
        scraper_not_found: false,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_release(&conn, &release).unwrap();

    // Apply resolution to set release_date
    apply_disagreement_resolution(
        &conn,
        "release",
        "nes:smb:nes:usa",
        "release_date",
        "1985-10-18",
    )
    .unwrap();

    // Verify it was applied
    let updated = find_release(&conn, "nes:smb", "nes", "usa", "", "")
        .unwrap()
        .unwrap();
    assert_eq!(updated.release_date.as_deref(), Some("1985-10-18"));
}

#[test]
fn apply_disagreement_resolution_rejects_unsafe_field() {
    let conn = open_memory().unwrap();
    let result = apply_disagreement_resolution(
        &conn,
        "release",
        "nes:smb:nes:usa",
        "work_id",
        "different-work",
    );
    assert!(result.is_err());
}
