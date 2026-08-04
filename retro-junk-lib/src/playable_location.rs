//! Where a release's built playables actually are.
//!
//! Build evidence records where an output was *written*. That is not always
//! where it lives: outputs written before playables were filed by system
//! folder record a bare file name, and an output written under the archive's
//! platform folder (`ps1`) sits in the frontend's system folder (`psx`). The
//! evidence still describes the file — one directory over.
//!
//! Two questions come out of that, and every caller needs both answered
//! together: *which frontend folder does this release publish into*, and
//! *where does its evidence therefore point*. The archive crate owns the
//! second and has no business knowing the first, so this pairs them, and is
//! the only place in the glue layer that should be turning build evidence
//! into a path. Reading `relative_output_path` and joining it to the playable
//! root is the bug this module exists to make unnecessary: it silently drops
//! every release whose archive folder and frontend folder differ.

#[cfg(test)]
#[path = "tests/playable_location_tests.rs"]
mod tests;

use std::path::{Path, PathBuf};

use retro_junk_archive::{BuildEvidence, IndexedDump, IndexedRelease, RepresentationPresence};

/// The frontend system folder a release's playables are published into.
#[must_use]
pub fn release_system_directory(release: &IndexedRelease) -> String {
    retro_junk_frontend::esde::system_directory(
        &release.manifest.platform_id,
        Some(&release.manifest.region),
    )
}

/// Where one of a release's build outputs is, relative to the playable root.
///
/// Says nothing about whether the output is current — use
/// [`release_output_presence`] when that matters.
#[must_use]
pub fn release_output_relative(
    release: &IndexedRelease,
    playable_root: &Path,
    evidence: &BuildEvidence,
) -> String {
    retro_junk_archive::resolve_playable_path(
        playable_root,
        evidence,
        &release_system_directory(release),
    )
}

/// The same location as an absolute path.
#[must_use]
pub fn release_output_path(
    release: &IndexedRelease,
    playable_root: &Path,
    evidence: &BuildEvidence,
) -> PathBuf {
    playable_root.join(release_output_relative(release, playable_root, evidence))
}

/// Where one of a release's build outputs is, and whether it is there and
/// still built from the dump the archive holds now.
///
/// The dump supplies the fingerprint of its current manifest: an output built
/// from an earlier dump of the same carrier is reported
/// [`RepresentationPresence::Stale`] however findable its file is, because
/// what is on disk no longer reproduces what the archive preserves.
#[must_use]
pub fn release_output_presence(
    release: &IndexedRelease,
    playable_root: &Path,
    dump: &IndexedDump,
    evidence: &BuildEvidence,
) -> (String, RepresentationPresence) {
    retro_junk_archive::resolve_playable(
        playable_root,
        &dump.manifest_sha256,
        evidence,
        &release_system_directory(release),
    )
}
