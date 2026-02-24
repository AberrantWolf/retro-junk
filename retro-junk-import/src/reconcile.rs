//! Work reconciliation by ScreenScraper ID.
//!
//! During DAT import, work IDs are generated as `{platform_id}:{slugified_title}`.
//! When the same game has different regional DAT titles, separate works get created.
//! After ScreenScraper enrichment, both releases receive the same `screenscraper_id`,
//! proving they're the same game. This module detects shared IDs and merges the
//! duplicate works into one canonical work.

use retro_junk_db::{operations, queries};
use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error("Database error: {0}")]
    Db(#[from] operations::OperationError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Options controlling the reconciliation pass.
pub struct ReconcileOptions {
    /// Filter to specific platforms, or empty for all.
    pub platform_ids: Vec<String>,
    /// Report without mutating.
    pub dry_run: bool,
}

/// Statistics from a reconciliation run.
#[derive(Debug, Default)]
pub struct ReconcileStats {
    pub groups_found: usize,
    pub works_merged: usize,
    pub works_deleted: usize,
    pub releases_reassigned: usize,
    pub releases_merged: usize,
    pub media_moved: usize,
}

/// Detail for a single merge operation (used for CLI output).
#[derive(Debug)]
pub struct MergeDetail {
    pub platform_id: String,
    pub absorbed_names: Vec<String>,
    pub surviving_name: String,
    pub total_releases: i64,
}

/// Result of reconciliation including stats and per-group details.
pub struct ReconcileResult {
    pub stats: ReconcileStats,
    pub details: Vec<MergeDetail>,
}

/// A work candidate with metadata for tie-breaking.
struct WorkCandidate {
    id: String,
    canonical_name: String,
    release_count: i64,
    created_at: String,
}

/// Run work reconciliation, merging duplicate works that share a ScreenScraper ID.
///
/// Returns statistics and per-group details for CLI display.
pub fn reconcile_works(
    conn: &Connection,
    options: &ReconcileOptions,
) -> Result<ReconcileResult, ReconcileError> {
    let mut stats = ReconcileStats::default();
    let mut details = Vec::new();

    let groups = queries::find_reconcilable_works(conn)?;

    // Filter by platform if requested
    let groups: Vec<_> = if options.platform_ids.is_empty() {
        groups
    } else {
        groups
            .into_iter()
            .filter(|g| options.platform_ids.contains(&g.platform_id))
            .collect()
    };

    stats.groups_found = groups.len();

    if groups.is_empty() {
        return Ok(ReconcileResult { stats, details });
    }

    // Wrap everything in a transaction (skipped for dry runs)
    if !options.dry_run {
        conn.execute_batch("BEGIN IMMEDIATE")?;
    }

    let result = reconcile_groups(conn, &groups, options, &mut stats, &mut details);

    match result {
        Ok(()) if !options.dry_run => {
            // Clean up orphaned works
            let deleted = operations::delete_orphan_works(conn)?;
            stats.works_deleted = deleted as usize;
            conn.execute_batch("COMMIT")?;
        }
        Ok(()) => {
            // Dry run: estimate deletions = works_merged
            stats.works_deleted = stats.works_merged;
        }
        Err(e) => {
            if !options.dry_run {
                let _ = conn.execute_batch("ROLLBACK");
            }
            return Err(e);
        }
    }

    Ok(ReconcileResult { stats, details })
}

/// Process all reconcile groups.
fn reconcile_groups(
    conn: &Connection,
    groups: &[queries::ReconcileGroup],
    options: &ReconcileOptions,
    stats: &mut ReconcileStats,
    details: &mut Vec<MergeDetail>,
) -> Result<(), ReconcileError> {
    for group in groups {
        // Fetch candidates with release counts and created_at for tie-breaking
        let mut candidates = Vec::new();
        for work_id in &group.work_ids {
            let work = queries::get_work_by_id(conn, work_id)?;
            let count = queries::count_releases_for_work(conn, work_id)?;
            let created_at: String = conn.query_row(
                "SELECT created_at FROM works WHERE id = ?1",
                rusqlite::params![work_id],
                |row| row.get(0),
            )?;
            if let Some(w) = work {
                candidates.push(WorkCandidate {
                    id: w.id,
                    canonical_name: w.canonical_name,
                    release_count: count,
                    created_at,
                });
            }
        }

        if candidates.len() < 2 {
            continue;
        }

        // Pick surviving work: most releases, then earliest created_at
        candidates.sort_by(|a, b| {
            b.release_count
                .cmp(&a.release_count)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });

        let surviving = &candidates[0];
        let absorbed: Vec<&WorkCandidate> = candidates[1..].iter().collect();

        let mut detail = MergeDetail {
            platform_id: group.platform_id.clone(),
            absorbed_names: absorbed.iter().map(|w| w.canonical_name.clone()).collect(),
            surviving_name: surviving.canonical_name.clone(),
            total_releases: candidates.iter().map(|c| c.release_count).sum(),
        };

        for work in &absorbed {
            if !options.dry_run {
                merge_work_into(conn, &work.id, &surviving.id, stats)?;
            }
            stats.works_merged += 1;
        }

        // Try to update canonical name from ScreenScraper alt_title
        if !options.dry_run {
            if let Some(name) = pick_canonical_name(conn, &surviving.id)? {
                operations::update_work_name(conn, &surviving.id, &name)?;
                detail.surviving_name = name;
            }
        }

        details.push(detail);
    }

    Ok(())
}

/// Merge one absorbed work into the surviving work.
fn merge_work_into(
    conn: &Connection,
    absorbed_work_id: &str,
    surviving_work_id: &str,
    stats: &mut ReconcileStats,
) -> Result<(), ReconcileError> {
    // Check for release collisions first
    let collisions = queries::check_release_collision(conn, absorbed_work_id, surviving_work_id)?;

    for collision in &collisions {
        // Move media, assets, and disagreements from absorbed release to surviving
        let media_moved =
            operations::move_media_to_release(conn, &collision.absorbed_release_id, &collision.surviving_release_id)?;
        operations::move_assets_to_release(
            conn,
            &collision.absorbed_release_id,
            &collision.surviving_release_id,
        )?;
        operations::move_disagreements_for_release(
            conn,
            &collision.absorbed_release_id,
            &collision.surviving_release_id,
        )?;

        // Delete the now-empty absorbed release
        operations::delete_release(conn, &collision.absorbed_release_id)?;

        stats.releases_merged += 1;
        stats.media_moved += media_moved as usize;
    }

    // Move remaining (non-colliding) releases to surviving work
    let moved = operations::update_releases_work_id(conn, absorbed_work_id, surviving_work_id)?;
    stats.releases_reassigned += moved as usize;

    Ok(())
}

/// Pick the best canonical name from the surviving work's releases.
///
/// Prefers `alt_title` (set by ScreenScraper) from a USA release, falling back
/// to any release with an alt_title, then the existing title.
fn pick_canonical_name(
    conn: &Connection,
    work_id: &str,
) -> Result<Option<String>, ReconcileError> {
    // Try USA alt_title first
    let result: Result<String, _> = conn.query_row(
        "SELECT alt_title FROM releases WHERE work_id = ?1 AND alt_title IS NOT NULL AND region = 'USA' LIMIT 1",
        rusqlite::params![work_id],
        |row| row.get(0),
    );
    if let Ok(name) = result {
        return Ok(Some(name));
    }

    // Fall back to any alt_title
    let result: Result<String, _> = conn.query_row(
        "SELECT alt_title FROM releases WHERE work_id = ?1 AND alt_title IS NOT NULL LIMIT 1",
        rusqlite::params![work_id],
        |row| row.get(0),
    );
    if let Ok(name) = result {
        return Ok(Some(name));
    }

    Ok(None)
}
