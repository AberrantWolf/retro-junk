//! Multi-source merge logic with disagreement detection.
//!
//! When importing data from a second source (e.g., `ScreenScraper` after No-Intro),
//! this module detects conflicts between existing and new values, creating
//! disagreement records for manual resolution.

use retro_junk_catalog::types::{Disagreement, Override, Release};
use retro_junk_db::operations;
use rusqlite::Connection;

use crate::dat_import::ImportError;

/// The entity field a comparison is about.
pub struct FieldRef<'a> {
    pub entity_type: &'a str,
    pub entity_id: &'a str,
    pub field: &'a str,
}

/// One side of a field comparison: a source name and the value it reports.
/// An empty value means the source has no data for this field.
pub struct SourcedValue<'a> {
    pub source: &'a str,
    pub value: &'a str,
}

/// Compare a field's value from two sources and record a disagreement if
/// both have data and they differ. A side with no data (empty value) never
/// conflicts.
///
/// Returns `true` if a disagreement was recorded.
pub fn check_field(
    conn: &Connection,
    field: &FieldRef<'_>,
    a: &SourcedValue<'_>,
    b: &SourcedValue<'_>,
) -> Result<bool, ImportError> {
    if a.value.is_empty() || b.value.is_empty() || a.value == b.value {
        return Ok(false);
    }

    // Real conflict — create a disagreement record
    let disagreement = Disagreement {
        id: 0,
        entity_type: field.entity_type.to_string(),
        entity_id: field.entity_id.to_string(),
        field: field.field.to_string(),
        source_a: a.source.to_string(),
        value_a: a.value.to_string(),
        source_b: b.source.to_string(),
        value_b: b.value.to_string(),
        resolved: false,
        resolution: String::new(),
        resolved_at: String::new(),
        created_at: String::new(),
    };
    operations::insert_disagreement(conn, &disagreement)?;

    Ok(true)
}

/// Release field values reported by an enrichment source. Empty = no data.
pub struct ReleaseFieldValues<'a> {
    pub title: &'a str,
    pub release_date: &'a str,
    pub genre: &'a str,
    pub players: &'a str,
    pub description: &'a str,
}

/// Compare release fields from a new source against existing DB values.
///
/// Returns the number of disagreements found.
pub fn merge_release_fields(
    conn: &Connection,
    release_id: &str,
    existing: &Release,
    source: &str,
    new: &ReleaseFieldValues<'_>,
) -> Result<u32, ImportError> {
    let existing_source = "dat-import";
    let fields = [
        ("title", existing.title.as_str(), new.title),
        (
            "release_date",
            existing.release_date.as_str(),
            new.release_date,
        ),
        ("genre", existing.genre.as_str(), new.genre),
        ("players", existing.players.as_str(), new.players),
        (
            "description",
            existing.description.as_str(),
            new.description,
        ),
    ];

    let mut count = 0u32;
    for (field, existing_value, new_value) in fields {
        if check_field(
            conn,
            &FieldRef {
                entity_type: "release",
                entity_id: release_id,
                field,
            },
            &SourcedValue {
                source: existing_source,
                value: existing_value,
            },
            &SourcedValue {
                source,
                value: new_value,
            },
        )? {
            count += 1;
        }
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
        if !ovr.dat_name_pattern.is_empty() {
            let sql_pattern = glob_to_sql_like(&ovr.dat_name_pattern);
            let mut stmt = conn.prepare(
                "SELECT m.id, r.id as release_id FROM media m
                 JOIN releases r ON m.release_id = r.id
                 WHERE m.dat_name LIKE ?1 AND r.platform_id = ?2",
            )?;

            let matches: Vec<(String, String)> = stmt
                .query_map(rusqlite::params![sql_pattern, ovr.platform_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .filter_map(std::result::Result::ok)
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
        if !ovr.entity_id.is_empty() {
            apply_field_override(
                conn,
                &ovr.entity_type,
                &ovr.entity_id,
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
        log::warn!("Skipping override for unsafe field '{field}' on {table}.{entity_id}");
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
