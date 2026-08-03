//! Re-adopting playable outputs that moved out from under their evidence.
//!
//! Build evidence records where a derivative was written, and that recorded
//! path is what binds a scanned library row back to the archived carrier that
//! produced it. Anything that renames or moves the file outside the archive —
//! a frontend tidy-up, a bulk rename, a build that predates a naming fix —
//! severs that binding, and the release then reads as "archived, not playable"
//! beside a "playable, not archived" row for the very same bytes.
//!
//! The recorded SHA-256 still identifies the file, so the link is recoverable:
//! find the moved output by content and append new build evidence naming its
//! current location. Recording the result is what keeps this a one-time repair
//! rather than a hash sweep on every projection — and it is the same thing a
//! deliberate rename owes the archive.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::index::{ArchiveIndexSnapshot, IndexedDump, IndexedRelease};
use crate::manifest::{
    ArchiveReleaseId, BuildEvidence, BuildId, RepresentationId, ToolRecord, write_json_new,
};
use crate::presence::RepresentationPresence;

#[derive(Debug, thiserror::Error)]
pub enum AdoptError {
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error(transparent)]
    Manifest(#[from] crate::manifest::ManifestError),
    #[error("operation cancelled")]
    Cancelled,
}

/// The tool string recorded on adoption evidence, so the archive distinguishes
/// "this file was produced here" from "this file was found again here".
pub const ADOPTION_TOOL: &str = "retro-junk adopt";

/// A playable output whose recorded location no longer holds it.
#[derive(Debug, Clone)]
pub struct OrphanedPlayable {
    pub archive_release_id: ArchiveReleaseId,
    /// Release label for progress and reporting.
    pub label: String,
    /// Directory of the dump that owns the build evidence.
    pub dump_directory: PathBuf,
    /// The build evidence whose output went missing.
    pub evidence: BuildEvidence,
}

/// Every current playable build whose output cannot be found at all.
///
/// Only *current* builds count: evidence computed from a superseded dump
/// manifest is history, and a superseded build within one lineage has already
/// been replaced by a later one. Neither is an orphan.
///
/// `system_directory` maps a release to the frontend system directory its
/// outputs belong to, so this resolves exactly what the projection resolves —
/// a file merely sitting under that directory is found, not adopted. Without
/// it the two disagreed, and adoption would rewrite evidence for hundreds of
/// outputs the projection was already resolving correctly.
#[must_use]
pub fn orphaned_playables(
    snapshot: &ArchiveIndexSnapshot,
    playable_root: &Path,
    system_directory: &dyn Fn(&str, &str) -> String,
) -> Vec<OrphanedPlayable> {
    let mut orphans = Vec::new();
    for release in &snapshot.releases {
        let label = release_label(&release.manifest.title, &release.manifest.region);
        let directory = system_directory(&release.manifest.platform_id, &release.manifest.region);
        for dump in release
            .physical_copies
            .iter()
            .flat_map(|copy| &copy.carriers)
            .flat_map(|carrier| &carrier.dumps)
        {
            for evidence in current_build_evidence(dump) {
                let (_, presence) = crate::presence::resolve_playable(
                    playable_root,
                    &dump.manifest_sha256,
                    evidence,
                    &directory,
                );
                if presence == RepresentationPresence::Missing {
                    orphans.push(OrphanedPlayable {
                        archive_release_id: release.manifest.archive_release_id,
                        label: label.clone(),
                        dump_directory: dump.directory.clone(),
                        evidence: evidence.clone(),
                    });
                }
            }
        }
    }
    orphans
}

/// The build evidence that describes each derivative's present state.
///
/// `evidence/` is append-only, so one dump accumulates a record per build. A
/// lineage is what a derivative was built *from* plus the format it produced:
/// rebuilding under a corrected filename, or re-adopting a moved output, is
/// the same lineage rather than a second derivative, and only its newest
/// record describes a file that should exist now. Grouping by output path
/// instead would leave every superseded path behind as a permanently missing
/// representation. Build ids are time-ordered `UUIDv7` and the scanner sorts by
/// filename, so arrival order is time order.
///
/// Superseded records stay in the archive — this decides what is current, not
/// what is kept.
#[must_use]
pub fn current_build_evidence(dump: &IndexedDump) -> Vec<&BuildEvidence> {
    let mut current: BTreeMap<(RepresentationId, String), &BuildEvidence> = BTreeMap::new();
    for build in &dump.builds {
        current.insert(
            (
                build.evidence.parent_representation_id,
                format!("{:?}", build.evidence.format),
            ),
            &build.evidence,
        );
    }
    current.into_values().collect()
}

/// Every build across a release that describes a file which should exist now.
///
/// The release-level form of [`current_build_evidence`], for the projections
/// that publish a release rather than a dump: frontend media, gamelist
/// entries. They need the same answer, and getting it from `dump.builds`
/// directly is wrong in a way that compounds — `evidence/` is append-only, so
/// a release rebuilt under a corrected name has *both* names in its history,
/// and a projection that reads all of them keeps republishing artwork under a
/// name no file has carried for months.
#[must_use]
pub fn current_release_builds(release: &IndexedRelease) -> Vec<&BuildEvidence> {
    release
        .physical_copies
        .iter()
        .flat_map(|copy| &copy.carriers)
        .flat_map(|carrier| &carrier.dumps)
        .flat_map(current_build_evidence)
        .collect()
}

/// Builds a release has published under and no longer does.
///
/// What a projection has to clean up rather than write: the artwork and
/// gamelist entries left behind by an output that was rebuilt or re-adopted
/// under a different name. A path that some *other* lineage currently
/// publishes is excluded — two lineages can converge on one output name, and
/// that name is live.
#[must_use]
pub fn superseded_release_builds(release: &IndexedRelease) -> Vec<&BuildEvidence> {
    let current = current_release_builds(release)
        .into_iter()
        .map(|evidence| evidence.relative_output_path.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let current_ids = current_release_builds(release)
        .into_iter()
        .map(|evidence| evidence.build_id)
        .collect::<std::collections::BTreeSet<_>>();
    release
        .physical_copies
        .iter()
        .flat_map(|copy| &copy.carriers)
        .flat_map(|carrier| &carrier.dumps)
        .flat_map(|dump| &dump.builds)
        .map(|build| &build.evidence)
        .filter(|evidence| {
            !current_ids.contains(&evidence.build_id)
                && !current.contains(evidence.relative_output_path.as_str())
        })
        .collect()
}

/// Files below each of `directories`, grouped by size.
///
/// Size is the only discriminator available without reading file contents, and
/// it is a strong one: an exact byte-size collision between two different disc
/// images is rare enough that hashing is near-always a single file. One walk
/// serves every orphan in the run.
///
/// The caller chooses breadth. A stat per file is the cost here and a playable
/// library over a network share is thousands of them, so a run repairing one
/// release passes that release's system directories rather than the whole
/// root; a deliberate whole-library sweep passes the root itself.
pub fn index_by_size(directories: &[PathBuf]) -> Result<BTreeMap<u64, Vec<PathBuf>>, AdoptError> {
    let mut index: BTreeMap<u64, Vec<PathBuf>> = BTreeMap::new();
    // Sorted so an ancestor is always considered before its descendants: the
    // broader directory is the one worth walking, whatever order the caller
    // listed them in.
    let mut ordered: Vec<&Path> = directories.iter().map(PathBuf::as_path).collect();
    ordered.sort_unstable();
    let mut walked: Vec<&Path> = Vec::new();
    for directory in ordered {
        // Re-walking a nested or repeated directory would index the same file
        // twice, and a second orphan could then adopt a path the first
        // already claimed.
        if walked.iter().any(|seen| directory.starts_with(seen)) {
            continue;
        }
        walked.push(directory);
        collect_sizes(directory, &mut index)?;
    }
    Ok(index)
}

/// The directories a run should search for an orphan: the system directory its
/// recorded path names, which is where a rename leaves the file.
///
/// A file moved to a different system directory entirely is beyond what a
/// scoped repair should guess at; the whole-root sweep still finds it.
#[must_use]
pub fn search_directories(playable_root: &Path, orphans: &[OrphanedPlayable]) -> Vec<PathBuf> {
    let mut directories: BTreeSet<PathBuf> = BTreeSet::new();
    for orphan in orphans {
        let recorded = Path::new(&orphan.evidence.relative_output_path);
        // A bare filename (evidence predating platform directories) names the
        // root itself, which is the correct place to look for it.
        let system = recorded
            .components()
            .next()
            .filter(|_| recorded.components().count() > 1)
            .map_or_else(
                || playable_root.to_path_buf(),
                |first| playable_root.join(first.as_os_str()),
            );
        directories.insert(system);
    }
    directories.into_iter().collect()
}

fn collect_sizes(
    directory: &Path,
    index: &mut BTreeMap<u64, Vec<PathBuf>>,
) -> Result<(), AdoptError> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        // A playable root that does not exist yet simply holds nothing.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(AdoptError::Io {
                path: directory.display().to_string(),
                source,
            });
        }
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            collect_sizes(&path, index)?;
        } else if metadata.is_file() {
            index.entry(metadata.len()).or_default().push(path);
        }
    }
    Ok(())
}

/// Find the current location of an orphaned output by content.
///
/// `claimed` holds relative paths other evidence already names, so two
/// byte-identical outputs cannot both adopt the same file.
pub fn locate_by_content(
    playable_root: &Path,
    orphan: &OrphanedPlayable,
    by_size: &BTreeMap<u64, Vec<PathBuf>>,
    claimed: &BTreeSet<String>,
    cancel: &AtomicBool,
) -> Result<Option<String>, AdoptError> {
    if orphan.evidence.output_sha256.is_empty() {
        // Nothing to match on. Evidence this old cannot be re-adopted safely.
        return Ok(None);
    }
    let Some(candidates) = by_size.get(&orphan.evidence.output_size) else {
        return Ok(None);
    };
    for candidate in candidates {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(AdoptError::Cancelled);
        }
        let relative = relative_path(playable_root, candidate);
        if claimed.contains(&relative) {
            continue;
        }
        let (_, sha256) =
            crate::verify::sha256_file(candidate, cancel).map_err(|error| AdoptError::Io {
                path: candidate.display().to_string(),
                source: std::io::Error::other(error.to_string()),
            })?;
        if sha256.eq_ignore_ascii_case(&orphan.evidence.output_sha256) {
            return Ok(Some(relative));
        }
    }
    Ok(None)
}

/// Append evidence that the orphan's output now lives at `relative_path`.
///
/// A new child representation id is required, not optional: the old record
/// still names the old path, and the projection keys representation rows by
/// that id, so reusing it would collide. The new record supersedes the old one
/// within its lineage, which is what retires the stale path.
pub fn record_adoption(
    orphan: &OrphanedPlayable,
    relative_path: &str,
) -> Result<BuildEvidence, AdoptError> {
    let evidence = BuildEvidence {
        build_id: BuildId::new(),
        child_representation_id: RepresentationId::new(),
        performed_at: chrono::Utc::now().to_rfc3339(),
        relative_output_path: relative_path.to_owned(),
        tool: Some(ToolRecord {
            name: ADOPTION_TOOL.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            build: String::new(),
        }),
        ..orphan.evidence.clone()
    };
    write_build_evidence(&orphan.dump_directory, &evidence)?;
    Ok(evidence)
}

/// Write one build evidence record into a dump's append-only `evidence/`.
///
/// The filename encodes the build id, whose `UUIDv7` ordering is what lets the
/// scanner recover arrival order from a sorted directory listing. One writer,
/// so that contract cannot drift between the build, import, and adopt paths.
pub fn write_build_evidence(
    dump_directory: &Path,
    evidence: &BuildEvidence,
) -> Result<PathBuf, AdoptError> {
    let directory = dump_directory.join("evidence");
    std::fs::create_dir_all(&directory).map_err(|source| AdoptError::Io {
        path: directory.display().to_string(),
        source,
    })?;
    let path = directory.join(format!("build-{}.json", evidence.build_id));
    write_json_new(&path, evidence)?;
    Ok(path)
}

/// The relative paths current build evidence already names, so adoption never
/// hands one file to two representations.
#[must_use]
pub fn claimed_playable_paths(snapshot: &ArchiveIndexSnapshot) -> BTreeSet<String> {
    snapshot
        .releases
        .iter()
        .flat_map(|release| &release.physical_copies)
        .flat_map(|copy| &copy.carriers)
        .flat_map(|carrier| &carrier.dumps)
        .flat_map(current_build_evidence)
        .map(|evidence| evidence.relative_output_path.clone())
        .collect()
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn release_label(title: &str, region: &str) -> String {
    if region.is_empty() {
        title.to_owned()
    } else {
        format!("{title} ({region})")
    }
}

#[cfg(test)]
#[path = "tests/adopt_tests.rs"]
mod tests;
