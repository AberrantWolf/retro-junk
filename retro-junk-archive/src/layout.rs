use std::path::{Component, Path, PathBuf};

use crate::{ArchiveReleaseId, DumpId};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LayoutError {
    #[error("path must be relative, Unicode, non-empty, and contain no parent traversal")]
    UnsafeRelativePath,
}

/// File name of the archive root manifest, which carries the archive's
/// portable identity.
pub const ROOT_MANIFEST_FILE: &str = "retro-junk-archive.toml";

/// Path of the root manifest for an archive root.
#[must_use]
pub fn root_manifest_path(archive_root: &Path) -> PathBuf {
    archive_root.join(ROOT_MANIFEST_FILE)
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
    pub fn root_manifest_path(&self) -> PathBuf {
        root_manifest_path(&self.root)
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

/// Make a name safe to use as a filename on every filesystem a collection
/// travels across, changing as little as possible.
///
/// Catalog names are written by people, for people, and are used verbatim as
/// playable filenames — that is the whole point of naming a playable after its
/// DAT entry. But a name is not a filename: `Harvest Moon: Boy Meets Girl`
/// carries a colon, which is legal on macOS at the POSIX layer and **illegal
/// on exFAT and NTFS**, so writing it fails on exactly the removable and
/// Windows-shared drives a collection lives on. A `/` would be worse: it would
/// silently mean a directory.
///
/// So the illegal set is replaced rather than the name being slugified.
/// Slugifying is the wrong tool here — it would turn `Castlevania - Symphony
/// of the Night (USA)` into `castlevania-symphony-of-the-night-usa`, which no
/// frontend, scraper, or person asked for. A colon becomes ` -`, matching the
/// convention No-Intro and Redump already use for subtitles; the rest become
/// `-`.
///
/// Trailing dots and spaces are stripped because Windows silently drops them,
/// which would make a file the archive recorded and the file on disk differ by
/// a name nobody can see.
#[must_use]
pub fn safe_file_stem(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(current) = chars.next() {
        match current {
            // A subtitle separator, and the most common offender by far.
            // `Moon: Boy` reads as `Moon - Boy` rather than `Moon- Boy`.
            ':' => {
                out.push_str(" -");
                // The space that usually follows would otherwise double up.
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
                out.push(' ');
            }
            '/' | '\\' | '*' | '?' | '"' | '<' | '>' | '|' => out.push('-'),
            // Control characters are illegal everywhere and invisible, so a
            // name carrying one cannot be checked by looking at it.
            control if control.is_control() => {}
            other => out.push(other),
        }
    }
    let trimmed = out.trim().trim_end_matches(['.', ' ']).trim_end();
    if trimmed.is_empty() {
        "untitled".to_owned()
    } else {
        trimmed.to_owned()
    }
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
