//! Cheap console-folder change detection.
//!
//! A fingerprint is a hash of the folder's entry names (one directory level
//! deep) — no file contents, sizes, or timestamps — so a scan can tell "this
//! folder changed since last time" without touching any file data.

use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct FolderFingerprint {
    /// Hash of sorted filenames in the folder (no metadata/stat calls needed).
    pub name_hash: String,
}

/// Compute a quick fingerprint using directory names and file types only.
pub fn compute_fingerprint(path: &Path) -> FolderFingerprint {
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Ok(file_type) = entry.file_type()
                && file_type.is_dir()
                && let Ok(children) = std::fs::read_dir(entry.path())
            {
                for child in children.flatten() {
                    names.push(format!("{name}/{}", child.file_name().to_string_lossy()));
                }
            }
            names.push(name);
        }
    }
    names.sort();
    let hash = Sha256::digest(names.join("\n").as_bytes());
    let name_hash = hash[..8].iter().fold(String::new(), |mut output, byte| {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
        output
    });
    FolderFingerprint { name_hash }
}
