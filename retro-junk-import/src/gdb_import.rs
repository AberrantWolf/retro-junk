//! `GameDataBase` (GDB) catalog enrichment.
//!
//! Enriches catalog releases with metadata from `PigSaint`'s `GameDataBase`:
//! Japanese titles, developer/publisher, release dates, genre, player count.
//! Matches are performed by SHA1 hash from media entries.
//!
//! Data source: <https://github.com/PigSaint/GameDataBase>
//! License: CC BY 4.0 — Attribution to `PigSaint` required.

use retro_junk_dat::gdb::{self, GdbGame};
use retro_junk_dat::gdb_cache;
use retro_junk_db::{Connection, queries};
use rusqlite::params;

use crate::ImportError;
use crate::merge;

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
/// When found, fills in missing `alt_title`, developer, publisher, `release_date`,
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

            if media.sha1.is_empty() {
                stats.skipped_no_hash += 1;
                continue;
            }

            // Try SHA1 lookup, fall back to MD5
            let gdb_game = index.lookup_sha1(&media.sha1).or_else(|| {
                (!media.md5.is_empty())
                    .then(|| index.lookup_md5(&media.md5))
                    .flatten()
            });

            let Some(gdb_game) = gdb_game else {
                continue;
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

/// Source label recorded for values already in the DB when GDB disagrees.
const EXISTING_SOURCE: &str = "screenscraper";
/// Source label for GDB-provided values.
const GDB_SOURCE: &str = "gdb";

/// Fill an empty release text column with a GDB value, or record a
/// disagreement when both have data and differ. Returns true if updated.
///
/// `column` must be a compile-time column name (it is interpolated into SQL).
fn fill_or_check(
    conn: &Connection,
    release_id: &str,
    column: &'static str,
    existing: &str,
    new_value: &str,
    stats: &mut GdbEnrichStats,
) -> Result<bool, ImportError> {
    if new_value.is_empty() {
        return Ok(false);
    }
    if existing.is_empty() {
        conn.execute(
            &format!(
                "UPDATE releases SET {column} = ?2, updated_at = datetime('now')
                 WHERE id = ?1 AND {column} = ''"
            ),
            params![release_id, new_value],
        )?;
        return Ok(true);
    }
    let disagreed = merge::check_field(
        conn,
        &merge::FieldRef {
            entity_type: "release",
            entity_id: release_id,
            field: column,
        },
        &merge::SourcedValue {
            source: EXISTING_SOURCE,
            value: existing,
        },
        &merge::SourcedValue {
            source: GDB_SOURCE,
            value: new_value,
        },
    )?;
    if disagreed {
        stats.disagreements += 1;
    }
    Ok(false)
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

    // Extract native (Japanese) title from screen_title
    let (_, native_title) = gdb::split_title(&gdb_game.screen_title);

    // -- alt_title --
    if let Some(native) = native_title {
        updated |= fill_or_check(
            conn,
            release_id,
            "alt_title",
            &release.alt_title,
            native,
            stats,
        )?;
    }

    // -- screen_title / cover_title (native/original language portion only) --
    // Overwrite if unset or if existing value contains '@' (stale full-string format)
    let title_columns = [
        (
            "screen_title",
            &release.screen_title,
            &gdb_game.screen_title,
        ),
        ("cover_title", &release.cover_title, &gdb_game.cover_title),
    ];
    for (column, existing, gdb_full) in title_columns {
        let (_, native) = gdb::split_title(gdb_full);
        if let Some(native) = native
            && (existing.is_empty() || existing.contains('@'))
        {
            conn.execute(
                &format!(
                    "UPDATE releases SET {column} = ?2, updated_at = datetime('now') WHERE id = ?1"
                ),
                params![release_id, native],
            )?;
            updated = true;
        }
    }

    // -- developer / publisher (nullable company FKs) --
    let company_columns = [
        (
            "developer",
            "developer_id",
            &release.developer_id,
            &gdb_game.developer,
        ),
        (
            "publisher",
            "publisher_id",
            &release.publisher_id,
            &gdb_game.publisher,
        ),
    ];
    for (field, column, existing_id, gdb_name) in company_columns {
        updated |= enrich_company(
            conn,
            release_id,
            field,
            column,
            existing_id.as_deref(),
            gdb_name,
            stats,
        )?;
    }

    // -- release_date --
    updated |= fill_or_check(
        conn,
        release_id,
        "release_date",
        &release.release_date,
        &gdb_game.release_date,
        stats,
    )?;

    // -- genre (from first genre tag path, joined with " > ") --
    let genre = gdb_game
        .tags
        .genres
        .first()
        .map(|path| path.join(" > "))
        .unwrap_or_default();
    updated |= fill_or_check(conn, release_id, "genre", &release.genre, &genre, stats)?;

    // -- players (normalize "2:coop" → "2") --
    let players = gdb_game
        .tags
        .players
        .as_deref()
        .map(|p| p.split(':').next().unwrap_or(p))
        .unwrap_or_default();
    updated |= fill_or_check(
        conn,
        release_id,
        "players",
        &release.players,
        players,
        stats,
    )?;

    Ok(updated)
}

/// Fill an unset company FK column with a GDB-provided company, or record a
/// disagreement when both sides have a value and differ. Returns true if updated.
///
/// `column` must be a compile-time column name (it is interpolated into SQL).
fn enrich_company(
    conn: &Connection,
    release_id: &str,
    field: &'static str,
    column: &'static str,
    existing_id: Option<&str>,
    gdb_name: &str,
    stats: &mut GdbEnrichStats,
) -> Result<bool, ImportError> {
    if gdb_name.is_empty() {
        return Ok(false);
    }
    let company_id = find_or_create_company(conn, gdb_name, stats)?;
    match existing_id {
        None => {
            conn.execute(
                &format!(
                    "UPDATE releases SET {column} = ?2, updated_at = datetime('now')
                     WHERE id = ?1 AND {column} IS NULL"
                ),
                params![release_id, company_id],
            )?;
            Ok(true)
        }
        Some(existing_id) => {
            let existing_name = queries::get_company_name(conn, existing_id)
                .ok()
                .flatten()
                .unwrap_or_default();
            let disagreed = merge::check_field(
                conn,
                &merge::FieldRef {
                    entity_type: "release",
                    entity_id: release_id,
                    field,
                },
                &merge::SourcedValue {
                    source: EXISTING_SOURCE,
                    value: &existing_name,
                },
                &merge::SourcedValue {
                    source: GDB_SOURCE,
                    value: gdb_name,
                },
            )?;
            if disagreed {
                stats.disagreements += 1;
            }
            Ok(false)
        }
    }
}

/// Find or create a company by name, returning its ID.
fn find_or_create_company(
    conn: &Connection,
    name: &str,
    stats: &mut GdbEnrichStats,
) -> Result<String, ImportError> {
    let found = crate::companies::find_or_create_company(conn, name)?;
    if found.created {
        stats.companies_created += 1;
    }
    Ok(found.id)
}
