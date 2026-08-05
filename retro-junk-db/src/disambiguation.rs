//! Remembering which catalog entry a person picked for an ambiguous file.
//!
//! The choice is stored as a content-keyed mark beside the collection, not in
//! the database. Two reasons, and both are load-bearing:
//!
//! - The database is disposable. It is rebuilt from DATs and archive
//!   manifests whenever the catalog key changes, and a decision that lived
//!   only there would be lost every time — which is exactly what happened to
//!   465 dismissed suggestions.
//! - A choice keyed on a path would not survive a rename. Keyed on the
//!   content, it re-attaches to the same bytes under any name and means the
//!   same thing on another machine, like tags and region corrections already
//!   do.
//!
//! One store, used by every surface: the CLI, the GUI's chooser, and the
//! identification pass that consults it.

#[cfg(test)]
#[path = "tests/disambiguation_tests.rs"]
mod tests;

use std::collections::HashMap;
use std::path::Path;

use retro_junk_archive::{CollectionMark, MarkKind, MarkedContent};

/// Every disambiguation recorded for a collection, keyed by content digest.
///
/// Loaded once per pass rather than per file: identification asks about every
/// file it handles, and re-reading the directory each time would turn a
/// curation store into a hot path.
#[derive(Debug, Default, Clone)]
pub struct Disambiguations {
    by_content: HashMap<String, String>,
}

impl Disambiguations {
    /// Read every choice recorded for this collection.
    pub fn load(collection_root: &Path) -> Result<Self, String> {
        let marks = retro_junk_archive::load_marks(collection_root).map_err(|e| e.to_string())?;
        Ok(Self::from_marks(&marks))
    }

    #[must_use]
    pub fn from_marks(marks: &[CollectionMark]) -> Self {
        let mut by_content = HashMap::new();
        for mark in marks {
            if mark.kind != MarkKind::Disambiguation || mark.chosen_media_id.is_empty() {
                continue;
            }
            if let Some(key) = mark.content.key() {
                by_content.insert(key.to_ascii_lowercase(), mark.chosen_media_id.clone());
            }
        }
        Self { by_content }
    }

    /// The catalog medium chosen for this content, if a person chose one.
    #[must_use]
    pub fn chosen_for(&self, content: &MarkedContent) -> Option<&str> {
        let key = content.key()?.to_ascii_lowercase();
        self.by_content.get(&key).map(String::as_str)
    }

    /// The chosen medium among a set of candidates, whichever content it was
    /// recorded against.
    ///
    /// A carrier's identity is settled per file, but the caller resolving it
    /// holds a list of catalog ids rather than the digest that was marked. Any
    /// recorded choice naming one of these candidates is that decision.
    #[must_use]
    pub fn chosen_for_any<'a>(&self, candidates: impl Iterator<Item = &'a str>) -> Option<&str> {
        let wanted = candidates.collect::<Vec<_>>();
        self.by_content
            .values()
            .find(|chosen| wanted.contains(&chosen.as_str()))
            .map(String::as_str)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_content.is_empty()
    }
}

/// Record that a person chose `media_id` for this content.
///
/// `dat_name` is the portable half of the choice: media ids are minted per DAT
/// import and mean nothing on another machine or after a rebuild, so the name
/// is what lets the decision be re-resolved. Writing again replaces the
/// previous choice rather than accumulating.
pub fn choose(
    collection_root: &Path,
    platform_id: &str,
    content: &MarkedContent,
    media_id: &str,
    dat_name: &str,
) -> Result<(), String> {
    let mark = CollectionMark {
        schema_version: retro_junk_archive::marks::MARK_SCHEMA_VERSION,
        kind: MarkKind::Disambiguation,
        platform_id: platform_id.to_owned(),
        region: String::new(),
        name: dat_name.to_owned(),
        parent_work_id: String::new(),
        parent_dat_name: String::new(),
        content: content.clone(),
        chosen_media_id: media_id.to_owned(),
        chosen_dat_name: dat_name.to_owned(),
        note: String::new(),
    };
    retro_junk_archive::write_mark(collection_root, &mark)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Forget a choice, putting the file back to whatever the evidence says.
///
/// Returns whether there was one to forget.
pub fn clear(
    collection_root: &Path,
    platform_id: &str,
    content: &MarkedContent,
) -> Result<bool, String> {
    let mark = CollectionMark {
        schema_version: retro_junk_archive::marks::MARK_SCHEMA_VERSION,
        kind: MarkKind::Disambiguation,
        platform_id: platform_id.to_owned(),
        region: String::new(),
        name: String::new(),
        parent_work_id: String::new(),
        parent_dat_name: String::new(),
        content: content.clone(),
        chosen_media_id: String::new(),
        chosen_dat_name: String::new(),
        note: String::new(),
    };
    retro_junk_archive::remove_mark(collection_root, &mark).map_err(|error| error.to_string())
}

/// The catalog entries an ambiguous file could be.
///
/// Goes through the one identification ladder rather than querying the catalog
/// directly, so the chooser can only ever offer entries the evidence actually
/// leaves open — the whole point of restricting the list.
pub fn candidates_for(
    db_path: &Path,
    platform_id: &str,
    tracks: Vec<retro_junk_archive::TrackDigest>,
) -> Result<Vec<crate::identify::Candidate>, String> {
    let conn = crate::open_database(db_path).map_err(|error| error.to_string())?;
    let found = crate::identify::identify(
        &conn,
        &crate::identify::Evidence {
            platform_id,
            tracks,
            ..crate::identify::Evidence::default()
        },
    )
    .map_err(|error| error.to_string())?;
    Ok(found.candidates().to_vec())
}
