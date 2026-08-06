//! Portable collection marks: the things *you* decided about your files.
//!
//! A DAT override is curation — it is the same for everyone holding that DAT,
//! and it belongs with the code. A mark is the opposite: a homebrew title you
//! own, or a mod you applied to a game you own. Nothing outside your collection
//! can derive it, and re-deciding it on every device is not a workflow.
//!
//! So marks live beside the collection rather than in the database, which is
//! device-local and rebuildable from DATs — stored one file per mark by
//! [`crate::sidecar`], which explains why that layout survives being synced.
//!
//! Applying a mark to the catalog lives in `retro-junk-db`; this module only
//! owns the portable representation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::manifest::ManifestError;
use crate::sidecar::{self, SidecarError, SidecarRecord};

/// Marks live beside the archive and the playable library, not inside either:
/// a mod exists only as a playable, while homebrew may or may not be archived,
/// and one store has to cover both.
pub const MARKS_DIRECTORY: &str = ".retro-junk/marks";

/// Versioned independently of the archive manifests: marks are a separate
/// portable format with their own lifecycle, and tying them to the archive's
/// schema would make an unrelated archive migration invalidate every mark.
pub const MARK_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum MarkError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("a mark needs at least a CRC32 or a SHA-1 to identify its file")]
    NoDigest,
}

/// Marks are stored through the shared sidecar layer but keep their own error
/// type, so a caller that already handles `MarkError::NoDigest` keeps working.
impl From<SidecarError> for MarkError {
    fn from(error: SidecarError) -> Self {
        match error {
            SidecarError::Manifest(source) => Self::Manifest(source),
            SidecarError::Io { path, source } => Self::Io { path, source },
            SidecarError::Unnamed(_) => Self::NoDigest,
        }
    }
}

/// What the user decided this file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    /// A work with no catalogued parent — it is its own thing.
    Homebrew,
    /// A file derived from a catalogued work, which it inherits identity from.
    Modded,
    /// The user corrected which region this file is from. Unlike the other
    /// kinds this mints nothing in the catalog — it only overrides what the
    /// scan concluded, which is exactly the kind of decision no DAT records
    /// and therefore the kind most easily lost.
    RegionOverride,
    /// The user picked which catalog entry an ambiguous file is, from the
    /// candidates that were actually possible for it.
    ///
    /// Like a region override this mints nothing and only settles a question
    /// the evidence could not. It is deliberately stored here rather than in
    /// the database: the database is disposable and rebuilt from DATs and
    /// archive manifests, so a choice that lived only there would be lost the
    /// next time it was rebuilt.
    Disambiguation,
}

impl MarkKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Homebrew => "homebrew",
            Self::Modded => "modded",
            Self::RegionOverride => "region_override",
            Self::Disambiguation => "disambiguation",
        }
    }

    /// Which decision about a file this kind answers.
    ///
    /// Marks are stored one per slot per file, because the decisions are not
    /// all mutually exclusive. `Homebrew` and `Modded` are two answers to the
    /// same question — is this its own work, or derived from one — so
    /// re-deciding replaces. A region correction and a disambiguation each
    /// answer a different question entirely, and a file may carry one of each;
    /// keying them all on platform and digest alone silently overwrote
    /// whichever decision came first.
    #[must_use]
    pub const fn slot(self) -> &'static str {
        match self {
            Self::Homebrew | Self::Modded => "derivation",
            Self::RegionOverride => "region",
            Self::Disambiguation => "identity",
        }
    }
}

/// The digests that identify the marked file.
///
/// A mod matches no DAT name by definition — being unmatched is what makes it
/// a mod — so content is the only identity it has that survives a rename or a
/// move to another device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkedContent {
    pub size: u64,
    #[serde(default)]
    pub crc32: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub md5: String,
}

impl MarkedContent {
    /// The digest that names this mark's file on disk. SHA-1 when available,
    /// because a CRC-32 collision is reachable across a large collection.
    /// Public because every store keyed on marked content must agree on what
    /// names a file — one definition, or two indexes drift apart.
    #[must_use]
    pub fn key(&self) -> Option<&str> {
        [&self.sha1, &self.crc32]
            .into_iter()
            .find(|digest| !digest.is_empty())
            .map(String::as_str)
    }
}

/// One user decision about one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionMark {
    pub schema_version: u32,
    pub kind: MarkKind,
    pub platform_id: String,
    #[serde(default)]
    pub region: String,
    /// Homebrew: the work's name. Modded: the label for the derived file.
    pub name: String,
    /// The catalogued medium a mod is derived from, so it can inherit that
    /// game's identity and artwork instead of matching anything itself.
    ///
    /// A catalog media id is folded from the medium's own digests, so the same
    /// bytes produce the same id on every machine that ever imports that DAT.
    /// That is what makes this link portable, and why a mod names a medium
    /// rather than a work: works are grouped by hand and their ids are minted
    /// locally, while a medium simply *is* its bytes.
    ///
    /// Empty for homebrew, which has no parent.
    #[serde(default)]
    pub parent_media_id: String,
    /// What the parent is called, for reading and for the scraper's
    /// filename-based lookup.
    ///
    /// Deliberately a label and nothing more: it is allowed to be stale or
    /// wrong, and no resolution depends on it. `parent_media_id` carries the
    /// link.
    #[serde(default)]
    pub parent_dat_name: String,
    pub content: MarkedContent,
    /// Disambiguation: the catalog medium the user chose, by its content id.
    #[serde(default)]
    pub chosen_media_id: String,
    /// What the chosen entry is called, for reading. A label, like
    /// `parent_dat_name`; `chosen_media_id` is the choice.
    #[serde(default)]
    pub chosen_dat_name: String,
    #[serde(default)]
    pub note: String,
}

impl SidecarRecord for CollectionMark {
    const DIRECTORY: &'static str = MARKS_DIRECTORY;
    const SCHEMA_VERSION: u32 = MARK_SCHEMA_VERSION;
    const UNNAMED: &'static str = "a mark needs at least a CRC32 or a SHA-1 to identify its file";

    /// Decision slot, platform and content digest, so the same decision always
    /// lands on the same path.
    ///
    /// The slot is part of the name because a file can carry more than one
    /// kind of decision at once — homebrew *and* a region correction, say.
    /// Naming on platform and digest alone meant the second decision silently
    /// overwrote the first. Kinds that answer the same question share a slot,
    /// so re-deciding still replaces rather than accumulating.
    fn sidecar_name(&self) -> Option<String> {
        Some(format!(
            "{}-{}-{}.toml",
            self.kind.slot(),
            crate::layout::slugify(&self.platform_id),
            self.content.key()?.to_ascii_lowercase()
        ))
    }

    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

impl CollectionMark {
    /// The file name this mark is stored under.
    pub fn file_name(&self) -> Result<String, MarkError> {
        self.sidecar_name().ok_or(MarkError::NoDigest)
    }
}

/// Where marks live for a collection rooted at `collection_root`.
#[must_use]
pub fn marks_directory(collection_root: &Path) -> PathBuf {
    sidecar::directory::<CollectionMark>(collection_root)
}

/// Record a mark, replacing any earlier decision about the same file.
pub fn write_mark(collection_root: &Path, mark: &CollectionMark) -> Result<PathBuf, MarkError> {
    Ok(sidecar::write(collection_root, mark)?)
}

/// Forget a decision. Returns whether a mark was there to forget.
pub fn remove_mark(collection_root: &Path, mark: &CollectionMark) -> Result<bool, MarkError> {
    Ok(sidecar::remove(collection_root, mark)?)
}

/// Every mark recorded for this collection, in a stable order.
pub fn load_marks(collection_root: &Path) -> Result<Vec<CollectionMark>, MarkError> {
    Ok(sidecar::load_all(collection_root)?)
}

/// Every mark, indexed by what identifies the file it describes.
///
/// A pass that consults marks — a scrape deciding how to ask about each file,
/// a scan deciding what a stranger is — asks about every file it handles, and
/// re-reading the directory per file would turn a curation store into a hot
/// path. Loading is cheap: marks number in the hundreds at most, because each
/// one is a decision a human made.
#[derive(Debug, Default, Clone)]
pub struct MarkIndex {
    by_digest: HashMap<String, CollectionMark>,
    by_name: HashMap<(String, u64), CollectionMark>,
}

impl MarkIndex {
    /// Read every mark for a collection. A collection with no marks — the
    /// normal case — yields an empty index rather than an error.
    pub fn load(collection_root: &Path) -> Result<Self, MarkError> {
        Ok(Self::from_marks(load_marks(collection_root)?))
    }

    #[must_use]
    pub fn from_marks(marks: Vec<CollectionMark>) -> Self {
        let mut index = Self::default();
        for mark in marks {
            for digest in [&mark.content.sha1, &mark.content.crc32] {
                if !digest.is_empty() {
                    index
                        .by_digest
                        .insert(digest.to_ascii_lowercase(), mark.clone());
                }
            }
            if !mark.name.is_empty() {
                index
                    .by_name
                    .insert((mark.name.to_ascii_lowercase(), mark.content.size), mark);
            }
        }
        index
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_digest.is_empty() && self.by_name.is_empty()
    }

    /// The decision recorded about a file with these digests. Either digest
    /// resolves it, because a caller that computed only one still deserves an
    /// answer.
    #[must_use]
    pub fn find(&self, sha1: &str, crc32: &str) -> Option<&CollectionMark> {
        [sha1, crc32]
            .into_iter()
            .filter(|digest| !digest.is_empty())
            .find_map(|digest| self.by_digest.get(&digest.to_ascii_lowercase()))
    }

    /// The decision recorded about a file this pass never hashed.
    ///
    /// Hashing a whole disc image to find out whether a scrape should ask
    /// about it differently is not a trade worth making, so name and size
    /// stand in. They are weaker than content — a rename defeats them — but
    /// the digest index is tried first and catches exactly that case.
    #[must_use]
    pub fn find_by_name(&self, name: &str, size: u64) -> Option<&CollectionMark> {
        self.by_name.get(&(name.to_ascii_lowercase(), size))
    }
}

#[cfg(test)]
#[path = "tests/marks_tests.rs"]
mod tests;
