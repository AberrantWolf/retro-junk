//! Read queries for the catalog database.
//!
//! Provides lookup by hash, serial, platform, search, and listing.

use retro_junk_catalog::types::{
    Asset, AssetOwner, CatalogTag, CollectionEntry, CollectionId, Disagreement, DisagreementId,
    ImportLog, ImportLogId, Media, MediaAssetId, MediaStatus, Release,
};
use rusqlite::{Connection, params};

use crate::operations::OperationError;

/// A compact, joined catalog row used for runtime ROM identification.
#[derive(Debug, Clone)]
pub struct CatalogMediaMatch {
    pub media: Media,
    pub platform_id: String,
    pub region: String,
    pub release_revision: String,
    pub release_title: String,
    pub cover_title: String,
    pub screen_title: String,
}

#[derive(Debug, Clone)]
pub struct CatalogHashQuery {
    pub file_size: u64,
    pub crc32: String,
    pub sha1: String,
}

// ── Column Constants ────────────────────────────────────────────────────────

const MEDIA_COLUMNS: &str = "id, release_id, media_serial, disc_number, disc_label, \
     revision, status, tag, dat_name, rom_name, dat_source, dat_system, file_size, \
     crc32, sha1, md5, created_at, updated_at";

const JOINED_MEDIA_COLUMNS: &str = "m.id, m.release_id, m.media_serial, m.disc_number, m.disc_label, \
     m.revision, m.status, m.tag, m.dat_name, m.rom_name, m.dat_source, m.dat_system, m.file_size, \
     m.crc32, m.sha1, m.md5, m.created_at, m.updated_at";

const RELEASE_COLUMNS: &str = "id, work_id, platform_id, region, revision, variant, \
     title, alt_title, publisher_id, developer_id, release_date, \
     game_serial, genre, players, rating, description, \
     screen_title, cover_title, \
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
pub fn find_media_by_crc32(conn: &Connection, crc32: &str) -> Result<Vec<Media>, OperationError> {
    query_media(conn, "crc32 = lower(?1)", crc32)
}

/// Find media entries by SHA1 hash.
pub fn find_media_by_sha1(conn: &Connection, sha1: &str) -> Result<Vec<Media>, OperationError> {
    query_media(conn, "sha1 = lower(?1)", sha1)
}

/// Find media entries by MD5 hash.
pub fn find_media_by_md5(conn: &Connection, md5: &str) -> Result<Vec<Media>, OperationError> {
    query_media(conn, "md5 = lower(?1)", md5)
}

/// Match one ROM using indexed catalog hashes, constrained to its platform.
/// CRC32 additionally requires the DAT size; SHA1 is used as the fallback.
pub fn match_media_by_hash(
    conn: &Connection,
    platform_id: &str,
    file_size: u64,
    crc32: Option<&str>,
    sha1: Option<&str>,
) -> Result<Vec<CatalogMediaMatch>, OperationError> {
    let sql = format!(
        "SELECT {JOINED_MEDIA_COLUMNS}, r.platform_id, r.region, r.title, r.cover_title, r.screen_title, r.revision \
         FROM media m JOIN releases r ON r.id=m.release_id \
         WHERE r.platform_id=?1 AND (\
           (?2<>'' AND m.crc32=lower(?2) AND m.file_size=?3) OR \
           (?4<>'' AND m.sha1=lower(?4))\
         ) ORDER BY CASE WHEN ?2<>'' AND m.crc32=lower(?2) AND m.file_size=?3 THEN 0 ELSE 1 END, \
                    r.region, m.dat_name"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            platform_id,
            crc32.unwrap_or_default(),
            i64::try_from(file_size).unwrap_or(i64::MAX),
            sha1.unwrap_or_default()
        ],
        |row| {
            Ok(CatalogMediaMatch {
                media: row_to_media(row)?,
                platform_id: row.get(18)?,
                region: row.get(19)?,
                release_title: row.get(20)?,
                cover_title: row.get(21)?,
                screen_title: row.get(22)?,
                release_revision: row.get(23)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Match a normalized header serial against catalog media or release serials,
/// constrained to the selected platform.
pub fn match_media_by_serial(
    conn: &Connection,
    platform_id: &str,
    serial: &str,
) -> Result<Vec<CatalogMediaMatch>, OperationError> {
    let serial_key = serial.to_ascii_uppercase().replace([' ', '-'], "");
    if serial_key.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT {JOINED_MEDIA_COLUMNS}, r.platform_id, r.region, r.title, r.cover_title, r.screen_title, r.revision \
         FROM media m JOIN releases r ON r.id=m.release_id \
         LEFT JOIN media_serial_keys msk ON msk.media_id=m.id \
         WHERE r.platform_id=?1 AND (\
           msk.serial_key=?2 OR \
           upper(replace(replace(r.game_serial, '-', ''), ' ', ''))=?2\
         ) ORDER BY r.region, m.dat_name"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![platform_id, serial_key], |row| {
        Ok(CatalogMediaMatch {
            media: row_to_media(row)?,
            platform_id: row.get(18)?,
            region: row.get(19)?,
            release_title: row.get(20)?,
            cover_title: row.get(21)?,
            screen_title: row.get(22)?,
            release_revision: row.get(23)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Resolve a cluster of serials in one platform-scoped SQL query.
pub fn match_media_by_serials(
    conn: &Connection,
    platform_id: &str,
    serials: &[String],
) -> Result<Vec<Vec<CatalogMediaMatch>>, OperationError> {
    let mut grouped = vec![Vec::new(); serials.len()];
    let normalized = serials
        .iter()
        .enumerate()
        .filter_map(|(index, serial)| {
            let key = serial.to_ascii_uppercase().replace([' ', '-'], "");
            (!key.is_empty()).then_some((index, key))
        })
        .collect::<Vec<_>>();
    if normalized.is_empty() {
        return Ok(grouped);
    }
    let values = std::iter::repeat_n("(?, ?)", normalized.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "WITH input(request_index,serial_key) AS (VALUES {values}) \
         , hits(request_index,media_id) AS ( \
           SELECT input.request_index,msk.media_id \
           FROM input JOIN media_serial_keys msk ON msk.serial_key=input.serial_key \
           JOIN media hit_media ON hit_media.id=msk.media_id \
           JOIN releases hit_release ON hit_release.id=hit_media.release_id \
           WHERE hit_release.platform_id=? \
           UNION \
           SELECT input.request_index,hit_media.id \
           FROM input JOIN releases AS hit_release INDEXED BY idx_release_serial_normalized \
             ON upper(replace(replace(hit_release.game_serial, '-', ''), ' ', ''))=input.serial_key \
           JOIN media hit_media ON hit_media.release_id=hit_release.id \
           WHERE hit_release.platform_id=? \
         ) \
         SELECT {JOINED_MEDIA_COLUMNS}, r.platform_id, r.region, r.title, r.cover_title, r.screen_title, r.revision, hits.request_index \
         FROM hits JOIN media m ON m.id=hits.media_id JOIN releases r ON r.id=m.release_id \
         ORDER BY hits.request_index, r.region, m.dat_name"
    );
    let mut parameters: Vec<rusqlite::types::Value> = Vec::with_capacity(normalized.len() * 2 + 2);
    for (index, serial) in normalized {
        parameters.push(i64::try_from(index).unwrap_or(i64::MAX).into());
        parameters.push(serial.into());
    }
    parameters.push(platform_id.to_owned().into());
    parameters.push(platform_id.to_owned().into());
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(rusqlite::params_from_iter(parameters), |row| {
        Ok((
            row.get::<_, usize>(24)?,
            CatalogMediaMatch {
                media: row_to_media(row)?,
                platform_id: row.get(18)?,
                region: row.get(19)?,
                release_title: row.get(20)?,
                cover_title: row.get(21)?,
                screen_title: row.get(22)?,
                release_revision: row.get(23)?,
            },
        ))
    })?;
    for row in rows {
        let (index, found) = row?;
        if let Some(matches) = grouped.get_mut(index) {
            matches.push(found);
        }
    }
    Ok(grouped)
}

/// Resolve a cluster of hashes in one platform-scoped SQL query.
pub fn match_media_by_hashes(
    conn: &Connection,
    platform_id: &str,
    requests: &[CatalogHashQuery],
) -> Result<Vec<Vec<CatalogMediaMatch>>, OperationError> {
    if requests.is_empty() {
        return Ok(Vec::new());
    }
    let values = std::iter::repeat_n("(?, ?, ?, ?)", requests.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "WITH input(request_index,file_size,crc32,sha1) AS (VALUES {values}), \
         hits(request_index,media_id) AS ( \
           SELECT input.request_index,m.id FROM input \
           JOIN releases r ON r.platform_id=? \
           JOIN media m ON m.release_id=r.id AND ( \
             (input.crc32<>'' AND m.crc32=lower(input.crc32) AND m.file_size=input.file_size) OR \
             (input.sha1<>'' AND m.sha1=lower(input.sha1))) \
           UNION \
           SELECT input.request_index,mt.media_id FROM input \
           JOIN media_tracks mt ON ( \
             (input.crc32<>'' AND mt.crc32=lower(input.crc32) AND mt.file_size=input.file_size) OR \
             (input.sha1<>'' AND mt.sha1=lower(input.sha1))) \
           JOIN media track_media ON track_media.id=mt.media_id \
           JOIN releases track_release ON track_release.id=track_media.release_id \
           WHERE track_release.platform_id=? \
         ) \
         SELECT {JOINED_MEDIA_COLUMNS}, r.platform_id, r.region, r.title, r.cover_title, r.screen_title, r.revision, hits.request_index \
         FROM hits JOIN media m ON m.id=hits.media_id JOIN releases r ON r.id=m.release_id \
         ORDER BY hits.request_index, r.region, m.dat_name"
    );
    let mut parameters: Vec<rusqlite::types::Value> = Vec::with_capacity(requests.len() * 4 + 2);
    for (index, request) in requests.iter().enumerate() {
        parameters.push(i64::try_from(index).unwrap_or(i64::MAX).into());
        parameters.push(i64::try_from(request.file_size).unwrap_or(i64::MAX).into());
        parameters.push(request.crc32.clone().into());
        parameters.push(request.sha1.clone().into());
    }
    parameters.push(platform_id.to_owned().into());
    parameters.push(platform_id.to_owned().into());
    let mut grouped = vec![Vec::new(); requests.len()];
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(rusqlite::params_from_iter(parameters), |row| {
        Ok((
            row.get::<_, usize>(24)?,
            CatalogMediaMatch {
                media: row_to_media(row)?,
                platform_id: row.get(18)?,
                region: row.get(19)?,
                release_title: row.get(20)?,
                cover_title: row.get(21)?,
                screen_title: row.get(22)?,
                release_revision: row.get(23)?,
            },
        ))
    })?;
    for row in rows {
        let (index, found) = row?;
        if let Some(matches) = grouped.get_mut(index) {
            matches.push(found);
        }
    }
    Ok(grouped)
}

/// Find disc media whose imported track hashes match a physical track.
pub fn match_media_ids_by_track_hash(
    conn: &Connection,
    platform_id: &str,
    file_size: u64,
    crc32: &str,
    sha1: Option<&str>,
) -> Result<Vec<String>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT mt.media_id FROM media_tracks mt
         JOIN media m ON m.id=mt.media_id JOIN releases r ON r.id=m.release_id
         WHERE r.platform_id=?1 AND (
           (mt.crc32=lower(?2) AND mt.file_size=?3) OR
           (?4<>'' AND mt.sha1=lower(?4))
         ) ORDER BY mt.media_id",
    )?;
    let rows = stmt.query_map(
        params![
            platform_id,
            crc32,
            i64::try_from(file_size).unwrap_or(i64::MAX),
            sha1.unwrap_or_default()
        ],
        |row| row.get(0),
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Find media entries by serial number.
pub fn find_media_by_serial(conn: &Connection, serial: &str) -> Result<Vec<Media>, OperationError> {
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

/// Find every carrier variant for one logical work/platform/region.
///
/// Multi-disc physical copies can legitimately contain discs from different
/// mastering-specific catalog releases. Callers use the distinct disc numbers
/// as the logical required slots while retaining exact media IDs per carrier.
pub fn media_for_work_scope(
    conn: &Connection,
    work_id: &str,
    platform_id: &str,
    region: &str,
) -> Result<Vec<Media>, OperationError> {
    let sql = format!(
        "SELECT {JOINED_MEDIA_COLUMNS} FROM media m
         JOIN releases r ON r.id=m.release_id
         WHERE r.work_id=?1 AND r.platform_id=?2 AND r.region=?3
         ORDER BY m.disc_number,m.dat_name,m.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![work_id, platform_id, region], row_to_media)?;
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
pub fn search_releases(conn: &Connection, query: &str) -> Result<Vec<Release>, OperationError> {
    let pattern = format!("%{query}%");
    query_releases(conn, "title LIKE ?1 ORDER BY title LIMIT 100", &pattern)
}

/// Search releases by title with optional platform filter and configurable limit.
pub fn search_releases_filtered(
    conn: &Connection,
    query: &str,
    platform_id: Option<&str>,
    limit: u32,
) -> Result<Vec<Release>, OperationError> {
    let pattern = format!("%{query}%");
    let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match platform_id {
        Some(pid) => (
            format!(
                "SELECT {RELEASE_COLUMNS} FROM releases \
                 WHERE title LIKE ?1 AND platform_id = ?2 \
                 ORDER BY title LIMIT {limit}"
            ),
            vec![Box::new(pattern), Box::new(pid.to_string())],
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
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect();
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

/// Find releases that need `ScreenScraper` enrichment.
///
/// Returns releases for the given platform that have at least one media entry
/// (needed for lookup) and optionally filters to only those without a
/// `screenscraper_id`.
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
                r.screen_title, r.cover_title, \
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

/// Count releases that need `ScreenScraper` enrichment.
///
/// Same filtering logic as `releases_to_enrich` but returns just the count,
/// used for progress reporting before batched processing begins.
pub fn count_releases_to_enrich(
    conn: &Connection,
    platform_id: &str,
    skip_existing: bool,
) -> Result<u32, OperationError> {
    let extra_filter = if skip_existing {
        " AND r.screenscraper_id IS NULL AND r.scraper_not_found = 0"
    } else {
        ""
    };
    let sql = format!(
        "SELECT COUNT(DISTINCT r.id) \
         FROM releases r \
         JOIN media m ON m.release_id = r.id \
         WHERE r.platform_id = ?1{extra_filter}"
    );
    let count: u32 = conn.query_row(&sql, params![platform_id], |row| row.get(0))?;
    Ok(count)
}

/// Get a single release by its ID.
pub fn get_release_by_id(conn: &Connection, id: &str) -> Result<Option<Release>, OperationError> {
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
    /// Console generation. 0 = unknown.
    pub generation: u32,
    pub media_type: String,
    /// First release year. 0 = unknown.
    pub release_year: u32,
    /// retro-junk-core Platform variant name; empty when unsupported.
    pub core_platform: String,
}

/// A lightweight work row for search results.
#[derive(Debug)]
pub struct WorkRow {
    pub id: String,
    pub canonical_name: String,
    pub tag: Option<CatalogTag>,
}

/// A work row with release count for a specific platform.
#[derive(Debug)]
pub struct WorkWithCount {
    pub id: String,
    pub canonical_name: String,
    pub release_count: i64,
}

/// List works that have releases on a given platform, with release count per work.
pub fn works_for_platform(
    conn: &Connection,
    platform_id: &str,
) -> Result<Vec<WorkWithCount>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT w.id, w.canonical_name, COUNT(r.id) as release_count \
         FROM works w \
         JOIN releases r ON r.work_id = w.id \
         WHERE r.platform_id = ?1 \
         GROUP BY w.id \
         ORDER BY w.canonical_name",
    )?;
    let rows = stmt.query_map(params![platform_id], |row| {
        Ok(WorkWithCount {
            id: row.get(0)?,
            canonical_name: row.get(1)?,
            release_count: row.get(2)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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
    let collection: i64 =
        conn.query_row("SELECT COUNT(*) FROM collection WHERE owned = 1", [], |r| {
            r.get(0)
        })?;
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
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect();
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok(Disagreement {
            id: DisagreementId(row.get(0)?),
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
    id: DisagreementId,
) -> Result<Option<Disagreement>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, entity_type, entity_id, field, source_a, value_a,
                source_b, value_b, resolved, resolution, resolved_at, created_at
         FROM disagreements WHERE id = ?1",
    )?;
    let result = stmt.query_row(rusqlite::params![id.0], |row| {
        Ok(Disagreement {
            id: DisagreementId(row.get(0)?),
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
    pub collection_id: CollectionId,
    pub media_id: String,
    pub release_id: String,
    pub platform_id: String,
    pub title: String,
    pub region: String,
    pub dat_name: String,
    pub crc32: String,
    pub sha1: String,
    pub rom_path: String,
    /// Timestamp of last hash verification. Empty = never verified.
    pub verified_at: String,
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
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect();
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok(CollectionRow {
            collection_id: CollectionId(row.get(0)?),
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
            id: CollectionId(row.get(0)?),
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
            id: ImportLogId(row.get(0)?),
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
) -> Result<Vec<Asset>, OperationError> {
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
/// Returns rows of (`asset_type`, count).
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
/// Returns (`release_id`, title, region) for releases that have no asset of the
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
/// Returns (`release_id`, title, region).
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
/// Returns (`total_releases`, `releases_with_any_asset`, `total_assets`).
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
    let pattern = format!("%{query}%");
    let sql = format!(
        "SELECT id, canonical_name, tag FROM works \
         WHERE canonical_name LIKE ?1 \
         ORDER BY canonical_name LIMIT {limit} OFFSET {offset}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![pattern], row_to_work)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Search works that have at least one release on the selected platform.
pub fn search_works_for_platform(
    conn: &Connection,
    query: &str,
    platform_id: &str,
    limit: u32,
) -> Result<Vec<WorkRow>, OperationError> {
    let pattern = format!("%{query}%");
    let sql = format!(
        "SELECT DISTINCT w.id,w.canonical_name,w.tag
         FROM works w
         JOIN releases r ON r.work_id=w.id
         WHERE w.canonical_name LIKE ?1 AND r.platform_id=?2
         ORDER BY w.canonical_name
         LIMIT {limit}"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params![pattern, platform_id], row_to_work)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Search media by `dat_name` with optional platform filter and pagination.
pub fn search_media(
    conn: &Connection,
    query: &str,
    platform_id: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Vec<Media>, OperationError> {
    let pattern = format!("%{query}%");
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
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect();
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
    let pattern = format!("%{query}%");
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
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect();
    let rows = stmt.query_map(params.as_slice(), row_to_release)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Get a single work by its ID.
pub fn get_work_by_id(conn: &Connection, id: &str) -> Result<Option<WorkRow>, OperationError> {
    let result = conn.query_row(
        "SELECT id, canonical_name, tag FROM works WHERE id = ?1",
        params![id],
        row_to_work,
    );
    match result {
        Ok(w) => Ok(Some(w)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Get a single media entry by its ID.
pub fn get_media_by_id(conn: &Connection, id: &str) -> Result<Option<Media>, OperationError> {
    let sql = format!("SELECT {MEDIA_COLUMNS} FROM media WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let result = stmt.query_row(params![id], row_to_media);
    match result {
        Ok(m) => Ok(Some(m)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Find works with a specific tag.
pub fn find_works_by_tag(
    conn: &Connection,
    tag: CatalogTag,
) -> Result<Vec<WorkRow>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, canonical_name, tag FROM works WHERE tag = ?1 ORDER BY canonical_name",
    )?;
    let rows = stmt.query_map(params![tag.as_str()], row_to_work)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Find media entries with a specific tag.
pub fn find_media_by_tag(conn: &Connection, tag: CatalogTag) -> Result<Vec<Media>, OperationError> {
    let sql = format!("SELECT {MEDIA_COLUMNS} FROM media WHERE tag = ?1 ORDER BY id");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![tag.as_str()], row_to_media)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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
pub fn releases_for_work(conn: &Connection, work_id: &str) -> Result<Vec<Release>, OperationError> {
    let sql = format!(
        "SELECT {RELEASE_COLUMNS} FROM releases \
         WHERE work_id = ?1 ORDER BY platform_id, region"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![work_id], row_to_release)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Count releases grouped by platform.
pub fn platform_release_counts(conn: &Connection) -> Result<Vec<(String, i64)>, OperationError> {
    let mut stmt =
        conn.prepare("SELECT platform_id, COUNT(*) FROM releases GROUP BY platform_id")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Count media entries grouped by platform (via releases join).
pub fn platform_media_counts(conn: &Connection) -> Result<Vec<(String, i64)>, OperationError> {
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

// ── Reconciliation Queries ───────────────────────────────────────────────────

/// A group of works that share the same `ScreenScraper` ID on one platform.
#[derive(Debug)]
pub struct ReconcileGroup {
    pub screenscraper_id: String,
    pub platform_id: String,
    pub work_ids: Vec<String>,
}

/// A collision between two releases that share the same natural key.
#[derive(Debug)]
pub struct ReleaseCollision {
    pub absorbed_release_id: String,
    pub surviving_release_id: String,
    pub region: String,
    pub revision: String,
    pub variant: String,
}

/// Find groups of releases sharing a `screenscraper_id` on the same platform
/// but belonging to different works.
pub fn find_reconcilable_works(conn: &Connection) -> Result<Vec<ReconcileGroup>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT r.screenscraper_id, r.platform_id,
                GROUP_CONCAT(DISTINCT r.work_id) as work_ids
         FROM releases r
         WHERE r.screenscraper_id IS NOT NULL
         GROUP BY r.screenscraper_id, r.platform_id
         HAVING COUNT(DISTINCT r.work_id) > 1",
    )?;
    let rows = stmt.query_map([], |row| {
        let work_ids_str: String = row.get(2)?;
        Ok(ReconcileGroup {
            screenscraper_id: row.get(0)?,
            platform_id: row.get(1)?,
            work_ids: work_ids_str
                .split(',')
                .map(std::string::ToString::to_string)
                .collect(),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Check for release collisions between two works — releases that share the
/// same (`platform_id`, region, revision, variant) natural key.
pub fn check_release_collision(
    conn: &Connection,
    absorbed_work_id: &str,
    surviving_work_id: &str,
) -> Result<Vec<ReleaseCollision>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT a.id, s.id, a.region, a.revision, a.variant
         FROM releases a JOIN releases s
           ON a.platform_id = s.platform_id AND a.region = s.region
              AND a.revision = s.revision AND a.variant = s.variant
         WHERE a.work_id = ?1 AND s.work_id = ?2",
    )?;
    let rows = stmt.query_map(params![absorbed_work_id, surviving_work_id], |row| {
        Ok(ReleaseCollision {
            absorbed_release_id: row.get(0)?,
            surviving_release_id: row.get(1)?,
            region: row.get(2)?,
            revision: row.get(3)?,
            variant: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Count releases belonging to a work.
pub fn count_releases_for_work(conn: &Connection, work_id: &str) -> Result<i64, OperationError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM releases WHERE work_id = ?1",
        params![work_id],
        |r| r.get(0),
    )?;
    Ok(count)
}

/// Count enriched releases for a platform (have `screenscraper_id` or `scraper_not_found`).
///
/// If `after_title` is provided, only counts releases whose title sorts at or after
/// that value (case-insensitive).
pub fn count_enriched_releases(
    conn: &Connection,
    platform_id: &str,
    after_title: Option<&str>,
) -> Result<u64, OperationError> {
    let count: i64 = if let Some(after) = after_title {
        conn.query_row(
            "SELECT COUNT(*) FROM releases
             WHERE platform_id = ?1
               AND (screenscraper_id IS NOT NULL OR scraper_not_found = 1)
               AND LOWER(title) >= LOWER(?2)",
            params![platform_id, after],
            |r| r.get(0),
        )?
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM releases
             WHERE platform_id = ?1
               AND (screenscraper_id IS NOT NULL OR scraper_not_found = 1)",
            params![platform_id],
            |r| r.get(0),
        )?
    };
    Ok(count as u64)
}

// ── Count Queries (for pagination) ──────────────────────────────────────────

/// Count releases matching a title search with optional platform filter.
pub fn count_releases_search(
    conn: &Connection,
    query: &str,
    platform_id: Option<&str>,
) -> Result<i64, OperationError> {
    let pattern = format!("%{query}%");
    let count: i64 = match platform_id {
        Some(pid) => conn.query_row(
            "SELECT COUNT(*) FROM releases WHERE title LIKE ?1 AND platform_id = ?2",
            params![pattern, pid],
            |r| r.get(0),
        )?,
        None => conn.query_row(
            "SELECT COUNT(*) FROM releases WHERE title LIKE ?1",
            params![pattern],
            |r| r.get(0),
        )?,
    };
    Ok(count)
}

/// Count media matching a `dat_name` search with optional platform filter.
pub fn count_media_search(
    conn: &Connection,
    query: &str,
    platform_id: Option<&str>,
) -> Result<i64, OperationError> {
    let pattern = format!("%{query}%");
    let count: i64 = match platform_id {
        Some(pid) => conn.query_row(
            "SELECT COUNT(*) FROM media m \
             JOIN releases r ON m.release_id = r.id \
             WHERE m.dat_name LIKE ?1 AND r.platform_id = ?2",
            params![pattern, pid],
            |r| r.get(0),
        )?,
        None => conn.query_row(
            "SELECT COUNT(*) FROM media WHERE dat_name LIKE ?1",
            params![pattern],
            |r| r.get(0),
        )?,
    };
    Ok(count)
}

/// Count works matching a `canonical_name` search.
pub fn count_works_search(conn: &Connection, query: &str) -> Result<i64, OperationError> {
    let pattern = format!("%{query}%");
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM works WHERE canonical_name LIKE ?1",
        params![pattern],
        |r| r.get(0),
    )?;
    Ok(count)
}

/// Count collection entries with optional platform filter.
pub fn count_collection(
    conn: &Connection,
    platform_id: Option<&str>,
) -> Result<i64, OperationError> {
    let count: i64 = match platform_id {
        Some(pid) => conn.query_row(
            "SELECT COUNT(*) FROM collection c \
             JOIN media m ON c.media_id = m.id \
             JOIN releases r ON m.release_id = r.id \
             WHERE r.platform_id = ?1",
            params![pid],
            |r| r.get(0),
        )?,
        None => conn.query_row("SELECT COUNT(*) FROM collection", [], |r| r.get(0))?,
    };
    Ok(count)
}

// ── Company Queries ─────────────────────────────────────────────────────────

/// A company row from a query.
#[derive(Debug)]
pub struct CompanyRow {
    pub id: String,
    pub name: String,
    pub country: String,
    pub alias_count: i64,
}

/// Search companies by name (case-insensitive LIKE) with pagination.
pub fn search_companies(
    conn: &Connection,
    query: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<CompanyRow>, OperationError> {
    let pattern = format!("%{query}%");
    let sql = format!(
        "SELECT c.id, c.name, c.country, \
                (SELECT COUNT(*) FROM company_aliases ca WHERE ca.company_id = c.id) as alias_count \
         FROM companies c \
         WHERE c.name LIKE ?1 \
         ORDER BY c.name LIMIT {limit} OFFSET {offset}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![pattern], |row| {
        Ok(CompanyRow {
            id: row.get(0)?,
            name: row.get(1)?,
            country: row.get(2)?,
            alias_count: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Count companies matching a name search.
pub fn count_companies_search(conn: &Connection, query: &str) -> Result<i64, OperationError> {
    let pattern = format!("%{query}%");
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM companies WHERE name LIKE ?1",
        params![pattern],
        |r| r.get(0),
    )?;
    Ok(count)
}

/// List collection entries with optional platform filter and pagination.
pub fn list_collection_paged(
    conn: &Connection,
    platform_id: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Vec<CollectionRow>, OperationError> {
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
                 LIMIT {limit} OFFSET {offset}"
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
                 LIMIT {limit} OFFSET {offset}"
            ),
            vec![],
        ),
    };

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = param_values
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect();
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok(CollectionRow {
            collection_id: CollectionId(row.get(0)?),
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

// ── Row Mapping Helpers ─────────────────────────────────────────────────────

fn row_to_media(row: &rusqlite::Row<'_>) -> rusqlite::Result<Media> {
    let status_str: String = row.get(6)?;
    let tag_str: Option<String> = row.get(7)?;
    Ok(Media {
        id: row.get(0)?,
        release_id: row.get(1)?,
        media_serial: row.get(2)?,
        disc_number: row.get(3)?,
        disc_label: row.get(4)?,
        revision: row.get(5)?,
        status: MediaStatus::from_str_loose(&status_str),
        tag: tag_str.as_deref().and_then(CatalogTag::from_str_loose),
        dat_name: row.get(8)?,
        rom_name: row.get(9)?,
        dat_source: row.get(10)?,
        dat_system: row.get(11)?,
        file_size: row.get(12)?,
        crc32: row.get(13)?,
        sha1: row.get(14)?,
        md5: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

fn row_to_work(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkRow> {
    let tag_str: Option<String> = row.get(2)?;
    Ok(WorkRow {
        id: row.get(0)?,
        canonical_name: row.get(1)?,
        tag: tag_str.as_deref().and_then(CatalogTag::from_str_loose),
    })
}

fn row_to_asset(row: &rusqlite::Row<'_>) -> rusqlite::Result<Asset> {
    let release_id: Option<String> = row.get(1)?;
    let media_id: Option<String> = row.get(2)?;
    // The schema CHECK guarantees exactly one owner column is set.
    let owner = match (release_id, media_id) {
        (Some(id), None) => AssetOwner::Release(id),
        (None, Some(id)) => AssetOwner::Media(id),
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                "media_assets row must have exactly one of release_id/media_id".into(),
            ));
        }
    };
    Ok(Asset {
        id: MediaAssetId(row.get(0)?),
        owner,
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
        screen_title: row.get(16)?,
        cover_title: row.get(17)?,
        screenscraper_id: row.get(18)?,
        scraper_not_found: row.get(19)?,
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
    })
}
