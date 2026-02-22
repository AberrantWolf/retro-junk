//! SQLite persistence layer for the game catalog.
//!
//! Provides schema creation, CRUD operations, and query APIs
//! backed by SQLite (via rusqlite with bundled feature).

pub mod operations;
pub mod queries;
pub mod schema;

pub use operations::{
    apply_disagreement_resolution, find_company_by_alias, find_media_by_dat_name, find_release,
    find_work_by_name, insert_disagreement, insert_import_log, insert_media_asset, insert_work,
    resolve_disagreement, seed_from_catalog, update_release_enrichment,
    upsert_collection_entry, upsert_company, upsert_media, upsert_override, upsert_platform,
    upsert_release, OperationError, SeedStats,
};
pub use queries::{
    asset_counts_by_type, asset_coverage_summary, assets_for_release, catalog_stats,
    collection_counts_by_platform, find_collection_entry, find_media_by_crc32, find_media_by_md5,
    find_media_by_serial, find_media_by_sha1, find_release_by_serial, get_company_name,
    get_disagreement, get_platform_display_name, get_release_by_id, list_collection,
    list_import_logs, list_platforms, list_unresolved_disagreements, media_for_release,
    releases_for_platform, releases_missing_asset_type, releases_to_enrich, releases_with_no_assets,
    search_releases, search_releases_filtered, CatalogStats, CollectionRow, DisagreementFilter,
    PlatformRow,
};
pub use rusqlite::Connection;
pub use schema::{open_database, open_memory};
