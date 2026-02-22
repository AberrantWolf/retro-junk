//! Read queries for the catalog database.
//!
//! Provides lookup by hash, serial, platform, search, and listing.

use retro_junk_catalog::types::*;
use rusqlite::{params, Connection};

use crate::operations::OperationError;

// ── Media Lookups ───────────────────────────────────────────────────────────

/// Find media entries by CRC32 hash.
pub fn find_media_by_crc32(
    conn: &Connection,
    crc32: &str,
) -> Result<Vec<Media>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, release_id, media_serial, disc_number, disc_label,
                revision, status, dat_name, dat_source, file_size,
                crc32, sha1, md5, created_at, updated_at
         FROM media WHERE crc32 = ?1",
    )?;
    let rows = stmt.query_map(params![crc32], row_to_media)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Find media entries by SHA1 hash.
pub fn find_media_by_sha1(
    conn: &Connection,
    sha1: &str,
) -> Result<Vec<Media>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, release_id, media_serial, disc_number, disc_label,
                revision, status, dat_name, dat_source, file_size,
                crc32, sha1, md5, created_at, updated_at
         FROM media WHERE sha1 = ?1",
    )?;
    let rows = stmt.query_map(params![sha1], row_to_media)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Find media entries by serial number.
pub fn find_media_by_serial(
    conn: &Connection,
    serial: &str,
) -> Result<Vec<Media>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, release_id, media_serial, disc_number, disc_label,
                revision, status, dat_name, dat_source, file_size,
                crc32, sha1, md5, created_at, updated_at
         FROM media WHERE media_serial = ?1",
    )?;
    let rows = stmt.query_map(params![serial], row_to_media)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

// ── Release Lookups ─────────────────────────────────────────────────────────

/// List all releases for a platform.
pub fn releases_for_platform(
    conn: &Connection,
    platform_id: &str,
) -> Result<Vec<Release>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, work_id, platform_id, region, title, alt_title,
                publisher_id, developer_id, release_date, game_serial,
                genre, players, rating, description, screenscraper_id,
                created_at, updated_at
         FROM releases WHERE platform_id = ?1 ORDER BY title",
    )?;
    let rows = stmt.query_map(params![platform_id], row_to_release)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Search releases by title (case-insensitive LIKE).
pub fn search_releases(
    conn: &Connection,
    query: &str,
) -> Result<Vec<Release>, OperationError> {
    let pattern = format!("%{}%", query);
    let mut stmt = conn.prepare(
        "SELECT id, work_id, platform_id, region, title, alt_title,
                publisher_id, developer_id, release_date, game_serial,
                genre, players, rating, description, screenscraper_id,
                created_at, updated_at
         FROM releases WHERE title LIKE ?1 ORDER BY title LIMIT 100",
    )?;
    let rows = stmt.query_map(params![pattern], row_to_release)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Find a release by game serial.
pub fn find_release_by_serial(
    conn: &Connection,
    serial: &str,
) -> Result<Vec<Release>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, work_id, platform_id, region, title, alt_title,
                publisher_id, developer_id, release_date, game_serial,
                genre, players, rating, description, screenscraper_id,
                created_at, updated_at
         FROM releases WHERE game_serial = ?1",
    )?;
    let rows = stmt.query_map(params![serial], row_to_release)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

// ── Platform Queries ────────────────────────────────────────────────────────

/// List all platforms.
pub fn list_platforms(conn: &Connection) -> Result<Vec<PlatformRow>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, display_name, short_name, manufacturer, generation,
                media_type, release_year, core_platform
         FROM platforms ORDER BY manufacturer, release_year",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(PlatformRow {
            id: row.get(0)?,
            display_name: row.get(1)?,
            short_name: row.get(2)?,
            manufacturer: row.get(3)?,
            generation: row.get(4)?,
            media_type: row.get(5)?,
            release_year: row.get(6)?,
            core_platform: row.get(7)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// A platform row from a query (not the full YAML-loaded type).
#[derive(Debug)]
pub struct PlatformRow {
    pub id: String,
    pub display_name: String,
    pub short_name: String,
    pub manufacturer: String,
    pub generation: Option<u32>,
    pub media_type: String,
    pub release_year: Option<u32>,
    pub core_platform: Option<String>,
}

// ── Statistics ──────────────────────────────────────────────────────────────

/// Get overall catalog statistics.
pub fn catalog_stats(conn: &Connection) -> Result<CatalogStats, OperationError> {
    let platforms: i64 = conn.query_row("SELECT COUNT(*) FROM platforms", [], |r| r.get(0))?;
    let companies: i64 = conn.query_row("SELECT COUNT(*) FROM companies", [], |r| r.get(0))?;
    let works: i64 = conn.query_row("SELECT COUNT(*) FROM works", [], |r| r.get(0))?;
    let releases: i64 = conn.query_row("SELECT COUNT(*) FROM releases", [], |r| r.get(0))?;
    let media: i64 = conn.query_row("SELECT COUNT(*) FROM media", [], |r| r.get(0))?;
    let assets: i64 = conn.query_row("SELECT COUNT(*) FROM media_assets", [], |r| r.get(0))?;
    let collection: i64 = conn.query_row(
        "SELECT COUNT(*) FROM collection WHERE owned = 1",
        [],
        |r| r.get(0),
    )?;
    let unresolved: i64 = conn.query_row(
        "SELECT COUNT(*) FROM disagreements WHERE resolved = 0",
        [],
        |r| r.get(0),
    )?;

    Ok(CatalogStats {
        platforms,
        companies,
        works,
        releases,
        media,
        assets,
        collection_owned: collection,
        unresolved_disagreements: unresolved,
    })
}

/// Summary statistics for the catalog.
#[derive(Debug)]
pub struct CatalogStats {
    pub platforms: i64,
    pub companies: i64,
    pub works: i64,
    pub releases: i64,
    pub media: i64,
    pub assets: i64,
    pub collection_owned: i64,
    pub unresolved_disagreements: i64,
}

// ── Disagreement Queries ────────────────────────────────────────────────────

/// List unresolved disagreements, optionally filtered.
pub fn list_unresolved_disagreements(
    conn: &Connection,
    entity_type: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<Disagreement>, OperationError> {
    let limit = limit.unwrap_or(100);
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match entity_type {
        Some(et) => (
            format!(
                "SELECT id, entity_type, entity_id, field, source_a, value_a,
                        source_b, value_b, resolved, resolution, resolved_at, created_at
                 FROM disagreements WHERE resolved = 0 AND entity_type = ?1
                 ORDER BY created_at DESC LIMIT {limit}"
            ),
            vec![Box::new(et.to_string())],
        ),
        None => (
            format!(
                "SELECT id, entity_type, entity_id, field, source_a, value_a,
                        source_b, value_b, resolved, resolution, resolved_at, created_at
                 FROM disagreements WHERE resolved = 0
                 ORDER BY created_at DESC LIMIT {limit}"
            ),
            vec![],
        ),
    };

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|v| v.as_ref()).collect();
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok(Disagreement {
            id: row.get(0)?,
            entity_type: row.get(1)?,
            entity_id: row.get(2)?,
            field: row.get(3)?,
            source_a: row.get(4)?,
            value_a: row.get(5)?,
            source_b: row.get(6)?,
            value_b: row.get(7)?,
            resolved: row.get(8)?,
            resolution: row.get(9)?,
            resolved_at: row.get(10)?,
            created_at: row.get(11)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

// ── Import Log Queries ──────────────────────────────────────────────────────

/// List recent import logs.
pub fn list_import_logs(
    conn: &Connection,
    limit: Option<u32>,
) -> Result<Vec<ImportLog>, OperationError> {
    let limit = limit.unwrap_or(20);
    let mut stmt = conn.prepare(&format!(
        "SELECT id, source_type, source_name, source_version, imported_at,
                records_created, records_updated, records_unchanged, disagreements_found
         FROM import_log ORDER BY imported_at DESC LIMIT {limit}"
    ))?;
    let rows = stmt.query_map([], |row| {
        Ok(ImportLog {
            id: row.get(0)?,
            source_type: row.get(1)?,
            source_name: row.get(2)?,
            source_version: row.get(3)?,
            imported_at: row.get(4)?,
            records_created: row.get(5)?,
            records_updated: row.get(6)?,
            records_unchanged: row.get(7)?,
            disagreements_found: row.get(8)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

// ── Row Mapping Helpers ─────────────────────────────────────────────────────

fn row_to_media(row: &rusqlite::Row<'_>) -> rusqlite::Result<Media> {
    let status_str: String = row.get(6)?;
    Ok(Media {
        id: row.get(0)?,
        release_id: row.get(1)?,
        media_serial: row.get(2)?,
        disc_number: row.get(3)?,
        disc_label: row.get(4)?,
        revision: row.get(5)?,
        status: MediaStatus::from_str_loose(&status_str),
        dat_name: row.get(7)?,
        dat_source: row.get(8)?,
        file_size: row.get(9)?,
        crc32: row.get(10)?,
        sha1: row.get(11)?,
        md5: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn row_to_release(row: &rusqlite::Row<'_>) -> rusqlite::Result<Release> {
    Ok(Release {
        id: row.get(0)?,
        work_id: row.get(1)?,
        platform_id: row.get(2)?,
        region: row.get(3)?,
        title: row.get(4)?,
        alt_title: row.get(5)?,
        publisher_id: row.get(6)?,
        developer_id: row.get(7)?,
        release_date: row.get(8)?,
        game_serial: row.get(9)?,
        genre: row.get(10)?,
        players: row.get(11)?,
        rating: row.get(12)?,
        description: row.get(13)?,
        screenscraper_id: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
    })
}