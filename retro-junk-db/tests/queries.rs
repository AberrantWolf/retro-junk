use retro_junk_catalog::types::*;
use retro_junk_db::*;

fn setup_db() -> rusqlite::Connection {
    let conn = open_memory().unwrap();
    // Insert a platform and some test data
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
    insert_work(&conn, "smb1", "Super Mario Bros.").unwrap();
    insert_work(&conn, "zelda1", "The Legend of Zelda").unwrap();

    let smb_release = Release {
        id: "smb1-nes-usa".to_string(),
        work_id: "smb1".to_string(),
        platform_id: "nes".to_string(),
        region: "usa".to_string(),
        title: "Super Mario Bros.".to_string(),
        alt_title: None,
        publisher_id: None,
        developer_id: None,
        release_date: Some("1985-10-18".to_string()),
        game_serial: None,
        genre: None,
        players: None,
        rating: None,
        description: None,
        screenscraper_id: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_release(&conn, &smb_release).unwrap();

    let zelda_release = Release {
        id: "zelda1-nes-usa".to_string(),
        work_id: "zelda1".to_string(),
        platform_id: "nes".to_string(),
        region: "usa".to_string(),
        title: "The Legend of Zelda".to_string(),
        alt_title: None,
        publisher_id: None,
        developer_id: None,
        release_date: Some("1987-08-22".to_string()),
        game_serial: None,
        genre: None,
        players: None,
        rating: None,
        description: None,
        screenscraper_id: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_release(&conn, &zelda_release).unwrap();

    let smb_media = Media {
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
    upsert_media(&conn, &smb_media).unwrap();

    conn
}

#[test]
fn find_by_crc32() {
    let conn = setup_db();
    let results = find_media_by_crc32(&conn, "d445f698").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "smb1-nes-usa-v1");
}

#[test]
fn find_by_sha1() {
    let conn = setup_db();
    let results = find_media_by_sha1(
        &conn,
        "ea343f4e445a9050d4b4fbac2c77d0693b1d0922",
    )
    .unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn find_by_crc32_no_match() {
    let conn = setup_db();
    let results = find_media_by_crc32(&conn, "00000000").unwrap();
    assert!(results.is_empty());
}

#[test]
fn releases_for_platform_ordered() {
    let conn = setup_db();
    let releases = releases_for_platform(&conn, "nes").unwrap();
    assert_eq!(releases.len(), 2);
    // Should be alphabetically ordered
    assert_eq!(releases[0].title, "Super Mario Bros.");
    assert_eq!(releases[1].title, "The Legend of Zelda");
}

#[test]
fn search_releases_by_title() {
    let conn = setup_db();
    let results = search_releases(&conn, "Mario").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Super Mario Bros.");

    let results = search_releases(&conn, "zelda").unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn catalog_stats_counts() {
    let conn = setup_db();
    let stats = catalog_stats(&conn).unwrap();
    assert_eq!(stats.platforms, 1);
    assert_eq!(stats.works, 2);
    assert_eq!(stats.releases, 2);
    assert_eq!(stats.media, 1);
    assert_eq!(stats.assets, 0);
    assert_eq!(stats.collection_owned, 0);
}

#[test]
fn list_platforms_query() {
    let conn = setup_db();
    let platforms = list_platforms(&conn).unwrap();
    assert_eq!(platforms.len(), 1);
    assert_eq!(platforms[0].id, "nes");
    assert_eq!(platforms[0].core_platform.as_deref(), Some("Nes"));
}
