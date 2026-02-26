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
        revision: String::new(),
        variant: String::new(),
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
        screen_title: None,
        cover_title: None,
        screenscraper_id: None,
        scraper_not_found: false,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_release(&conn, &smb_release).unwrap();

    let zelda_release = Release {
        id: "zelda1-nes-usa".to_string(),
        work_id: "zelda1".to_string(),
        platform_id: "nes".to_string(),
        region: "usa".to_string(),
        revision: String::new(),
        variant: String::new(),
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
        screen_title: None,
        cover_title: None,
        screenscraper_id: None,
        scraper_not_found: false,
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
    let results = find_media_by_sha1(&conn, "ea343f4e445a9050d4b4fbac2c77d0693b1d0922").unwrap();
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

#[test]
fn disagreement_filter_by_field() {
    let conn = setup_db();

    // Insert two disagreements with different fields
    insert_disagreement(
        &conn,
        &Disagreement {
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
        },
    )
    .unwrap();
    insert_disagreement(
        &conn,
        &Disagreement {
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
        },
    )
    .unwrap();

    // No filter — both
    let all = list_unresolved_disagreements(&conn, &Default::default()).unwrap();
    assert_eq!(all.len(), 2);

    // Filter by field
    let dates_only = list_unresolved_disagreements(
        &conn,
        &DisagreementFilter {
            field: Some("release_date"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(dates_only.len(), 1);
    assert_eq!(dates_only[0].field, "release_date");
}

#[test]
fn disagreement_filter_by_platform() {
    let conn = setup_db();

    // Add a PS1 platform with a release
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
    insert_work(&conn, "ff7", "Final Fantasy VII").unwrap();
    upsert_release(
        &conn,
        &Release {
            id: "ff7-ps1-usa".to_string(),
            work_id: "ff7".to_string(),
            platform_id: "ps1".to_string(),
            region: "usa".to_string(),
            revision: String::new(),
            variant: String::new(),
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
            screen_title: None,
            cover_title: None,
            screenscraper_id: None,
            scraper_not_found: false,
            created_at: String::new(),
            updated_at: String::new(),
        },
    )
    .unwrap();

    // NES disagreement
    insert_disagreement(
        &conn,
        &Disagreement {
            id: 0,
            entity_type: "release".to_string(),
            entity_id: "smb1-nes-usa".to_string(),
            field: "release_date".to_string(),
            source_a: "a".to_string(),
            value_a: Some("1985".to_string()),
            source_b: "b".to_string(),
            value_b: Some("1986".to_string()),
            resolved: false,
            resolution: None,
            resolved_at: None,
            created_at: String::new(),
        },
    )
    .unwrap();

    // PS1 disagreement
    insert_disagreement(
        &conn,
        &Disagreement {
            id: 0,
            entity_type: "release".to_string(),
            entity_id: "ff7-ps1-usa".to_string(),
            field: "release_date".to_string(),
            source_a: "a".to_string(),
            value_a: Some("1997".to_string()),
            source_b: "b".to_string(),
            value_b: Some("1998".to_string()),
            resolved: false,
            resolution: None,
            resolved_at: None,
            created_at: String::new(),
        },
    )
    .unwrap();

    // All disagreements
    let all = list_unresolved_disagreements(&conn, &Default::default()).unwrap();
    assert_eq!(all.len(), 2);

    // Filter to NES only
    let nes_only = list_unresolved_disagreements(
        &conn,
        &DisagreementFilter {
            platform_id: Some("nes"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(nes_only.len(), 1);
    assert_eq!(nes_only[0].entity_id, "smb1-nes-usa");

    // Filter to PS1 only
    let ps1_only = list_unresolved_disagreements(
        &conn,
        &DisagreementFilter {
            platform_id: Some("ps1"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(ps1_only.len(), 1);
    assert_eq!(ps1_only[0].entity_id, "ff7-ps1-usa");
}

// ── Asset Query Tests ─────────────────────────────────────────────────────

fn setup_db_with_assets() -> rusqlite::Connection {
    let conn = setup_db();

    // Insert assets for SMB1
    insert_media_asset(
        &conn,
        &MediaAsset {
            id: 0,
            release_id: Some("smb1-nes-usa".to_string()),
            media_id: None,
            asset_type: "box-front".to_string(),
            region: Some("usa".to_string()),
            source: "screenscraper".to_string(),
            file_path: Some("/assets/smb1/box-front.png".to_string()),
            source_url: Some("https://example.com/box.png".to_string()),
            scraped: true,
            file_hash: None,
            width: Some(300),
            height: Some(400),
            created_at: String::new(),
        },
    )
    .unwrap();

    insert_media_asset(
        &conn,
        &MediaAsset {
            id: 0,
            release_id: Some("smb1-nes-usa".to_string()),
            media_id: None,
            asset_type: "screenshot".to_string(),
            region: None,
            source: "screenscraper".to_string(),
            file_path: Some("/assets/smb1/screenshot.png".to_string()),
            source_url: None,
            scraped: true,
            file_hash: None,
            width: Some(256),
            height: Some(240),
            created_at: String::new(),
        },
    )
    .unwrap();

    conn
}

#[test]
fn assets_for_release_returns_all() {
    let conn = setup_db_with_assets();

    let assets = assets_for_release(&conn, "smb1-nes-usa").unwrap();
    assert_eq!(assets.len(), 2);
    // Ordered by asset_type
    assert_eq!(assets[0].asset_type, "box-front");
    assert_eq!(assets[1].asset_type, "screenshot");

    // Empty for zelda (no assets)
    let empty = assets_for_release(&conn, "zelda1-nes-usa").unwrap();
    assert!(empty.is_empty());
}

#[test]
fn asset_coverage_summary_counts() {
    let conn = setup_db_with_assets();

    let (total, with_assets, asset_count) = asset_coverage_summary(&conn, "nes", false).unwrap();
    assert_eq!(total, 2); // SMB + Zelda
    assert_eq!(with_assets, 1); // Only SMB has assets
    assert_eq!(asset_count, 2); // 2 assets for SMB
}

#[test]
fn asset_counts_by_type_works() {
    let conn = setup_db_with_assets();

    let counts = asset_counts_by_type(&conn, "nes", false).unwrap();
    assert_eq!(counts.len(), 2);
    assert!(counts.iter().any(|(t, c)| t == "box-front" && *c == 1));
    assert!(counts.iter().any(|(t, c)| t == "screenshot" && *c == 1));
}

#[test]
fn releases_missing_asset_type_finds_gaps() {
    let conn = setup_db_with_assets();

    // Zelda is missing box-front
    let missing = releases_missing_asset_type(&conn, "nes", "box-front", false, None).unwrap();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].1, "The Legend of Zelda");

    // Both are missing fanart
    let missing = releases_missing_asset_type(&conn, "nes", "fanart", false, None).unwrap();
    assert_eq!(missing.len(), 2);
}

#[test]
fn releases_with_no_assets_finds_bare() {
    let conn = setup_db_with_assets();

    let bare = releases_with_no_assets(&conn, "nes", false, None).unwrap();
    assert_eq!(bare.len(), 1);
    assert_eq!(bare[0].1, "The Legend of Zelda");
}

#[test]
fn asset_queries_with_collection_filter() {
    let conn = setup_db_with_assets();

    // Add SMB to collection
    upsert_collection_entry(
        &conn,
        &CollectionEntry {
            id: 0,
            media_id: "smb1-nes-usa-v1".to_string(),
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

    // Collection-only coverage: only SMB is in collection
    let (total, with_assets, _) = asset_coverage_summary(&conn, "nes", true).unwrap();
    assert_eq!(total, 1); // Only SMB in collection
    assert_eq!(with_assets, 1); // SMB has assets

    // Collection-only: no releases with no assets (SMB has assets)
    let bare = releases_with_no_assets(&conn, "nes", true, None).unwrap();
    assert!(bare.is_empty());

    // Collection-only: SMB is not missing box-front
    let missing = releases_missing_asset_type(&conn, "nes", "box-front", true, None).unwrap();
    assert!(missing.is_empty());

    // Collection-only: SMB IS missing fanart
    let missing = releases_missing_asset_type(&conn, "nes", "fanart", true, None).unwrap();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].1, "Super Mario Bros.");
}

// ── Lookup Query Tests ────────────────────────────────────────────────────

#[test]
fn search_releases_filtered_no_platform() {
    let conn = setup_db();
    let results = search_releases_filtered(&conn, "Mario", None, 25).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Super Mario Bros.");
}

#[test]
fn search_releases_filtered_with_platform() {
    let conn = setup_db();
    // Should find Mario on NES
    let results = search_releases_filtered(&conn, "Mario", Some("nes"), 25).unwrap();
    assert_eq!(results.len(), 1);

    // Should not find Mario on a non-existent platform
    let results = search_releases_filtered(&conn, "Mario", Some("snes"), 25).unwrap();
    assert!(results.is_empty());
}

#[test]
fn search_releases_filtered_respects_limit() {
    let conn = setup_db();
    // Both releases match "%e%" (Super Mario Bros., The Legend of Zelda)
    let results = search_releases_filtered(&conn, "e", None, 1).unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn find_media_by_md5_no_match() {
    let conn = setup_db();
    let results = find_media_by_md5(&conn, "00000000000000000000000000000000").unwrap();
    assert!(results.is_empty());
}

#[test]
fn find_media_by_md5_with_match() {
    let conn = setup_db();
    // Insert a media entry with an MD5
    upsert_media(
        &conn,
        &Media {
            id: "zelda1-nes-usa-v1".to_string(),
            release_id: "zelda1-nes-usa".to_string(),
            media_serial: None,
            disc_number: None,
            disc_label: None,
            revision: None,
            status: MediaStatus::Verified,
            dat_name: Some("The Legend of Zelda (USA).nes".to_string()),
            dat_source: Some("no-intro".to_string()),
            file_size: Some(131088),
            crc32: Some("a12b1f68".to_string()),
            sha1: Some("a1234567890abcdef1234567890abcdef12345678".to_string()),
            md5: Some("abc123def456abc123def456abc123de".to_string()),
            created_at: String::new(),
            updated_at: String::new(),
        },
    )
    .unwrap();

    let results = find_media_by_md5(&conn, "abc123def456abc123def456abc123de").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "zelda1-nes-usa-v1");
}

#[test]
fn get_release_by_id_found() {
    let conn = setup_db();
    let release = get_release_by_id(&conn, "smb1-nes-usa").unwrap();
    assert!(release.is_some());
    assert_eq!(release.unwrap().title, "Super Mario Bros.");
}

#[test]
fn get_release_by_id_not_found() {
    let conn = setup_db();
    let release = get_release_by_id(&conn, "nonexistent").unwrap();
    assert!(release.is_none());
}

#[test]
fn get_company_name_found() {
    let conn = setup_db();
    upsert_company(
        &conn,
        &Company {
            id: "nintendo".to_string(),
            name: "Nintendo".to_string(),
            country: Some("Japan".to_string()),
            aliases: vec![],
        },
    )
    .unwrap();

    let name = get_company_name(&conn, "nintendo").unwrap();
    assert_eq!(name, Some("Nintendo".to_string()));
}

#[test]
fn get_company_name_not_found() {
    let conn = setup_db();
    let name = get_company_name(&conn, "nonexistent").unwrap();
    assert!(name.is_none());
}

#[test]
fn get_platform_display_name_found() {
    let conn = setup_db();
    let name = get_platform_display_name(&conn, "nes").unwrap();
    assert_eq!(name, Some("NES".to_string()));
}

#[test]
fn get_platform_display_name_not_found() {
    let conn = setup_db();
    let name = get_platform_display_name(&conn, "nonexistent").unwrap();
    assert!(name.is_none());
}
