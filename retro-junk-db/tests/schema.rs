use retro_junk_db::schema::{CURRENT_VERSION, create_schema, open_database};
use retro_junk_db::{database_needs_migration, open_memory};

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
fn v20_adds_work_identity_to_archive_releases() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("v19.db");
    let connection = rusqlite::Connection::open(&db_path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_version(version INTEGER NOT NULL);
             INSERT INTO schema_version(version) VALUES(19);
             CREATE TABLE works(id TEXT PRIMARY KEY);
             CREATE TABLE archive_releases(
                 id TEXT PRIMARY KEY,
                 profile_id TEXT NOT NULL,
                 catalog_release_id TEXT,
                 platform_id TEXT NOT NULL,
                 title TEXT NOT NULL,
                 region TEXT NOT NULL DEFAULT '',
                 revision TEXT NOT NULL DEFAULT '',
                 variant TEXT NOT NULL DEFAULT '',
                 manifest_path TEXT NOT NULL,
                 manifest_sha256 TEXT NOT NULL,
                 binding_state TEXT NOT NULL DEFAULT 'unresolved');",
        )
        .unwrap();
    drop(connection);

    let connection = open_database(&db_path).unwrap();
    connection
        .execute(
            "UPDATE archive_releases SET catalog_work_id=NULL WHERE 1=0",
            [],
        )
        .unwrap();
    let index_exists: bool = connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sqlite_master
                 WHERE type='index' AND name='idx_archive_releases_work')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(index_exists);
}

/// Upgrading re-keys playable bindings on the archived carrier and re-derives
/// them in place, so a database written before the fix stops listing an
/// archived release's own playable file a second time as "playable only".
#[test]
fn v25_rebinds_archived_playables_to_their_carrier() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("v24.db");
    let connection = rusqlite::Connection::open(&db_path).unwrap();
    create_schema(&connection).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             INSERT INTO library_roots(id,root_path) VALUES(1,'/playable');
             INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,
                                          fingerprint_hash,scan_state)
             VALUES(1,1,'Nes','nes','/playable/nes','fp','ready');
             INSERT INTO library_entries(id,console_id,entry_key,display_name,game_entry_json)
             VALUES(1,1,'file:Game.nes','Game.nes','{}'),
                   (2,1,'file:Other.nes','Other.nes','{}');
             INSERT INTO works(id,canonical_name) VALUES('w','Game');
             INSERT INTO platforms(id,display_name,short_name,manufacturer,media_type)
             VALUES('nes','NES','NES','Nintendo','cartridge');
             INSERT INTO releases(id,work_id,platform_id,region,title)
             VALUES('r','w','nes','usa','Game');
             INSERT INTO media(id,release_id,dat_source) VALUES('m','r','no-intro');
             INSERT INTO archive_profiles(id,display_name,manifest_path,manifest_sha256,
                                          archive_root,playable_root)
             VALUES('p','Collection','retro-junk-archive.toml','sha','/archive','/playable');
             INSERT INTO archive_releases(id,profile_id,platform_id,title,manifest_path,
                                          manifest_sha256)
             VALUES('ar','p','nes','Game','release.toml','sha');
             -- The carrier deliberately has no catalog medium, as an unbound or
             -- stale-bound archive does.
             INSERT INTO physical_copies(id,archive_release_id,copy_number,manifest_path,
                                         manifest_sha256)
             VALUES('pc','ar',1,'physical-copy.toml','sha');
             INSERT INTO carriers(id,physical_copy_id,manifest_path,manifest_sha256)
             VALUES('c','pc','carrier.toml','sha');
             INSERT INTO representations(id,carrier_id,role,format,location_role,relative_path,
                                         presence_state)
             VALUES('rep','c','playable','rom','playable','nes/Game.nes','present');
             -- Rebuild the pre-v25 binding table and its single legacy row.
             DROP TABLE library_entry_media_bindings;
             CREATE TABLE library_entry_media_bindings(
                 library_entry_id INTEGER NOT NULL REFERENCES library_entries(id) ON DELETE CASCADE,
                 catalog_media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
                 representation_id TEXT REFERENCES representations(id) ON DELETE SET NULL,
                 match_method TEXT NOT NULL DEFAULT '',
                 PRIMARY KEY(library_entry_id, catalog_media_id));
             INSERT INTO library_entry_media_bindings(
                 library_entry_id,catalog_media_id,representation_id,match_method)
             VALUES(2,'m',NULL,'catalog_adoption');
             DELETE FROM schema_version;
             INSERT INTO schema_version(version) VALUES(24);
             PRAGMA foreign_keys=ON;",
        )
        .unwrap();
    drop(connection);

    let connection = open_database(&db_path).unwrap();
    // The archived carrier now owns the playable file its evidence points at,
    // with no catalog medium involved.
    let derived: (String, Option<String>, String) = connection
        .query_row(
            "SELECT carrier_id,catalog_media_id,match_method
             FROM library_entry_media_bindings WHERE library_entry_id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        derived,
        ("c".to_owned(), None, "archive_output_path".to_owned())
    );
    // A binding no archive rule derives is carried across untouched.
    let adopted: (Option<String>, String) = connection
        .query_row(
            "SELECT carrier_id,catalog_media_id
             FROM library_entry_media_bindings WHERE library_entry_id=2",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(adopted, (None, "m".to_owned()));
}

#[test]
fn migration_probe_is_read_only_and_distinguishes_current_legacy_and_missing_databases() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.db");
    assert!(!database_needs_migration(&missing).unwrap());
    assert!(!missing.exists(), "the probe must not create a database");

    let current = dir.path().join("current.db");
    drop(open_database(&current).unwrap());
    assert!(!database_needs_migration(&current).unwrap());

    let legacy = dir.path().join("legacy.db");
    let connection = rusqlite::Connection::open(&legacy).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_version(version INTEGER NOT NULL);
             INSERT INTO schema_version(version) VALUES(18);",
        )
        .unwrap();
    drop(connection);
    assert!(database_needs_migration(&legacy).unwrap());
    let version: i32 = rusqlite::Connection::open(&legacy)
        .unwrap()
        .query_row("SELECT max(version) FROM schema_version", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, 18, "the probe must not perform the migration");
}

#[test]
fn v18_adds_current_evidence_and_derived_verification_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v17.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version(version INTEGER NOT NULL, applied_at TEXT NOT NULL DEFAULT (datetime('now')));
             INSERT INTO schema_version(version) VALUES(17);
             CREATE TABLE representations(id TEXT PRIMARY KEY);
             CREATE TABLE verification_events(id TEXT PRIMARY KEY);",
        )
        .unwrap();
    }
    let conn = open_database(&path).unwrap();
    assert!(
        conn.prepare("SELECT catalog_verified,round_trip_verified FROM representations LIMIT 0")
            .is_ok()
    );
    assert!(
        conn.prepare("SELECT input_manifest_sha256 FROM verification_events LIMIT 0")
            .is_ok()
    );
}

#[test]
fn v11_marks_lossy_all_unknown_library_consoles_stale() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("v10.db");
    {
        let conn = open_database(&db_path).unwrap();
        conn.execute("DELETE FROM schema_version", []).unwrap();
        conn.execute("INSERT INTO schema_version(version) VALUES (10)", [])
            .unwrap();
        conn.execute(
            "INSERT INTO library_roots(id,root_path) VALUES(1,'/roms')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state) VALUES(1,1,'NES','nes','/roms/nes','fp','ready'),(2,1,'SNES','snes','/roms/snes','fp','ready')", []).unwrap();
        conn.execute("INSERT INTO library_entries(console_id,entry_key,display_name,game_entry_json,status) VALUES(1,'file:a.nes','a.nes','{}','unknown'),(2,'file:b.sfc','b.sfc','{}','likely')", []).unwrap();
    }

    let conn = open_database(&db_path).unwrap();
    let stale: String = conn
        .query_row(
            "SELECT scan_state FROM library_consoles WHERE id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let ready: String = conn
        .query_row(
            "SELECT scan_state FROM library_consoles WHERE id=2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stale, "stale");
    assert_eq!(ready, "ready");
}

#[test]
fn v15_invalidates_only_cached_match_verdicts_that_need_recomputing() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("v14.db");
    {
        let conn = open_database(&db_path).unwrap();
        conn.execute("DELETE FROM schema_version", []).unwrap();
        conn.execute("INSERT INTO schema_version(version) VALUES (14)", [])
            .unwrap();
        conn.execute_batch(
            "INSERT INTO platforms(id,display_name,short_name,manufacturer,media_type)
               VALUES('nds','Nintendo DS','nds','Nintendo','cartridge');
             INSERT INTO works(id,canonical_name) VALUES('work','Game');
             INSERT INTO releases(id,work_id,platform_id,region,title)
               VALUES('release','work','nds','usa','Game');
             INSERT INTO media(id,release_id,file_size,crc32,sha1)
               VALUES('media','release',64,'11111111','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
             INSERT INTO library_roots(id,root_path) VALUES(1,'/roms');
             INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state)
               VALUES
                 (1,1,'nds','wrong-verified','/roms/1','fp','ready'),
                 (2,1,'nds','right-verified','/roms/2','fp','ready'),
                 (3,1,'nds','right-likely','/roms/3','fp','ready'),
                 (4,1,'nds','wrong-likely','/roms/4','fp','ready'),
                 (5,1,'nds','unrecognized','/roms/5','fp','ready');
             INSERT INTO library_entries(console_id,entry_key,display_name,game_entry_json,status,crc32,data_size)
               VALUES
                 (1,'file:1','1','{}','matched','22222222',64),
                 (2,'file:2','2','{}','matched','11111111',64),
                 (3,'file:3','3','{}','likely','11111111',64),
                 (4,'file:4','4','{}','likely','22222222',64),
                 (5,'file:5','5','{}','unrecognized','',0);",
        )
        .unwrap();
    }

    let conn = open_database(&db_path).unwrap();
    let states: Vec<String> = conn
        .prepare("SELECT scan_state FROM library_consoles ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        states,
        ["stale", "ready", "stale", "ready", "stale"],
        "only unresolved or hash-inconsistent cached verdicts should be rebuilt"
    );
}

#[test]
fn v16_invalidates_legacy_cue_verdicts_without_touching_flat_rom_hashes() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("v15.db");
    {
        let conn = open_database(&db_path).unwrap();
        conn.execute("DELETE FROM schema_version", []).unwrap();
        conn.execute("INSERT INTO schema_version(version) VALUES (15)", [])
            .unwrap();
        conn.execute_batch(
            "INSERT INTO library_roots(id,root_path) VALUES(1,'/roms');
             INSERT INTO library_consoles(id,root_id,platform,folder_name,folder_path,fingerprint_hash,scan_state)
               VALUES
                 (1,1,'ps1','psx','/roms/psx','fp','ready'),
                 (2,1,'nes','nes','/roms/nes','fp','ready');
             INSERT INTO library_entries(console_id,entry_key,display_name,game_entry_json,status,crc32,data_size,dat_game_name,dat_match_method,disc_identifications_json)
               VALUES
                 (1,'file:game.cue','game.cue','{\"SingleFile\":\"/roms/psx/game.cue\"}','matched','11111111',100,'Game','crc32','[]'),
                 (2,'file:game.nes','game.nes','{\"SingleFile\":\"/roms/nes/game.nes\"}','matched','22222222',200,'Game','crc32',NULL);",
        )
        .unwrap();
    }

    let conn = open_database(&db_path).unwrap();
    let cue: (String, String, i64, Option<String>) = conn
        .query_row(
            "SELECT status,crc32,data_size,disc_identifications_json FROM library_entries WHERE console_id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(cue, ("unknown".into(), String::new(), 0, None));
    assert_eq!(
        conn.query_row(
            "SELECT scan_state FROM library_consoles WHERE id=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "stale"
    );

    let flat: (String, String, i64) = conn
        .query_row(
            "SELECT status,crc32,data_size FROM library_entries WHERE console_id=2",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(flat, ("matched".into(), "22222222".into(), 200));
    assert_eq!(
        conn.query_row(
            "SELECT scan_state FROM library_consoles WHERE id=2",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "ready"
    );
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
        assert!(exists, "table '{table}' should exist");
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
fn overrides_unique_key_includes_pattern_selectors() {
    let conn = open_memory().unwrap();

    // Two overrides for the same field, differing only in dat_name_pattern,
    // are distinct rows under the widened natural key.
    conn.execute(
        "INSERT INTO overrides (entity_type, dat_name_pattern, field, override_value, reason)
         VALUES ('media', 'Game A (USA)%', 'game_serial', 'A-1', 'test')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO overrides (entity_type, dat_name_pattern, field, override_value, reason)
         VALUES ('media', 'Game B (USA)%', 'game_serial', 'B-1', 'test')",
        [],
    )
    .unwrap();

    // An exact duplicate of all selectors + field violates UNIQUE.
    let dup = conn.execute(
        "INSERT INTO overrides (entity_type, dat_name_pattern, field, override_value, reason)
         VALUES ('media', 'Game A (USA)%', 'game_serial', 'A-2', 'test')",
        [],
    );
    assert!(dup.is_err(), "duplicate natural key should be rejected");
}

// ── Migration Tests ─────────────────────────────────────────────────────────

/// Create a legacy (pre-v9) database by hand at the given schema version.
///
/// Column layouts mirror the historical nullable schema. Every table the
/// v8 → v9 rebuild touches must exist (later migrations create the library
/// and `media_tracks` tables themselves).
fn create_legacy_db(db_path: &std::path::Path, version: i32) {
    assert!(
        (4..=5).contains(&version),
        "helper models the v4/v5 layouts"
    );
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

    // works/media gained their tag columns in v5.
    let works_tag = if version >= 5 { "tag TEXT," } else { "" };
    let media_tag = if version >= 5 { "tag TEXT," } else { "" };

    conn.execute_batch(&format!(
        "CREATE TABLE schema_version (version INTEGER NOT NULL, applied_at TEXT NOT NULL DEFAULT (datetime('now')));
         INSERT INTO schema_version (version) VALUES ({version});
         CREATE TABLE works (id TEXT PRIMARY KEY, canonical_name TEXT NOT NULL, {works_tag} created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));
         CREATE TABLE platforms (id TEXT PRIMARY KEY, display_name TEXT NOT NULL, short_name TEXT NOT NULL, manufacturer TEXT NOT NULL, generation INTEGER, media_type TEXT NOT NULL, release_year INTEGER, description TEXT, core_platform TEXT);
         CREATE TABLE platform_regions (platform_id TEXT NOT NULL, region TEXT NOT NULL, release_date TEXT, PRIMARY KEY (platform_id, region));
         CREATE TABLE companies (id TEXT PRIMARY KEY, name TEXT NOT NULL, country TEXT);
         CREATE TABLE releases (id TEXT PRIMARY KEY, work_id TEXT NOT NULL, platform_id TEXT NOT NULL, region TEXT NOT NULL, revision TEXT NOT NULL DEFAULT '', variant TEXT NOT NULL DEFAULT '', title TEXT NOT NULL, alt_title TEXT, publisher_id TEXT, developer_id TEXT, release_date TEXT, game_serial TEXT, genre TEXT, players TEXT, rating REAL, description TEXT, screen_title TEXT, cover_title TEXT, screenscraper_id TEXT, scraper_not_found BOOLEAN NOT NULL DEFAULT 0, created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));
         CREATE TABLE media (id TEXT PRIMARY KEY, release_id TEXT NOT NULL, media_serial TEXT, disc_number INTEGER, disc_label TEXT, revision TEXT, status TEXT NOT NULL DEFAULT 'verified', {media_tag} dat_name TEXT, dat_source TEXT, file_size INTEGER, crc32 TEXT, sha1 TEXT, md5 TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')), updated_at TEXT NOT NULL DEFAULT (datetime('now')));
         CREATE TABLE media_assets (id INTEGER PRIMARY KEY AUTOINCREMENT, release_id TEXT, media_id TEXT, asset_type TEXT NOT NULL, region TEXT, source TEXT NOT NULL, file_path TEXT, source_url TEXT, scraped BOOLEAN NOT NULL DEFAULT 0, file_hash TEXT, width INTEGER, height INTEGER, created_at TEXT NOT NULL DEFAULT (datetime('now')));
         CREATE TABLE collection (id INTEGER PRIMARY KEY AUTOINCREMENT, media_id TEXT NOT NULL, user_id TEXT NOT NULL DEFAULT 'default', owned BOOLEAN NOT NULL DEFAULT 1, condition TEXT, notes TEXT, date_acquired TEXT, rom_path TEXT, verified_at TEXT, UNIQUE(media_id, user_id));
         CREATE TABLE import_log (id INTEGER PRIMARY KEY AUTOINCREMENT, source_type TEXT NOT NULL, source_name TEXT NOT NULL, source_version TEXT, imported_at TEXT NOT NULL, records_created INTEGER, records_updated INTEGER, records_unchanged INTEGER, disagreements_found INTEGER);
         CREATE TABLE disagreements (id INTEGER PRIMARY KEY AUTOINCREMENT, entity_type TEXT NOT NULL, entity_id TEXT NOT NULL, field TEXT NOT NULL, source_a TEXT NOT NULL, value_a TEXT, source_b TEXT NOT NULL, value_b TEXT, resolved BOOLEAN NOT NULL DEFAULT 0, resolution TEXT, resolved_at TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')));
         CREATE TABLE overrides (id INTEGER PRIMARY KEY AUTOINCREMENT, entity_type TEXT NOT NULL, entity_id TEXT, platform_id TEXT, dat_name_pattern TEXT, field TEXT NOT NULL, override_value TEXT NOT NULL, reason TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')), UNIQUE(entity_type, entity_id, field));"
    ))
    .unwrap();

    // Seed NULL-heavy rows so the v9 rebuild's NULL -> ''/0 conversion is
    // observable after migration.
    conn.execute_batch(
        "INSERT INTO platforms (id, display_name, short_name, manufacturer, media_type) VALUES ('nes', 'NES', 'NES', 'Nintendo', 'cartridge');
         INSERT INTO works (id, canonical_name) VALUES ('w1', 'Work 1');
         INSERT INTO releases (id, work_id, platform_id, region, title) VALUES ('r1', 'w1', 'nes', 'usa', 'Work 1');
         INSERT INTO media (id, release_id) VALUES ('m1', 'r1');
         INSERT INTO overrides (entity_type, field, override_value, reason) VALUES ('media', 'game_serial', 'X-1', 'test');
         INSERT INTO overrides (entity_type, field, override_value, reason) VALUES ('media', 'game_serial', 'X-1', 'test');",
    )
    .unwrap();
    // The duplicate override row above is deliberate: the old UNIQUE key
    // contained NULL entity_id, which never conflicts in SQLite, so real
    // databases accumulated duplicates from repeated imports. The v9 rebuild
    // must dedupe them instead of failing on the new wider key.
}

/// Assert the migrated database converted legacy NULLs to ''/0 defaults.
fn assert_nulls_became_defaults(conn: &rusqlite::Connection) {
    let (generation, release_year, description): (i64, i64, String) = conn
        .query_row(
            "SELECT generation, release_year, description FROM platforms WHERE id = 'nes'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(generation, 0);
    assert_eq!(release_year, 0);
    assert_eq!(description, "");

    let (alt_title, release_date, game_serial): (String, String, String) = conn
        .query_row(
            "SELECT alt_title, release_date, game_serial FROM releases WHERE id = 'r1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(alt_title, "");
    assert_eq!(release_date, "");
    assert_eq!(game_serial, "");

    let (disc_number, file_size, crc32): (i64, i64, String) = conn
        .query_row(
            "SELECT disc_number, file_size, crc32 FROM media WHERE id = 'm1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(disc_number, 0);
    assert_eq!(file_size, 0);
    assert_eq!(crc32, "");

    let (entity_id, platform_id, dat_name_pattern): (String, String, String) = conn
        .query_row(
            "SELECT entity_id, platform_id, dat_name_pattern FROM overrides WHERE field = 'game_serial'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(entity_id, "");
    assert_eq!(platform_id, "");
    assert_eq!(dat_name_pattern, "");

    // The legacy duplicate override rows must be deduped by the rebuild, not
    // abort it (the old NULL-containing UNIQUE key never fired).
    let override_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM overrides WHERE field = 'game_serial'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(override_count, 1);
}

#[test]
fn v4_migration_adds_tag_columns_and_default_values() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    create_legacy_db(&db_path, 4);

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

    // Check that tag columns exist
    conn.execute("UPDATE works SET tag = 'homebrew' WHERE 1=0", [])
        .unwrap();
    conn.execute("UPDATE media SET tag = 'modded' WHERE 1=0", [])
        .unwrap();

    // Legacy NULLs must come out as ''/0 after the v9 rebuild
    assert_nulls_became_defaults(&conn);
}

#[test]
fn v5_migration_adds_library_tables_and_default_values() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");
    create_legacy_db(&db_path, 5);

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
            "table '{table}' should exist after v5->v6 migration"
        );
    }

    // Legacy NULLs must come out as ''/0 after the v9 rebuild
    assert_nulls_became_defaults(&conn);
}
