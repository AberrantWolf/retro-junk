//! Cheap console-folder change detection.
//!
//! A fingerprint covers the folder's entry names *and their size and
//! modification time*, one directory level deep. It never reads file
//! contents, so it stays cheap, but it does notice a file replaced in place.
//!
//! Names alone are not enough. A re-dump, a CHD recompressed at a different
//! level, or a truncated copy keeps its filename, so a name-only fingerprint
//! reported "nothing changed" and the stored hashes, DAT match, and
//! verification verdict for the old bytes stayed on the row indefinitely.
//! One `stat` per entry is far cheaper than the scan it decides to skip.

use sha2::{Digest, Sha256};
use std::path::Path;

#[cfg(test)]
#[path = "fingerprint_tests.rs"]
mod tests;

#[derive(Debug, Clone)]
pub struct FolderFingerprint {
    /// Hash of the folder's sorted entries, each with its size and mtime.
    pub name_hash: String,
}

/// Describe one directory entry for the fingerprint: its path, and the size
/// and modification time when they can be read.
///
/// Unreadable metadata contributes a fixed marker rather than being skipped,
/// so a file that becomes unreadable is itself a change.
fn describe(label: &str, metadata: Option<std::fs::Metadata>) -> String {
    let Some(metadata) = metadata else {
        return format!("{label}\0?");
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(
            || "?".to_owned(),
            |since| format!("{}.{}", since.as_secs(), since.subsec_nanos()),
        );
    format!("{label}\0{}\0{modified}", metadata.len())
}

/// Compute a quick fingerprint from directory entries and their metadata.
pub fn compute_fingerprint(path: &Path) -> FolderFingerprint {
    let mut entries_described = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Ok(file_type) = entry.file_type()
                && file_type.is_dir()
                && let Ok(children) = std::fs::read_dir(entry.path())
            {
                for child in children.flatten() {
                    let label = format!("{name}/{}", child.file_name().to_string_lossy());
                    entries_described.push(describe(&label, child.metadata().ok()));
                }
            }
            entries_described.push(describe(&name, entry.metadata().ok()));
        }
    }
    entries_described.sort();
    let hash = Sha256::digest(entries_described.join("\n").as_bytes());
    let name_hash = hash[..8].iter().fold(String::new(), |mut output, byte| {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
        output
    });
    FolderFingerprint { name_hash }
}
