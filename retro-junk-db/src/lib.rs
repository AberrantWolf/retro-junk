//! SQLite persistence layer for the game catalog.
//!
//! Provides schema creation, CRUD operations, and query APIs
//! backed by SQLite (via rusqlite with bundled feature).

pub mod operations;
pub mod queries;
pub mod schema;

pub use operations::{
    find_company_by_alias, find_media_by_dat_name, find_release, find_work_by_name,
    insert_disagreement, insert_import_log, insert_media_asset, insert_work,
    resolve_disagreement, seed_from_catalog, update_release_enrichment,
    upsert_collection_entry, upsert_company, upsert_media, upsert_override, upsert_platform,
    upsert_release, OperationError, SeedStats,
};
pub use queries::{
    catalog_stats, find_media_by_crc32, find_media_by_serial, find_media_by_sha1,
    find_release_by_serial, list_import_logs, list_platforms, list_unresolved_disagreements,
    media_for_release, releases_for_platform, releases_to_enrich, search_releases, CatalogStats,
    PlatformRow,
};
pub use schema::{open_database, open_memory};
