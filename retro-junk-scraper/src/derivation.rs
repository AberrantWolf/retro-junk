//! How to ask a scraper about a file that is not what it appears to be.
//!
//! Two things in a collection have never been in `ScreenScraper`'s database
//! and never will be. A mod's bytes exist only on the machine that applied it:
//! no hash of them matches anything, and its filename names a game nobody
//! catalogued. Homebrew has no serial, because no publisher ever assigned one
//! — whatever bytes sit where a serial would be are somebody's placeholder,
//! and matching them against a commercial catalog is how a homebrew title ends
//! up wearing another game's box art.
//!
//! The user has already told us which is which; that is what a collection mark
//! is. This module is the one place that decides what a mark *means* for a
//! lookup, so the CLI's folder scrape, the GUI's artwork actions, and the
//! convergence executor cannot answer that question differently.

use crate::lookup::{RomHashes, RomInfo};

/// The catalogued work a derivative was made from, as far as the caller can
/// name it.
///
/// A caller with the catalog fills all of it; a caller holding only the mark
/// fills `filename` from the parent's DAT name. Both are usable identities —
/// they just land on different lookup tiers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParentIdentity {
    /// The parent's ROM or DAT name — what the filename tier asks about.
    pub filename: String,
    /// The parent's size in bytes. `0` when unknown, which is honest: the
    /// derivative's own size is not the parent's and must never stand in.
    pub file_size: u64,
    /// The parent's serial, from the catalog.
    pub serial: String,
    /// The parent's digest triple, when the catalog holds all three.
    pub hashes: Option<RomHashes>,
}

impl ParentIdentity {
    /// Whether this names the parent well enough to ask about it at all.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        !self.filename.trim().is_empty() || !self.serial.trim().is_empty() || self.hashes.is_some()
    }
}

/// What the user decided a file is, in the only terms a lookup cares about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Derivation {
    /// Catalogued: the file is what it appears to be, and is asked about as
    /// itself.
    #[default]
    Own,
    /// A mod, asked about as the work it was made from. It inherits that
    /// work's metadata and artwork; only its own name and media stem stay its
    /// own, because that is what distinguishes it in the collection.
    Parent(ParentIdentity),
    /// A mod nothing here can name a parent for — the mark says "modded" but
    /// no catalogued work is attached, or the catalog that would resolve it
    /// has not been imported on this machine. There is nothing to ask.
    UnknownParent,
    /// Homebrew: its own work, asked about by name.
    Standalone,
}

impl Derivation {
    /// What a collection mark means for a lookup, without a catalog to consult.
    ///
    /// This is the offline form: a mod names its parent by DAT game name,
    /// which is exactly what the filename tier wants. A caller holding the
    /// catalog should upgrade that to the parent's serial and digests.
    #[must_use]
    pub fn from_mark(mark: &retro_junk_archive::CollectionMark) -> Self {
        match mark.kind {
            retro_junk_archive::MarkKind::Homebrew => Self::Standalone,
            // A region correction says nothing about whose identity to ask
            // for — the file is still itself.
            retro_junk_archive::MarkKind::RegionOverride => Self::Own,
            retro_junk_archive::MarkKind::Modded => {
                let parent = ParentIdentity {
                    filename: mark.parent_dat_name.clone(),
                    ..ParentIdentity::default()
                };
                if parent.is_usable() {
                    Self::Parent(parent)
                } else {
                    Self::UnknownParent
                }
            }
        }
    }

    /// The identity to offer a scraper for a file whose own identity is `own`.
    ///
    /// `None` means the file cannot be asked about at all, which is a verdict
    /// rather than a failure: spending requests on bytes no catalog has ever
    /// seen is the behavior this whole module exists to stop.
    #[must_use]
    pub fn identify(&self, own: &RomInfo) -> Option<RomInfo> {
        match self {
            Self::Own => Some(own.clone()),
            // A homebrew title's serial belongs to nobody. Offering it risks
            // colliding with a commercial game's, and the collision would be
            // published into the archive as that game's artwork.
            Self::Standalone => Some(RomInfo {
                serial: String::new(),
                scraper_serial: String::new(),
                expects_serial: false,
                ..own.clone()
            }),
            Self::UnknownParent => None,
            Self::Parent(parent) if !parent.is_usable() => None,
            Self::Parent(parent) => {
                // A ROM hack usually leaves the original header untouched, so
                // the file's own serial identifies the parent. Prefer the
                // catalog's when it has one; keep the header's otherwise.
                let (serial, scraper_serial) = if parent.serial.trim().is_empty() {
                    (own.serial.clone(), own.scraper_serial.clone())
                } else {
                    (parent.serial.clone(), String::new())
                };
                Some(RomInfo {
                    serial,
                    scraper_serial,
                    filename: parent_filename(parent, &own.filename),
                    file_size: parent.file_size,
                    // Never the derivative's own digests: they match nothing,
                    // and the request spent proving that is the cost this
                    // avoids.
                    hashes: parent.hashes.clone(),
                    platform: own.platform,
                    // The expectation is about catalogued dumps. A missing
                    // serial here says nothing about the file.
                    expects_serial: false,
                })
            }
        }
    }

    /// What to tell the user about a lookup that did not ask about this file.
    ///
    /// Scraped metadata that silently describes a different game is worse than
    /// no metadata, so every inherited match says whose it is.
    #[must_use]
    pub fn note(&self) -> Option<String> {
        match self {
            Self::Own | Self::Standalone => None,
            Self::Parent(parent) => Some(format!(
                "Marked as a mod: identified as its parent, {}",
                if parent.filename.trim().is_empty() {
                    parent.serial.as_str()
                } else {
                    parent.filename.as_str()
                }
            )),
            Self::UnknownParent => Some(
                "Marked as a mod with no parent recorded, so there is nothing to look up"
                    .to_owned(),
            ),
        }
    }
}

/// The parent's name as a ROM filename.
///
/// A DAT game name carries no extension, and the filename tier is matched
/// against ROM names, so the derivative's own extension is borrowed. Testing
/// the suffix rather than parsing one off matters: `Path::extension` reads
/// `Dr. Mario (USA)` as having the extension ` Mario (USA)`.
fn parent_filename(parent: &ParentIdentity, own_filename: &str) -> String {
    let name = parent.filename.trim();
    if name.is_empty() {
        return String::new();
    }
    let Some(extension) = std::path::Path::new(own_filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
    else {
        return name.to_owned();
    };
    let suffix = format!(".{extension}");
    if name
        .to_ascii_lowercase()
        .ends_with(&suffix.to_ascii_lowercase())
    {
        name.to_owned()
    } else {
        format!("{name}{suffix}")
    }
}

#[cfg(test)]
#[path = "tests/derivation_tests.rs"]
mod tests;
