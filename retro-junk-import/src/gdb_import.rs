//! GameDataBase (GDB) catalog enrichment.
//!
//! Enriches catalog releases with metadata from PigSaint's GameDataBase:
//! Japanese titles, developer/publisher, release dates, genre, player count.
//! Matches are performed by SHA1 hash from media entries.
//!
//! Data source: <https://github.com/PigSaint/GameDataBase>
//! License: CC BY 4.0 — Attribution to PigSaint required.

use retro_junk_catalog::types::Company;
use retro_junk_dat::gdb::{self, GdbGame};
use retro_junk_dat::gdb_cache;
use retro_junk_db::{Connection, operations, queries};
use rusqlite::params;

use crate::ImportError;
use crate::merge;
use crate::slugify;

/// Statistics from a GDB enrichment run.
#[derive(Debug, Default)]
pub struct GdbEnrichStats {
    /// Total media entries checked
    pub media_checked: u32,
    /// Media entries matched in GDB by SHA1
    pub matched: u32,
    /// Releases updated with new data
    pub enriched: u32,
    /// Fields where GDB and existing data disagree
    pub disagreements: u32,
    /// Media entries with no SHA1 hash (skipped)
    pub skipped_no_hash: u32,
    /// Companies created during enrichment
    pub companies_created: u32,
}

/// Options for GDB enrichment.
pub struct GdbEnrichOptions {
    /// Platform ID to enrich (e.g., "nes", "snes")
    pub platform_id: String,
    /// Maximum releases to process (None = all)
    pub limit: Option<u32>,
    /// Directory containing GDB CSV files (None = use cache)
    pub gdb_dir: Option<std::path::PathBuf>,
}

/// Enrich catalog releases for a platform using GDB data.
///
/// For each media entry with a SHA1, looks up the hash in the GDB index.
/// When found, fills in missing alt_title, developer, publisher, release_date,
/// genre, and players on the parent release.
pub fn enrich_gdb(
    conn: &Connection,
    csv_names: &[&str],
    options: &GdbEnrichOptions,
) -> Result<GdbEnrichStats, ImportError> {
    let mut stats = GdbEnrichStats::default();

    // Load GDB index
    let index = if let Some(ref dir) = options.gdb_dir {
        gdb_cache::load_gdb_index_from_dir(csv_names, dir)
            .map_err(|e| ImportError::Dat(e.to_string()))?
    } else {
        gdb_cache::load_gdb_index(csv_names).map_err(|e| ImportError::Dat(e.to_string()))?
    };

    log::info!(
        "Loaded GDB index: {} games, {} SHA1 entries",
        index.len(),
        index.sha1_count()
    );

    // Get all releases for this platform (with media)
    let releases = queries::releases_for_platform(conn, &options.platform_id)?;
    let release_count = releases.len();
    let limit = options.limit.unwrap_or(u32::MAX) as usize;

    log::info!(
        "Processing {} releases for platform '{}'",
        release_count.min(limit),
        options.platform_id,
    );

    for (i, release) in releases.iter().enumerate() {
        if i >= limit {
            break;
        }

        // Get media for this release
        let media_list = queries::media_for_release(conn, &release.id)?;

        for media in &media_list {
            stats.media_checked += 1;

            let sha1 = match media.sha1.as_deref() {
                Some(h) if !h.is_empty() => h,
                _ => {
                    stats.skipped_no_hash += 1;
                    continue;
                }
            };

            // Try SHA1 lookup, fall back to MD5
            let gdb_game = index
                .lookup_sha1(sha1)
                .or_else(|| media.md5.as_deref().and_then(|md5| index.lookup_md5(md5)));

            let gdb_game = match gdb_game {
                Some(g) => g,
                None => continue,
            };

            stats.matched += 1;

            // Enrich the parent release
            let updated = enrich_release(conn, &release.id, release, gdb_game, &mut stats)?;
            if updated {
                stats.enriched += 1;
            }

            // Only need one match per release — break after first matched media
            break;
        }
    }

    Ok(stats)
}

/// Enrich a single release with GDB data. Returns true if any field was updated.
fn enrich_release(
    conn: &Connection,
    release_id: &str,
    release: &retro_junk_catalog::types::Release,
    gdb_game: &GdbGame,
    stats: &mut GdbEnrichStats,
) -> Result<bool, ImportError> {
    let mut updated = false;
    let source = "gdb";

    // Extract native (Japanese) title from screen_title
    let (_, native_title) = gdb::split_title(&gdb_game.screen_title);

    // -- alt_title --
    if let Some(native) = native_title {
        if release.alt_title.is_none() {
            conn.execute(
                "UPDATE releases SET alt_title = ?2, updated_at = datetime('now') WHERE id = ?1 AND alt_title IS NULL",
                params![release_id, native],
            )?;
            updated = true;
        } else {
            // Check for disagreement with existing alt_title
            let disagreed = merge::check_field(
                conn,
                "release",
                release_id,
                "alt_title",
                "screenscraper",
                release.alt_title.as_deref(),
                source,
                Some(native),
            )?;
            if disagreed {
                stats.disagreements += 1;
            }
        }
    }

    // -- screen_title (native/original language portion only) --
    // Overwrite if NULL or if existing value contains '@' (stale full-string format)
    let (_, native_screen) = gdb::split_title(&gdb_game.screen_title);
    if let Some(native) = native_screen {
        let needs_update = release.screen_title.is_none()
            || release
                .screen_title
                .as_deref()
                .is_some_and(|s| s.contains('@'));
        if needs_update {
            conn.execute(
                "UPDATE releases SET screen_title = ?2, updated_at = datetime('now') WHERE id = ?1",
                params![release_id, native],
            )?;
            updated = true;
        }
    }

    // -- cover_title (native/original language portion only) --
    let (_, native_cover) = gdb::split_title(&gdb_game.cover_title);
    if let Some(native) = native_cover {
        let needs_update = release.cover_title.is_none()
            || release
                .cover_title
                .as_deref()
                .is_some_and(|s| s.contains('@'));
        if needs_update {
            conn.execute(
                "UPDATE releases SET cover_title = ?2, updated_at = datetime('now') WHERE id = ?1",
                params![release_id, native],
            )?;
            updated = true;
        }
    }

    // -- developer --
    if !gdb_game.developer.is_empty() {
        let dev_id = find_or_create_company(conn, &gdb_game.developer, stats)?;
        if release.developer_id.is_none() {
            conn.execute(
                "UPDATE releases SET developer_id = ?2, updated_at = datetime('now') WHERE id = ?1 AND developer_id IS NULL",
                params![release_id, dev_id],
            )?;
            updated = true;
        } else {
            let existing_name = release
                .developer_id
                .as_deref()
                .and_then(|id| queries::get_company_name(conn, id).ok().flatten());
            let disagreed = merge::check_field(
                conn,
                "release",
                release_id,
                "developer",
                "screenscraper",
                existing_name.as_deref(),
                source,
                Some(&gdb_game.developer),
            )?;
            if disagreed {
                stats.disagreements += 1;
            }
        }
    }

    // -- publisher --
    if !gdb_game.publisher.is_empty() {
        let pub_id = find_or_create_company(conn, &gdb_game.publisher, stats)?;
        if release.publisher_id.is_none() {
            conn.execute(
                "UPDATE releases SET publisher_id = ?2, updated_at = datetime('now') WHERE id = ?1 AND publisher_id IS NULL",
                params![release_id, pub_id],
            )?;
            updated = true;
        } else {
            let existing_name = release
                .publisher_id
                .as_deref()
                .and_then(|id| queries::get_company_name(conn, id).ok().flatten());
            let disagreed = merge::check_field(
                conn,
                "release",
                release_id,
                "publisher",
                "screenscraper",
                existing_name.as_deref(),
                source,
                Some(&gdb_game.publisher),
            )?;
            if disagreed {
                stats.disagreements += 1;
            }
        }
    }

    // -- release_date --
    if !gdb_game.release_date.is_empty() {
        if release.release_date.is_none() {
            conn.execute(
                "UPDATE releases SET release_date = ?2, updated_at = datetime('now') WHERE id = ?1 AND release_date IS NULL",
                params![release_id, &gdb_game.release_date],
            )?;
            updated = true;
        } else {
            let disagreed = merge::check_field(
                conn,
                "release",
                release_id,
                "release_date",
                "screenscraper",
                release.release_date.as_deref(),
                source,
                Some(&gdb_game.release_date),
            )?;
            if disagreed {
                stats.disagreements += 1;
            }
        }
    }

    // -- genre (from first genre tag path, joined with " > ") --
    let genre_str = gdb_game.tags.genres.first().map(|path| path.join(" > "));

    if let Some(ref genre) = genre_str {
        if release.genre.is_none() {
            conn.execute(
                "UPDATE releases SET genre = ?2, updated_at = datetime('now') WHERE id = ?1 AND genre IS NULL",
                params![release_id, genre],
            )?;
            updated = true;
        } else {
            let disagreed = merge::check_field(
                conn,
                "release",
                release_id,
                "genre",
                "screenscraper",
                release.genre.as_deref(),
                source,
                Some(genre),
            )?;
            if disagreed {
                stats.disagreements += 1;
            }
        }
    }

    // -- players --
    if let Some(ref players) = gdb_game.tags.players {
        // Normalize: "2:coop" → "2", "2:vs" → "2"
        let player_count = players.split(':').next().unwrap_or(players);
        if release.players.is_none() {
            conn.execute(
                "UPDATE releases SET players = ?2, updated_at = datetime('now') WHERE id = ?1 AND players IS NULL",
                params![release_id, player_count],
            )?;
            updated = true;
        } else {
            let disagreed = merge::check_field(
                conn,
                "release",
                release_id,
                "players",
                "screenscraper",
                release.players.as_deref(),
                source,
                Some(player_count),
            )?;
            if disagreed {
                stats.disagreements += 1;
            }
        }
    }

    Ok(updated)
}

/// Find or create a company by name, returning its ID.
fn find_or_create_company(
    conn: &Connection,
    name: &str,
    stats: &mut GdbEnrichStats,
) -> Result<String, ImportError> {
    // Check by alias first
    if let Some(company_id) = operations::find_company_by_alias(conn, name)? {
        return Ok(company_id);
    }

    // Check by slug
    let slug = slugify(name);
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM companies WHERE id = ?1)",
        [&slug],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(slug);
    }

    // Create new company
    let company = Company {
        id: slug.clone(),
        name: name.to_string(),
        country: None,
        aliases: vec![name.to_string()],
    };
    operations::upsert_company(conn, &company)?;
    stats.companies_created += 1;
    log::debug!("Created new company: {} ({})", name, slug);

    Ok(slug)
}
