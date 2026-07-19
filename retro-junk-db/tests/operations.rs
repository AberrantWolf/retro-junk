use retro_junk_catalog::types::*;
use retro_junk_db::*;

fn test_platform() -> CatalogPlatform {
    CatalogPlatform {
        id: "nes".to_string(),
        display_name: "Nintendo Entertainment System".to_string(),
        short_name: "NES".to_string(),
        manufacturer: "Nintendo".to_string(),
        generation: 3,
        media_type: MediaType::Cartridge,
        release_year: 1985,
        description: String::new(),
        core_platform: "Nes".to_string(),
        regions: vec![PlatformRegion {
            region: "usa".to_string(),
            release_date: "1985-10-18".to_string(),
        }],
        relationships: vec![],
    }
}

fn test_company() -> Company {
    Company {
        id: "nintendo".to_string(),
        name: "Nintendo Co., Ltd.".to_string(),
        country: "Japan".to_string(),
        aliases: vec!["Nintendo".to_string(), "Nintendo EAD".to_string()],
    }
}

/// A release with every unset field at its empty default.
fn test_release(id: &str, work_id: &str, title: &str) -> Release {
    Release {
        id: id.to_string(),
        work_id: work_id.to_string(),
        platform_id: "nes".to_string(),
        region: "usa".to_string(),
        revision: String::new(),
        variant: String::new(),
        title: title.to_string(),
        alt_title: String::new(),
        publisher_id: None,
        developer_id: None,
        release_date: String::new(),
        game_serial: String::new(),
        genre: String::new(),
        players: String::new(),
        rating: None,
        description: String::new(),
        screen_title: String::new(),
        cover_title: String::new(),
        screenscraper_id: None,
        scraper_not_found: false,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

/// A media entry with every unset field at its empty default.
fn test_media(id: &str, release_id: &str) -> Media {
    Media {
        id: id.to_string(),
        release_id: release_id.to_string(),
        media_serial: String::new(),
        disc_number: 0,
        disc_label: String::new(),
        revision: String::new(),
        status: MediaStatus::Verified,
        tag: None,
        dat_name: String::new(),
        dat_source: String::new(),
        file_size: 0,
        crc32: String::new(),
        sha1: String::new(),
        md5: String::new(),
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn test_disagreement(entity_id: &str, field: &str, value_a: &str, value_b: &str) -> Disagreement {
    Disagreement {
        id: DisagreementId(0),
        entity_type: "release".to_string(),
        entity_id: entity_id.to_string(),
        field: field.to_string(),
        source_a: "no-intro".to_string(),
        value_a: value_a.to_string(),
        source_b: "screenscraper".to_string(),
        value_b: value_b.to_string(),
        resolved: false,
        resolution: String::new(),
        resolved_at: String::new(),
        created_at: String::new(),
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

    let mut release = test_release("smb1-nes-usa", "smb1", "Super Mario Bros.");
    release.release_date = "1985-10-18".to_string();
    release.genre = "Platform".to_string();
    release.players = "1-2".to_string();
    upsert_release(&conn, &release).unwrap();

    let found = find_release(&conn, "smb1", "nes", "usa", "", "").unwrap();
    assert!(found.is_some());
    let r = found.unwrap();
    assert_eq!(r.title, "Super Mario Bros.");
    assert_eq!(r.genre, "Platform");
    // Unset fields round-trip as empty strings, not NULL.
    assert_eq!(r.alt_title, "");
    assert_eq!(r.game_serial, "");
}

#[test]
fn media_upsert_and_find() {
    let conn = open_memory().unwrap();
    upsert_platform(&conn, &test_platform()).unwrap();
    insert_work(&conn, "smb1", "Super Mario Bros.").unwrap();
    upsert_release(
        &conn,
        &test_release("smb1-nes-usa", "smb1", "Super Mario Bros."),
    )
    .unwrap();

    let mut media = test_media("smb1-nes-usa-v1", "smb1-nes-usa");
    media.dat_name = "Super Mario Bros. (USA).nes".to_string();
    media.dat_source = "no-intro".to_string();
    media.file_size = 40976;
    media.crc32 = "d445f698".to_string();
    media.sha1 = "ea343f4e445a9050d4b4fbac2c77d0693b1d0922".to_string();
    upsert_media(&conn, &media).unwrap();

    let found = find_media_by_dat_name(&conn, "Super Mario Bros. (USA).nes").unwrap();
    assert!(found.is_some());
    let m = found.unwrap();
    assert_eq!(m.crc32, "d445f698");
    // Unset fields round-trip as empty/zero defaults, not NULL.
    assert_eq!(m.md5, "");
    assert_eq!(m.disc_number, 0);
}

#[test]
fn disagreement_lifecycle() {
    let conn = open_memory().unwrap();
    let d = test_disagreement("smb1-nes-usa", "release_date", "1985-10-18", "1985-09-13");
    let id = insert_disagreement(&conn, &d).unwrap();
    assert!(id > DisagreementId(0));

    resolve_disagreement(&conn, id, "source_a").unwrap();

    let resolved: bool = conn
        .query_row(
            "SELECT resolved FROM disagreements WHERE id = ?1",
            [id.0],
            |row| row.get(0),
        )
        .unwrap();
    assert!(resolved);
}

#[test]
fn get_disagreement_returns_record() {
    let conn = open_memory().unwrap();
    let d = test_disagreement(
        "smb1-nes-usa",
        "title",
        "Super Mario Bros.",
        "Super Mario Brothers",
    );
    let id = insert_disagreement(&conn, &d).unwrap();

    let found = get_disagreement(&conn, id).unwrap();
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.field, "title");
    assert_eq!(found.value_a, "Super Mario Bros.");
    // Unresolved rows carry empty resolution fields, not NULL.
    assert_eq!(found.resolution, "");
    assert_eq!(found.resolved_at, "");

    // Not found
    let missing = get_disagreement(&conn, DisagreementId(9999)).unwrap();
    assert!(missing.is_none());
}

#[test]
fn apply_disagreement_resolution_updates_entity() {
    let conn = open_memory().unwrap();
    let platform = test_platform();
    upsert_platform(&conn, &platform).unwrap();
    insert_work(&conn, "nes:smb", "Super Mario Bros.").unwrap();
    upsert_release(
        &conn,
        &test_release("nes:smb:nes:usa", "nes:smb", "Super Mario Bros."),
    )
    .unwrap();

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
    assert_eq!(updated.release_date, "1985-10-18");
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

#[test]
fn set_and_clear_work_tag() {
    let conn = open_memory().unwrap();
    insert_work(&conn, "test:homebrew-game", "Homebrew Game").unwrap();

    // Set tag
    set_work_tag(
        &conn,
        "test:homebrew-game",
        Some(retro_junk_catalog::CatalogTag::Homebrew),
    )
    .unwrap();
    let tag: Option<String> = conn
        .query_row(
            "SELECT tag FROM works WHERE id = 'test:homebrew-game'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tag.as_deref(), Some("homebrew"));

    // Clear tag
    set_work_tag(&conn, "test:homebrew-game", None).unwrap();
    let tag: Option<String> = conn
        .query_row(
            "SELECT tag FROM works WHERE id = 'test:homebrew-game'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(tag.is_none());
}

#[test]
fn set_and_clear_media_tag() {
    let conn = open_memory().unwrap();
    let platform = test_platform();
    upsert_platform(&conn, &platform).unwrap();
    insert_work(&conn, "test:game", "Test Game").unwrap();
    upsert_release(
        &conn,
        &test_release("test:game:nes:usa", "test:game", "Test Game"),
    )
    .unwrap();
    upsert_media(&conn, &test_media("test-media", "test:game:nes:usa")).unwrap();

    // Set tag
    set_media_tag(
        &conn,
        "test-media",
        Some(retro_junk_catalog::CatalogTag::Modded),
    )
    .unwrap();
    let tag: Option<String> = conn
        .query_row("SELECT tag FROM media WHERE id = 'test-media'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(tag.as_deref(), Some("modded"));

    // Clear tag
    set_media_tag(&conn, "test-media", None).unwrap();
    let tag: Option<String> = conn
        .query_row("SELECT tag FROM media WHERE id = 'test-media'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(tag.is_none());
}

#[test]
fn create_homebrew_work_creates_work_release_media() {
    let conn = open_memory().unwrap();
    let platform = test_platform();
    upsert_platform(&conn, &platform).unwrap();

    let work_id = create_homebrew_work(&conn, "My Homebrew Game", "nes", "usa").unwrap();

    // Work should exist with homebrew tag
    let tag: Option<String> = conn
        .query_row("SELECT tag FROM works WHERE id = ?1", [&work_id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(tag.as_deref(), Some("homebrew"));

    // Release should exist
    let release_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM releases WHERE work_id = ?1",
            [&work_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(release_count, 1);

    // Media should exist with homebrew tag
    let media_tag: Option<String> = conn
        .query_row(
            "SELECT m.tag FROM media m JOIN releases r ON m.release_id = r.id WHERE r.work_id = ?1",
            [&work_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(media_tag.as_deref(), Some("homebrew"));
}

#[test]
fn create_modded_media_and_detach() {
    let conn = open_memory().unwrap();
    let platform = test_platform();
    upsert_platform(&conn, &platform).unwrap();
    insert_work(&conn, "nes:smb", "Super Mario Bros.").unwrap();
    upsert_release(
        &conn,
        &test_release("nes:smb:nes:usa", "nes:smb", "Super Mario Bros."),
    )
    .unwrap();

    let media_id = create_modded_media(&conn, "nes:smb", "nes", "usa", None).unwrap();

    // Media should exist with modded tag
    let tag: Option<String> = conn
        .query_row("SELECT tag FROM media WHERE id = ?1", [&media_id], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(tag.as_deref(), Some("modded"));

    // Detach should remove the media
    detach_modded_media(&conn, &media_id).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM media WHERE id = ?1",
            [&media_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
