//! CRUD operations for all catalog entity types.

use retro_junk_catalog::types::*;
use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OperationError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Entity not found: {entity_type} with id '{id}'")]
    NotFound { entity_type: String, id: String },
}

// ── Platform Operations ─────────────────────────────────────────────────────

/// Insert or update a platform from catalog data.
pub fn upsert_platform(conn: &Connection, platform: &CatalogPlatform) -> Result<(), OperationError> {
    conn.execute(
        "INSERT INTO platforms (id, display_name, short_name, manufacturer, generation, media_type, release_year, description, core_platform)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
             display_name = excluded.display_name,
             short_name = excluded.short_name,
             manufacturer = excluded.manufacturer,
             generation = excluded.generation,
             media_type = excluded.media_type,
             release_year = excluded.release_year,
             description = excluded.description,
             core_platform = excluded.core_platform",
        params![
            platform.id,
            platform.display_name,
            platform.short_name,
            platform.manufacturer,
            platform.generation,
            media_type_str(&platform.media_type),
            platform.release_year,
            platform.description,
            platform.core_platform,
        ],
    )?;

    // Upsert regions
    for region in &platform.regions {
        conn.execute(
            "INSERT INTO platform_regions (platform_id, region, release_date)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(platform_id, region) DO UPDATE SET
                 release_date = excluded.release_date",
            params![platform.id, region.region, region.release_date],
        )?;
    }

    // Insert relationships (skip if referenced platform doesn't exist yet)
    for rel in &platform.relationships {
        conn.execute(
            "INSERT OR IGNORE INTO platform_relationships (platform_a, platform_b, relationship)
             VALUES (?1, ?2, ?3)",
            params![
                platform.id,
                rel.platform,
                relationship_str(&rel.relationship_type),
            ],
        )?;
    }

    Ok(())
}

// ── Company Operations ──────────────────────────────────────────────────────

/// Insert or update a company.
pub fn upsert_company(conn: &Connection, company: &Company) -> Result<(), OperationError> {
    conn.execute(
        "INSERT INTO companies (id, name, country)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             country = excluded.country",
        params![company.id, company.name, company.country],
    )?;

    // Clear and re-insert aliases
    conn.execute(
        "DELETE FROM company_aliases WHERE company_id = ?1",
        params![company.id],
    )?;
    for alias in &company.aliases {
        conn.execute(
            "INSERT INTO company_aliases (company_id, alias) VALUES (?1, ?2)",
            params![company.id, alias],
        )?;
    }

    Ok(())
}

/// Find a company by alias name (case-insensitive).
pub fn find_company_by_alias(
    conn: &Connection,
    alias: &str,
) -> Result<Option<String>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT company_id FROM company_aliases WHERE LOWER(alias) = LOWER(?1) LIMIT 1",
    )?;
    let result = stmt.query_row(params![alias], |row| row.get::<_, String>(0));
    match result {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

// ── Work Operations ─────────────────────────────────────────────────────────

/// Insert a new work. Returns the generated ID.
pub fn insert_work(conn: &Connection, id: &str, canonical_name: &str) -> Result<(), OperationError> {
    conn.execute(
        "INSERT INTO works (id, canonical_name) VALUES (?1, ?2)",
        params![id, canonical_name],
    )?;
    Ok(())
}

/// Find a work by canonical name (exact match).
pub fn find_work_by_name(conn: &Connection, name: &str) -> Result<Option<String>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id FROM works WHERE canonical_name = ?1 LIMIT 1",
    )?;
    let result = stmt.query_row(params![name], |row| row.get::<_, String>(0));
    match result {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Update a work's canonical name.
pub fn update_work_name(
    conn: &Connection,
    id: &str,
    canonical_name: &str,
) -> Result<(), OperationError> {
    let changed = conn.execute(
        "UPDATE works SET canonical_name = ?2, updated_at = datetime('now') WHERE id = ?1",
        params![id, canonical_name],
    )?;
    if changed == 0 {
        return Err(OperationError::NotFound {
            entity_type: "work".to_string(),
            id: id.to_string(),
        });
    }
    Ok(())
}

// ── Release Operations ──────────────────────────────────────────────────────

/// Insert or update a release.
pub fn upsert_release(conn: &Connection, release: &Release) -> Result<(), OperationError> {
    conn.execute(
        "INSERT INTO releases (id, work_id, platform_id, region, title, alt_title,
             publisher_id, developer_id, release_date, game_serial, genre, players,
             rating, description, screenscraper_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(id) DO UPDATE SET
             title = excluded.title,
             alt_title = excluded.alt_title,
             publisher_id = excluded.publisher_id,
             developer_id = excluded.developer_id,
             release_date = excluded.release_date,
             game_serial = excluded.game_serial,
             genre = excluded.genre,
             players = excluded.players,
             rating = excluded.rating,
             description = excluded.description,
             screenscraper_id = excluded.screenscraper_id,
             updated_at = datetime('now')",
        params![
            release.id,
            release.work_id,
            release.platform_id,
            release.region,
            release.title,
            release.alt_title,
            release.publisher_id,
            release.developer_id,
            release.release_date,
            release.game_serial,
            release.genre,
            release.players,
            release.rating,
            release.description,
            release.screenscraper_id,
        ],
    )?;
    Ok(())
}

/// Find a release by work + platform + region (the natural key).
pub fn find_release(
    conn: &Connection,
    work_id: &str,
    platform_id: &str,
    region: &str,
) -> Result<Option<Release>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, work_id, platform_id, region, title, alt_title,
                publisher_id, developer_id, release_date, game_serial,
                genre, players, rating, description, screenscraper_id,
                created_at, updated_at
         FROM releases WHERE work_id = ?1 AND platform_id = ?2 AND region = ?3",
    )?;
    let result = stmt.query_row(params![work_id, platform_id, region], |row| {
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
    });
    match result {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

// ── Media Operations ────────────────────────────────────────────────────────

/// Insert or update a media entry.
pub fn upsert_media(conn: &Connection, media: &Media) -> Result<(), OperationError> {
    conn.execute(
        "INSERT INTO media (id, release_id, media_serial, disc_number, disc_label,
             revision, status, dat_name, dat_source, file_size, crc32, sha1, md5)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(id) DO UPDATE SET
             release_id = excluded.release_id,
             media_serial = excluded.media_serial,
             disc_number = excluded.disc_number,
             disc_label = excluded.disc_label,
             revision = excluded.revision,
             status = excluded.status,
             dat_name = excluded.dat_name,
             dat_source = excluded.dat_source,
             file_size = excluded.file_size,
             crc32 = excluded.crc32,
             sha1 = excluded.sha1,
             md5 = excluded.md5,
             updated_at = datetime('now')",
        params![
            media.id,
            media.release_id,
            media.media_serial,
            media.disc_number,
            media.disc_label,
            media.revision,
            media.status.as_str(),
            media.dat_name,
            media.dat_source,
            media.file_size,
            media.crc32,
            media.sha1,
            media.md5,
        ],
    )?;
    Ok(())
}

/// Find media by DAT name (exact match).
pub fn find_media_by_dat_name(
    conn: &Connection,
    dat_name: &str,
) -> Result<Option<Media>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, release_id, media_serial, disc_number, disc_label,
                revision, status, dat_name, dat_source, file_size,
                crc32, sha1, md5, created_at, updated_at
         FROM media WHERE dat_name = ?1 LIMIT 1",
    )?;
    row_to_media(&mut stmt, params![dat_name])
}

fn row_to_media(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> Result<Option<Media>, OperationError> {
    let result = stmt.query_row(params, |row| {
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
    });
    match result {
        Ok(m) => Ok(Some(m)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

// ── Media Asset Operations ──────────────────────────────────────────────────

/// Insert a media asset.
pub fn insert_media_asset(
    conn: &Connection,
    asset: &MediaAsset,
) -> Result<i64, OperationError> {
    conn.execute(
        "INSERT INTO media_assets (release_id, media_id, asset_type, region, source,
             file_path, source_url, scraped, file_hash, width, height)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            asset.release_id,
            asset.media_id,
            asset.asset_type,
            asset.region,
            asset.source,
            asset.file_path,
            asset.source_url,
            asset.scraped,
            asset.file_hash,
            asset.width,
            asset.height,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

// ── Collection Operations ───────────────────────────────────────────────────

/// Insert or update a collection entry.
pub fn upsert_collection_entry(
    conn: &Connection,
    entry: &CollectionEntry,
) -> Result<(), OperationError> {
    conn.execute(
        "INSERT INTO collection (media_id, user_id, owned, condition, notes, date_acquired, rom_path, verified_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(media_id, user_id) DO UPDATE SET
             owned = excluded.owned,
             condition = excluded.condition,
             notes = excluded.notes,
             date_acquired = excluded.date_acquired,
             rom_path = excluded.rom_path,
             verified_at = excluded.verified_at",
        params![
            entry.media_id,
            entry.user_id,
            entry.owned,
            entry.condition,
            entry.notes,
            entry.date_acquired,
            entry.rom_path,
            entry.verified_at,
        ],
    )?;
    Ok(())
}

// ── Import Log Operations ───────────────────────────────────────────────────

/// Insert an import log entry. Returns the generated ID.
pub fn insert_import_log(conn: &Connection, log: &ImportLog) -> Result<i64, OperationError> {
    conn.execute(
        "INSERT INTO import_log (source_type, source_name, source_version, imported_at,
             records_created, records_updated, records_unchanged, disagreements_found)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            log.source_type,
            log.source_name,
            log.source_version,
            log.imported_at,
            log.records_created,
            log.records_updated,
            log.records_unchanged,
            log.disagreements_found,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

// ── Disagreement Operations ─────────────────────────────────────────────────

/// Insert a disagreement record.
pub fn insert_disagreement(
    conn: &Connection,
    d: &Disagreement,
) -> Result<i64, OperationError> {
    conn.execute(
        "INSERT INTO disagreements (entity_type, entity_id, field, source_a, value_a,
             source_b, value_b)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            d.entity_type,
            d.entity_id,
            d.field,
            d.source_a,
            d.value_a,
            d.source_b,
            d.value_b,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Resolve a disagreement.
pub fn resolve_disagreement(
    conn: &Connection,
    id: i64,
    resolution: &str,
) -> Result<(), OperationError> {
    let changed = conn.execute(
        "UPDATE disagreements SET resolved = 1, resolution = ?2, resolved_at = datetime('now')
         WHERE id = ?1",
        params![id, resolution],
    )?;
    if changed == 0 {
        return Err(OperationError::NotFound {
            entity_type: "disagreement".to_string(),
            id: id.to_string(),
        });
    }
    Ok(())
}

// ── Override Operations ─────────────────────────────────────────────────────

/// Insert or update an override from YAML.
pub fn upsert_override(
    conn: &Connection,
    ovr: &retro_junk_catalog::types::Override,
) -> Result<(), OperationError> {
    conn.execute(
        "INSERT INTO overrides (entity_type, entity_id, platform_id, dat_name_pattern,
             field, override_value, reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(entity_type, entity_id, field) DO UPDATE SET
             platform_id = excluded.platform_id,
             dat_name_pattern = excluded.dat_name_pattern,
             override_value = excluded.override_value,
             reason = excluded.reason",
        params![
            ovr.entity_type,
            ovr.entity_id,
            ovr.platform_id,
            ovr.dat_name_pattern,
            ovr.field,
            ovr.override_value,
            ovr.reason,
        ],
    )?;
    Ok(())
}

// ── Seed Loading ────────────────────────────────────────────────────────────

/// Load all YAML catalog data into the database.
///
/// This loads platforms, companies, and overrides from the catalog directory
/// into the SQLite database. Safe to call repeatedly (uses upsert).
pub fn seed_from_catalog(
    conn: &Connection,
    catalog_dir: &std::path::Path,
) -> Result<SeedStats, OperationError> {
    let (platforms, companies, overrides) =
        retro_junk_catalog::yaml::load_catalog(catalog_dir).map_err(|e| {
            OperationError::Sqlite(rusqlite::Error::InvalidParameterName(e.to_string()))
        })?;

    let mut stats = SeedStats::default();

    for platform in &platforms {
        upsert_platform(conn, platform)?;
        stats.platforms += 1;
    }

    // Second pass for relationships (all platforms now exist)
    for platform in &platforms {
        for rel in &platform.relationships {
            conn.execute(
                "INSERT OR IGNORE INTO platform_relationships (platform_a, platform_b, relationship)
                 VALUES (?1, ?2, ?3)",
                params![
                    platform.id,
                    rel.platform,
                    relationship_str(&rel.relationship_type),
                ],
            )?;
        }
    }

    for company in &companies {
        upsert_company(conn, company)?;
        stats.companies += 1;
    }

    for ovr in &overrides {
        upsert_override(conn, ovr)?;
        stats.overrides += 1;
    }

    Ok(stats)
}

/// Statistics from seeding the database.
#[derive(Debug, Default)]
pub struct SeedStats {
    pub platforms: usize,
    pub companies: usize,
    pub overrides: usize,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn media_type_str(mt: &MediaType) -> &'static str {
    match mt {
        MediaType::Cartridge => "cartridge",
        MediaType::Disc => "disc",
        MediaType::Card => "card",
        MediaType::Digital => "digital",
    }
}

fn relationship_str(r: &PlatformRelationship) -> &'static str {
    match r {
        PlatformRelationship::RegionalVariant => "regional_variant",
        PlatformRelationship::Successor => "successor",
        PlatformRelationship::Addon => "addon",
        PlatformRelationship::Compatible => "compatible",
    }
}