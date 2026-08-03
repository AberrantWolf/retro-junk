//! CRUD operations for all catalog entity types.

use retro_junk_catalog::types::{
    Asset, CatalogPlatform, CatalogTag, CollectionEntry, Company, Disagreement, DisagreementId,
    ImportLog, ImportLogId, Media, MediaStatus, MediaType, PlatformRelationship, Release,
};
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OperationError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Entity not found: {entity_type} with id '{id}'")]
    NotFound { entity_type: String, id: String },
    #[error("Invalid field: {0}")]
    InvalidField(String),
    #[error("Invalid data: {0}")]
    InvalidData(String),
}

// ── Platform Operations ─────────────────────────────────────────────────────

/// Insert or update a platform from catalog data.
pub fn upsert_platform(
    conn: &Connection,
    platform: &CatalogPlatform,
) -> Result<(), OperationError> {
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
            media_type_str(platform.media_type),
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

    // Note: relationships are NOT inserted here because referenced platforms
    // may not exist yet. seed_from_catalog() handles relationships in a second
    // pass after all platforms have been inserted.

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
    let mut stmt = conn
        .prepare("SELECT company_id FROM company_aliases WHERE LOWER(alias) = LOWER(?1) LIMIT 1")?;
    let result = stmt.query_row(params![alias], |row| row.get::<_, String>(0));
    match result {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

// ── Work Operations ─────────────────────────────────────────────────────────

/// Insert a new work. Returns the generated ID.
pub fn insert_work(
    conn: &Connection,
    id: &str,
    canonical_name: &str,
) -> Result<(), OperationError> {
    conn.execute(
        "INSERT INTO works (id, canonical_name) VALUES (?1, ?2)",
        params![id, canonical_name],
    )?;
    Ok(())
}

/// Find a work by canonical name (exact match).
pub fn find_work_by_name(conn: &Connection, name: &str) -> Result<Option<String>, OperationError> {
    let mut stmt = conn.prepare("SELECT id FROM works WHERE canonical_name = ?1 LIMIT 1")?;
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
        "INSERT INTO releases (id, work_id, platform_id, region, revision, variant,
             title, alt_title, publisher_id, developer_id, release_date, game_serial,
             genre, players, rating, description, screen_title, cover_title,
             screenscraper_id, scraper_not_found)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
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
             screen_title = excluded.screen_title,
             cover_title = excluded.cover_title,
             screenscraper_id = excluded.screenscraper_id,
             scraper_not_found = excluded.scraper_not_found,
             updated_at = datetime('now')",
        params![
            release.id,
            release.work_id,
            release.platform_id,
            release.region,
            release.revision,
            release.variant,
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
            release.screen_title,
            release.cover_title,
            release.screenscraper_id,
            release.scraper_not_found,
        ],
    )?;
    Ok(())
}

/// Find a release by the natural key (work + platform + region + revision + variant).
pub fn find_release(
    conn: &Connection,
    work_id: &str,
    platform_id: &str,
    region: &str,
    revision: &str,
    variant: &str,
) -> Result<Option<Release>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, work_id, platform_id, region, revision, variant,
                title, alt_title, publisher_id, developer_id, release_date,
                game_serial, genre, players, rating, description,
                screen_title, cover_title,
                screenscraper_id, scraper_not_found, created_at, updated_at
         FROM releases
         WHERE work_id = ?1 AND platform_id = ?2 AND region = ?3
           AND revision = ?4 AND variant = ?5",
    )?;
    let result = stmt.query_row(
        params![work_id, platform_id, region, revision, variant],
        |row| {
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
        },
    );
    match result {
        Ok(r) => Ok(Some(r)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Fields scraped from `ScreenScraper` for one release. Empty strings mean the
/// scraper had no value; `publisher_id`/`developer_id` are nullable FKs and
/// `rating` has no empty sentinel, so those stay `Option`.
pub struct ReleaseEnrichment<'a> {
    pub screenscraper_id: &'a str,
    pub title: &'a str,
    pub release_date: &'a str,
    pub genre: &'a str,
    pub players: &'a str,
    pub rating: Option<f64>,
    pub description: &'a str,
    pub publisher_id: Option<&'a str>,
    pub developer_id: Option<&'a str>,
}

/// Update release fields from `ScreenScraper` enrichment.
///
/// Only fills fields that are currently unset, preserving values already set
/// by DAT import. The `screenscraper_id` is always set to mark this release as
/// enriched.
pub fn update_release_enrichment(
    conn: &Connection,
    release_id: &str,
    enrichment: &ReleaseEnrichment<'_>,
) -> Result<(), OperationError> {
    conn.execute(
        "UPDATE releases SET
             screenscraper_id = ?2,
             scraper_not_found = 0,
             release_date = CASE WHEN release_date = '' THEN ?3 ELSE release_date END,
             genre = CASE WHEN genre = '' THEN ?4 ELSE genre END,
             players = CASE WHEN players = '' THEN ?5 ELSE players END,
             rating = COALESCE(rating, ?6),
             description = CASE WHEN description = '' THEN ?7 ELSE description END,
             publisher_id = COALESCE(publisher_id, ?8),
             developer_id = COALESCE(developer_id, ?9),
             updated_at = datetime('now')
         WHERE id = ?1",
        params![
            release_id,
            enrichment.screenscraper_id,
            enrichment.release_date,
            enrichment.genre,
            enrichment.players,
            enrichment.rating,
            enrichment.description,
            enrichment.publisher_id,
            enrichment.developer_id,
        ],
    )?;
    // Title handled separately — only record as alt_title when it differs from
    // the DAT-imported title and no alt_title is set yet.
    if !enrichment.title.is_empty() {
        conn.execute(
            "UPDATE releases SET alt_title = ?2, updated_at = datetime('now')
             WHERE id = ?1 AND alt_title = '' AND title != ?2",
            params![release_id, enrichment.title],
        )?;
    }
    Ok(())
}

/// Mark a release as not found on `ScreenScraper`.
pub fn mark_release_not_found(conn: &Connection, release_id: &str) -> Result<(), OperationError> {
    conn.execute(
        "UPDATE releases SET scraper_not_found = 1, updated_at = datetime('now') WHERE id = ?1",
        params![release_id],
    )?;
    Ok(())
}

/// Clear all not-found flags for a platform (used with --force).
pub fn clear_not_found_flags(conn: &Connection, platform_id: &str) -> Result<u64, OperationError> {
    let changed = conn.execute(
        "UPDATE releases SET scraper_not_found = 0, updated_at = datetime('now')
         WHERE platform_id = ?1 AND scraper_not_found = 1",
        params![platform_id],
    )?;
    Ok(changed as u64)
}

/// Clear enrichment status (`screenscraper_id` and `scraper_not_found`) for releases.
///
/// If `after_title` is provided, only affects releases whose title sorts at or after
/// that value (case-insensitive). Returns the number of releases updated.
pub fn unenrich_releases(
    conn: &Connection,
    platform_id: &str,
    after_title: Option<&str>,
) -> Result<u64, OperationError> {
    let changed = if let Some(after) = after_title {
        conn.execute(
            "UPDATE releases SET
                 screenscraper_id = NULL,
                 scraper_not_found = 0,
                 updated_at = datetime('now')
             WHERE platform_id = ?1
               AND (screenscraper_id IS NOT NULL OR scraper_not_found = 1)
               AND LOWER(title) >= LOWER(?2)",
            params![platform_id, after],
        )?
    } else {
        conn.execute(
            "UPDATE releases SET
                 screenscraper_id = NULL,
                 scraper_not_found = 0,
                 updated_at = datetime('now')
             WHERE platform_id = ?1
               AND (screenscraper_id IS NOT NULL OR scraper_not_found = 1)",
            params![platform_id],
        )?
    };
    Ok(changed as u64)
}

// ── Media Operations ────────────────────────────────────────────────────────

/// Insert or update a media entry.
pub fn upsert_media(conn: &Connection, media: &Media) -> Result<(), OperationError> {
    conn.execute(
        "INSERT INTO media (id, release_id, media_serial, disc_number, disc_label,
             revision, status, tag, dat_name, rom_name, dat_source, file_size, crc32, sha1, md5)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(id) DO UPDATE SET
             release_id = excluded.release_id,
             media_serial = excluded.media_serial,
             disc_number = excluded.disc_number,
             disc_label = excluded.disc_label,
             revision = excluded.revision,
             status = excluded.status,
             tag = excluded.tag,
             dat_name = excluded.dat_name,
             rom_name = excluded.rom_name,
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
            media.tag.map(|t| t.as_str()),
            media.dat_name,
            media.rom_name,
            media.dat_source,
            media.file_size,
            media.crc32,
            media.sha1,
            media.md5,
        ],
    )?;
    conn.execute(
        "DELETE FROM media_serial_keys WHERE media_id=?1",
        params![media.id],
    )?;
    for key in crate::schema::serial_keys(&media.media_serial) {
        conn.execute(
            "INSERT INTO media_serial_keys(media_id,serial_key) VALUES(?1,?2)",
            params![media.id, key],
        )?;
    }
    Ok(())
}

/// Find media by DAT name (exact match).
pub fn find_media_by_dat_name(
    conn: &Connection,
    dat_name: &str,
) -> Result<Option<Media>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, release_id, media_serial, disc_number, disc_label,
                revision, status, tag, dat_name, rom_name, dat_source, file_size,
                crc32, sha1, md5, created_at, updated_at
         FROM media WHERE dat_name = ?1 LIMIT 1",
    )?;
    row_to_media(&mut stmt, params![dat_name])
}

/// Find the specific ROM representation attached to a release.
///
/// A DAT can contain multiple records with the same game name but different
/// ROM byte orders or container formats (notably N64 `.z64` and `.v64`).  The
/// ROM filename, not the display/game name, distinguishes those fingerprints.
pub fn find_media_by_release_and_rom_name(
    conn: &Connection,
    release_id: &str,
    rom_name: &str,
) -> Result<Option<Media>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT id, release_id, media_serial, disc_number, disc_label,
                revision, status, tag, dat_name, rom_name, dat_source, file_size,
                crc32, sha1, md5, created_at, updated_at
         FROM media WHERE release_id = ?1 AND rom_name = ?2 LIMIT 1",
    )?;
    row_to_media(&mut stmt, params![release_id, rom_name])
}

fn row_to_media(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> Result<Option<Media>, OperationError> {
    let result = stmt.query_row(params, |row| {
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
            file_size: row.get(11)?,
            crc32: row.get(12)?,
            sha1: row.get(13)?,
            md5: row.get(14)?,
            created_at: row.get(15)?,
            updated_at: row.get(16)?,
        })
    });
    match result {
        Ok(m) => Ok(Some(m)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

// ── Media Track Operations ─────────────────────────────────────────────────

/// A single track within a disc-based media entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTrack {
    pub media_id: String,
    pub track_number: i32,
    pub track_name: String,
    /// Size in bytes. 0 = unknown.
    pub file_size: i64,
    pub crc32: String,
    pub sha1: String,
    pub md5: String,
}

/// Insert a media track entry.
pub fn insert_media_track(conn: &Connection, track: &MediaTrack) -> Result<(), OperationError> {
    conn.execute(
        "INSERT INTO media_tracks (media_id, track_number, track_name, file_size, crc32, sha1, md5)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(media_id, track_number) DO UPDATE SET
             track_name = excluded.track_name,
             file_size = excluded.file_size,
             crc32 = excluded.crc32,
             sha1 = excluded.sha1,
             md5 = excluded.md5",
        params![
            track.media_id,
            track.track_number,
            track.track_name,
            track.file_size,
            track.crc32,
            track.sha1,
            track.md5,
        ],
    )?;
    Ok(())
}

/// Find all tracks for a given media entry, ordered by track number.
pub fn find_media_tracks(
    conn: &Connection,
    media_id: &str,
) -> Result<Vec<MediaTrack>, OperationError> {
    let mut stmt = conn.prepare(
        "SELECT media_id, track_number, track_name, file_size, crc32, sha1, md5
         FROM media_tracks WHERE media_id = ?1 ORDER BY track_number",
    )?;
    let tracks = stmt
        .query_map(params![media_id], |row| {
            Ok(MediaTrack {
                media_id: row.get(0)?,
                track_number: row.get(1)?,
                track_name: row.get(2)?,
                file_size: row.get(3)?,
                crc32: row.get(4)?,
                sha1: row.get(5)?,
                md5: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tracks)
}

/// Find every track belonging to a cluster of catalog media in one query.
pub fn find_media_tracks_for_media_ids(
    conn: &Connection,
    media_ids: &[String],
) -> Result<Vec<MediaTrack>, OperationError> {
    if media_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    // Stay below SQLite's commonly configured 999-variable limit while
    // retaining clustered lookups rather than falling back to one query per
    // candidate.
    for cluster in media_ids.chunks(900) {
        let placeholders = std::iter::repeat_n("?", cluster.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT media_id,track_number,track_name,file_size,crc32,sha1,md5 \
             FROM media_tracks WHERE media_id IN ({placeholders}) \
             ORDER BY media_id,track_number"
        );
        found.extend(
            conn.prepare(&sql)?
                .query_map(rusqlite::params_from_iter(cluster), |row| {
                    Ok(MediaTrack {
                        media_id: row.get(0)?,
                        track_number: row.get(1)?,
                        track_name: row.get(2)?,
                        file_size: row.get(3)?,
                        crc32: row.get(4)?,
                        sha1: row.get(5)?,
                        md5: row.get(6)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    Ok(found)
}

// ── Tag Operations ─────────────────────────────────────────────────────────

/// Set or clear a tag on a Work.
pub fn set_work_tag(
    conn: &Connection,
    work_id: &str,
    tag: Option<CatalogTag>,
) -> Result<(), OperationError> {
    let changed = conn.execute(
        "UPDATE works SET tag = ?2, updated_at = datetime('now') WHERE id = ?1",
        params![work_id, tag.map(|t| t.as_str())],
    )?;
    if changed == 0 {
        return Err(OperationError::NotFound {
            entity_type: "work".to_string(),
            id: work_id.to_string(),
        });
    }
    Ok(())
}

/// Set or clear a tag on a Media entry.
pub fn set_media_tag(
    conn: &Connection,
    media_id: &str,
    tag: Option<CatalogTag>,
) -> Result<(), OperationError> {
    let changed = conn.execute(
        "UPDATE media SET tag = ?2, updated_at = datetime('now') WHERE id = ?1",
        params![media_id, tag.map(|t| t.as_str())],
    )?;
    if changed == 0 {
        return Err(OperationError::NotFound {
            entity_type: "media".to_string(),
            id: media_id.to_string(),
        });
    }
    Ok(())
}

/// The medium a homebrew work's single release holds.
///
/// Derived, never stored: the same three inputs always name the same row, so
/// applying a mark twice — here and on another machine — lands on it rather
/// than minting a second. One definition, because two would differ silently.
#[must_use]
pub fn homebrew_media_id(work_id: &str, platform_id: &str, region: &str) -> String {
    format!("{work_id}:{platform_id}:{region}:media")
}

/// Create a homebrew Work with a Release and empty Media entry in a transaction.
///
/// Returns the created Work ID.
pub fn create_homebrew_work(
    conn: &Connection,
    name: &str,
    platform_id: &str,
    region: &str,
) -> Result<String, OperationError> {
    let slug = slugify(name);
    let work_id = format!("{platform_id}:homebrew:{slug}");
    let release_id = format!("{work_id}:{platform_id}:{region}");
    let media_id = homebrew_media_id(&work_id, platform_id, region);

    conn.execute(
        "INSERT INTO works (id, canonical_name, tag) VALUES (?1, ?2, 'homebrew')
         ON CONFLICT(id) DO UPDATE SET canonical_name = excluded.canonical_name, updated_at = datetime('now')",
        params![work_id, name],
    )?;
    conn.execute(
        "INSERT INTO releases (id, work_id, platform_id, region, title)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET title = excluded.title, updated_at = datetime('now')",
        params![release_id, work_id, platform_id, region, name],
    )?;
    conn.execute(
        "INSERT INTO media (id, release_id, tag) VALUES (?1, ?2, 'homebrew')
         ON CONFLICT(id) DO UPDATE SET tag = 'homebrew', updated_at = datetime('now')",
        params![media_id, release_id],
    )?;

    Ok(work_id)
}

/// Hash parameters for creating a media entry.
#[derive(Debug, Clone)]
pub struct MediaHashes {
    pub crc32: String,
    pub sha1: Option<String>,
    pub md5: Option<String>,
    pub file_size: i64,
}

/// Create a modded Media entry linked to an existing Work.
///
/// Finds or creates a Release under the given Work, then creates
/// a Media entry tagged as modded. Returns the created Media ID.
pub fn create_modded_media(
    conn: &Connection,
    work_id: &str,
    platform_id: &str,
    region: &str,
    disc_number: Option<u32>,
    hashes: Option<&MediaHashes>,
) -> Result<String, OperationError> {
    if disc_number == Some(0) {
        return Err(OperationError::InvalidField(
            "disc number must be greater than zero".to_owned(),
        ));
    }
    // Find an existing release or create one
    let release_id = if let Some(r) = find_release(conn, work_id, platform_id, region, "", "")? {
        r.id
    } else {
        let rid = format!("{work_id}:{platform_id}:{region}:modded");
        // Get work name for the release title
        let work_name: String = conn
            .query_row(
                "SELECT canonical_name FROM works WHERE id = ?1",
                params![work_id],
                |row| row.get(0),
            )
            .map_err(|_| OperationError::NotFound {
                entity_type: "work".to_string(),
                id: work_id.to_string(),
            })?;
        conn.execute(
            "INSERT INTO releases (id, work_id, platform_id, region, title)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![rid, work_id, platform_id, region, work_name],
        )?;
        rid
    };

    // Use hash or system time for uniqueness
    let media_suffix = match hashes {
        Some(h) => h.crc32.clone(),
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string(),
    };
    let disc_number = disc_number.unwrap_or(0);
    let media_scope = if disc_number == 0 {
        "game".to_owned()
    } else {
        format!("disc-{disc_number}")
    };
    let media_id = format!("{release_id}:modded:{media_scope}:{media_suffix}");
    let (crc32, sha1, md5, file_size) = match hashes {
        Some(h) => (
            h.crc32.as_str(),
            h.sha1.as_deref().unwrap_or(""),
            h.md5.as_deref().unwrap_or(""),
            h.file_size,
        ),
        None => ("", "", "", 0),
    };

    conn.execute(
        "INSERT INTO media (id, release_id, tag, disc_number, crc32, sha1, md5, file_size)
         VALUES (?1, ?2, 'modded', ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
           tag = 'modded', disc_number = excluded.disc_number,
           crc32 = excluded.crc32, sha1 = excluded.sha1,
           md5 = excluded.md5, file_size = excluded.file_size, updated_at = datetime('now')",
        params![
            media_id,
            release_id,
            disc_number,
            crc32,
            sha1,
            md5,
            file_size
        ],
    )?;

    Ok(media_id)
}

/// Remove a modded Media entry and clean up its Release if no other media reference it.
pub fn detach_modded_media(conn: &Connection, media_id: &str) -> Result<(), OperationError> {
    let release_id: String = conn
        .query_row(
            "SELECT release_id FROM media WHERE id = ?1",
            params![media_id],
            |row| row.get(0),
        )
        .map_err(|_| OperationError::NotFound {
            entity_type: "media".to_string(),
            id: media_id.to_string(),
        })?;

    conn.execute("DELETE FROM media WHERE id = ?1", params![media_id])?;

    // Clean up release if no other media reference it
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM media WHERE release_id = ?1",
        params![release_id],
        |row| row.get(0),
    )?;
    if count == 0 {
        conn.execute("DELETE FROM releases WHERE id = ?1", params![release_id])?;
    }

    Ok(())
}

// ── Media Asset Operations ──────────────────────────────────────────────────

/// Insert an asset.
pub fn insert_asset(
    conn: &Connection,
    asset: &Asset,
) -> Result<retro_junk_catalog::MediaAssetId, OperationError> {
    conn.execute(
        "INSERT INTO media_assets (release_id, media_id, asset_type, region, source,
             file_path, source_url, scraped, file_hash, width, height)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            asset.owner.release_id(),
            asset.owner.media_id(),
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
    Ok(retro_junk_catalog::MediaAssetId(
        conn.last_insert_rowid() as u64
    ))
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
pub fn insert_import_log(
    conn: &Connection,
    log: &ImportLog,
) -> Result<ImportLogId, OperationError> {
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
    Ok(ImportLogId(conn.last_insert_rowid() as u64))
}

// ── Disagreement Operations ─────────────────────────────────────────────────

/// Insert a disagreement record.
pub fn insert_disagreement(
    conn: &Connection,
    d: &Disagreement,
) -> Result<DisagreementId, OperationError> {
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
    Ok(DisagreementId(conn.last_insert_rowid() as u64))
}

/// Resolve a disagreement.
pub fn resolve_disagreement(
    conn: &Connection,
    id: DisagreementId,
    resolution: &str,
) -> Result<(), OperationError> {
    let changed = conn.execute(
        "UPDATE disagreements SET resolved = 1, resolution = ?2, resolved_at = datetime('now')
         WHERE id = ?1",
        params![id.0, resolution],
    )?;
    if changed == 0 {
        return Err(OperationError::NotFound {
            entity_type: "disagreement".to_string(),
            id: id.to_string(),
        });
    }
    Ok(())
}

/// Apply a disagreement resolution by updating the entity field to the chosen value.
///
/// Only allows updates to a whitelist of safe fields.
pub fn apply_disagreement_resolution(
    conn: &Connection,
    entity_type: &str,
    entity_id: &str,
    field: &str,
    value: &str,
) -> Result<(), OperationError> {
    let safe_fields = [
        "title",
        "alt_title",
        "release_date",
        "game_serial",
        "genre",
        "players",
        "description",
        "media_serial",
        "revision",
        "status",
    ];

    if !safe_fields.contains(&field) {
        return Err(OperationError::InvalidField(format!(
            "Field '{field}' cannot be updated via resolution"
        )));
    }

    let table = match entity_type {
        "release" => "releases",
        "media" => "media",
        _ => {
            return Err(OperationError::InvalidField(format!(
                "Unknown entity type '{entity_type}'"
            )));
        }
    };

    let sql =
        format!("UPDATE {table} SET {field} = ?1, updated_at = datetime('now') WHERE id = ?2");

    let changed = conn.execute(&sql, params![value, entity_id])?;
    if changed == 0 {
        return Err(OperationError::NotFound {
            entity_type: entity_type.to_string(),
            id: entity_id.to_string(),
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
         ON CONFLICT(entity_type, entity_id, platform_id, dat_name_pattern, field) DO UPDATE SET
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
/// into the `SQLite` database. Safe to call repeatedly (uses upsert).
pub fn seed_from_catalog(
    conn: &Connection,
    catalog_dir: &std::path::Path,
) -> Result<SeedStats, OperationError> {
    let (platforms, companies, overrides) = retro_junk_catalog::yaml::load_catalog(catalog_dir)
        .map_err(|e| {
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
                    relationship_str(rel.relationship_type),
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

// ── Reconciliation Operations ────────────────────────────────────────────────

/// Move all releases from one work to another.
pub fn update_releases_work_id(
    conn: &Connection,
    old_work_id: &str,
    new_work_id: &str,
) -> Result<u64, OperationError> {
    let changed = conn.execute(
        "UPDATE releases SET work_id = ?2, updated_at = datetime('now') WHERE work_id = ?1",
        params![old_work_id, new_work_id],
    )?;
    Ok(changed as u64)
}

/// Move all media from one release to another.
pub fn move_media_to_release(
    conn: &Connection,
    from_release_id: &str,
    to_release_id: &str,
) -> Result<u64, OperationError> {
    let changed = conn.execute(
        "UPDATE media SET release_id = ?2, updated_at = datetime('now') WHERE release_id = ?1",
        params![from_release_id, to_release_id],
    )?;
    Ok(changed as u64)
}

/// Move all media assets from one release to another.
pub fn move_assets_to_release(
    conn: &Connection,
    from_release_id: &str,
    to_release_id: &str,
) -> Result<u64, OperationError> {
    let changed = conn.execute(
        "UPDATE media_assets SET release_id = ?2 WHERE release_id = ?1",
        params![from_release_id, to_release_id],
    )?;
    Ok(changed as u64)
}

/// Move disagreements referencing one release entity to another.
pub fn move_disagreements_for_release(
    conn: &Connection,
    from_release_id: &str,
    to_release_id: &str,
) -> Result<u64, OperationError> {
    let changed = conn.execute(
        "UPDATE disagreements SET entity_id = ?2 WHERE entity_type = 'release' AND entity_id = ?1",
        params![from_release_id, to_release_id],
    )?;
    Ok(changed as u64)
}

/// Delete a single release by ID.
pub fn delete_release(conn: &Connection, id: &str) -> Result<(), OperationError> {
    conn.execute("DELETE FROM releases WHERE id = ?1", params![id])?;
    Ok(())
}

/// Delete works that have no remaining releases.
pub fn delete_orphan_works(conn: &Connection) -> Result<u64, OperationError> {
    let changed = conn.execute(
        "DELETE FROM works WHERE id NOT IN (SELECT DISTINCT work_id FROM releases)",
        [],
    )?;
    Ok(changed as u64)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Convert a string to a URL-friendly slug (lowercase, hyphens, no trailing hyphen).
fn slugify(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_separator = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator && !result.is_empty() {
            result.push('-');
            last_was_separator = true;
        }
    }
    if result.ends_with('-') {
        result.pop();
    }
    result
}

fn media_type_str(mt: MediaType) -> &'static str {
    match mt {
        MediaType::Cartridge => "cartridge",
        MediaType::Disc => "disc",
        MediaType::Card => "card",
        MediaType::Digital => "digital",
    }
}

fn relationship_str(r: PlatformRelationship) -> &'static str {
    match r {
        PlatformRelationship::RegionalVariant => "regional_variant",
        PlatformRelationship::Successor => "successor",
        PlatformRelationship::Addon => "addon",
        PlatformRelationship::Compatible => "compatible",
    }
}

// ── Collection Marks ────────────────────────────────────────────────────────

/// What applying one portable mark did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMark {
    pub media_id: String,
    pub work_id: String,
    pub tag: &'static str,
}

/// Apply one user decision to the catalog, creating whatever it names.
///
/// Marks carry the *inputs* that catalog ids are minted from — name, platform,
/// region, and the parent's DAT game name — never the ids themselves, because
/// media ids are minted per DAT release and do not survive a re-import on
/// another machine. Rebuilding the rows from those inputs is what makes a mark
/// portable; the ids come out the same because they are derived, not stored.
///
/// A mod resolves its parent work through `parent_dat_name` first, falling
/// back to `parent_work_id`. Nothing is created for a mod whose parent this
/// machine's catalog does not know — the decision is kept, waiting for the DAT
/// that gives it meaning, rather than manufacturing an orphan work.
pub fn apply_collection_mark(
    conn: &Connection,
    mark: &retro_junk_archive::CollectionMark,
) -> Result<Option<AppliedMark>, OperationError> {
    let hashes = MediaHashes {
        crc32: mark.content.crc32.clone(),
        sha1: Some(mark.content.sha1.clone()).filter(|value| !value.is_empty()),
        md5: Some(mark.content.md5.clone()).filter(|value| !value.is_empty()),
        file_size: i64::try_from(mark.content.size).unwrap_or(0),
    };
    match mark.kind {
        // A region correction creates nothing: it is applied directly to the
        // rows whose content it names, by the caller.
        retro_junk_archive::MarkKind::RegionOverride => Ok(None),
        retro_junk_archive::MarkKind::Homebrew => {
            let work_id = create_homebrew_work(conn, &mark.name, &mark.platform_id, &mark.region)?;
            // `create_homebrew_work` mints the row but records no digests, so
            // on its own the file it describes can never be matched back to
            // it. The mark is the only place those digests exist.
            let media_id = homebrew_media_id(&work_id, &mark.platform_id, &mark.region);
            set_media_hashes(conn, &media_id, &hashes)?;
            Ok(Some(AppliedMark {
                media_id,
                work_id,
                tag: "homebrew",
            }))
        }
        retro_junk_archive::MarkKind::Modded => {
            let Some(work_id) = resolve_parent_work(conn, mark)? else {
                log::debug!(
                    "Mark for {} names parent '{}', which this catalog does not have yet",
                    mark.name,
                    mark.parent_dat_name
                );
                return Ok(None);
            };
            let media_id = create_modded_media(
                conn,
                &work_id,
                &mark.platform_id,
                &mark.region,
                None,
                Some(&hashes),
            )?;
            Ok(Some(AppliedMark {
                media_id,
                work_id,
                tag: "modded",
            }))
        }
    }
}

/// The work a mod is derived from: by the parent's name where the catalog has
/// it, else by the recorded work id.
///
/// The name is tried against DAT game names first and canonical work names
/// second, because that is the order the writing side prefers them in — a work
/// with no DAT-derived medium has only a canonical name to be known by. The
/// work id comes last: it is a local shortcut that says nothing on a machine
/// whose catalog was built from a different import.
fn resolve_parent_work(
    conn: &Connection,
    mark: &retro_junk_archive::CollectionMark,
) -> Result<Option<String>, OperationError> {
    if !mark.parent_dat_name.is_empty() {
        let found: Option<String> = conn
            .query_row(
                "SELECT r.work_id FROM media m
                 JOIN releases r ON r.id=m.release_id
                 WHERE m.dat_name=?1 AND r.platform_id=?2
                 ORDER BY m.id LIMIT 1",
                params![mark.parent_dat_name, mark.platform_id],
                |row| row.get(0),
            )
            .optional()?;
        if found.is_some() {
            return Ok(found);
        }
        let by_name: Option<String> = conn
            .query_row(
                "SELECT w.id FROM works w
                 JOIN releases r ON r.work_id=w.id
                 WHERE w.canonical_name=?1 AND r.platform_id=?2
                 ORDER BY w.id LIMIT 1",
                params![mark.parent_dat_name, mark.platform_id],
                |row| row.get(0),
            )
            .optional()?;
        if by_name.is_some() {
            return Ok(by_name);
        }
    }
    if mark.parent_work_id.is_empty() {
        return Ok(None);
    }
    Ok(conn
        .query_row(
            "SELECT id FROM works WHERE id=?1",
            [&mark.parent_work_id],
            |row| row.get(0),
        )
        .optional()?)
}

fn set_media_hashes(
    conn: &Connection,
    media_id: &str,
    hashes: &MediaHashes,
) -> Result<(), OperationError> {
    conn.execute(
        "UPDATE media SET crc32=?2,sha1=?3,md5=?4,file_size=?5,updated_at=datetime('now')
         WHERE id=?1",
        params![
            media_id,
            hashes.crc32,
            hashes.sha1.clone().unwrap_or_default(),
            hashes.md5.clone().unwrap_or_default(),
            hashes.file_size,
        ],
    )?;
    Ok(())
}
