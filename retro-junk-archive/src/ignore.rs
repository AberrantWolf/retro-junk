//! Files you have decided you never want asked about again.
//!
//! The adoption sweep walks the playable library and files everything it
//! cannot account for as a review row. That is right the first time and wrong
//! the fourth: a collection contains readmes, cheat lists, and whole formats
//! the archive cannot accept yet, and none of those become accountable by
//! being listed again. Dismissing a review row only closes that row — the next
//! sweep re-files the same file — so dismissal alone turns into a chore you
//! repeat forever.
//!
//! An ignore rule is the durable version of that decision. It describes a
//! group of files by path pattern rather than naming one, so "every `.txt`"
//! is one decision instead of nine hundred, and the sweep consults it *before*
//! hashing anything — which also means an ignored file is never read.
//!
//! Rules live beside the collection, stored by [`crate::sidecar`], for the
//! same reason marks do: nothing can derive them, so they have to travel with
//! the files. Ignoring is always reversible — a rule is one file, and removing
//! it makes the next sweep file those strays again.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha1::{Digest as _, Sha1};

use crate::sidecar::{self, SidecarError, SidecarRecord};

/// Beside the collection, next to marks: an ignore rule is about the playable
/// library, which is not inside the archive.
pub const IGNORE_DIRECTORY: &str = ".retro-junk/ignore";

/// Versioned independently of the archive manifests, like marks.
pub const IGNORE_SCHEMA_VERSION: u32 = 1;

/// One "stop asking me about these" decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoreRule {
    pub schema_version: u32,
    /// The path pattern this rule covers, in the syntax of
    /// [`retro_junk_io::glob`], matched against a file's path relative to the
    /// playable root.
    ///
    /// Stored already lowercased for ASCII, because matching ignores that case
    /// anyway: `*.TXT` and `*.txt` are one decision, and storing them as typed
    /// would let two machines write two files for it.
    pub pattern: String,
    /// Why, in the user's words. Free text; empty is normal.
    #[serde(default)]
    pub note: String,
}

impl IgnoreRule {
    /// A rule covering `pattern`.
    #[must_use]
    pub fn new(pattern: &str, note: &str) -> Self {
        Self {
            schema_version: IGNORE_SCHEMA_VERSION,
            pattern: pattern.trim().to_ascii_lowercase(),
            note: note.trim().to_owned(),
        }
    }

    /// A pattern with nothing in it would ignore the entire library, which is
    /// never what anyone means — callers reject these rather than storing one.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pattern.is_empty()
    }
}

impl SidecarRecord for IgnoreRule {
    const DIRECTORY: &'static str = IGNORE_DIRECTORY;
    const SCHEMA_VERSION: u32 = IGNORE_SCHEMA_VERSION;
    const UNNAMED: &'static str = "an ignore rule needs a pattern";

    /// A readable stem plus a digest of the exact pattern: the stem so the
    /// directory can be understood by looking at it, the digest so two rules
    /// that slugify alike (`*.txt` and `*/txt/*`) cannot collide, and both
    /// derived only from the pattern so the same rule made twice lands on the
    /// same file.
    fn sidecar_name(&self) -> Option<String> {
        if self.pattern.is_empty() {
            return None;
        }
        let digest = Sha1::digest(self.pattern.as_bytes());
        let fingerprint = digest.iter().take(6).fold(String::new(), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        });
        let stem = crate::layout::slugify(&self.pattern);
        let stem = if stem.is_empty() {
            "pattern".to_owned()
        } else {
            stem.chars().take(48).collect()
        };
        Some(format!("{stem}-{fingerprint}.toml"))
    }

    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

/// Where ignore rules live for a collection rooted at `collection_root`.
#[must_use]
pub fn ignore_directory(collection_root: &Path) -> PathBuf {
    sidecar::directory::<IgnoreRule>(collection_root)
}

/// Record a rule. Making the same decision twice is a no-op.
pub fn write_rule(collection_root: &Path, rule: &IgnoreRule) -> Result<PathBuf, SidecarError> {
    sidecar::write(collection_root, rule)
}

/// Revoke a rule. Returns whether there was one to revoke.
pub fn remove_rule(collection_root: &Path, rule: &IgnoreRule) -> Result<bool, SidecarError> {
    sidecar::remove(collection_root, rule)
}

/// Every rule recorded for this collection, in a stable order.
pub fn load_rules(collection_root: &Path) -> Result<Vec<IgnoreRule>, SidecarError> {
    sidecar::load_all(collection_root)
}

/// Every rule, parsed once and ready to test many paths against.
///
/// The sweep asks about every file in the library, so re-reading the directory
/// or re-parsing patterns per file would turn a handful of decisions into a
/// hot path.
#[derive(Debug, Default, Clone)]
pub struct IgnoreRules {
    rules: Vec<(retro_junk_io::glob::Pattern, IgnoreRule)>,
}

impl IgnoreRules {
    /// Read every rule for a collection. A collection with no rules — the
    /// normal case — yields an empty set rather than an error.
    pub fn load(collection_root: &Path) -> Result<Self, SidecarError> {
        Ok(Self::from_rules(load_rules(collection_root)?))
    }

    #[must_use]
    pub fn from_rules(rules: Vec<IgnoreRule>) -> Self {
        Self {
            rules: rules
                .into_iter()
                .filter(|rule| !rule.is_empty())
                .map(|rule| (retro_junk_io::glob::Pattern::new(&rule.pattern), rule))
                .collect(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// The rule covering this path, if any. The rule itself comes back rather
    /// than a bare yes, so a caller can say *which* decision suppressed a file
    /// instead of leaving it mysteriously absent.
    #[must_use]
    pub fn matching(&self, relative_path: &str) -> Option<&IgnoreRule> {
        self.rules
            .iter()
            .find(|(pattern, _)| pattern.matches(relative_path))
            .map(|(_, rule)| rule)
    }

    /// Every rule, for showing and revoking.
    pub fn rules(&self) -> impl Iterator<Item = &IgnoreRule> {
        self.rules.iter().map(|(_, rule)| rule)
    }
}

#[cfg(test)]
#[path = "tests/ignore_tests.rs"]
mod tests;
