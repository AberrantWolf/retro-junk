//! Cheap, device-local representation presence checks.
//!
//! Presence is intentionally distinct from integrity. These checks inspect file
//! type and recorded size only; cryptographic verification remains an explicit
//! operation with portable evidence.

use std::path::Path;

use crate::{ArchivedFile, BuildEvidence, DumpManifest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepresentationPresence {
    Present,
    Missing,
    Partial,
    Modified,
    Stale,
}

impl RepresentationPresence {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Missing => "missing",
            Self::Partial => "partial",
            Self::Modified => "modified",
            Self::Stale => "stale",
        }
    }
}

#[must_use]
pub fn preservation_presence(
    dump_directory: &Path,
    manifest: &DumpManifest,
) -> RepresentationPresence {
    archived_files_presence(&dump_directory.join("raw"), &manifest.files)
}

#[must_use]
pub fn archived_files_presence(
    raw_directory: &Path,
    files: &[ArchivedFile],
) -> RepresentationPresence {
    if files.is_empty() {
        return RepresentationPresence::Modified;
    }
    let mut present = 0_usize;
    for expected in files {
        let path = raw_directory.join(&expected.path);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                present += 1;
                if metadata.len() != expected.size {
                    return RepresentationPresence::Modified;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) | Err(_) => return RepresentationPresence::Modified,
        }
    }
    match present {
        0 => RepresentationPresence::Missing,
        count if count == files.len() => RepresentationPresence::Present,
        _ => RepresentationPresence::Partial,
    }
}

/// Where a build's output actually is, and whether it is there.
///
/// Portable build evidence is historical: it records where the output *was*
/// written, which is not always where it lives now. Evidence written before
/// outputs were filed under a platform directory records a bare file name, and
/// re-pointing a collection at a different frontend moves files between system
/// directories. Neither is a moved file in any sense that matters — the
/// evidence still describes it — so both resolve here.
///
/// This has to be the only definition. The projection resolved these and the
/// orphan scan did not, so 248 outputs the projection reported `present` were
/// simultaneously "missing" to adoption, which would have appended a
/// redundant evidence record for every one of them.
///
/// `system_directory` is the frontend system directory this release's outputs
/// belong to; pass `""` to resolve against the recorded path alone.
#[must_use]
pub fn resolve_playable(
    playable_root: &Path,
    current_input_manifest_sha256: &str,
    evidence: &BuildEvidence,
    system_directory: &str,
) -> (String, RepresentationPresence) {
    let presence = playable_presence(playable_root, current_input_manifest_sha256, evidence);
    if presence != RepresentationPresence::Missing {
        return (evidence.relative_output_path.clone(), presence);
    }
    let resolved = resolve_playable_path(playable_root, evidence, system_directory);
    if resolved == evidence.relative_output_path {
        return (resolved, presence);
    }
    let mut relocated = evidence.clone();
    relocated.relative_output_path = resolved;
    let relocated_presence =
        playable_presence(playable_root, current_input_manifest_sha256, &relocated);
    if relocated_presence == RepresentationPresence::Present {
        (relocated.relative_output_path, relocated_presence)
    } else {
        (evidence.relative_output_path.clone(), presence)
    }
}

/// Where a build's output is, as a path relative to the playable root.
///
/// The location half of [`resolve_playable`], on its own. It answers only
/// "which file is this evidence talking about", and takes no view on whether
/// those bytes are current — so a caller that just needs to find the file
/// cannot get the freshness question wrong by accident, and cannot be tempted
/// to read `relative_output_path` directly instead.
///
/// Reading that field directly is the mistake this exists to prevent: it says
/// where the output was *written*, and a playable written under the archive's
/// platform folder (`ps1`) while the frontend files it under `psx` is then
/// reported missing by everything that looks for it literally.
#[must_use]
pub fn resolve_playable_path(
    playable_root: &Path,
    evidence: &BuildEvidence,
    system_directory: &str,
) -> String {
    let recorded = Path::new(&evidence.relative_output_path);
    if system_directory.is_empty() || recorded.as_os_str().is_empty() {
        return evidence.relative_output_path.clone();
    }
    // Where the evidence says, if something is actually there. Only a file
    // that is absent is worth looking elsewhere for.
    if std::fs::symlink_metadata(playable_root.join(recorded)).is_ok() {
        return evidence.relative_output_path.clone();
    }
    // Only a leading *directory* names the old location. Evidence that records
    // a bare file name has no directory to replace — the file name is the tail
    // and the system directory is simply prepended.
    let mut components = recorded.components();
    let leading = components.next();
    let tail = components.as_path();
    let (leading, tail) = if tail.as_os_str().is_empty() {
        (None, recorded)
    } else {
        match leading.and_then(|component| component.as_os_str().to_str()) {
            Some(directory) => (Some(directory), tail),
            None => return evidence.relative_output_path.clone(),
        }
    };
    if leading.is_some_and(|directory| directory.eq_ignore_ascii_case(system_directory)) {
        return evidence.relative_output_path.clone();
    }
    let relocated = Path::new(system_directory)
        .join(tail)
        .to_string_lossy()
        .replace('\\', "/");
    // Accept the new location only if the file there is the one the evidence
    // describes. A same-named file of a different size is somebody else's.
    match std::fs::symlink_metadata(playable_root.join(&relocated)) {
        Ok(metadata)
            if metadata.file_type().is_file() && metadata.len() == evidence.output_size =>
        {
            log::debug!(
                "Resolved playable {} -> {relocated}",
                evidence.relative_output_path
            );
            relocated
        }
        Ok(_) | Err(_) => evidence.relative_output_path.clone(),
    }
}

/// Whether the output the evidence names is where the evidence says.
///
/// Most callers want [`resolve_playable`] instead: this is the raw predicate
/// it is built from, and on its own it calls a file that merely lives under
/// the frontend's system directory missing.
#[must_use]
pub fn playable_presence(
    playable_root: &Path,
    current_input_manifest_sha256: &str,
    evidence: &BuildEvidence,
) -> RepresentationPresence {
    if evidence.input_manifest_sha256 != current_input_manifest_sha256 {
        return RepresentationPresence::Stale;
    }
    let path = playable_root.join(&evidence.relative_output_path);
    match std::fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_file() && metadata.len() == evidence.output_size =>
        {
            RepresentationPresence::Present
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            RepresentationPresence::Missing
        }
        Ok(_) | Err(_) => RepresentationPresence::Modified,
    }
}
