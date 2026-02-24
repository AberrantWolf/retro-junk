//! Read queries for the catalog database.
//!
//! Provides lookup by hash, serial, platform, search, and listing.

use retro_junk_catalog::types::*;
use rusqlite::{params, Connection};

use crate::operations::OperationError;

// ── Column Constants ────────────────────────────────────────────────────────

const MEDIA_COLUMNS: &str =
    "id, release_id, media_serial, disc_number, disc_label, \
     revision, status, dat_name, dat_source, file_size, \
     crc32, sha1, md5, created_at, updated_at";

const RELEASE_COLUMNS: &str =
    "id, work_id, platform_id, region, revision, variant, \
     title, alt_title, publisher_id, developer_id, release_date, \
     game_serial, genre, players, rating, description, \
     screenscraper_id, scraper_not_found, created_at, updated_at";

// ── Media Lookups ───────────────────────────────────────────────────────────

/// Query media with a single-param WHERE clause.
fn query_media(
    conn: &Connection,
    where_clause: &str,
    param: &str,
) -> Result<Vec<Media>, OperationError> {
    let sql = format!("SELECT {MEDIA_COLUMNS} FROM media WHERE {where_clause}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![param], row_to_media)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Find media entries by CRC32 hash.
pub fn find_media_by_crc32(
    conn: &Connection,
    crc32: &str,
) -> Result<Vec<Media>, OperationError> {
    query_media(conn, "crc32 = ?1", crc32)
}

/// Find media entries by SHA1 hash.
pub fn find_media_by_sha1(
    conn: &Connection,
    sha1: &str,
) -> Result<Vec<Media>, OperationError> {
    query_media(conn, "sha1 = ?1", sha1)
}

/// Find media entries by MD5 hash.
pub fn find_media_by_md5(
    conn: &Connection,
    md5: &str,
) -> Result<Vec<Media>, OperationError> {
    query_media(conn, "md5 = ?1", md5)
}

/// Find media entries by serial number.
pub fn find_media_by_serial(
    conn: &Connection,
    serial: &str,
) -> Result<Vec<Media>, OperationError> {
    query_media(conn, "media_serial = ?1", serial)
}

/// Find all media entries for a given release.
pub fn media_for_release(
    conn: &Connection,
    release_id: &str,
) -> Result<Vec<Media>, OperationError> {
    let sql = format!(
        "SELECT {MEDIA_COLUMNS} FROM media \
         WHERE release_id = ?1 ORDER BY disc_number, dat_name"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![release_id], row_to_media)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

// ── Release Lookups ─────────────────────────────────────────────────────────

/// Query releases with a single-param WHERE clause.
fn query_releases(
    conn: &Connection,
    where_and_tail: &str,
    param: &str,
) -> Result<Vec<Release>, OperationError> {
    let sql = format!("SELECT {RELEASE_COLUMNS} FROM releases WHERE {where_and_tail}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![param], row_to_release)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// List all releases for a platform.
pub fn releases_for_platform(
    conn: &Connection,
    platform_id: &str,
) -> Result<Vec<Release>, OperationError> {
    query_releases(conn, "platform_id = ?1 ORDER BY title", platform_id)
}

/// Search releases by title (case-insensitive LIKE).
pub fn search_releases(
    conn: &Connection,
    query: &str,
) -> Result<Vec<Release>, OperationError> {
    let pattern = format!("%{}%", query);
    query_releases(conn, "title LIKE ?1 ORDER BY title LIMIT 100", &pattern)
}

/// Search releases by title with optional platform filter and configurable limit.
pub fn search_releases_filtered(
    conn: &Connection,
    query: &str,
    platform_id: Option<&str>,
    limit: u32,
) -> Result<Vec<Release>, OperationError> {
    let pattern = format!("%{}%", query);
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match platform_id {
        Some(pid) => (
            format!(
                "SELECT {RELEASE_COLUMNS} FROM releases \
                 WHERE title LIKE ?1 AND platform_id = ?2 \
                 ORDER BY title LIMIT {limit}"
            ),
            vec![
                Box::new(pattern),
                Box::new(pid.to_string()),
            ],
        ),
        None => (
            format!(
                "SELECT {RELEASE_COLUMNS} FROM releases \
                 WHERE title LIKE ?1 ORDER BY title LIMIT {limit}"
            ),
            vec![Box::new(pattern)],
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|v| v.as_ref()).collect();
    let rows = stmt.query_map(params.as_slice(), row_to_release)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Find a release by game serial.
pub fn find_release_by_serial(
    conn: &Connection,
    serial: &str,
) -> Result<Vec<Release>, OperationError> {
    query_releases(conn, "game_serial = ?1", serial)
}

/// Find releases that need ScreenScraper enrichment.
///
/// Returns releases for the given platform that have at least one media entry
/// (needed for lookup) and optionally filters to only those without a
/// screenscraper_id.
pub fn releases_to_enrich(
    conn: &Connection,
    platform_id: &str,
    skip_existing: bool,
    limit: Option<u32>,
) -> Result<Vec<Release>, OperationError> {
    let limit = limit.unwrap_or(u32::MAX);
    let extra_filter = if skip_existing {
        " AND r.screenscraper_id IS NULL AND r.scraper_not_found = 0"
    } else {
        ""
    };
    let sql = format!(
        "SELECT DISTINCT r.id, r.work_id, r.platform_id, r.region, r.revision, r.variant, \
                r.title, r.alt_title, r.publisher_id, r.developer_id, r.release_date, \
                r.game_serial, r.genre, r.players, r.rating, r.description, \
                r.screenscraper_id, r.scraper_not_found, r.created_at, r.updated_at \
         FROM releases r \
         JOIN media m ON m.release_id = r.id \
         WHERE r.platform_id = ?1{extra_filter} \
         ORDER BY r.title \
         LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![platform_id], row_to_release)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Get a single release by its ID.
pub fn get_release_by_id(
    conn: &Connection,
    id: &str,
) -> Result<Option<Release>, OperationError> {
    let sql = format!("SELECT {RELEASE_COLUMNS} FROM releases WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let result = stmt.query_row(params![id], row_to_release);
    match result {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Resolve a company ID to its display name.
pub fn get_company_name(
    conn: &Connection,
    company_id: &str,
) -> Result<Option<String>, OperationError> {
    let result = conn.query_row(
        "SELECT name FROM companies WHERE id = ?1",
        params![company_id],
        |row| row.get(0),
    );
    match result {
        Ok(name) => Ok(Some(name)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Resolve a platform ID to its display name.
pub fn get_platform_display_name(
    conn: &Connection,
    platform_id: &str,
) -> Result<Option<String>, OperationError> {
    let result = conn.query_row(
        "SELECT short_name FROM platforms WHERE id = ?1",
        params![platform_id],
        |row| row.get(0),
    );
    match result {
        Ok(name) => Ok(Some(name)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
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

/// A lightweight work row for search results.
#[derive(Debug)]
pub struct WorkRow {
    pub id: String,
    pub canonical_name: String,
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

/// Options for filtering disagreement queries.
#[derive(Debug, Default)]
pub struct DisagreementFilter<'a> {
    pub entity_type: Option<&'a str>,
    pub field: Option<&'a str>,
    pub platform_id: Option<&'a str>,
    pub limit: Option<u32>,
}

/// List unresolved disagreements, optionally filtered.
pub fn list_unresolved_disagreements(
    conn: &Connection,
    filter: &DisagreementFilter<'_>,
) -> Result<Vec<Disagreement>, OperationError> {
    let limit = filter.limit.unwrap_or(100);
    let mut conditions = vec!["resolved = 0".to_string()];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut param_idx = 1;

    if let Some(et) = filter.entity_type {
        conditions.push(format!("entity_type = ?{param_idx}"));
        param_values.push(Box::new(et.to_string()));
        param_idx += 1;
    }

    if let Some(field) = filter.field {
        conditions.push(format!("field = ?{param_idx}"));
        param_values.push(Box::new(field.to_string()));
        param_idx += 1;
    }

    if let Some(pid) = filter.platform_id {
        // Filter by platform: check if entity is a release on this platform,
        // or a media item whose release is on this platform.
        conditions.push(format!(
            "((entity_type = 'release' AND entity_id IN \
                (SELECT id FROM releases WHERE platform_id = ?{param_idx})) \
             OR (entity_type = 'media' AND entity_id IN \
                (SELECT id FROM media WHERE release_id IN \
                    (SELECT id FROM releases WHERE platform_id = ?{param_idx}))))"
        ));
        param_values.push(Box::new(pid.to_string()));
        // param_idx += 1;
    }

    let where_clause = conditions.join(" AND ");
    let sql = format!(
        "SELECT id, entity_type, entity_id, field, source_a, value_a,
                source_b, value_b, resolved, resolution, resolved_at, created_at
         FROM disagreements WHERE {where_clause}
         ORDER BY created_at DESC LIMIT {limit}"
    );

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

/// Get a single disagreement by ID.
pub fn get_disagreement(
    conn: &Connection,
    id: i64,
) -> Result<Option<Disagreement>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, entity_type, entity_id, field, source_a, value_a,
                source_b, value_b, resolved, resolution, resolved_at, created_at
         FROM disagreements WHERE id = ?1",
    )?;
    let result = stmt.query_row(rusqlite::params![id], |row| {
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
    });
    match result {
        Ok(d) => Ok(Some(d)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

// ── Collection Queries ──────────────────────────────────────────────────────

/// A collection entry joined with its release and media info.
#[derive(Debug)]
pub struct CollectionRow {
    pub collection_id: i64,
    pub media_id: String,
    pub release_id: String,
    pub platform_id: String,
    pub title: String,
    pub region: String,
    pub dat_name: Option<String>,
    pub crc32: Option<String>,
    pub sha1: Option<String>,
    pub rom_path: Option<String>,
    pub verified_at: Option<String>,
    pub owned: bool,
}

/// List collection entries, optionally filtered by platform.
pub fn list_collection(
    conn: &Connection,
    platform_id: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<CollectionRow>, OperationError> {
    let limit = limit.unwrap_or(1000);
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match platform_id {
        Some(pid) => (
            format!(
                "SELECT c.id, c.media_id, m.release_id, r.platform_id, r.title, r.region,
                        m.dat_name, m.crc32, m.sha1, c.rom_path, c.verified_at, c.owned
                 FROM collection c
                 JOIN media m ON c.media_id = m.id
                 JOIN releases r ON m.release_id = r.id
                 WHERE r.platform_id = ?1
                 ORDER BY r.title
                 LIMIT {limit}"
            ),
            vec![Box::new(pid.to_string())],
        ),
        None => (
            format!(
                "SELECT c.id, c.media_id, m.release_id, r.platform_id, r.title, r.region,
                        m.dat_name, m.crc32, m.sha1, c.rom_path, c.verified_at, c.owned
                 FROM collection c
                 JOIN media m ON c.media_id = m.id
                 JOIN releases r ON m.release_id = r.id
                 ORDER BY r.platform_id, r.title
                 LIMIT {limit}"
            ),
            vec![],
        ),
    };

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|v| v.as_ref()).collect();
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok(CollectionRow {
            collection_id: row.get(0)?,
            media_id: row.get(1)?,
            release_id: row.get(2)?,
            platform_id: row.get(3)?,
            title: row.get(4)?,
            region: row.get(5)?,
            dat_name: row.get(6)?,
            crc32: row.get(7)?,
            sha1: row.get(8)?,
            rom_path: row.get(9)?,
            verified_at: row.get(10)?,
            owned: row.get(11)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Find a collection entry by media ID and user.
pub fn find_collection_entry(
    conn: &Connection,
    media_id: &str,
    user_id: &str,
) -> Result<Option<CollectionEntry>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, media_id, user_id, owned, condition, notes, date_acquired, rom_path, verified_at
         FROM collection WHERE media_id = ?1 AND user_id = ?2",
    )?;
    let result = stmt.query_row(params![media_id, user_id], |row| {
        Ok(CollectionEntry {
            id: row.get(0)?,
            media_id: row.get(1)?,
            user_id: row.get(2)?,
            owned: row.get(3)?,
            condition: row.get(4)?,
            notes: row.get(5)?,
            date_acquired: row.get(6)?,
            rom_path: row.get(7)?,
            verified_at: row.get(8)?,
        })
    });
    match result {
        Ok(e) => Ok(Some(e)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Count collection entries grouped by platform.
pub fn collection_counts_by_platform(
    conn: &Connection,
) -> Result<Vec<(String, i64)>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT r.platform_id, COUNT(*)
         FROM collection c
         JOIN media m ON c.media_id = m.id
         JOIN releases r ON m.release_id = r.id
         WHERE c.owned = 1
         GROUP BY r.platform_id
         ORDER BY r.platform_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
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

// ── Asset Queries ─────────────────────────────────────────────────────────

/// List all assets for a release.
pub fn assets_for_release(
    conn: &Connection,
    release_id: &str,
) -> Result<Vec<MediaAsset>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, release_id, media_id, asset_type, region, source,
                file_path, source_url, scraped, file_hash, width, height, created_at
         FROM media_assets WHERE release_id = ?1
         ORDER BY asset_type, region",
    )?;
    let rows = stmt.query_map(params![release_id], row_to_asset)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Count assets per type for a platform, optionally restricted to collection.
///
/// Returns rows of (asset_type, count).
pub fn asset_counts_by_type(
    conn: &Connection,
    platform_id: &str,
    collection_only: bool,
) -> Result<Vec<(String, i64)>, OperationError> {
    let sql = if collection_only {
        "SELECT a.asset_type, COUNT(DISTINCT a.id)
         FROM media_assets a
         JOIN releases r ON a.release_id = r.id
         JOIN media m ON m.release_id = r.id
         JOIN collection c ON c.media_id = m.id AND c.owned = 1
         WHERE r.platform_id = ?1
         GROUP BY a.asset_type
         ORDER BY a.asset_type"
    } else {
        "SELECT a.asset_type, COUNT(DISTINCT a.id)
         FROM media_assets a
         JOIN releases r ON a.release_id = r.id
         WHERE r.platform_id = ?1
         GROUP BY a.asset_type
         ORDER BY a.asset_type"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![platform_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Find releases missing a specific asset type.
///
/// Returns (release_id, title, region) for releases that have no asset of the
/// given type. Optionally filtered to collection-only releases.
pub fn releases_missing_asset_type(
    conn: &Connection,
    platform_id: &str,
    asset_type: &str,
    collection_only: bool,
    limit: Option<u32>,
) -> Result<Vec<(String, String, String)>, OperationError> {
    let limit = limit.unwrap_or(100);
    let sql = if collection_only {
        format!(
            "SELECT r.id, r.title, r.region
             FROM releases r
             JOIN media m ON m.release_id = r.id
             JOIN collection c ON c.media_id = m.id AND c.owned = 1
             WHERE r.platform_id = ?1
               AND r.id NOT IN (
                   SELECT release_id FROM media_assets
                   WHERE asset_type = ?2 AND release_id IS NOT NULL
               )
             GROUP BY r.id
             ORDER BY r.title
             LIMIT {limit}"
        )
    } else {
        format!(
            "SELECT r.id, r.title, r.region
             FROM releases r
             WHERE r.platform_id = ?1
               AND r.id NOT IN (
                   SELECT release_id FROM media_assets
                   WHERE asset_type = ?2 AND release_id IS NOT NULL
               )
             ORDER BY r.title
             LIMIT {limit}"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![platform_id, asset_type], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Find releases with no assets at all.
///
/// Returns (release_id, title, region).
pub fn releases_with_no_assets(
    conn: &Connection,
    platform_id: &str,
    collection_only: bool,
    limit: Option<u32>,
) -> Result<Vec<(String, String, String)>, OperationError> {
    let limit = limit.unwrap_or(100);
    let sql = if collection_only {
        format!(
            "SELECT r.id, r.title, r.region
             FROM releases r
             JOIN media m ON m.release_id = r.id
             JOIN collection c ON c.media_id = m.id AND c.owned = 1
             WHERE r.platform_id = ?1
               AND r.id NOT IN (
                   SELECT release_id FROM media_assets WHERE release_id IS NOT NULL
               )
             GROUP BY r.id
             ORDER BY r.title
             LIMIT {limit}"
        )
    } else {
        format!(
            "SELECT r.id, r.title, r.region
             FROM releases r
             WHERE r.platform_id = ?1
               AND r.id NOT IN (
                   SELECT release_id FROM media_assets WHERE release_id IS NOT NULL
               )
             ORDER BY r.title
             LIMIT {limit}"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![platform_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Asset coverage summary for a platform.
///
/// Returns (total_releases, releases_with_any_asset, total_assets).
pub fn asset_coverage_summary(
    conn: &Connection,
    platform_id: &str,
    collection_only: bool,
) -> Result<(i64, i64, i64), OperationError> {
    let (total_sql, with_assets_sql, asset_count_sql) = if collection_only {
        (
            "SELECT COUNT(DISTINCT r.id)
             FROM releases r
             JOIN media m ON m.release_id = r.id
             JOIN collection c ON c.media_id = m.id AND c.owned = 1
             WHERE r.platform_id = ?1",
            "SELECT COUNT(DISTINCT r.id)
             FROM releases r
             JOIN media m ON m.release_id = r.id
             JOIN collection c ON c.media_id = m.id AND c.owned = 1
             JOIN media_assets a ON a.release_id = r.id
             WHERE r.platform_id = ?1",
            "SELECT COUNT(*)
             FROM media_assets a
             JOIN releases r ON a.release_id = r.id
             JOIN media m ON m.release_id = r.id
             JOIN collection c ON c.media_id = m.id AND c.owned = 1
             WHERE r.platform_id = ?1",
        )
    } else {
        (
            "SELECT COUNT(*) FROM releases WHERE platform_id = ?1",
            "SELECT COUNT(DISTINCT r.id)
             FROM releases r
             JOIN media_assets a ON a.release_id = r.id
             WHERE r.platform_id = ?1",
            "SELECT COUNT(*)
             FROM media_assets a
             JOIN releases r ON a.release_id = r.id
             WHERE r.platform_id = ?1",
        )
    };

    let total: i64 = conn.query_row(total_sql, params![platform_id], |r| r.get(0))?;
    let with_assets: i64 = conn.query_row(with_assets_sql, params![platform_id], |r| r.get(0))?;
    let asset_count: i64 = conn.query_row(asset_count_sql, params![platform_id], |r| r.get(0))?;

    Ok((total, with_assets, asset_count))
}

// ── Catalog List Queries ────────────────────────────────────────────────────

/// Search works by canonical name (case-insensitive LIKE).
pub fn search_works(
    conn: &Connection,
    query: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<WorkRow>, OperationError> {
    let pattern = format!("%{}%", query);
    let sql = format!(
        "SELECT id, canonical_name FROM works \
         WHERE canonical_name LIKE ?1 \
         ORDER BY canonical_name LIMIT {limit} OFFSET {offset}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![pattern], |row| {
        Ok(WorkRow {
            id: row.get(0)?,
            canonical_name: row.get(1)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Search media by dat_name with optional platform filter and pagination.
pub fn search_media(
    conn: &Connection,
    query: &str,
    platform_id: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Vec<Media>, OperationError> {
    let pattern = format!("%{}%", query);
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match platform_id {
        Some(pid) => (
            format!(
                "SELECT {MEDIA_COLUMNS} FROM media m \
                 JOIN releases r ON m.release_id = r.id \
                 WHERE m.dat_name LIKE ?1 AND r.platform_id = ?2 \
                 ORDER BY m.dat_name LIMIT {limit} OFFSET {offset}"
            ),
            vec![Box::new(pattern), Box::new(pid.to_string())],
        ),
        None => (
            format!(
                "SELECT {MEDIA_COLUMNS} FROM media m \
                 WHERE m.dat_name LIKE ?1 \
                 ORDER BY m.dat_name LIMIT {limit} OFFSET {offset}"
            ),
            vec![Box::new(pattern)],
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|v| v.as_ref()).collect();
    let rows = stmt.query_map(params.as_slice(), row_to_media)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Search releases by title with optional platform filter and pagination.
pub fn search_releases_paged(
    conn: &Connection,
    query: &str,
    platform_id: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Vec<Release>, OperationError> {
    let pattern = format!("%{}%", query);
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match platform_id {
        Some(pid) => (
            format!(
                "SELECT {RELEASE_COLUMNS} FROM releases \
                 WHERE title LIKE ?1 AND platform_id = ?2 \
                 ORDER BY title LIMIT {limit} OFFSET {offset}"
            ),
            vec![Box::new(pattern), Box::new(pid.to_string())],
        ),
        None => (
            format!(
                "SELECT {RELEASE_COLUMNS} FROM releases \
                 WHERE title LIKE ?1 ORDER BY title LIMIT {limit} OFFSET {offset}"
            ),
            vec![Box::new(pattern)],
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|v| v.as_ref()).collect();
    let rows = stmt.query_map(params.as_slice(), row_to_release)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Get a single work by its ID.
pub fn get_work_by_id(
    conn: &Connection,
    id: &str,
) -> Result<Option<WorkRow>, OperationError> {
    let result = conn.query_row(
        "SELECT id, canonical_name FROM works WHERE id = ?1",
        params![id],
        |row| {
            Ok(WorkRow {
                id: row.get(0)?,
                canonical_name: row.get(1)?,
            })
        },
    );
    match result {
        Ok(w) => Ok(Some(w)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Get a single media entry by its ID.
pub fn get_media_by_id(
    conn: &Connection,
    id: &str,
) -> Result<Option<Media>, OperationError> {
    let sql = format!("SELECT {MEDIA_COLUMNS} FROM media WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let result = stmt.query_row(params![id], row_to_media);
    match result {
        Ok(m) => Ok(Some(m)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Get a single platform by its ID.
pub fn get_platform_by_id(
    conn: &Connection,
    id: &str,
) -> Result<Option<PlatformRow>, OperationError> {
    let result = conn.query_row(
        "SELECT id, display_name, short_name, manufacturer, generation,
                media_type, release_year, core_platform
         FROM platforms WHERE id = ?1",
        params![id],
        |row| {
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
        },
    );
    match result {
        Ok(p) => Ok(Some(p)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Get all releases for a given work.
pub fn releases_for_work(
    conn: &Connection,
    work_id: &str,
) -> Result<Vec<Release>, OperationError> {
    let sql = format!(
        "SELECT {RELEASE_COLUMNS} FROM releases \
         WHERE work_id = ?1 ORDER BY platform_id, region"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![work_id], row_to_release)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Count releases grouped by platform.
pub fn platform_release_counts(
    conn: &Connection,
) -> Result<Vec<(String, i64)>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT platform_id, COUNT(*) FROM releases GROUP BY platform_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Count media entries grouped by platform (via releases join).
pub fn platform_media_counts(
    conn: &Connection,
) -> Result<Vec<(String, i64)>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT r.platform_id, COUNT(*) FROM media m \
         JOIN releases r ON m.release_id = r.id \
         GROUP BY r.platform_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
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

fn row_to_asset(row: &rusqlite::Row<'_>) -> rusqlite::Result<MediaAsset> {
    Ok(MediaAsset {
        id: row.get(0)?,
        release_id: row.get(1)?,
        media_id: row.get(2)?,
        asset_type: row.get(3)?,
        region: row.get(4)?,
        source: row.get(5)?,
        file_path: row.get(6)?,
        source_url: row.get(7)?,
        scraped: row.get(8)?,
        file_hash: row.get(9)?,
        width: row.get(10)?,
        height: row.get(11)?,
        created_at: row.get(12)?,
    })
}

fn row_to_release(row: &rusqlite::Row<'_>) -> rusqlite::Result<Release> {
    Ok(Release {
        id: row.get(0)?,
        work_id: row.get(1)?,
        platform_id: row.get(2)?,
        region: row.get(3)?,
        revision: row.get(4)?,
        variant: row.get(5)?,
        title: row.get(6)?,
        alt_title: row.get(7)?,
        publisher_id: row.get(8)?,
        developer_id: row.get(9)?,
        release_date: row.get(10)?,
        game_serial: row.get(11)?,
        genre: row.get(12)?,
        players: row.get(13)?,
        rating: row.get(14)?,
        description: row.get(15)?,
        screenscraper_id: row.get(16)?,
        scraper_not_found: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
    })
}