//! SQLite schema creation and migration.

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Migration error: expected version {expected}, found {found}")]
    VersionMismatch { expected: i32, found: i32 },
}

/// Current schema version. Increment when adding migrations.
pub const CURRENT_VERSION: i32 = 2;

/// Create all tables and indexes if they don't exist.
///
/// This is idempotent — safe to call on an existing database.
pub fn create_schema(conn: &Connection) -> Result<(), SchemaError> {
    conn.execute_batch(SCHEMA_SQL)?;
    set_schema_version(conn, CURRENT_VERSION)?;
    Ok(())
}

/// Open or create a catalog database at the given path.
pub fn open_database(path: &std::path::Path) -> Result<Connection, SchemaError> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

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
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    create_schema(&conn)?;
    Ok(conn)
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
            _ => {}
        }
        version += 1;
        set_schema_version(conn, version)?;
    }

    Ok(())
}

const SCHEMA_SQL: &str = r#"
-- Schema version tracking
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Platforms/Consoles
CREATE TABLE IF NOT EXISTS platforms (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    short_name TEXT NOT NULL,
    manufacturer TEXT NOT NULL,
    generation INTEGER,
    media_type TEXT NOT NULL,
    release_year INTEGER,
    description TEXT,
    core_platform TEXT
);

CREATE TABLE IF NOT EXISTS platform_regions (
    platform_id TEXT NOT NULL REFERENCES platforms(id),
    region TEXT NOT NULL,
    release_date TEXT,
    PRIMARY KEY (platform_id, region)
);

CREATE TABLE IF NOT EXISTS platform_relationships (
    platform_a TEXT NOT NULL REFERENCES platforms(id),
    platform_b TEXT NOT NULL REFERENCES platforms(id),
    relationship TEXT NOT NULL,
    PRIMARY KEY (platform_a, platform_b, relationship)
);

-- Companies (publishers, developers)
CREATE TABLE IF NOT EXISTS companies (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    country TEXT
);

CREATE TABLE IF NOT EXISTS company_aliases (
    company_id TEXT NOT NULL REFERENCES companies(id),
    alias TEXT NOT NULL,
    PRIMARY KEY (company_id, alias)
);

-- Abstract game concept
CREATE TABLE IF NOT EXISTS works (
    id TEXT PRIMARY KEY,
    canonical_name TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Relationships between works
CREATE TABLE IF NOT EXISTS work_relationships (
    work_a TEXT NOT NULL REFERENCES works(id),
    work_b TEXT NOT NULL REFERENCES works(id),
    relationship TEXT NOT NULL,
    PRIMARY KEY (work_a, work_b, relationship)
);

-- Regional release of a work on a platform
CREATE TABLE IF NOT EXISTS releases (
    id TEXT PRIMARY KEY,
    work_id TEXT NOT NULL REFERENCES works(id),
    platform_id TEXT NOT NULL REFERENCES platforms(id),
    region TEXT NOT NULL,
    title TEXT NOT NULL,
    alt_title TEXT,
    publisher_id TEXT REFERENCES companies(id),
    developer_id TEXT REFERENCES companies(id),
    release_date TEXT,
    game_serial TEXT,
    genre TEXT,
    players TEXT,
    rating REAL,
    description TEXT,
    screenscraper_id TEXT,
    scraper_not_found BOOLEAN NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_releases_natural ON releases(work_id, platform_id, region);

-- Physical/digital media artifact
CREATE TABLE IF NOT EXISTS media (
    id TEXT PRIMARY KEY,
    release_id TEXT NOT NULL REFERENCES releases(id),
    media_serial TEXT,
    disc_number INTEGER,
    disc_label TEXT,
    revision TEXT,
    status TEXT NOT NULL DEFAULT 'verified',
    dat_name TEXT,
    dat_source TEXT,
    file_size INTEGER,
    crc32 TEXT,
    sha1 TEXT,
    md5 TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_media_release ON media(release_id);
CREATE INDEX IF NOT EXISTS idx_media_crc32 ON media(crc32);
CREATE INDEX IF NOT EXISTS idx_media_sha1 ON media(sha1);
CREATE INDEX IF NOT EXISTS idx_media_serial ON media(media_serial);
CREATE INDEX IF NOT EXISTS idx_media_dat_name ON media(dat_name);

-- Art, screenshots, scans, etc.
CREATE TABLE IF NOT EXISTS media_assets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    release_id TEXT REFERENCES releases(id),
    media_id TEXT REFERENCES media(id),
    asset_type TEXT NOT NULL,
    region TEXT,
    source TEXT NOT NULL,
    file_path TEXT,
    source_url TEXT,
    scraped BOOLEAN NOT NULL DEFAULT 0,
    file_hash TEXT,
    width INTEGER,
    height INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_assets_release ON media_assets(release_id);
CREATE INDEX IF NOT EXISTS idx_assets_type_region ON media_assets(asset_type, region);

-- Collection / ownership
CREATE TABLE IF NOT EXISTS collection (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    media_id TEXT NOT NULL REFERENCES media(id),
    user_id TEXT NOT NULL DEFAULT 'default',
    owned BOOLEAN NOT NULL DEFAULT 1,
    condition TEXT,
    notes TEXT,
    date_acquired TEXT,
    rom_path TEXT,
    verified_at TEXT,
    UNIQUE(media_id, user_id)
);

-- Import tracking
CREATE TABLE IF NOT EXISTS import_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_type TEXT NOT NULL,
    source_name TEXT NOT NULL,
    source_version TEXT,
    imported_at TEXT NOT NULL,
    records_created INTEGER DEFAULT 0,
    records_updated INTEGER DEFAULT 0,
    records_unchanged INTEGER DEFAULT 0,
    disagreements_found INTEGER DEFAULT 0
);

-- Disagreements between data sources
CREATE TABLE IF NOT EXISTS disagreements (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    field TEXT NOT NULL,
    source_a TEXT NOT NULL,
    value_a TEXT,
    source_b TEXT NOT NULL,
    value_b TEXT,
    resolved BOOLEAN NOT NULL DEFAULT 0,
    resolution TEXT,
    resolved_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_disagreements_unresolved ON disagreements(resolved) WHERE resolved = 0;

-- Overrides for known data corrections
CREATE TABLE IF NOT EXISTS overrides (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_type TEXT NOT NULL,
    entity_id TEXT,
    platform_id TEXT,
    dat_name_pattern TEXT,
    field TEXT NOT NULL,
    override_value TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(entity_type, entity_id, field)
);
"#;