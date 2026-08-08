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
//! The fingerprint covers sources only, so a person deleting a projected file
//! by hand is invisible to it. That is what [`forget_projections`] is for —
//! `sync --force-projections` and `archive project-assets` still redo the work
//! unconditionally.

use rusqlite::{Connection, OptionalExtension, params};
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
    Ok(stored_fingerprint(conn, of)?.as_deref() == Some(current.as_str()))
}

/// Write down that this output is now current.
pub fn record_projection(conn: &Connection, of: ProjectionOf<'_>) -> Result<(), LibraryError> {
    let current = fingerprint(conn, of)?;
    match of {
        ProjectionOf::Assets { archive_release_id } => {
            conn.execute(
                "INSERT INTO projected_assets(profile_id, archive_release_id, fingerprint)
                 SELECT ar.profile_id, ar.id, ?2 FROM archive_releases ar WHERE ar.id=?1
                 ON CONFLICT(archive_release_id) DO UPDATE SET fingerprint=excluded.fingerprint",
                params![archive_release_id, current],
            )?;
        }
        ProjectionOf::Gamelist {
            profile_id,
            directory,
        } => {
            conn.execute(
                "INSERT INTO projected_gamelists(profile_id, console, fingerprint)
                 VALUES(?1,?2,?3)
                 ON CONFLICT(profile_id, console) DO UPDATE SET fingerprint=excluded.fingerprint",
                params![profile_id, directory, current],
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

fn stored_fingerprint(
    conn: &Connection,
    of: ProjectionOf<'_>,
) -> Result<Option<String>, LibraryError> {
    let stored = match of {
        ProjectionOf::Assets { archive_release_id } => conn
            .query_row(
                "SELECT fingerprint FROM projected_assets WHERE archive_release_id=?1",
                [archive_release_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?,
        ProjectionOf::Gamelist {
            profile_id,
            directory,
        } => conn
            .query_row(
                "SELECT fingerprint FROM projected_gamelists WHERE profile_id=?1 AND console=?2",
                params![profile_id, directory],
                |row| row.get::<_, String>(0),
            )
            .optional()?,
    };
    Ok(stored)
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
         WHERE archive_release_id=?1 AND category IN ('artwork','video')
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
