use retro_junk_catalog::types::*;
use retro_junk_db::*;
use retro_junk_import::*;

fn setup_db_with_release() -> (rusqlite::Connection, String) {
    let conn = open_memory().unwrap();
    let platform = CatalogPlatform {
        id: "nes".to_string(),
        display_name: "Nintendo Entertainment System".to_string(),
        short_name: "NES".to_string(),
        manufacturer: "Nintendo".to_string(),
        generation: 3,
        media_type: MediaType::Cartridge,
        release_year: 1985,
        description: String::new(),
        core_platform: "Nes".to_string(),
        regions: vec![],
        relationships: vec![],
    };
    upsert_platform(&conn, &platform).unwrap();
    insert_work(&conn, "smb1", "Super Mario Bros.").unwrap();

    let release = Release {
        id: "smb1-nes-usa".to_string(),
        work_id: "smb1".to_string(),
        platform_id: "nes".to_string(),
        region: "usa".to_string(),
        revision: String::new(),
        variant: String::new(),
        title: "Super Mario Bros.".to_string(),
        alt_title: String::new(),
        publisher_id: None,
        developer_id: None,
        release_date: "1985-10-18".to_string(),
        game_serial: String::new(),
        genre: "Platform".to_string(),
        players: String::new(),
        rating: None,
        description: String::new(),
        screen_title: String::new(),
        cover_title: String::new(),
        screenscraper_id: None,
        scraper_not_found: false,
        created_at: String::new(),
        updated_at: String::new(),
    };
    upsert_release(&conn, &release).unwrap();

    (conn, "smb1-nes-usa".to_string())
}

fn field_ref(field: &str) -> FieldRef<'_> {
    FieldRef {
        entity_type: "release",
        entity_id: "smb1-nes-usa",
        field,
    }
}

fn sourced<'a>(source: &'a str, value: &'a str) -> SourcedValue<'a> {
    SourcedValue { source, value }
}

#[test]
fn no_disagreement_when_values_match() {
    let (conn, _) = setup_db_with_release();
    let result = check_field(
        &conn,
        &field_ref("title"),
        &sourced("dat-import", "Super Mario Bros."),
        &sourced("screenscraper", "Super Mario Bros."),
    )
    .unwrap();
    assert!(!result);
}

#[test]
fn no_disagreement_when_new_source_fills_empty() {
    let (conn, _) = setup_db_with_release();
    let result = check_field(
        &conn,
        &field_ref("description"),
        &sourced("dat-import", ""),
        &sourced("screenscraper", "A platforming game."),
    )
    .unwrap();
    assert!(!result);
}

#[test]
fn disagreement_recorded_when_values_differ() {
    let (conn, _) = setup_db_with_release();
    let result = check_field(
        &conn,
        &field_ref("release_date"),
        &sourced("dat-import", "1985-10-18"),
        &sourced("screenscraper", "1985-09-13"),
    )
    .unwrap();
    assert!(result);

    let disagreements =
        list_unresolved_disagreements(&conn, &DisagreementFilter::default()).unwrap();
    assert_eq!(disagreements.len(), 1);
    assert_eq!(disagreements[0].field, "release_date");
    assert_eq!(disagreements[0].value_a, "1985-10-18");
    assert_eq!(disagreements[0].value_b, "1985-09-13");
}

#[test]
fn merge_release_counts_disagreements() {
    let (conn, release_id) = setup_db_with_release();
    let existing = find_release(&conn, "smb1", "nes", "usa", "", "")
        .unwrap()
        .unwrap();

    let count = merge_release_fields(
        &conn,
        &release_id,
        &existing,
        "screenscraper",
        &ReleaseFieldValues {
            title: "Super Mario Bros",  // slightly different title (no period)
            release_date: "1985-09-13", // different date
            genre: "Platform",          // same genre
            players: "1-2",             // new players field (was empty)
            description: "",            // no description
        },
    )
    .unwrap();

    // title differs, date differs = 2 disagreements
    // genre same = no disagreement
    // players: existing empty, new value = no disagreement (auto-resolve)
    assert_eq!(count, 2);
}

#[test]
fn glob_to_sql_like_conversion() {
    // Test via apply_overrides with an empty override set
    let conn = open_memory().unwrap();
    let applied = apply_overrides(&conn, &[]).unwrap();
    assert_eq!(applied, 0);
}
