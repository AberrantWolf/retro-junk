//! Shared reuse of Redumper's split output between archive operations.
//!
//! Regenerating a disc's tracks from a Redumper raw dump is the single most
//! expensive thing this codebase does. `redumper split` writes its BIN/CUE/log
//! output beside its input, so the raw set is copied to scratch storage first
//! rather than handing the archive to a tool that would write into it — and for
//! a CD that means moving roughly a disc-and-a-half of bytes, splitting them,
//! and hashing the result.
//!
//! Two different operations need the outcome of that work. Identifying a
//! carrier needs the per-track digests to look up in the catalog; building a
//! playable CHD needs the split CUE/BIN files themselves. Both used to do the
//! whole copy-and-split independently, so a convergence run that identified a
//! disc and then built it paid the cost twice.
//!
//! This module is the one place that performs it, keyed on the dump manifest
//! hash so a cache entry can only ever describe the exact bytes it was derived
//! from. Callers say whether the result is worth keeping for the next step.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use retro_junk_archive::{Redumper, RedumperAudit};
use retro_junk_io::{PhaseProgressFn, ProgressUnit};

use crate::playable_build::PlayableBuildError;

/// Filename of the cached audit record, stored beside the `raw/` split output.
const AUDIT_FILE: &str = "audit.json";

/// Where a dump's reusable split output lives.
///
/// Keyed on the dump manifest hash rather than the dump id: a repaired or
/// re-ingested dump gets a new hash, so it can never read a stale entry left
/// behind by its former contents.
#[must_use]
pub fn cache_directory(workspace_root: &Path, manifest_sha256: &str) -> PathBuf {
    workspace_root
        .join("prepared-redumper")
        .join(manifest_sha256)
}

/// A dump's regenerated tracks, plus the split files they were measured from.
pub struct Prepared {
    directory: PathBuf,
    audit: RedumperAudit,
    reused: bool,
}

impl Prepared {
    /// The regenerated per-track digests and the tool that produced them.
    #[must_use]
    pub fn audit(&self) -> &RedumperAudit {
        &self.audit
    }

    /// Whether this came from cache — that is, whether the copy-and-split was
    /// skipped entirely.
    #[must_use]
    pub fn reused(&self) -> bool {
        self.reused
    }

    /// The cache directory holding `audit.json` and the `raw/` split output.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The CUE (or ISO) a converter should read.
    pub fn entrypoint(&self) -> Result<PathBuf, PlayableBuildError> {
        crate::playable_build::find_input(&self.directory.join("raw"), &["cue", "iso"])
    }

    /// Leave the split output in place for whatever runs next.
    pub fn keep(self) {}

    /// Reclaim the scratch space, because nothing is going to use these files.
    ///
    /// A disc the catalog could not resolve gets no build, so keeping several
    /// hundred megabytes of split output for it would grow the workspace
    /// without bound across convergence runs.
    pub fn discard(self) {
        if let Err(error) = std::fs::remove_dir_all(&self.directory)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!(
                "could not remove prepared Redumper cache {}: {error}",
                self.directory.display()
            );
        }
    }
}

/// Regenerate a raw dump's tracks, reusing an earlier run's output when one is
/// available for these exact bytes.
///
/// On a cache hit this performs no copy, runs no tool, and returns immediately.
/// On a miss it copies the raw set to scratch storage, runs `split` and `hash`,
/// then moves the split output plus the audit record into the cache so the next
/// operation over the same dump is a hit.
pub fn prepare(
    redumper_path: &Path,
    raw_directory: &Path,
    workspace_root: &Path,
    manifest_sha256: &str,
    progress: &PhaseProgressFn<'_>,
    cancelled: &AtomicBool,
) -> Result<Prepared, PlayableBuildError> {
    let directory = cache_directory(workspace_root, manifest_sha256);
    if let Some(audit) = cached_audit(&directory) {
        progress(
            "Reusing prepared Redumper output",
            ProgressUnit::Items,
            1,
            1,
        );
        return Ok(Prepared {
            directory,
            audit,
            reused: true,
        });
    }
    // A partial entry means an earlier attempt died between moving the split
    // output and writing the audit record. It cannot be trusted, and it would
    // block the rename below, so it goes.
    if directory.exists() {
        std::fs::remove_dir_all(&directory)?;
    }
    let redumper = Redumper::detect(redumper_path)
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    let prepared = redumper
        .prepare_with_progress(raw_directory, workspace_root, cancelled, progress)
        .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
    let audit = prepared.audit.clone();
    store(&prepared, &directory, &audit, cancelled)?;
    Ok(Prepared {
        directory,
        audit,
        reused: false,
    })
}

/// Read a complete cache entry, or nothing.
///
/// Both halves have to be present: an audit record without its split output is
/// useless to a build, and split output without an audit record cannot be
/// trusted to describe the dump it sits under.
fn cached_audit(directory: &Path) -> Option<RedumperAudit> {
    if !directory.join("raw").is_dir() {
        return None;
    }
    let path = directory.join(AUDIT_FILE);
    if !path.is_file() {
        return None;
    }
    match retro_junk_archive::read_json::<RedumperAudit>(&path) {
        Ok(audit) if !audit.tracks.is_empty() => Some(audit),
        Ok(_) => None,
        Err(error) => {
            log::warn!(
                "ignoring unreadable prepared Redumper audit {}: {error}",
                path.display()
            );
            None
        }
    }
}

/// Move the split output and its audit record into the cache as one unit.
///
/// Built in a hidden staging directory and published by a single rename, so a
/// crash mid-write leaves either no entry or a complete one — never a directory
/// that looks reusable but is missing tracks.
fn store(
    prepared: &retro_junk_archive::RedumperWorkspace,
    destination: &Path,
    audit: &RedumperAudit,
    cancelled: &AtomicBool,
) -> Result<(), PlayableBuildError> {
    let parent = destination.parent().ok_or_else(|| {
        PlayableBuildError::Message("prepared cache has no parent directory".to_owned())
    })?;
    std::fs::create_dir_all(parent)?;
    let staging = parent.join(format!(
        ".prepared-staging-{}",
        retro_junk_archive::BuildId::new()
    ));
    let raw = staging.join("raw");
    std::fs::create_dir_all(&raw)?;
    let result = (|| {
        let mut sources =
            std::fs::read_dir(prepared.entrypoint.parent().ok_or_else(|| {
                PlayableBuildError::Message("invalid Redumper output".to_owned())
            })?)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| {
                        matches!(value.to_ascii_lowercase().as_str(), "cue" | "bin" | "iso")
                    })
            })
            .collect::<Vec<_>>();
        sources.sort();
        for source in sources {
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(PlayableBuildError::Message(
                    "operation cancelled".to_owned(),
                ));
            }
            let name = source.file_name().ok_or_else(|| {
                PlayableBuildError::Message(format!(
                    "prepared file has no filename: {}",
                    source.display()
                ))
            })?;
            std::fs::rename(&source, raw.join(name))?;
        }
        retro_junk_archive::write_json_new(&staging.join(AUDIT_FILE), audit)
            .map_err(|error| PlayableBuildError::Message(error.to_string()))?;
        std::fs::rename(&staging, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

#[cfg(test)]
#[path = "tests/redumper_cache_tests.rs"]
mod tests;
