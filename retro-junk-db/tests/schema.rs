use retro_junk_db::open_memory;
use retro_junk_db::schema::{create_schema, CURRENT_VERSION};

#[test]
fn create_schema_in_memory() {
    let conn = open_memory().unwrap();
    let version: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, CURRENT_VERSION);
}

#[test]
fn schema_is_idempotent() {
    let conn = open_memory().unwrap();
    // Creating again should not error
    create_schema(&conn).unwrap();
}

#[test]
fn foreign_keys_enabled() {
    let conn = open_memory().unwrap();
    let fk: i32 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fk, 1);
}

#[test]
fn all_tables_exist() {
    let conn = open_memory().unwrap();
    let tables = [
        "schema_version",
        "platforms",
        "platform_regions",
        "platform_relationships",
        "companies",
        "company_aliases",
        "works",
        "work_relationships",
        "releases",
        "media",
        "media_assets",
        "collection",
        "import_log",
        "disagreements",
        "overrides",
    ];
    for table in tables {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "table '{}' should exist", table);
    }
}
