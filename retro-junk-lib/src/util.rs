use std::path::{Path, PathBuf};

// Re-export core utilities for backwards compatibility.
pub use retro_junk_core::util::*;

/// Compute the default media directory for a given ROM root path.
///
/// Convention: `{parent}/{folder_name}-media`.
/// For `/path/to/roms` → `/path/to/roms-media`.
pub fn default_media_dir(root: &Path) -> PathBuf {
    root.parent().unwrap_or(root).join(format!(
        "{}-media",
        root.file_name().unwrap_or_default().to_string_lossy()
    ))
}
