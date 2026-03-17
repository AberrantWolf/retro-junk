use retro_junk_db::open_memory;
use retro_junk_db::schema::{CURRENT_VERSION, create_schema, open_database};

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
        "library_roots",
        "library_consoles",
        "library_entries",
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

#[test]
fn works_table_has_tag_column() {
    let conn = open_memory().unwrap();
    // Inserting a work with a tag should succeed
    conn.execute(
        "INSERT INTO works (id, canonical_name, tag) VALUES ('test-work', 'Test Work', 'homebrew')",
        [],
    )
    .unwrap();
    let tag: Option<String> = conn
        .query_row("SELECT tag FROM works WHERE id = 'test-work'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(tag.as_deref(), Some("homebrew"));
}

#[test]
fn media_table_has_tag_column() {
    let conn = open_memory().unwrap();
    // Set up prerequisite rows
    conn.execute(
        "INSERT INTO works (id, canonical_name) VALUES ('w1', 'Work 1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO platforms (id, display_name, short_name, manufacturer, media_type) VALUES ('nes', 'NES', 'NES', 'Nintendo', 'cartridge')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO releases (id, work_id, platform_id, region, title) VALUES ('r1', 'w1', 'nes', 'usa', 'Work 1')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO media (id, release_id, tag) VALUES ('m1', 'r1', 'modded')",
        [],
    )
    .unwrap();
    let tag: Option<String> = conn
        .query_row("SELECT tag FROM media WHERE id = 'm1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(tag.as_deref(), Some("modded"));
}

#[test]
fn v4_to_v5_migration_adds_tag_columns() {
    // Create a temporary database at v4, then migrate
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    // Create a v4 database manually
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        // Create minimal v4 schema
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL, applied_at TEXT NOT NULL DEFAULT (datetime('now')));
             INSERT INTO schema_version (version) VALUES (4);
             CREATE TABLE works (id TEXT PRIMARY KEY, canonical_name TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));
             CREATE TABLE platforms (id TEXT PRIMARY KEY, display_name TEXT NOT NULL, short_name TEXT NOT NULL, manufacturer TEXT NOT NULL, generation INTEGER, media_type TEXT NOT NULL, release_year INTEGER, description TEXT, core_platform TEXT);
             CREATE TABLE releases (id TEXT PRIMARY KEY, work_id TEXT NOT NULL, platform_id TEXT NOT NULL, region TEXT NOT NULL, revision TEXT NOT NULL DEFAULT '', variant TEXT NOT NULL DEFAULT '', title TEXT NOT NULL, alt_title TEXT, publisher_id TEXT, developer_id TEXT, release_date TEXT, game_serial TEXT, genre TEXT, players TEXT, rating REAL, description TEXT, screen_title TEXT, cover_title TEXT, screenscraper_id TEXT, scraper_not_found BOOLEAN NOT NULL DEFAULT 0, created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));
             CREATE TABLE media (id TEXT PRIMARY KEY, release_id TEXT NOT NULL, media_serial TEXT, disc_number INTEGER, disc_label TEXT, revision TEXT, status TEXT NOT NULL DEFAULT 'verified', dat_name TEXT, dat_source TEXT, file_size INTEGER, crc32 TEXT, sha1 TEXT, md5 TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));",
        )
        .unwrap();
    }

    // Open with migration
    let conn = open_database(&db_path).unwrap();

    // Check that version is now 5
    let version: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, CURRENT_VERSION);

    // Check that tag columns exist
    conn.execute("UPDATE works SET tag = 'homebrew' WHERE 1=0", [])
        .unwrap();
    conn.execute("UPDATE media SET tag = 'modded' WHERE 1=0", [])
        .unwrap();
}

#[test]
fn v5_to_v6_migration_adds_library_tables() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    // Create a v5 database manually
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL, applied_at TEXT NOT NULL DEFAULT (datetime('now')));
             INSERT INTO schema_version (version) VALUES (5);
             CREATE TABLE works (id TEXT PRIMARY KEY, canonical_name TEXT NOT NULL, tag TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));
             CREATE TABLE platforms (id TEXT PRIMARY KEY, display_name TEXT NOT NULL, short_name TEXT NOT NULL, manufacturer TEXT NOT NULL, generation INTEGER, media_type TEXT NOT NULL, release_year INTEGER, description TEXT, core_platform TEXT);
             CREATE TABLE releases (id TEXT PRIMARY KEY, work_id TEXT NOT NULL, platform_id TEXT NOT NULL, region TEXT NOT NULL, revision TEXT NOT NULL DEFAULT '', variant TEXT NOT NULL DEFAULT '', title TEXT NOT NULL, alt_title TEXT, publisher_id TEXT, developer_id TEXT, release_date TEXT, game_serial TEXT, genre TEXT, players TEXT, rating REAL, description TEXT, screen_title TEXT, cover_title TEXT, screenscraper_id TEXT, scraper_not_found BOOLEAN NOT NULL DEFAULT 0, created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));
             CREATE TABLE media (id TEXT PRIMARY KEY, release_id TEXT NOT NULL, media_serial TEXT, disc_number INTEGER, disc_label TEXT, revision TEXT, status TEXT NOT NULL DEFAULT 'verified', tag TEXT, dat_name TEXT, dat_source TEXT, file_size INTEGER, crc32 TEXT, sha1 TEXT, md5 TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));",
        )
        .unwrap();
    }

    // Open with migration
    let conn = open_database(&db_path).unwrap();

    // Check that version is now current
    let version: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, CURRENT_VERSION);

    // Check that library tables exist
    for table in ["library_roots", "library_consoles", "library_entries"] {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            exists,
            "table '{}' should exist after v5->v6 migration",
            table
        );
    }
}
