use std::path::{Component, Path, PathBuf};

use crate::{ArchiveReleaseId, DumpId};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LayoutError {
    #[error("path must be relative, Unicode, non-empty, and contain no parent traversal")]
    UnsafeRelativePath,
}

#[derive(Debug, Clone)]
pub struct ArchiveLayout {
    root: PathBuf,
}

impl ArchiveLayout {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn release_dir(
        &self,
        platform: &str,
        title: &str,
        region: &str,
        revision: &str,
        id: ArchiveReleaseId,
    ) -> PathBuf {
        let mut display = title.to_owned();
        if !region.is_empty() {
            display.push_str("--");
            display.push_str(region);
        }
        if !revision.is_empty() {
            display.push('-');
            display.push_str(revision);
        }
        let key = slugify(&display);
        let path = self.root.join(slugify(platform)).join(&key);
        if path.exists() {
            self.root
                .join(slugify(platform))
                .join(format!("{key}--{}", &id.to_string()[..8]))
        } else {
            path
        }
    }

    #[must_use]
    pub fn physical_copy_dir(release_dir: &Path, copy_number: u32) -> PathBuf {
        release_dir
            .join("physical-copies")
            .join(format!("copy-{copy_number:02}"))
    }

    #[must_use]
    pub fn carrier_dir(copy_dir: &Path, serial: &str, sequence_number: u32) -> PathBuf {
        let key = if !serial.trim().is_empty() {
            let serial = slugify(serial);
            if sequence_number > 1 {
                format!("{serial}-disc-{sequence_number}")
            } else {
                serial
            }
        } else if sequence_number > 0 {
            format!("carrier-{sequence_number}")
        } else {
            "carrier".to_owned()
        };
        copy_dir.join("carriers").join(key)
    }

    #[must_use]
    pub fn dump_dir(carrier_dir: &Path, captured_at: &str, dump_id: DumpId) -> PathBuf {
        let date = captured_at.get(..10).unwrap_or("undated");
        carrier_dir
            .join("dumps")
            .join(format!("{date}-{}", &dump_id.to_string()[..8]))
    }
}

pub fn normalize_relative_path(path: &Path) -> Result<String, LayoutError> {
    if path.is_absolute() {
        return Err(LayoutError::UnsafeRelativePath);
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or(LayoutError::UnsafeRelativePath)?
                    .to_owned(),
            ),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(LayoutError::UnsafeRelativePath);
            }
        }
    }
    if parts.is_empty() {
        return Err(LayoutError::UnsafeRelativePath);
    }
    Ok(parts.join("/"))
}

#[must_use]
pub fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in value.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "untitled".to_owned()
    } else {
        out
    }
}
