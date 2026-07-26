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
pub const CURRENT_VERSION: i32 = 21;

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
          hash_warnings_json TEXT,
          disc_verification TEXT NOT NULL DEFAULT 'not_applicable',
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
    (
        "archive_profiles",
        "(id TEXT PRIMARY KEY,
          display_name TEXT NOT NULL,
          manifest_path TEXT NOT NULL,
          manifest_sha256 TEXT NOT NULL,
          archive_root TEXT NOT NULL,
          playable_root TEXT NOT NULL DEFAULT '',
          workspace_root TEXT NOT NULL DEFAULT '',
          indexed_at TEXT NOT NULL DEFAULT (datetime('now')))",
    ),
    (
        "archive_releases",
        "(id TEXT PRIMARY KEY,
          profile_id TEXT NOT NULL REFERENCES archive_profiles(id) ON DELETE CASCADE,
          catalog_work_id TEXT REFERENCES works(id) ON DELETE SET NULL,
          catalog_release_id TEXT REFERENCES releases(id) ON DELETE SET NULL,
          platform_id TEXT NOT NULL,
          title TEXT NOT NULL,
          region TEXT NOT NULL DEFAULT '',
          revision TEXT NOT NULL DEFAULT '',
          variant TEXT NOT NULL DEFAULT '',
          manifest_path TEXT NOT NULL,
          manifest_sha256 TEXT NOT NULL,
          binding_state TEXT NOT NULL DEFAULT 'unresolved',
          UNIQUE(profile_id, manifest_path))",
    ),
    (
        "physical_copies",
        "(id TEXT PRIMARY KEY,
          archive_release_id TEXT NOT NULL REFERENCES archive_releases(id) ON DELETE CASCADE,
          copy_number INTEGER NOT NULL,
          owner_id TEXT NOT NULL DEFAULT 'default',
          label TEXT NOT NULL DEFAULT '',
          condition TEXT NOT NULL DEFAULT '',
          notes TEXT NOT NULL DEFAULT '',
          date_acquired TEXT NOT NULL DEFAULT '',
          provenance TEXT NOT NULL DEFAULT '',
          manifest_path TEXT NOT NULL,
          manifest_sha256 TEXT NOT NULL)",
    ),
    (
        "carriers",
        "(id TEXT PRIMARY KEY,
          physical_copy_id TEXT NOT NULL REFERENCES physical_copies(id) ON DELETE CASCADE,
          catalog_media_id TEXT REFERENCES media(id) ON DELETE SET NULL,
          kind TEXT NOT NULL DEFAULT 'unknown',
          serial TEXT NOT NULL DEFAULT '',
          sequence_number INTEGER NOT NULL DEFAULT 0,
          label TEXT NOT NULL DEFAULT '',
          manifest_path TEXT NOT NULL,
          manifest_sha256 TEXT NOT NULL,
          binding_state TEXT NOT NULL DEFAULT 'unresolved')",
    ),
    (
        "dump_events",
        "(id TEXT PRIMARY KEY,
          carrier_id TEXT NOT NULL REFERENCES carriers(id) ON DELETE CASCADE,
          representation_id TEXT NOT NULL UNIQUE,
          format TEXT NOT NULL,
          captured_at TEXT NOT NULL,
          manifest_path TEXT NOT NULL,
          manifest_sha256 TEXT NOT NULL,
          integrity_state TEXT NOT NULL DEFAULT 'unknown',
          catalog_state TEXT NOT NULL DEFAULT 'not_attempted')",
    ),
    (
        "representations",
        "(id TEXT PRIMARY KEY,
          carrier_id TEXT NOT NULL REFERENCES carriers(id) ON DELETE CASCADE,
          dump_id TEXT REFERENCES dump_events(id) ON DELETE SET NULL,
          role TEXT NOT NULL,
          format TEXT NOT NULL,
          location_role TEXT NOT NULL,
          relative_path TEXT NOT NULL,
          presence_state TEXT NOT NULL DEFAULT 'not_present',
          input_manifest_sha256 TEXT NOT NULL DEFAULT '',
          content_sha256 TEXT NOT NULL DEFAULT '',
          content_size INTEGER NOT NULL DEFAULT 0,
          catalog_verified BOOLEAN NOT NULL DEFAULT 0,
          round_trip_verified BOOLEAN NOT NULL DEFAULT 0,
          recipe_version INTEGER NOT NULL DEFAULT 0,
          UNIQUE(location_role, relative_path))",
    ),
    (
        "representation_files",
        "(representation_id TEXT NOT NULL REFERENCES representations(id) ON DELETE CASCADE,
          relative_path TEXT NOT NULL,
          file_size INTEGER NOT NULL,
          sha256 TEXT NOT NULL,
          PRIMARY KEY(representation_id, relative_path))",
    ),
    (
        "verification_events",
        "(id TEXT PRIMARY KEY,
          representation_id TEXT NOT NULL REFERENCES representations(id) ON DELETE CASCADE,
          kind TEXT NOT NULL,
          outcome TEXT NOT NULL,
          performed_at TEXT NOT NULL,
          input_manifest_sha256 TEXT NOT NULL DEFAULT '',
          evidence_path TEXT NOT NULL,
          catalog_source TEXT NOT NULL DEFAULT '',
          catalog_version TEXT NOT NULL DEFAULT '',
          catalog_game TEXT NOT NULL DEFAULT '',
          complete_track_set BOOLEAN NOT NULL DEFAULT 0,
          detail TEXT NOT NULL DEFAULT '')",
    ),
    (
        "derivations",
        "(id TEXT PRIMARY KEY,
          parent_representation_id TEXT NOT NULL REFERENCES representations(id) ON DELETE CASCADE,
          child_representation_id TEXT NOT NULL REFERENCES representations(id) ON DELETE CASCADE,
          recipe_version INTEGER NOT NULL,
          evidence_path TEXT NOT NULL,
          created_at TEXT NOT NULL)",
    ),
    (
        "playable_policies",
        "(scope_type TEXT NOT NULL,
          scope_id TEXT NOT NULL,
          format TEXT NOT NULL,
          retain_intermediate BOOLEAN NOT NULL DEFAULT 0,
          allow_unverified BOOLEAN NOT NULL DEFAULT 0,
          options_json TEXT NOT NULL DEFAULT '{}',
          PRIMARY KEY(scope_type, scope_id))",
    ),
    (
        "library_entry_media_bindings",
        "(library_entry_id INTEGER NOT NULL REFERENCES library_entries(id) ON DELETE CASCADE,
          catalog_media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
          representation_id TEXT REFERENCES representations(id) ON DELETE SET NULL,
          match_method TEXT NOT NULL DEFAULT '',
          PRIMARY KEY(library_entry_id, catalog_media_id))",
    ),
    (
        "catalog_source_snapshots",
        "(id INTEGER PRIMARY KEY AUTOINCREMENT,
          source TEXT NOT NULL,
          system TEXT NOT NULL,
          version TEXT NOT NULL,
          imported_at TEXT NOT NULL,
          content_sha256 TEXT NOT NULL DEFAULT '',
          UNIQUE(source, system, version, content_sha256))",
    ),
    (
        "physical_copy_files",
        "(id TEXT PRIMARY KEY,
          physical_copy_id TEXT NOT NULL REFERENCES physical_copies(id) ON DELETE CASCADE,
          category TEXT NOT NULL,
          asset_type TEXT NOT NULL,
          relative_path TEXT NOT NULL,
          sha256 TEXT NOT NULL DEFAULT '',
          caption TEXT NOT NULL DEFAULT '',
          source TEXT NOT NULL DEFAULT '',
          UNIQUE(physical_copy_id, relative_path))",
    ),
    (
        "archive_release_files",
        "(id TEXT PRIMARY KEY,
          archive_release_id TEXT NOT NULL REFERENCES archive_releases(id) ON DELETE CASCADE,
          category TEXT NOT NULL,
          asset_type TEXT NOT NULL,
          relative_path TEXT NOT NULL,
          file_size INTEGER NOT NULL,
          sha256 TEXT NOT NULL,
          source TEXT NOT NULL DEFAULT '',
          source_url TEXT NOT NULL DEFAULT '',
          caption TEXT NOT NULL DEFAULT '',
          captured_at TEXT NOT NULL,
          manifest_path TEXT NOT NULL,
          manifest_sha256 TEXT NOT NULL)",
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

const ARCHIVE_INDEXES_SQL: &str = "
CREATE INDEX IF NOT EXISTS idx_archive_releases_profile ON archive_releases(profile_id, platform_id, title);
CREATE INDEX IF NOT EXISTS idx_archive_releases_work ON archive_releases(catalog_work_id, platform_id, region);
CREATE INDEX IF NOT EXISTS idx_physical_copies_release ON physical_copies(archive_release_id);
CREATE INDEX IF NOT EXISTS idx_carriers_copy ON carriers(physical_copy_id);
CREATE INDEX IF NOT EXISTS idx_dump_events_media ON dump_events(carrier_id);
CREATE INDEX IF NOT EXISTS idx_representations_media ON representations(carrier_id, role);
CREATE INDEX IF NOT EXISTS idx_verifications_representation ON verification_events(representation_id, performed_at);
CREATE INDEX IF NOT EXISTS idx_library_binding_media ON library_entry_media_bindings(catalog_media_id);
CREATE INDEX IF NOT EXISTS idx_archive_release_files_release ON archive_release_files(archive_release_id, category, asset_type);
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
    conn.execute_batch(ARCHIVE_INDEXES_SQL)?;
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

/// Return whether an existing database needs schema creation or migration.
///
/// This read-only probe lets GUI callers decide whether database preparation
/// must block interaction without accidentally performing the migration as
/// part of the probe. A missing database is considered new, not migrated.
pub fn database_needs_migration(path: &std::path::Path) -> Result<bool, SchemaError> {
    if !path.is_file() {
        return Ok(false);
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let version = get_schema_version(&conn)?;
    Ok(version != CURRENT_VERSION)
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
    // Writers are serialized in-process, but archive/catalog maintenance may
    // use another connection. Wait for that transaction instead of discarding
    // completed background work after an arbitrary five seconds.
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=60000;")?;
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
#[allow(clippy::too_many_lines)]
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
            15 => {
                let has_library: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='library_entries')",
                    [],
                    |row| row.get(0),
                )?;
                if has_library {
                    if conn
                        .prepare("SELECT hash_warnings_json FROM library_entries LIMIT 0")
                        .is_err()
                    {
                        conn.execute_batch(
                            "ALTER TABLE library_entries ADD COLUMN hash_warnings_json TEXT;",
                        )?;
                    }
                    if conn
                        .prepare("SELECT disc_verification FROM library_entries LIMIT 0")
                        .is_err()
                    {
                        conn.execute_batch(
                            "ALTER TABLE library_entries ADD COLUMN disc_verification TEXT NOT NULL DEFAULT 'not_applicable';",
                        )?;
                    }
                    // Older releases treated a matching data track as a
                    // verified CUE/BIN disc. Those cached hashes cannot prove
                    // that every Redump track existed, so discard only their
                    // derived verdicts and let the normal stale-console scan
                    // rebuild identification before an explicit rehash.
                    conn.execute_batch(
                        "UPDATE library_entries
                         SET status='unknown',crc32='',sha1='',md5='',data_size=0,
                             hash_warnings_json=NULL,disc_verification='not_applicable',
                             dat_game_name='',dat_rom_name='',dat_match_method='',
                             disc_identifications_json=NULL,ambiguous_candidates_json=NULL,
                             revision=revision+1
                         WHERE instr(lower(game_entry_json),'.cue')>0;
                         UPDATE library_consoles
                         SET scan_state='stale',revision=revision+1
                         WHERE id IN (
                           SELECT DISTINCT console_id FROM library_entries
                           WHERE instr(lower(game_entry_json),'.cue')>0
                         );",
                    )?;
                }
            }
            16 => {
                for name in [
                    "archive_profiles",
                    "archive_releases",
                    "physical_copies",
                    "carriers",
                    "dump_events",
                    "representations",
                    "representation_files",
                    "verification_events",
                    "derivations",
                    "playable_policies",
                    "library_entry_media_bindings",
                    "catalog_source_snapshots",
                    "physical_copy_files",
                    "archive_release_files",
                ] {
                    let body = table_body(name)?;
                    conn.execute_batch(&format!("CREATE TABLE IF NOT EXISTS {name} {body};"))?;
                }
                conn.execute_batch(ARCHIVE_INDEXES_SQL)?;
            }
            17 => {
                let has_representations: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='representations')",
                    [],
                    |row| row.get(0),
                )?;
                if has_representations {
                    if conn
                        .prepare("SELECT catalog_verified FROM representations LIMIT 0")
                        .is_err()
                    {
                        conn.execute_batch(
                            "ALTER TABLE representations ADD COLUMN catalog_verified BOOLEAN NOT NULL DEFAULT 0;",
                        )?;
                    }
                    if conn
                        .prepare("SELECT round_trip_verified FROM representations LIMIT 0")
                        .is_err()
                    {
                        conn.execute_batch(
                            "ALTER TABLE representations ADD COLUMN round_trip_verified BOOLEAN NOT NULL DEFAULT 0;",
                        )?;
                    }
                }
                let has_verifications: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='verification_events')",
                    [],
                    |row| row.get(0),
                )?;
                if has_verifications
                    && conn
                        .prepare("SELECT input_manifest_sha256 FROM verification_events LIMIT 0")
                        .is_err()
                {
                    conn.execute_batch(
                        "ALTER TABLE verification_events ADD COLUMN input_manifest_sha256 TEXT NOT NULL DEFAULT '';",
                    )?;
                }
            }
            18 => {
                // The 0.4 archive prototype was explicitly superseded before release.
                // Its SQLite state is only a rebuildable projection, so replace it
                // without touching catalog or playable-library rows.
                conn.execute_batch("PRAGMA foreign_keys=OFF; BEGIN;")?;
                let result = (|| -> Result<(), SchemaError> {
                    conn.execute_batch(
                        "DROP TABLE IF EXISTS library_entry_media_bindings;
                         DROP TABLE IF EXISTS playable_policies;
                         DROP TABLE IF EXISTS derivations;
                         DROP TABLE IF EXISTS verification_events;
                         DROP TABLE IF EXISTS representation_files;
                         DROP TABLE IF EXISTS representations;
                         DROP TABLE IF EXISTS dump_events;
                         DROP TABLE IF EXISTS physical_copy_files;
                         DROP TABLE IF EXISTS archive_release_files;
                         DROP TABLE IF EXISTS carriers;
                         DROP TABLE IF EXISTS physical_copies;
                         DROP TABLE IF EXISTS collection_item_assets;
                         DROP TABLE IF EXISTS archive_release_assets;
                         DROP TABLE IF EXISTS collection_item_media;
                         DROP TABLE IF EXISTS collection_items;
                         DROP TABLE IF EXISTS archive_releases;
                         DROP TABLE IF EXISTS archive_profiles;",
                    )?;
                    for name in [
                        "archive_profiles",
                        "archive_releases",
                        "physical_copies",
                        "carriers",
                        "dump_events",
                        "representations",
                        "representation_files",
                        "verification_events",
                        "derivations",
                        "playable_policies",
                        "library_entry_media_bindings",
                        "physical_copy_files",
                        "archive_release_files",
                    ] {
                        let body = table_body(name)?;
                        conn.execute_batch(&format!("CREATE TABLE {name} {body};"))?;
                    }
                    conn.execute_batch(ARCHIVE_INDEXES_SQL)?;
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
            19 => {
                let has_archive_releases: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='archive_releases')",
                    [],
                    |row| row.get(0),
                )?;
                if has_archive_releases
                    && conn
                        .prepare("SELECT catalog_work_id FROM archive_releases LIMIT 0")
                        .is_err()
                {
                    conn.execute_batch(
                        "ALTER TABLE archive_releases
                         ADD COLUMN catalog_work_id TEXT REFERENCES works(id) ON DELETE SET NULL;
                         CREATE INDEX IF NOT EXISTS idx_archive_releases_work
                         ON archive_releases(catalog_work_id, platform_id, region);",
                    )?;
                }
            }
            20 => {
                // Losslessly collapse only media rows with complete, identical
                // release identity and byte/track evidence.
                let has_catalog: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='media')",
                    [],
                    |row| row.get(0),
                )?;
                if has_catalog {
                    crate::deduplicate::deduplicate_catalog(conn, None)?;
                }
            }
            14 => {
                let has_match_state: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='library_entries')
                         AND EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='library_consoles')
                         AND EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='media')
                         AND EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='releases')",
                    [],
                    |row| row.get(0),
                )?;
                if has_match_state {
                    // Matching policy now has one authoritative verdict. Only
                    // invalidate consoles whose cached status is unresolved or
                    // disagrees with the catalog's definitive hash evidence.
                    conn.execute_batch(
                        "UPDATE library_consoles AS c
                         SET scan_state='stale', revision=revision+1
                         WHERE c.scan_state='ready' AND EXISTS (
                           SELECT 1 FROM library_entries e
                           WHERE e.console_id=c.id AND (
                             e.status IN ('unknown','ambiguous','unrecognized') OR
                             (e.status='matched' AND NOT EXISTS (
                               SELECT 1 FROM media m JOIN releases r ON r.id=m.release_id
                               WHERE r.platform_id=c.platform AND (
                                 (e.crc32<>'' AND m.crc32=lower(e.crc32) AND m.file_size=e.data_size) OR
                                 (e.sha1<>'' AND m.sha1=lower(e.sha1))
                               )
                             )) OR
                             (e.status='likely' AND EXISTS (
                               SELECT 1 FROM media m JOIN releases r ON r.id=m.release_id
                               WHERE r.platform_id=c.platform AND (
                                 (e.crc32<>'' AND m.crc32=lower(e.crc32) AND m.file_size=e.data_size) OR
                                 (e.sha1<>'' AND m.sha1=lower(e.sha1))
                               )
                             ))
                           )
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
