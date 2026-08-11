//! What has already been projected to the frontend, and whether it is current.
//!
//! Projecting artwork and writing gamelists are cheap-ish but not free, and
//! both are idempotent — so for a long time nothing tracked them, and
//! derivation proposed them for every release on every pass forever. That made
//! `status` report hundreds of pending items on a fully converged library and
//! made an explicit run redo work it had just done.
//!
//! The fix is to write down what a projection was made *from*. A fingerprint is
//! a digest of every archive fact the output depends on; if the stored
//! fingerprint still matches, the output on disk is what this archive would
//! produce, and there is nothing owed. Derivation asks
//! [`projection_is_current`]; the executor calls [`record_projection`] once the
//! work succeeds. One definition, read by both, so "current" cannot mean two
//! things.
//!
//! Source fingerprints are paired with cheap signatures of the outputs that a
//! successful projection actually wrote. A person deleting, replacing, or
//! editing a projected file therefore makes the projection owed again without
//! hashing the whole frontend tree. `sync --force-projections` and
//! `archive project-assets` remain available to redo projections
//! unconditionally.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::convergence::Scope;
use crate::library::LibraryError;

/// A projected output, named by what it is derived from.
#[derive(Debug, Clone, Copy)]
pub enum ProjectionOf<'a> {
    /// One release's artwork copied into the frontend media tree.
    Assets { archive_release_id: &'a str },
    /// One frontend folder's `gamelist.xml`.
    Gamelist {
        profile_id: &'a str,
        directory: &'a str,
    },
}

impl<'a> ProjectionOf<'a> {
    #[must_use]
    pub const fn assets(archive_release_id: &'a str) -> Self {
        Self::Assets { archive_release_id }
    }

    #[must_use]
    pub const fn gamelist(profile_id: &'a str, directory: &'a str) -> Self {
        Self::Gamelist {
            profile_id,
            directory,
        }
    }
}

/// Whether the output this names is already what the archive would produce.
pub fn projection_is_current(
    conn: &Connection,
    of: ProjectionOf<'_>,
) -> Result<bool, LibraryError> {
    let current = fingerprint(conn, of)?;
    let Some((stored, outputs)) = stored_projection(conn, of)? else {
        return Ok(false);
    };
    Ok(stored == current && outputs_are_current(&outputs))
}

/// Write down that this output is now current.
pub fn record_projection(conn: &Connection, of: ProjectionOf<'_>) -> Result<(), LibraryError> {
    record_projection_outputs(conn, of, &[])
}

/// Record a successful projection and the files it actually produced.
///
/// Paths are supplied by the backend operation that performed the write; the
/// database owns their durable signatures and freshness semantics. This keeps
/// filesystem layout out of UI code while still making manual output deletion
/// observable on the next derivation.
pub fn record_projection_outputs(
    conn: &Connection,
    of: ProjectionOf<'_>,
    outputs: &[PathBuf],
) -> Result<(), LibraryError> {
    let current = fingerprint(conn, of)?;
    let outputs = serde_json::to_string(&output_signatures(outputs)?)
        .map_err(|error| LibraryError::InvalidScanState(error.to_string()))?;
    match of {
        ProjectionOf::Assets { archive_release_id } => {
            conn.execute(
                "INSERT INTO projected_assets(profile_id,archive_release_id,fingerprint,outputs_json)
                 SELECT ar.profile_id,ar.id,?2,?3 FROM archive_releases ar WHERE ar.id=?1
                 ON CONFLICT(archive_release_id) DO UPDATE SET
                    fingerprint=excluded.fingerprint,outputs_json=excluded.outputs_json",
                params![archive_release_id, current, outputs],
            )?;
        }
        ProjectionOf::Gamelist {
            profile_id,
            directory,
        } => {
            conn.execute(
                "INSERT INTO projected_gamelists(profile_id,console,fingerprint,outputs_json)
                 VALUES(?1,?2,?3,?4)
                 ON CONFLICT(profile_id, console) DO UPDATE SET
                    fingerprint=excluded.fingerprint,outputs_json=excluded.outputs_json",
                params![profile_id, directory, current, outputs],
            )?;
        }
    }
    Ok(())
}

/// Forget what is current in this scope, so the next derivation proposes it
/// all again. Returns how many records were dropped.
pub fn forget_projections(conn: &Connection, scope: &Scope) -> Result<usize, LibraryError> {
    let mut forgotten = 0_usize;
    // Gamelists are per folder, so a release-level scope still forgets the
    // whole folder — the file that release appears in has to be rewritten
    // either way.
    for profile_id in crate::convergence::profiles_for_scope(conn, scope)? {
        forgotten += conn.execute(
            "DELETE FROM projected_gamelists WHERE profile_id=?1",
            [profile_id.as_str()],
        )?;
        forgotten += match scope {
            Scope::Release { archive_release_id } => conn.execute(
                "DELETE FROM projected_assets WHERE archive_release_id=?1",
                [archive_release_id.as_str()],
            )?,
            Scope::Releases(ids) => {
                let mut dropped = 0;
                for id in ids {
                    dropped += conn.execute(
                        "DELETE FROM projected_assets WHERE archive_release_id=?1",
                        [id.as_str()],
                    )?;
                }
                dropped
            }
            _ => conn.execute(
                "DELETE FROM projected_assets WHERE profile_id=?1",
                [profile_id.as_str()],
            )?,
        };
    }
    Ok(forgotten)
}

fn stored_projection(
    conn: &Connection,
    of: ProjectionOf<'_>,
) -> Result<Option<(String, String)>, LibraryError> {
    let stored = match of {
        ProjectionOf::Assets { archive_release_id } => conn
            .query_row(
                "SELECT fingerprint,outputs_json FROM projected_assets WHERE archive_release_id=?1",
                [archive_release_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?,
        ProjectionOf::Gamelist {
            profile_id,
            directory,
        } => conn
            .query_row(
                "SELECT fingerprint,outputs_json FROM projected_gamelists WHERE profile_id=?1 AND console=?2",
                params![profile_id, directory],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?,
    };
    Ok(stored)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OutputSignature {
    path: PathBuf,
    size: u64,
    modified_seconds: u64,
    modified_nanos: u32,
}

fn output_signatures(outputs: &[PathBuf]) -> Result<Vec<OutputSignature>, LibraryError> {
    let unique = outputs
        .iter()
        .map(|path| (path.to_string_lossy().into_owned(), path))
        .collect::<BTreeMap<_, _>>();
    unique
        .into_values()
        .map(|path| {
            output_signature(path).ok_or_else(|| {
                LibraryError::InvalidScanState(format!(
                    "projected output is not a readable regular file: {}",
                    path.display()
                ))
            })
        })
        .collect()
}

fn output_signature(path: &Path) -> Option<OutputSignature> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(OutputSignature {
        path: path.to_path_buf(),
        size: metadata.len(),
        modified_seconds: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}

fn outputs_are_current(serialized: &str) -> bool {
    serde_json::from_str::<Vec<OutputSignature>>(serialized).is_ok_and(|outputs| {
        outputs
            .iter()
            .all(|expected| output_signature(&expected.path).as_ref() == Some(expected))
    })
}

/// Digest every archive fact this output is derived from.
fn fingerprint(conn: &Connection, of: ProjectionOf<'_>) -> Result<String, LibraryError> {
    match of {
        ProjectionOf::Assets { archive_release_id } => asset_fingerprint(conn, archive_release_id),
        ProjectionOf::Gamelist {
            profile_id,
            directory,
        } => gamelist_fingerprint(conn, profile_id, directory),
    }
}

/// What a release's projected artwork depends on: which archived files it
/// holds, their content, and the output names that decide what the copies are
/// called.
fn asset_fingerprint(conn: &Connection, archive_release_id: &str) -> Result<String, LibraryError> {
    let mut parts = Vec::new();
    let mut statement = conn.prepare(
        "SELECT asset_type, sha256, file_size FROM archive_release_files
         WHERE archive_release_id=?1 AND presence_state='present'
           AND category IN ('artwork','video')
         ORDER BY asset_type, relative_path",
    )?;
    for row in statement.query_map([archive_release_id], |row| {
        Ok(format!(
            "{}:{}:{}",
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?
        ))
    })? {
        parts.push(row?);
    }
    // The stems come from the output paths, so a rename changes where the
    // artwork has to land even though the artwork itself did not change.
    let mut outputs = conn.prepare(
        "SELECT rep.relative_path FROM representations rep
         JOIN carriers c ON c.id=rep.carrier_id
         JOIN physical_copies pc ON pc.id=c.physical_copy_id
         WHERE pc.archive_release_id=?1 AND rep.role='playable'
         ORDER BY rep.relative_path",
    )?;
    for row in outputs.query_map([archive_release_id], |row| row.get::<_, String>(0))? {
        parts.push(row?);
    }
    Ok(digest(&parts))
}

/// What a folder's gamelist depends on: every release publishing into it, the
/// name and path each one contributes, and that release's artwork — because the
/// entry's asset tags are read back off the media tree the asset projection
/// writes.
fn gamelist_fingerprint(
    conn: &Connection,
    profile_id: &str,
    directory: &str,
) -> Result<String, LibraryError> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT ar.id, ar.title, rep.relative_path
         FROM archive_releases ar
         JOIN physical_copies pc ON pc.archive_release_id=ar.id
         JOIN carriers c ON c.physical_copy_id=pc.id
         JOIN representations rep ON rep.carrier_id=c.id
         WHERE ar.profile_id=?1
           AND rep.role='playable' AND rep.presence_state='present'
           AND substr(rep.relative_path, 1, instr(rep.relative_path,'/')-1) = ?2
         ORDER BY ar.id, rep.relative_path",
    )?;
    let rows = statement
        .query_map(params![profile_id, directory], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut parts = Vec::with_capacity(rows.len());
    for (release_id, title, relative_path) in rows {
        let assets = asset_fingerprint(conn, &release_id)?;
        parts.push(format!("{release_id}:{title}:{relative_path}:{assets}"));
    }
    Ok(digest(&parts))
}

/// A digest of ordered parts, separated so that moving a character across a
/// boundary cannot produce the same input.
fn digest(parts: &[String]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0_u8]);
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_a_recorded_output_makes_its_projection_owed_again() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("covers/Game.png");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(&output, b"image").unwrap();

        let conn = crate::open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO archive_profiles(id,display_name,manifest_path,manifest_sha256,archive_root)
             VALUES('profile','Profile','archive.toml','sha','/archive');
             INSERT INTO archive_releases(id,profile_id,platform_id,title,region,revision,variant,manifest_path,manifest_sha256)
             VALUES('release','profile','ps1','Game','USA','','','release.toml','release-sha');",
        )
        .unwrap();
        let projection = ProjectionOf::assets("release");
        record_projection_outputs(&conn, projection, std::slice::from_ref(&output)).unwrap();
        assert!(projection_is_current(&conn, projection).unwrap());

        std::fs::remove_file(output).unwrap();
        assert!(!projection_is_current(&conn, projection).unwrap());
    }
}
