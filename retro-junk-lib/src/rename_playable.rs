//! Bring a built playable's filename back to what the catalog calls it.
//!
//! A playable built before the naming rule was corrected — or before a DAT
//! renamed the game — keeps its old name forever. Nothing rebuilt it, because
//! a file that exists satisfies the build gap, and nothing renamed it, because
//! until now nothing compared a playable's name to the rule.
//!
//! The repair moves the file, the scraped media named after it, and the
//! playlist entry pointing at it, then appends build evidence naming the new
//! location. Evidence is append-only history, so the record of where the file
//! used to be is kept rather than rewritten.

#[cfg(test)]
#[path = "tests/rename_playable_tests.rs"]
mod tests;

use std::path::{Path, PathBuf};

use retro_junk_archive::{ArchiveIndexSnapshot, IndexedDump, IndexedRelease};

/// One playable to rename, as the caller identified it.
pub struct RenamePlayableRequest<'a> {
    pub snapshot: &'a ArchiveIndexSnapshot,
    pub playable_root: &'a Path,
    /// The representation whose file is misnamed.
    pub representation_id: &'a str,
    /// The filename the naming rule says it should have.
    pub canonical_file_name: &'a str,
    /// Frontend media directory, when scraped artwork should follow the
    /// rename. Artwork is named after the playable's stem, so leaving it
    /// behind would silently unlink a game from its own box art.
    pub media_root: Option<&'a Path>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RenamePlayableReport {
    pub from: String,
    pub to: String,
    /// Scraped media files that followed the rename.
    pub media_renamed: usize,
    /// Playlists rewritten to point at the new name.
    pub playlists_updated: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum RenamePlayableError {
    #[error("no built playable {0} is recorded in the archive")]
    UnknownRepresentation(String),
    #[error("the playable file is not where its evidence says: {0}")]
    SourceMissing(String),
    #[error("something already exists at the canonical name: {0}")]
    TargetExists(String),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Message(String),
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> RenamePlayableError + '_ {
    move |source| RenamePlayableError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Find the release and dump whose build evidence currently names this
/// representation, and that evidence.
///
/// The *current* record specifically: `evidence/` is append-only, so a file
/// that has been renamed or rebuilt before has several records, and only the
/// live one describes where it is now.
///
/// The release comes back with it because finding the file needs it: which
/// frontend folder a playable sits in is a property of its release, not of
/// the evidence record.
fn locate<'a>(
    snapshot: &'a ArchiveIndexSnapshot,
    representation_id: &str,
) -> Option<(
    &'a IndexedRelease,
    &'a IndexedDump,
    &'a retro_junk_archive::BuildEvidence,
)> {
    for release in &snapshot.releases {
        for copy in &release.physical_copies {
            for carrier in &copy.carriers {
                for dump in &carrier.dumps {
                    if let Some(evidence) = retro_junk_archive::current_build_evidence(dump)
                        .into_iter()
                        .find(|evidence| {
                            evidence.child_representation_id.to_string() == representation_id
                        })
                    {
                        return Some((release, dump, evidence));
                    }
                }
            }
        }
    }
    None
}

/// Rename a built playable to its canonical name, taking its companions with
/// it. Returns what moved.
pub fn rename_playable(
    request: &RenamePlayableRequest<'_>,
) -> Result<RenamePlayableReport, RenamePlayableError> {
    let (release, dump, current) =
        locate(request.snapshot, request.representation_id).ok_or_else(|| {
            RenamePlayableError::UnknownRepresentation(request.representation_id.to_owned())
        })?;
    // Ask where the file *is*, not where it was written, by the same route the
    // projection took when it decided this playable was misnamed. Reading the
    // recorded path directly moved nothing for every release whose archive
    // folder differs from its frontend folder.
    let relative_path =
        crate::playable_location::release_output_relative(release, request.playable_root, current);
    let source = request.playable_root.join(&relative_path);
    if !source.is_file() {
        return Err(RenamePlayableError::SourceMissing(
            source.display().to_string(),
        ));
    }
    let parent = source
        .parent()
        .ok_or_else(|| RenamePlayableError::Message("playable has no parent directory".into()))?;
    let target = parent.join(request.canonical_file_name);
    if target == source {
        return Ok(RenamePlayableReport {
            from: relative_path.clone(),
            to: relative_path,
            ..RenamePlayableReport::default()
        });
    }
    // Renaming onto an existing file would destroy it, and a name collision
    // here means something the tool did not expect — two playables that the
    // rule says share one name. Stop and let a person look.
    if target.exists() {
        return Err(RenamePlayableError::TargetExists(
            target.display().to_string(),
        ));
    }
    let old_stem = file_stem(&source);
    let new_stem = file_stem(&target);

    std::fs::rename(&source, &target).map_err(io(&source))?;

    let mut report = RenamePlayableReport {
        from: relative_path.clone(),
        to: relative_to(request.playable_root, &target),
        ..RenamePlayableReport::default()
    };
    report.media_renamed = rename_companion_media(request.media_root, &old_stem, &new_stem);
    report.playlists_updated = rewrite_playlists(parent, &source, &target);

    // Append, never rewrite: the earlier record is still true about where the
    // file used to be, and the archive keeps its history.
    //
    // The new record is the old one with a new path. A rename changes what a
    // file is called and nothing else, so every other field is carried
    // forward — including `format`, which together with the parent
    // representation identifies the build *lineage*. Substituting the dump's
    // own format here started a second lineage, leaving two live records
    // claiming one representation id, which the projection rejects. Copying
    // also preserves what was verified about these bytes: re-deriving
    // `round_trip_verified` would quietly downgrade a playable that had been
    // proven to reproduce its master.
    let digests =
        retro_junk_archive::hash_file_digests(&target, &std::sync::atomic::AtomicBool::new(false))
            .map_err(|error| RenamePlayableError::Message(error.to_string()))?;
    let evidence = retro_junk_archive::BuildEvidence {
        build_id: retro_junk_archive::BuildId::new(),
        performed_at: chrono::Utc::now().to_rfc3339(),
        relative_output_path: report.to.clone(),
        // Re-hashed rather than copied: the bytes should be identical, but
        // recording a digest this run did not observe would be a claim
        // rather than a measurement.
        output_sha256: digests.sha256,
        output_size: digests.size,
        ..current.clone()
    };
    retro_junk_archive::write_build_evidence(&dump.directory, &evidence)
        .map_err(|error| RenamePlayableError::Message(error.to_string()))?;
    Ok(report)
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_owned()
}

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Move scraped artwork named after the old stem onto the new one.
///
/// Best-effort by design: artwork that cannot be moved is re-scrapable, and
/// failing the rename over it would leave the playable at its wrong name.
fn rename_companion_media(media_root: Option<&Path>, old_stem: &str, new_stem: &str) -> usize {
    let Some(media_root) = media_root else {
        return 0;
    };
    if old_stem.is_empty() || new_stem.is_empty() || !media_root.is_dir() {
        return 0;
    }
    let mut moved = 0;
    let mut directories = vec![media_root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
                continue;
            }
            if file_stem(&path) != old_stem {
                continue;
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let target = path.with_file_name(if extension.is_empty() {
                new_stem.to_owned()
            } else {
                format!("{new_stem}.{extension}")
            });
            if !target.exists() && std::fs::rename(&path, &target).is_ok() {
                moved += 1;
            }
        }
    }
    moved
}

/// Point any playlist in the same directory at the new filename.
///
/// A multi-disc set's `.m3u` lists its discs by name, so a renamed disc that
/// is not also renamed in the playlist breaks the set.
fn rewrite_playlists(directory: &Path, source: &Path, target: &Path) -> usize {
    let (Some(old_name), Some(new_name)) = (
        source.file_name().and_then(|name| name.to_str()),
        target.file_name().and_then(|name| name.to_str()),
    ) else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    let mut updated = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|ext| !ext.eq_ignore_ascii_case("m3u"))
        {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !contents.contains(old_name) {
            continue;
        }
        let rewritten = contents.replace(old_name, new_name);
        if std::fs::write(&path, rewritten).is_ok() {
            updated += 1;
        }
    }
    updated
}

/// Where a playable's companion media live for one console folder.
#[must_use]
pub fn media_directory(media_root: &Path, console_folder: &str) -> PathBuf {
    media_root.join(console_folder)
}
