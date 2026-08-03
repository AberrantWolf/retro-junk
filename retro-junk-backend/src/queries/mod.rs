//! The backend query surface.
//!
//! Read-only questions, answered from the projection and the catalog, with
//! all status semantics applied here (via [`crate::completion`]) so every
//! frontend renders the same answer. No frontend runs SQL or folds facts.

pub mod catalog;
pub mod collection;
pub mod releases;
pub mod work;

use std::path::Path;

use retro_junk_db::Connection;

/// Open the catalog database.
///
/// Frontends that keep a connection alive (rather than re-opening for every
/// question) get it from here, so opening the catalog stays one decision even
/// though the connection itself is held elsewhere.
pub fn open_catalog(path: &Path) -> Result<Connection, String> {
    retro_junk_db::open_database(path).map_err(|error| error.to_string())
}

/// Answers: "would opening this catalog have to migrate its schema first?"
///
/// A missing or unreadable file answers `true`, so a caller that shows a
/// "migrating…" notice shows it in exactly the cases where opening might take
/// a while.
#[must_use]
pub fn catalog_needs_migration(path: &Path) -> bool {
    retro_junk_db::database_needs_migration(path).unwrap_or(true)
}
