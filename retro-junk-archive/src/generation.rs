use std::path::{Path, PathBuf};

use crate::ManifestError;

const GENERATION_FILE: &str = "projection-generation";

#[must_use]
pub fn projection_generation_path(root: &Path) -> PathBuf {
    root.join(".retro-junk").join(GENERATION_FILE)
}

/// Generation of the authoritative archive tree seen by rebuildable
/// projections. Archives predating the marker start at zero.
pub fn projection_generation(root: &Path) -> Result<u64, ManifestError> {
    let path = projection_generation_path(root);
    match std::fs::read_to_string(&path) {
        Ok(value) => value.trim().parse().map_err(|source| ManifestError::Io {
            path: path.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(source) => Err(ManifestError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// Advance the archive generation atomically. Callers hold the archive lock.
/// Recording it before the filesystem mutation means a crash may cause one
/// harmless deep refresh, but can never hide a committed change.
pub fn advance_projection_generation(root: &Path) -> Result<u64, ManifestError> {
    let next = projection_generation(root)?.saturating_add(1);
    let path = projection_generation_path(root);
    let parent = path.parent().expect("generation marker has a parent");
    std::fs::create_dir_all(parent).map_err(|source| ManifestError::Io {
        path: parent.display().to_string(),
        source,
    })?;
    let temporary = parent.join(format!(".{GENERATION_FILE}.tmp"));
    std::fs::write(&temporary, format!("{next}\n")).map_err(|source| ManifestError::Io {
        path: temporary.display().to_string(),
        source,
    })?;
    std::fs::rename(&temporary, &path).map_err(|source| ManifestError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(next)
}
