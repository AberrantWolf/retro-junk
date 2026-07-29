use std::path::{Path, PathBuf};

// Re-export core utilities for backwards compatibility.
pub use retro_junk_core::util::*;

/// Compute the default media directory for a given ROM root path.
///
/// Convention: `{parent}/{folder_name}-media`.
/// For `/path/to/roms` → `/path/to/roms-media`.
#[must_use]
pub fn default_media_dir(root: &Path) -> PathBuf {
    default_sibling_dir(root, "media")
}

/// Compute the default metadata directory (gamelist.xml etc.) for a ROM root.
///
/// Convention: `{parent}/{folder_name}-metadata`.
#[must_use]
pub fn default_metadata_dir(root: &Path) -> PathBuf {
    default_sibling_dir(root, "metadata")
}

fn default_sibling_dir(root: &Path, suffix: &str) -> PathBuf {
    root.parent().unwrap_or(root).join(format!(
        "{}-{suffix}",
        root.file_name().unwrap_or_default().to_string_lossy()
    ))
}

/// Per-console media directory honoring the user's assets-dir setting.
///
/// Empty setting = the `{root}-media` sibling convention; otherwise the
/// setting is a path, absolute or relative to `root_path`. Returns `None`
/// only when the sibling convention is unusable (rootless path).
#[must_use]
pub fn asset_dir_for_console(
    root_path: &Path,
    folder_name: &str,
    setting: &str,
) -> Option<PathBuf> {
    if setting.is_empty() {
        root_path.parent()?;
        root_path.file_name()?;
        Some(default_media_dir(root_path).join(folder_name))
    } else {
        Some(resolve_dir(root_path, setting).join(folder_name))
    }
}

/// Per-console metadata directory honoring the user's metadata-dir setting.
///
/// The setting is a path, absolute or relative to `root_path`. Setting `"."`
/// places metadata inline with ROMs (ES-DE legacy mode).
#[must_use]
pub fn metadata_dir_for_console(root_path: &Path, folder_name: &str, setting: &str) -> PathBuf {
    resolve_dir(root_path, setting).join(folder_name)
}

/// Resolve a directory setting: absolute paths as-is, relative paths from
/// `root_path`.
fn resolve_dir(root_path: &Path, setting: &str) -> PathBuf {
    let p = Path::new(setting);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root_path.join(p)
    }
}
