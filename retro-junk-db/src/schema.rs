//! `SQLite` schema creation and migration.
//!
//! Table definitions live in [`TABLES`] as `(name, body)` pairs so that fresh
//! `CREATE TABLE` statements and migration table-rebuilds always share one
//! canonical definition.

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Migration error: expected version {expected}, found {found}")]
    VersionMismatch { expected: i32, found: i32 },
    #[error("Unknown table in migration: {0}")]
    UnknownTable(&'static str),
    #[error("Library migration error: {0}")]
    LibraryMigration(String),
}

/// Current schema version. Increment when adding migrations.
pub const CURRENT_VERSION: i32 = 14;

/// Canonical table definitions: `(name, column body)`.
///
/// Text columns default to `''` and numeric columns to `0` for "not set";
/// a column is nullable only where NULL is load-bearing (FK targets,
/// enrichment sentinels, optional tags, JSON blobs).
const TABLES: &[(&str, &str)] = &[
    (
        "schema_version",
        "(version INTEGER NOT NULL,
          applied_at TEXT NOT NULL DEFAULT (datetime('now')))",
    ),
    (
        "platforms",
        "(id TEXT PRIMARY KEY,
          display_name TEXT NOT NULL,
          short_name TEXT NOT NULL,
          manufacturer TEXT NOT NULL,
          generation INTEGER NOT NULL DEFAULT 0,
          media_type TEXT NOT NULL,
          release_year INTEGER NOT NULL DEFAULT 0,
          description TEXT NOT NULL DEFAULT '',
          core_platform TEXT NOT NULL DEFAULT '')",
    ),
    (
        "platform_regions",
        "(platform_id TEXT NOT NULL REFERENCES platforms(id),
          region TEXT NOT NULL,
          release_date TEXT NOT NULL DEFAULT '',
          PRIMARY KEY (platform_id, region))",
    ),
    (
        "platform_relationships",
        "(platform_a TEXT NOT NULL REFERENCES platforms(id),
          platform_b TEXT NOT NULL REFERENCES platforms(id),
          relationship TEXT NOT NULL,
          PRIMARY KEY (platform_a, platform_b, relationship))",
    ),
    (
        "companies",
        "(id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          country TEXT NOT NULL DEFAULT '')",
    ),
    (
        "company_aliases",
        "(company_id TEXT NOT NULL REFERENCES companies(id),
          alias TEXT NOT NULL,
          PRIMARY KEY (company_id, alias))",
    ),
    (
        "works",
        "(id TEXT PRIMARY KEY,
          canonical_name TEXT NOT NULL,
          tag TEXT,
          created_at TEXT NOT NULL DEFAULT (datetime('now')),
          updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
    ),
    (
        "work_relationships",
        "(work_a TEXT NOT NULL REFERENCES works(id),
          work_b TEXT NOT NULL REFERENCES works(id),
          relationship TEXT NOT NULL,
          PRIMARY KEY (work_a, work_b, relationship))",
    ),
    (
        "releases",
        "(id TEXT PRIMARY KEY,
          work_id TEXT NOT NULL REFERENCES works(id),
          platform_id TEXT NOT NULL REFERENCES platforms(id),
          region TEXT NOT NULL,
          revision TEXT NOT NULL DEFAULT '',
          variant TEXT NOT NULL DEFAULT '',
          title TEXT NOT NULL,
          alt_title TEXT NOT NULL DEFAULT '',
          publisher_id TEXT REFERENCES companies(id),
          developer_id TEXT REFERENCES companies(id),
          release_date TEXT NOT NULL DEFAULT '',
          game_serial TEXT NOT NULL DEFAULT '',
          genre TEXT NOT NULL DEFAULT '',
          players TEXT NOT NULL DEFAULT '',
          rating REAL,
          description TEXT NOT NULL DEFAULT '',
          screen_title TEXT NOT NULL DEFAULT '',
          cover_title TEXT NOT NULL DEFAULT '',
          screenscraper_id TEXT,
          scraper_not_found BOOLEAN NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL DEFAULT (datetime('now')),
          updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
    ),
    (
        "media",
        "(id TEXT PRIMARY KEY,
          release_id TEXT NOT NULL REFERENCES releases(id),
          media_serial TEXT NOT NULL DEFAULT '',
          disc_number INTEGER NOT NULL DEFAULT 0,
          disc_label TEXT NOT NULL DEFAULT '',
          revision TEXT NOT NULL DEFAULT '',
          status TEXT NOT NULL DEFAULT 'verified',
          tag TEXT,
          dat_name TEXT NOT NULL DEFAULT '',
          rom_name TEXT NOT NULL DEFAULT '',
          dat_source TEXT NOT NULL DEFAULT '',
          file_size INTEGER NOT NULL DEFAULT 0,
          crc32 TEXT NOT NULL DEFAULT '',
          sha1 TEXT NOT NULL DEFAULT '',
          md5 TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL DEFAULT (datetime('now')),
          updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
    ),
    (
        "media_serial_keys",
        "(media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
          serial_key TEXT NOT NULL,
          PRIMARY KEY (media_id, serial_key))",
    ),
    (
        "media_assets",
        "(id INTEGER PRIMARY KEY AUTOINCREMENT,
          release_id TEXT REFERENCES releases(id),
          media_id TEXT REFERENCES media(id),
          asset_type TEXT NOT NULL,
          region TEXT NOT NULL DEFAULT '',
          source TEXT NOT NULL,
          file_path TEXT NOT NULL DEFAULT '',
          source_url TEXT NOT NULL DEFAULT '',
          scraped BOOLEAN NOT NULL DEFAULT 0,
          file_hash TEXT NOT NULL DEFAULT '',
          width INTEGER NOT NULL DEFAULT 0,
          height INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL DEFAULT (datetime('now')),
          CHECK ((release_id IS NULL) != (media_id IS NULL)))",
    ),
    (
        "collection",
        "(id INTEGER PRIMARY KEY AUTOINCREMENT,
          media_id TEXT NOT NULL REFERENCES media(id),
          user_id TEXT NOT NULL DEFAULT 'default',
          owned BOOLEAN NOT NULL DEFAULT 1,
          condition TEXT NOT NULL DEFAULT '',
          notes TEXT NOT NULL DEFAULT '',
          date_acquired TEXT NOT NULL DEFAULT '',
          rom_path TEXT NOT NULL DEFAULT '',
          verified_at TEXT NOT NULL DEFAULT '',
          UNIQUE(media_id, user_id))",
    ),
    (
        "import_log",
        "(id INTEGER PRIMARY KEY AUTOINCREMENT,
          source_type TEXT NOT NULL,
          source_name TEXT NOT NULL,
          source_version TEXT NOT NULL DEFAULT '',
          imported_at TEXT NOT NULL,
          records_created INTEGER NOT NULL DEFAULT 0,
          records_updated INTEGER NOT NULL DEFAULT 0,
          records_unchanged INTEGER NOT NULL DEFAULT 0,
          disagreements_found INTEGER NOT NULL DEFAULT 0)",
    ),
    (
        "disagreements",
        "(id INTEGER PRIMARY KEY AUTOINCREMENT,
          entity_type TEXT NOT NULL,
          entity_id TEXT NOT NULL,
          field TEXT NOT NULL,
          source_a TEXT NOT NULL,
          value_a TEXT NOT NULL DEFAULT '',
          source_b TEXT NOT NULL,
          value_b TEXT NOT NULL DEFAULT '',
          resolved BOOLEAN NOT NULL DEFAULT 0,
          resolution TEXT NOT NULL DEFAULT '',
          resolved_at TEXT NOT NULL DEFAULT '',
          created_at TEXT NOT NULL DEFAULT (datetime('now')))",
    ),
    (
        "overrides",
        "(id INTEGER PRIMARY KEY AUTOINCREMENT,
          entity_type TEXT NOT NULL,
          entity_id TEXT NOT NULL DEFAULT '',
          platform_id TEXT NOT NULL DEFAULT '',
          dat_name_pattern TEXT NOT NULL DEFAULT '',
          field TEXT NOT NULL,
          override_value TEXT NOT NULL,
          reason TEXT NOT NULL,
          created_at TEXT NOT NULL DEFAULT (datetime('now')),
          UNIQUE(entity_type, entity_id, platform_id, dat_name_pattern, field))",
    ),
    (
        "media_tracks",
        "(id INTEGER PRIMARY KEY AUTOINCREMENT,
          media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
          track_number INTEGER NOT NULL,
          track_name TEXT NOT NULL,
          file_size INTEGER NOT NULL DEFAULT 0,
          crc32 TEXT NOT NULL DEFAULT '',
          sha1 TEXT NOT NULL DEFAULT '',
          md5 TEXT NOT NULL DEFAULT '',
          UNIQUE(media_id, track_number))",
    ),
    (
        "library_roots",
        "(id INTEGER PRIMARY KEY AUTOINCREMENT,
          root_path TEXT NOT NULL UNIQUE,
          revision INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL DEFAULT (datetime('now')))",
    ),
    (
        "library_consoles",
        "(id INTEGER PRIMARY KEY AUTOINCREMENT,
          root_id INTEGER NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
          platform TEXT NOT NULL,
          folder_name TEXT NOT NULL,
          folder_path TEXT NOT NULL,
          fingerprint_hash TEXT NOT NULL,
          dat_game_count INTEGER NOT NULL DEFAULT 0,
          revision INTEGER NOT NULL DEFAULT 0,
          scan_generation INTEGER NOT NULL DEFAULT 0,
          scan_state TEXT NOT NULL DEFAULT 'unscanned'
              CHECK (scan_state IN ('unscanned', 'ready', 'stale')),
          UNIQUE(root_id, folder_name))",
    ),
    // The *_json columns hold serialized blobs where NULL = "never computed"
    // and a present value = "computed" (possibly an empty list). That
    // distinction is load-bearing in the GUI, so they stay nullable.
    (
        "library_entries",
        "(id INTEGER PRIMARY KEY AUTOINCREMENT,
          console_id INTEGER NOT NULL REFERENCES library_consoles(id) ON DELETE CASCADE,
          entry_key TEXT NOT NULL,
          display_name TEXT NOT NULL,
          game_entry_json TEXT NOT NULL,
          revision INTEGER NOT NULL DEFAULT 0,
          source_revision INTEGER NOT NULL DEFAULT 0,
          source_fingerprint TEXT NOT NULL DEFAULT '',
          status TEXT NOT NULL DEFAULT 'unknown',
          tag TEXT NOT NULL DEFAULT '',
          crc32 TEXT NOT NULL DEFAULT '',
          sha1 TEXT NOT NULL DEFAULT '',
          md5 TEXT NOT NULL DEFAULT '',
          data_size INTEGER NOT NULL DEFAULT 0,
          dat_game_name TEXT NOT NULL DEFAULT '',
          dat_rom_name TEXT NOT NULL DEFAULT '',
          dat_match_method TEXT NOT NULL DEFAULT '',
          region_override TEXT NOT NULL DEFAULT '',
          cover_title TEXT NOT NULL DEFAULT '',
          screen_title TEXT NOT NULL DEFAULT '',
          identification_json TEXT,
          disc_identifications_json TEXT,
          broken_references_json TEXT,
          ambiguous_candidates_json TEXT,
          cue_compat_issues_json TEXT,
          UNIQUE(console_id, entry_key))",
    ),
];

const INDEXES_SQL: &str = "
CREATE UNIQUE INDEX IF NOT EXISTS idx_releases_natural ON releases(work_id, platform_id, region, revision, variant);
CREATE INDEX IF NOT EXISTS idx_media_release ON media(release_id);
CREATE INDEX IF NOT EXISTS idx_media_crc32 ON media(crc32);
CREATE INDEX IF NOT EXISTS idx_media_sha1 ON media(sha1);
CREATE INDEX IF NOT EXISTS idx_media_serial ON media(media_serial);
CREATE INDEX IF NOT EXISTS idx_media_serial_normalized ON media(upper(replace(replace(media_serial, '-', ''), ' ', '')));
CREATE INDEX IF NOT EXISTS idx_release_serial_normalized ON releases(upper(replace(replace(game_serial, '-', ''), ' ', '')));
CREATE INDEX IF NOT EXISTS idx_media_dat_name ON media(dat_name);
CREATE INDEX IF NOT EXISTS idx_assets_release ON media_assets(release_id);
CREATE INDEX IF NOT EXISTS idx_assets_type_region ON media_assets(asset_type, region);
CREATE INDEX IF NOT EXISTS idx_disagreements_unresolved ON disagreements(resolved) WHERE resolved = 0;
CREATE INDEX IF NOT EXISTS idx_media_tracks_crc32 ON media_tracks(crc32);
CREATE INDEX IF NOT EXISTS idx_media_tracks_sha1 ON media_tracks(sha1);
CREATE INDEX IF NOT EXISTS idx_library_entries_console ON library_entries(console_id);
CREATE INDEX IF NOT EXISTS idx_library_entries_display ON library_entries(console_id, display_name COLLATE NOCASE, id);
";

/// Tables rebuilt by the v8 → v9 migration, with the SELECT expressions that
/// map the old (nullable) layout onto the canonical one in [`TABLES`].
const V9_REBUILDS: &[(&str, &str)] = &[
    (
        "platforms",
        "id, display_name, short_name, manufacturer, COALESCE(generation, 0), media_type,
         COALESCE(release_year, 0), COALESCE(description, ''), COALESCE(core_platform, '')",
    ),
    (
        "platform_regions",
        "platform_id, region, COALESCE(release_date, '')",
    ),
    ("companies", "id, name, COALESCE(country, '')"),
    (
        "releases",
        "id, work_id, platform_id, region, revision, variant, title, COALESCE(alt_title, ''),
         publisher_id, developer_id, COALESCE(release_date, ''), COALESCE(game_serial, ''),
         COALESCE(genre, ''), COALESCE(players, ''), rating, COALESCE(description, ''),
         COALESCE(screen_title, ''), COALESCE(cover_title, ''), screenscraper_id,
         scraper_not_found, created_at, updated_at",
    ),
    (
        "media",
        "id, release_id, COALESCE(media_serial, ''), COALESCE(disc_number, 0),
         COALESCE(disc_label, ''), COALESCE(revision, ''), status, tag,
         COALESCE(dat_name, ''), '', COALESCE(dat_source, ''), COALESCE(file_size, 0),
         COALESCE(crc32, ''), COALESCE(sha1, ''), COALESCE(md5, ''), created_at, updated_at",
    ),
    (
        "media_assets",
        "id, release_id, media_id, asset_type, COALESCE(region, ''), source,
         COALESCE(file_path, ''), COALESCE(source_url, ''), scraped,
         COALESCE(file_hash, ''), COALESCE(width, 0), COALESCE(height, 0), created_at",
    ),
    (
        "collection",
        "id, media_id, user_id, owned, COALESCE(condition, ''), COALESCE(notes, ''),
         COALESCE(date_acquired, ''), COALESCE(rom_path, ''), COALESCE(verified_at, '')",
    ),
    (
        "import_log",
        "id, source_type, source_name, COALESCE(source_version, ''), imported_at,
         COALESCE(records_created, 0), COALESCE(records_updated, 0),
         COALESCE(records_unchanged, 0), COALESCE(disagreements_found, 0)",
    ),
    (
        "disagreements",
        "id, entity_type, entity_id, field, source_a, COALESCE(value_a, ''), source_b,
         COALESCE(value_b, ''), resolved, COALESCE(resolution, ''),
         COALESCE(resolved_at, ''), created_at",
    ),
    (
        "overrides",
        "id, entity_type, COALESCE(entity_id, ''), COALESCE(platform_id, ''),
         COALESCE(dat_name_pattern, ''), field, override_value, reason, created_at",
    ),
    (
        "media_tracks",
        "id, media_id, track_number, track_name, COALESCE(file_size, 0),
         COALESCE(crc32, ''), COALESCE(sha1, ''), COALESCE(md5, '')",
    ),
];

/// Create all tables and indexes if they don't exist.
///
/// This is idempotent — safe to call on an existing database.
pub fn create_schema(conn: &Connection) -> Result<(), SchemaError> {
    for (name, body) in TABLES {
        conn.execute_batch(&format!("CREATE TABLE IF NOT EXISTS {name} {body};"))?;
    }
    conn.execute_batch(INDEXES_SQL)?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_media_serial_key ON media_serial_keys(serial_key);",
    )?;
    set_schema_version(conn, CURRENT_VERSION)?;
    Ok(())
}

/// Open or create a catalog database at the given path.
pub fn open_database(path: &std::path::Path) -> Result<Connection, SchemaError> {
    let conn = Connection::open(path)?;
    configure_connection(&conn, true)?;

    let version = get_schema_version(&conn)?;
    if version == 0 {
        create_schema(&conn)?;
    } else if version < CURRENT_VERSION {
        migrate(&conn, version)?;
    }

    Ok(conn)
}

/// Open an in-memory database with the full schema. Useful for testing.
pub fn open_memory() -> Result<Connection, SchemaError> {
    let conn = Connection::open_in_memory()?;
    configure_connection(&conn, false)?;
    create_schema(&conn)?;
    Ok(conn)
}

/// Configure a connection used by either the catalog or library APIs.
pub fn configure_connection(conn: &Connection, wal: bool) -> Result<(), SchemaError> {
    if wal {
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    }
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    Ok(())
}

/// Get the current schema version, or 0 if no schema exists.
fn get_schema_version(conn: &Connection) -> Result<i32, SchemaError> {
    // Check if schema_version table exists
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_version')",
        [],
        |row| row.get(0),
    )?;

    if !exists {
        return Ok(0);
    }

    let version: i32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

/// Record a schema version.
fn set_schema_version(conn: &Connection, version: i32) -> Result<(), SchemaError> {
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        [version],
    )?;
    Ok(())
}

/// The canonical column body for a table, from [`TABLES`].
fn table_body(name: &'static str) -> Result<&'static str, SchemaError> {
    TABLES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, body)| *body)
        .ok_or(SchemaError::UnknownTable(name))
}

/// Rebuild `name` to its canonical [`TABLES`] definition, converting existing
/// rows via `select_exprs` (`SQLite` has no `ALTER COLUMN`, so constraint
/// changes require the create-copy-drop-rename dance).
///
/// Caller must have `PRAGMA foreign_keys=OFF` so the rename doesn't rewrite
/// other tables' foreign-key references.
fn rebuild_table(
    conn: &Connection,
    name: &'static str,
    select_exprs: &str,
) -> Result<(), SchemaError> {
    let body = table_body(name)?;
    // INSERT OR IGNORE: under the old schema, UNIQUE keys containing NULL
    // columns never conflicted (SQLite treats NULLs as distinct), so legacy
    // databases can hold duplicate rows — e.g. overrides re-inserted by every
    // catalog import. The rebuild keeps the first copy and drops duplicates.
    conn.execute_batch(&format!(
        "CREATE TABLE {name}_new {body};
         INSERT OR IGNORE INTO {name}_new SELECT {select_exprs} FROM {name};
         DROP TABLE {name};
         ALTER TABLE {name}_new RENAME TO {name};"
    ))?;
    Ok(())
}

/// Run migrations from `from_version` up to `CURRENT_VERSION`.
fn migrate(conn: &Connection, from_version: i32) -> Result<(), SchemaError> {
    if from_version > CURRENT_VERSION {
        return Err(SchemaError::VersionMismatch {
            expected: CURRENT_VERSION,
            found: from_version,
        });
    }

    let mut version = from_version;
    while version < CURRENT_VERSION {
        match version {
            1 => {
                conn.execute_batch(
                    "ALTER TABLE releases ADD COLUMN scraper_not_found BOOLEAN NOT NULL DEFAULT 0;",
                )?;
            }
            2 => {
                conn.execute_batch(
                    "ALTER TABLE releases ADD COLUMN revision TEXT NOT NULL DEFAULT '';
                     ALTER TABLE releases ADD COLUMN variant TEXT NOT NULL DEFAULT '';
                     DROP INDEX IF EXISTS idx_releases_natural;
                     CREATE UNIQUE INDEX idx_releases_natural
                         ON releases(work_id, platform_id, region, revision, variant);",
                )?;
            }
            3 => {
                conn.execute_batch(
                    "ALTER TABLE releases ADD COLUMN screen_title TEXT;
                     ALTER TABLE releases ADD COLUMN cover_title TEXT;",
                )?;
            }
            4 => {
                conn.execute_batch(
                    "ALTER TABLE works ADD COLUMN tag TEXT;
                     ALTER TABLE media ADD COLUMN tag TEXT;",
                )?;
            }
            5 => {
                // Historical: created the library tables. Creating them at the
                // canonical layout is fine — the v9 rebuild is a no-op for them.
                for name in ["library_roots", "library_consoles", "library_entries"] {
                    let body = table_body(name)?;
                    conn.execute_batch(&format!("CREATE TABLE IF NOT EXISTS {name} {body};"))?;
                }
            }
            6 => {
                let body = table_body("media_tracks")?;
                conn.execute_batch(&format!("CREATE TABLE IF NOT EXISTS media_tracks {body};"))?;
            }
            7 => {
                // Column may already exist if tables were created fresh by migration 5
                let has_column: bool = conn
                    .prepare("SELECT cue_compat_issues_json FROM library_entries LIMIT 0")
                    .is_ok();
                if !has_column {
                    conn.execute_batch(
                        "ALTER TABLE library_entries ADD COLUMN cue_compat_issues_json TEXT;",
                    )?;
                }
            }
            8 => {
                // v9: NULL-able "maybe" columns become NOT NULL with ''/0
                // defaults, and the overrides natural key widens to include
                // its pattern selectors (NULL entity_ids were previously
                // distinct under UNIQUE; empty strings are not).
                conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
                let result = (|| -> Result<(), SchemaError> {
                    conn.execute_batch("BEGIN;")?;
                    for (name, select_exprs) in V9_REBUILDS {
                        rebuild_table(conn, name, select_exprs)?;
                    }
                    conn.execute_batch(INDEXES_SQL)?;
                    conn.execute_batch("COMMIT;")?;
                    Ok(())
                })();
                if result.is_err() {
                    let _ = conn.execute_batch("ROLLBACK;");
                }
                conn.execute_batch("PRAGMA foreign_keys=ON;")?;
                result?;
                conn.execute_batch("PRAGMA foreign_key_check;")?;
            }
            9 => crate::library::migrate_library_v10(conn)?,
            10 => crate::library::migrate_library_v11(conn)?,
            11 => {
                let has_media: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='media')",
                    [],
                    |row| row.get(0),
                )?;
                if has_media && conn.prepare("SELECT rom_name FROM media LIMIT 0").is_err() {
                    conn.execute_batch(
                        "ALTER TABLE media ADD COLUMN rom_name TEXT NOT NULL DEFAULT '';",
                    )?;
                }
                if has_media {
                    conn.execute_batch(
                        "UPDATE media SET crc32=lower(crc32),sha1=lower(sha1),md5=lower(md5);",
                    )?;
                }
            }
            12 => {
                let has_catalog: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='media')",
                    [],
                    |row| row.get(0),
                )?;
                if has_catalog {
                    conn.execute_batch(INDEXES_SQL)?;
                }
            }
            13 => {
                let has_media: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='media')",
                    [],
                    |row| row.get(0),
                )?;
                if !has_media {
                    version += 1;
                    set_schema_version(conn, version)?;
                    continue;
                }
                let body = table_body("media_serial_keys")?;
                conn.execute_batch(&format!(
                    "CREATE TABLE IF NOT EXISTS media_serial_keys {body};
                     CREATE INDEX IF NOT EXISTS idx_media_serial_key ON media_serial_keys(serial_key);"
                ))?;
                let rows: Vec<(String, String)> = conn
                    .prepare("SELECT id,media_serial FROM media WHERE media_serial<>''")?
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<_, _>>()?;
                for (media_id, serial) in rows {
                    for key in serial_keys(&serial) {
                        conn.execute(
                            "INSERT OR IGNORE INTO media_serial_keys(media_id,serial_key) VALUES(?1,?2)",
                            rusqlite::params![media_id, key],
                        )?;
                    }
                }
                let has_library: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='library_consoles')",
                    [],
                    |row| row.get(0),
                )?;
                if has_library {
                    conn.execute_batch(
                        "UPDATE library_consoles SET scan_state='stale'
                         WHERE id IN (
                           SELECT DISTINCT console_id FROM library_entries
                           WHERE status IN ('unknown','ambiguous','unrecognized')
                         );",
                    )?;
                }
            }
            _ => {}
        }
        version += 1;
        set_schema_version(conn, version)?;
    }

    Ok(())
}

pub(crate) fn serial_keys(serials: &str) -> Vec<String> {
    let mut keys = std::collections::BTreeSet::new();
    for serial in serials.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        keys.insert(serial.to_ascii_uppercase().replace([' ', '-'], ""));
        for segment in serial.split('-') {
            let segment = segment.trim();
            if segment.len() == 4 && segment.chars().all(|c| c.is_ascii_alphanumeric()) {
                keys.insert(segment.to_ascii_uppercase());
            }
        }
    }
    keys.into_iter().filter(|key| !key.is_empty()).collect()
}
