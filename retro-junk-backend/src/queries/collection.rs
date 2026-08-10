//! Questions about the physical collection: what a frontend needs to show and
//! edit one archived release's copy and carrier.

use retro_junk_db::{ArchiveCollectionDetails, Connection};

/// Answers: "what is recorded about this archived release's physical copy?"
///
/// A release that has left the archive index answers `Ok(None)` — the caller
/// decides whether that is an error or simply a stale click.
pub fn physical_copy_details(
    conn: &Connection,
    release_id: &str,
) -> Result<Option<ArchiveCollectionDetails>, String> {
    retro_junk_db::load_archive_collection_details(conn, release_id)
        .map_err(|error| error.to_string())
}

/// Answers: "when was this profile's archive projection last committed?"
///
/// `Ok(None)` means the projection has never been built, so a caller should
/// build it before painting from it.
pub fn projection_indexed_at(
    conn: &Connection,
    profile_id: &str,
) -> Result<Option<String>, String> {
    retro_junk_db::archive_profile_indexed_at(conn, profile_id).map_err(|error| error.to_string())
}

/// Generation of the authoritative archive tree represented by this profile.
pub fn projection_source_generation(
    conn: &Connection,
    profile_id: &str,
) -> Result<Option<u64>, String> {
    retro_junk_db::archive_profile_source_generation(conn, profile_id)
        .map_err(|error| error.to_string())
}

/// Answers whether the committed projection reflects both authoritative
/// archive manifests and the catalog generation used to bind their hashes.
pub fn projection_is_current(
    conn: &Connection,
    profile_id: &str,
    source_generation: u64,
    source_fingerprint: &str,
) -> Result<bool, String> {
    retro_junk_db::archive_profile_projection_is_current(
        conn,
        profile_id,
        source_generation,
        source_fingerprint,
    )
    .map_err(|error| error.to_string())
}
