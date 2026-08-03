//! One file per decision, stored beside the collection.
//!
//! Some things about a collection cannot be derived from anything: which file
//! is a mod of which game, which strays you never want asked about again. The
//! database can be rebuilt from DATs, so it is the wrong home for a decision
//! nobody can reconstruct — the decision has to travel with the files it is
//! about.
//!
//! Storing that as one big file per collection would be the obvious shape and
//! the wrong one, because collections get moved around by tools that know
//! nothing about this format: Syncthing to a handheld, rsync overnight, an SMB
//! mount on another machine. Two properties make those tools harmless:
//!
//! * **One file per decision, named from the decision's own content.** Two
//!   machines deciding different things never touch the same file, so there is
//!   nothing for Syncthing to raise a conflict over and nothing for rsync to
//!   clobber.
//! * **The same decision always produces the same bytes.** Deciding the same
//!   thing on two machines writes two identical files, so copies converge
//!   instead of fighting. That is why nothing stored this way records a
//!   timestamp — a timestamp would make two identical decisions differ, which
//!   is exactly the conflict this layout exists to avoid.
//!
//! This module owns that machinery; each record type owns only its own shape
//! and its own naming rule.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::manifest::{ManifestError, write_toml_atomic};

#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    /// The record carries too little to name a file from, so storing it would
    /// mean inventing an identity it does not have.
    #[error("{0}")]
    Unnamed(&'static str),
}

/// A decision that can be stored beside a collection.
pub trait SidecarRecord: Serialize + DeserializeOwned + PartialEq + Sized {
    /// Where records of this type live, relative to the collection root.
    const DIRECTORY: &'static str;
    /// The newest schema this build writes and understands. Versioned per
    /// record type: these are separate portable formats with separate
    /// lifecycles, and an archive migration must not invalidate them.
    const SCHEMA_VERSION: u32;
    /// What to say when a record cannot be named.
    const UNNAMED: &'static str;

    /// The file name this record is stored under, derived from the record
    /// itself so the same decision always lands on the same path. `None` when
    /// the record lacks whatever its naming rule needs.
    fn sidecar_name(&self) -> Option<String>;

    /// The schema this particular record was written under.
    fn schema_version(&self) -> u32;
}

/// Where records of this type live for a collection rooted at
/// `collection_root`.
#[must_use]
pub fn directory<R: SidecarRecord>(collection_root: &Path) -> PathBuf {
    collection_root.join(R::DIRECTORY)
}

/// Record a decision, replacing any earlier one stored under the same name.
///
/// Rewriting an identical record is a no-op down to the filesystem, so a
/// second machine reaching the same conclusion does not churn the file and
/// gives a syncing tool no modification to propagate.
pub fn write<R: SidecarRecord>(
    collection_root: &Path,
    record: &R,
) -> Result<PathBuf, SidecarError> {
    let name = record
        .sidecar_name()
        .ok_or(SidecarError::Unnamed(R::UNNAMED))?;
    let directory = directory::<R>(collection_root);
    std::fs::create_dir_all(&directory).map_err(|source| SidecarError::Io {
        path: directory.display().to_string(),
        source,
    })?;
    let path = directory.join(name);
    if let Ok(existing) = read::<R>(&path)
        && existing == *record
    {
        return Ok(path);
    }
    write_toml_atomic(&path, record)?;
    Ok(path)
}

/// Forget a decision. Returns whether there was one to forget.
pub fn remove<R: SidecarRecord>(collection_root: &Path, record: &R) -> Result<bool, SidecarError> {
    let name = record
        .sidecar_name()
        .ok_or(SidecarError::Unnamed(R::UNNAMED))?;
    let path = directory::<R>(collection_root).join(name);
    remove_at(&path)
}

/// Forget the decision stored at an exact path.
pub fn remove_at(path: &Path) -> Result<bool, SidecarError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(SidecarError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// Every record of this type, in a stable order.
///
/// An unreadable or future-schema record is skipped with a warning rather than
/// failing the load: one bad file must not cost you every other decision, and
/// a newer machine's records landing here is a normal consequence of syncing.
pub fn load_all<R: SidecarRecord>(collection_root: &Path) -> Result<Vec<R>, SidecarError> {
    let directory = directory::<R>(collection_root);
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(SidecarError::Io {
                path: directory.display().to_string(),
                source,
            });
        }
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            !retro_junk_io::is_noise_path(path)
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    let mut records = Vec::with_capacity(paths.len());
    for path in paths {
        match read::<R>(&path) {
            Ok(record) if record.schema_version() <= R::SCHEMA_VERSION => records.push(record),
            Ok(record) => log::warn!(
                "Skipping {} written by a newer schema (v{})",
                path.display(),
                record.schema_version()
            ),
            Err(error) => log::warn!("Skipping unreadable {}: {error}", path.display()),
        }
    }
    Ok(records)
}

/// Parse one record. The schema is checked by the caller, so a record from a
/// newer machine can be recognized and skipped rather than failing to parse.
pub fn read<R: SidecarRecord>(path: &Path) -> Result<R, SidecarError> {
    let contents = std::fs::read_to_string(path).map_err(|source| SidecarError::Io {
        path: path.display().to_string(),
        source,
    })?;
    toml::from_str(&contents).map_err(|source| {
        SidecarError::Manifest(ManifestError::TomlDecode {
            path: path.display().to_string(),
            source,
        })
    })
}
