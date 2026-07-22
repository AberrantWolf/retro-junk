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
