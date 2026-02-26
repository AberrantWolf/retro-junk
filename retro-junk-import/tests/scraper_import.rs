use retro_junk_catalog::types::{
    self, CatalogPlatform, MediaStatus, MediaType, Release,
};
use retro_junk_db::*;
use retro_junk_import::scraper_import::*;
use retro_junk_scraper::types::*;

// Alias to avoid conflict with scraper's Media type
type CatalogMedia = types::Media;

fn setup_db() -> rusqlite::Connection {
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
    conn
}

fn sample_game_info() -> GameInfo {
    GameInfo {
        id: "12345".to_string(),
        romid: Some("99999".to_string()),
        notgame: None,
        noms: vec![
            RegionText {
                region: "us".to_string(),
                text: "Super Mario Bros.".to_string(),
            },
            RegionText {
                region: "jp".to_string(),
                text: "Super Mario Brothers".to_string(),
            },
        ],
        synopsis: vec![
            LangueText {
                langue: "en".to_string(),
                text: "A classic platformer.".to_string(),
            },
            LangueText {
                langue: "ja".to_string(),
                text: "A classic platformer in Japanese.".to_string(),
            },
        ],
        dates: vec![
            RegionText {
                region: "us".to_string(),
                text: "1985-10-18".to_string(),
            },
            RegionText {
                region: "jp".to_string(),
                text: "1985-09-13".to_string(),
            },
        ],
        medias: vec![],
        editeur: Some(IdText {
            id: Some("100".to_string()),
            text: "Nintendo".to_string(),
        }),
        developpeur: Some(IdText {
            id: Some("101".to_string()),
            text: "Nintendo EAD".to_string(),
        }),
        joueurs: Some(IdText {
            id: None,
            text: "1-2".to_string(),
        }),
        note: Some(IdText {
            id: None,
            text: "18".to_string(),
        }),
        genres: vec![Genre {
            id: "1".to_string(),
            noms: vec![LangueText {
                langue: "en".to_string(),
                text: "Platform".to_string(),
            }],
        }],
        systeme: Some(IdText {
            id: Some("3".to_string()),
            text: "Nintendo Entertainment System".to_string(),
        }),
    }
}

#[test]
fn map_game_info_extracts_us_name() {
    let game = sample_game_info();
    let mapped = map_game_info(&game, "usa", "en");
    assert_eq!(mapped.title.as_deref(), Some("Super Mario Bros."));
}

#[test]
fn map_game_info_extracts_jp_name() {
    let game = sample_game_info();
    let mapped = map_game_info(&game, "japan", "ja");
    assert_eq!(mapped.title.as_deref(), Some("Super Mario Brothers"));
}

#[test]
fn map_game_info_extracts_release_date() {
    let game = sample_game_info();
    let mapped = map_game_info(&game, "usa", "en");
    assert_eq!(mapped.release_date.as_deref(), Some("1985-10-18"));
}

#[test]
fn map_game_info_extracts_jp_release_date() {
    let game = sample_game_info();
    let mapped = map_game_info(&game, "japan", "en");
    assert_eq!(mapped.release_date.as_deref(), Some("1985-09-13"));
}

#[test]
fn map_game_info_extracts_genre() {
    let game = sample_game_info();
    let mapped = map_game_info(&game, "usa", "en");
    assert_eq!(mapped.genre.as_deref(), Some("Platform"));
}

#[test]
fn map_game_info_extracts_players() {
    let game = sample_game_info();
    let mapped = map_game_info(&game, "usa", "en");
    assert_eq!(mapped.players.as_deref(), Some("1-2"));
}

#[test]
fn map_game_info_normalizes_rating() {
    let game = sample_game_info();
    let mapped = map_game_info(&game, "usa", "en");
    // Rating 18/20 = 0.9
    assert!(mapped.rating.is_some());
    let rating = mapped.rating.unwrap();
    assert!((rating - 0.9).abs() < 0.01);
}

#[test]
fn map_game_info_extracts_description() {
    let game = sample_game_info();
    let mapped = map_game_info(&game, "usa", "en");
    assert_eq!(mapped.description.as_deref(), Some("A classic platformer."));
}

#[test]
fn map_game_info_extracts_publisher() {
    let game = sample_game_info();
    let mapped = map_game_info(&game, "usa", "en");
    assert_eq!(mapped.publisher.as_deref(), Some("Nintendo"));
}

#[test]
fn map_game_info_extracts_developer() {
    let game = sample_game_info();
    let mapped = map_game_info(&game, "usa", "en");
    assert_eq!(mapped.developer.as_deref(), Some("Nintendo EAD"));
}

#[test]
fn map_game_info_handles_missing_fields() {
    let game = GameInfo {
        id: "1".to_string(),
        romid: None,
        notgame: None,
        noms: vec![],
        synopsis: vec![],
        dates: vec![],
        medias: vec![],
        editeur: None,
        developpeur: None,
        joueurs: None,
        note: None,
        genres: vec![],
        systeme: None,
    };
    let mapped = map_game_info(&game, "usa", "en");
    assert!(mapped.title.is_none());
    assert!(mapped.release_date.is_none());
    assert!(mapped.genre.is_none());
    assert!(mapped.players.is_none());
    assert!(mapped.rating.is_none());
    assert!(mapped.description.is_none());
    assert!(mapped.publisher.is_none());
    assert!(mapped.developer.is_none());
}

#[test]
fn catalog_region_to_ss_mappings() {
    assert_eq!(catalog_region_to_ss("usa"), "us");
    assert_eq!(catalog_region_to_ss("japan"), "jp");
    assert_eq!(catalog_region_to_ss("europe"), "eu");
    assert_eq!(catalog_region_to_ss("world"), "wor");
    assert_eq!(catalog_region_to_ss("unknown"), "us");
}

#[test]
fn ss_region_to_catalog_mappings() {
    assert_eq!(ss_region_to_catalog("us"), "usa");
    assert_eq!(ss_region_to_catalog("jp"), "japan");
    assert_eq!(ss_region_to_catalog("eu"), "europe");
    assert_eq!(ss_region_to_catalog("wor"), "world");
    assert_eq!(ss_region_to_catalog("xx"), "unknown");
}

#[test]
fn ss_media_type_mappings() {
    assert_eq!(ss_media_type_to_asset_type("box-2D"), Some("box-front"));
    assert_eq!(ss_media_type_to_asset_type("box-2D-back"), Some("box-back"));
    assert_eq!(ss_media_type_to_asset_type("ss"), Some("screenshot"));
    assert_eq!(ss_media_type_to_asset_type("sstitle"), Some("title-screen"));
    assert_eq!(ss_media_type_to_asset_type("wheel-hd"), Some("wheel"));
    assert_eq!(ss_media_type_to_asset_type("wheel"), Some("wheel"));
    assert_eq!(ss_media_type_to_asset_type("fanart"), Some("fanart"));
    assert_eq!(ss_media_type_to_asset_type("support-2D"), Some("cart-front"));
    assert_eq!(ss_media_type_to_asset_type("video-normalized"), Some("video"));
    assert_eq!(ss_media_type_to_asset_type("unknown-type"), None);
}

#[test]
fn enrichment_updates_release_fields() {
    let conn = setup_db();

    // Create a work and release with minimal data (as DAT import would)
    insert_work(&conn, "nes:super-mario-bros", "Super Mario Bros.").unwrap();
    let release = Release {
        id: "nes:super-mario-bros:nes:usa".to_string(),
        work_id: "nes:super-mario-bros".to_string(),
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

    // Create companies first (FK constraint requires them)
    use retro_junk_catalog::types::Company;
    upsert_company(
        &conn,
        &Company {
            id: "nintendo".to_string(),
            name: "Nintendo".to_string(),
            country: Some("Japan".to_string()),
            aliases: vec!["Nintendo".to_string()],
        },
    )
    .unwrap();
    upsert_company(
        &conn,
        &Company {
            id: "nintendo-ead".to_string(),
            name: "Nintendo EAD".to_string(),
            country: Some("Japan".to_string()),
            aliases: vec!["Nintendo EAD".to_string()],
        },
    )
    .unwrap();

    // Simulate enrichment update
    update_release_enrichment(
        &conn,
        "nes:super-mario-bros:nes:usa",
        "12345",
        Some("Super Mario Bros."),
        Some("1985-10-18"),
        Some("Platform"),
        Some("1-2"),
        Some(0.9),
        Some("A classic platformer."),
        Some("nintendo"),
        Some("nintendo-ead"),
    )
    .unwrap();

    // Verify the update
    let updated = find_release(&conn, "nes:super-mario-bros", "nes", "usa", "", "")
        .unwrap()
        .unwrap();
    assert_eq!(updated.screenscraper_id.as_deref(), Some("12345"));
    assert_eq!(updated.release_date.as_deref(), Some("1985-10-18"));
    assert_eq!(updated.genre.as_deref(), Some("Platform"));
    assert_eq!(updated.players.as_deref(), Some("1-2"));
    assert_eq!(updated.description.as_deref(), Some("A classic platformer."));
    assert_eq!(updated.publisher_id.as_deref(), Some("nintendo"));
    assert_eq!(updated.developer_id.as_deref(), Some("nintendo-ead"));
}

#[test]
fn enrichment_preserves_existing_fields() {
    let conn = setup_db();

    insert_work(&conn, "nes:zelda", "The Legend of Zelda").unwrap();
    let release = Release {
        id: "nes:zelda:nes:usa".to_string(),
        work_id: "nes:zelda".to_string(),
        platform_id: "nes".to_string(),
        region: "usa".to_string(),
        revision: String::new(),
        variant: String::new(),
        title: "The Legend of Zelda".to_string(),
        alt_title: None,
        publisher_id: None,
        developer_id: None,
        release_date: Some("1987-08-22".to_string()), // Already set by DAT
        game_serial: None,
        genre: Some("Adventure".to_string()), // Already set
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

    // Enrichment tries to set different date and genre
    update_release_enrichment(
        &conn,
        "nes:zelda:nes:usa",
        "67890",
        Some("The Legend of Zelda"),
        Some("1986-02-21"),  // Different date — should NOT overwrite
        Some("Action-Adventure"),  // Different genre — should NOT overwrite
        Some("1"),
        Some(0.95),
        Some("An epic adventure."),
        None,
        None,
    )
    .unwrap();

    let updated = find_release(&conn, "nes:zelda", "nes", "usa", "", "")
        .unwrap()
        .unwrap();
    // screenscraper_id is always updated
    assert_eq!(updated.screenscraper_id.as_deref(), Some("67890"));
    // Existing fields should be preserved (COALESCE keeps existing non-NULL values)
    assert_eq!(updated.release_date.as_deref(), Some("1987-08-22"));
    assert_eq!(updated.genre.as_deref(), Some("Adventure"));
    // NULL fields should be filled
    assert_eq!(updated.players.as_deref(), Some("1"));
    assert_eq!(updated.description.as_deref(), Some("An epic adventure."));
}

#[test]
fn releases_to_enrich_query() {
    let conn = setup_db();

    insert_work(&conn, "nes:smb", "Super Mario Bros.").unwrap();
    insert_work(&conn, "nes:zelda", "The Legend of Zelda").unwrap();

    let release1 = Release {
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
        screenscraper_id: None, // Not enriched
        scraper_not_found: false,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_release(&conn, &release1).unwrap();

    let release2 = Release {
        id: "nes:zelda:nes:usa".to_string(),
        work_id: "nes:zelda".to_string(),
        platform_id: "nes".to_string(),
        region: "usa".to_string(),
        revision: String::new(),
        variant: String::new(),
        title: "The Legend of Zelda".to_string(),
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
        screenscraper_id: Some("99999".to_string()), // Already enriched
        scraper_not_found: false,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_release(&conn, &release2).unwrap();

    // Add media for both (required for join)
    let media1 = CatalogMedia {
        id: "m1".to_string(),
        release_id: "nes:smb:nes:usa".to_string(),
        media_serial: None,
        disc_number: None,
        disc_label: None,
        revision: None,
        status: MediaStatus::Verified,
        dat_name: Some("Super Mario Bros. (USA).nes".to_string()),
        dat_source: Some("no-intro".to_string()),
        file_size: Some(40976),
        crc32: Some("d445f698".to_string()),
        sha1: None,
        md5: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_media(&conn, &media1).unwrap();

    let media2 = CatalogMedia {
        id: "m2".to_string(),
        release_id: "nes:zelda:nes:usa".to_string(),
        media_serial: None,
        disc_number: None,
        disc_label: None,
        revision: None,
        status: MediaStatus::Verified,
        dat_name: Some("Legend of Zelda, The (USA).nes".to_string()),
        dat_source: Some("no-intro".to_string()),
        file_size: Some(131088),
        crc32: Some("a12d74c1".to_string()),
        sha1: None,
        md5: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_media(&conn, &media2).unwrap();

    // With skip_existing=true, should only return SMB (Zelda already enriched)
    let to_enrich = releases_to_enrich(&conn, "nes", true, None).unwrap();
    assert_eq!(to_enrich.len(), 1);
    assert_eq!(to_enrich[0].title, "Super Mario Bros.");

    // With skip_existing=false, should return both
    let all = releases_to_enrich(&conn, "nes", false, None).unwrap();
    assert_eq!(all.len(), 2);

    // With limit
    let limited = releases_to_enrich(&conn, "nes", false, Some(1)).unwrap();
    assert_eq!(limited.len(), 1);
}

#[test]
fn media_for_release_query() {
    let conn = setup_db();
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

    let media1 = CatalogMedia {
        id: "m1".to_string(),
        release_id: "nes:smb:nes:usa".to_string(),
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
    upsert_media(&conn, &media1).unwrap();

    let results = media_for_release(&conn, "nes:smb:nes:usa").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].crc32.as_deref(), Some("d445f698"));
}
