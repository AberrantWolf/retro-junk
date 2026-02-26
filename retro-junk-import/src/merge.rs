//! Multi-source merge logic with disagreement detection.
//!
//! When importing data from a second source (e.g., ScreenScraper after No-Intro),
//! this module detects conflicts between existing and new values, creating
//! disagreement records for manual resolution.

use retro_junk_catalog::types::*;
use retro_junk_db::operations;
use rusqlite::Connection;

use crate::dat_import::ImportError;

/// Compare two optional string values and record a disagreement if they differ.
///
/// Returns `true` if a disagreement was recorded.
#[allow(clippy::too_many_arguments)]
pub fn check_field(
    conn: &Connection,
    entity_type: &str,
    entity_id: &str,
    field: &str,
    source_a: &str,
    value_a: Option<&str>,
    source_b: &str,
    value_b: Option<&str>,
) -> Result<bool, ImportError> {
    // No conflict if both are None
    if value_a.is_none() && value_b.is_none() {
        return Ok(false);
    }

    // No conflict if they're the same
    if value_a == value_b {
        return Ok(false);
    }

    // Auto-resolve: if one is None and the other has data, no real conflict
    if value_a.is_none() || value_b.is_none() {
        return Ok(false);
    }

    // Real conflict — create a disagreement record
    let disagreement = Disagreement {
        id: 0,
        entity_type: entity_type.to_string(),
        entity_id: entity_id.to_string(),
        field: field.to_string(),
        source_a: source_a.to_string(),
        value_a: value_a.map(|s| s.to_string()),
        source_b: source_b.to_string(),
        value_b: value_b.map(|s| s.to_string()),
        resolved: false,
        resolution: None,
        resolved_at: None,
        created_at: String::new(),
    };
    operations::insert_disagreement(conn, &disagreement)?;

    Ok(true)
}

/// Compare release fields from a new source against existing DB values.
///
/// Returns the number of disagreements found.
#[allow(clippy::too_many_arguments)]
pub fn merge_release_fields(
    conn: &Connection,
    release_id: &str,
    existing: &Release,
    source: &str,
    new_title: Option<&str>,
    new_release_date: Option<&str>,
    new_genre: Option<&str>,
    new_players: Option<&str>,
    new_description: Option<&str>,
) -> Result<u32, ImportError> {
    let existing_source = "dat-import";
    let mut count = 0u32;

    if check_field(
        conn,
        "release",
        release_id,
        "title",
        existing_source,
        Some(&existing.title),
        source,
        new_title,
    )? {
        count += 1;
    }

    if check_field(
        conn,
        "release",
        release_id,
        "release_date",
        existing_source,
        existing.release_date.as_deref(),
        source,
        new_release_date,
    )? {
        count += 1;
    }

    if check_field(
        conn,
        "release",
        release_id,
        "genre",
        existing_source,
        existing.genre.as_deref(),
        source,
        new_genre,
    )? {
        count += 1;
    }

    if check_field(
        conn,
        "release",
        release_id,
        "players",
        existing_source,
        existing.players.as_deref(),
        source,
        new_players,
    )? {
        count += 1;
    }

    if check_field(
        conn,
        "release",
        release_id,
        "description",
        existing_source,
        existing.description.as_deref(),
        source,
        new_description,
    )? {
        count += 1;
    }

    Ok(count)
}

/// Apply YAML overrides to the database.
///
/// For each override, find matching entities by pattern and update the field.
/// This should be called after import to apply known corrections.
pub fn apply_overrides(conn: &Connection, overrides: &[Override]) -> Result<u32, ImportError> {
    let mut applied = 0u32;

    for ovr in overrides {
        // Pattern-based matching on dat_name
        if let Some(ref pattern) = ovr.dat_name_pattern {
            let sql_pattern = glob_to_sql_like(pattern);
            let mut stmt = conn.prepare(
                "SELECT m.id, r.id as release_id FROM media m
                 JOIN releases r ON m.release_id = r.id
                 WHERE m.dat_name LIKE ?1 AND r.platform_id = ?2",
            )?;

            let platform_id = ovr.platform_id.as_deref().unwrap_or("");
            let matches: Vec<(String, String)> = stmt
                .query_map(rusqlite::params![sql_pattern, platform_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();

            for (media_id, release_id) in &matches {
                let entity_id = match ovr.entity_type.as_str() {
                    "media" => media_id.as_str(),
                    "release" => release_id.as_str(),
                    _ => continue,
                };

                apply_field_override(
                    conn,
                    &ovr.entity_type,
                    entity_id,
                    &ovr.field,
                    &ovr.override_value,
                )?;
                applied += 1;
            }
        }

        // Direct entity_id matching
        if let Some(ref entity_id) = ovr.entity_id {
            apply_field_override(
                conn,
                &ovr.entity_type,
                entity_id,
                &ovr.field,
                &ovr.override_value,
            )?;
            applied += 1;
        }
    }

    Ok(applied)
}

/// Apply a single field override to a specific entity.
fn apply_field_override(
    conn: &Connection,
    entity_type: &str,
    entity_id: &str,
    field: &str,
    value: &str,
) -> Result<(), ImportError> {
    let table = match entity_type {
        "release" => "releases",
        "media" => "media",
        _ => return Ok(()),
    };

    // Only allow overriding known safe fields
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
        log::warn!(
            "Skipping override for unsafe field '{}' on {}.{}",
            field,
            table,
            entity_id
        );
        return Ok(());
    }

    // Use parameterized field name via format (safe because we validated above)
    let sql =
        format!("UPDATE {table} SET {field} = ?1, updated_at = datetime('now') WHERE id = ?2");
    conn.execute(&sql, rusqlite::params![value, entity_id])?;

    Ok(())
}

/// Convert a glob pattern to SQL LIKE pattern.
///
/// `*` → `%`, `?` → `_`
fn glob_to_sql_like(pattern: &str) -> String {
    let mut result = String::with_capacity(pattern.len());
    for c in pattern.chars() {
        match c {
            '*' => result.push('%'),
            '?' => result.push('_'),
            '%' => result.push_str("\\%"),
            '_' => result.push_str("\\_"),
            _ => result.push(c),
        }
    }
    result
}
