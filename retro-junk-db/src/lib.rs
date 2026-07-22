//! `SQLite` persistence layer for the game catalog.
//!
//! Provides schema creation, CRUD operations, and query APIs
//! backed by `SQLite` (via rusqlite with bundled feature).

pub mod archive;
pub mod library;
pub mod operations;
pub mod queries;
pub mod schema;

pub use archive::{
    ArchiveCollectionDetails, ArchiveReleaseSummary, CompleteCatalogMediaMatch,
    bind_library_entries_by_hash, list_archive_release_summaries, load_archive_collection_details,
    match_catalog_file, match_catalog_file_any_platform, match_catalog_serial_any_platform,
    match_complete_catalog_media, match_complete_catalog_media_any_platform,
    reconcile_archive_snapshot,
};
pub use library::{
    ConsoleScanToken, EntryAnalysisCommand, EntryAnalysisUpdate, EntryHashUpdate, LibraryChangeSet,
    LibraryConsoleDescriptor, LibraryConsoleId, LibraryConsoleRow, LibraryConsoleSummary,
    LibraryEntryCounts, LibraryEntryDetail, LibraryEntryFilter, LibraryEntryId,
    LibraryEntryListItem, LibraryEntryListPage, LibraryEntryListQuery, LibraryEntryRow,
    LibraryEntrySortField, LibraryError, LibraryExportEntry, LibraryRootId, LibraryScanState,
    LibrarySourceKey, ScannedLibraryEntry, SortDirection, SourceFileDescriptor, SourceFileKind,
    apply_entry_analysis, apply_entry_analysis_batch, apply_entry_hash_update,
    apply_filesystem_transition, begin_console_scan, clear_library_cache,
    create_homebrew_and_tag_entry, create_modded_and_tag_entry, delete_library_root,
    ensure_library_console, file_source_key, get_library_root_id, list_console_summaries,
    load_consoles_for_root, load_entries_for_console, load_entry_detail, load_entry_details,
    load_entry_details_for_console, load_export_entries_for_console, mark_console_stale,
    normalize_relative_path, query_entry_list, reconcile_console_scan, set_entry_region_override,
    set_entry_tag, set_source_key, source_fingerprint, source_fingerprint_from_game_entry_json,
    source_key_from_game_entry_json, upsert_library_root,
};
pub use operations::{
    MediaHashes, MediaTrack, OperationError, SeedStats, apply_disagreement_resolution,
    clear_not_found_flags, create_homebrew_work, create_modded_media, delete_orphan_works,
    delete_release, detach_modded_media, find_company_by_alias, find_media_by_dat_name,
    find_media_by_release_and_rom_name, find_media_tracks, find_media_tracks_for_media_ids,
    find_release, find_work_by_name, insert_asset, insert_disagreement, insert_import_log,
    insert_media_track, insert_work, mark_release_not_found, move_assets_to_release,
    move_disagreements_for_release, move_media_to_release, resolve_disagreement, seed_from_catalog,
    set_media_tag, set_work_tag, unenrich_releases, update_release_enrichment,
    update_releases_work_id, update_work_name, upsert_collection_entry, upsert_company,
    upsert_media, upsert_override, upsert_platform, upsert_release,
};
pub use queries::{
    CatalogHashQuery, CatalogMediaMatch, CatalogStats, CollectionRow, CompanyRow,
    DisagreementFilter, PlatformRow, ReconcileGroup, ReleaseCollision, WorkRow, WorkWithCount,
    asset_counts_by_type, asset_coverage_summary, assets_for_release, catalog_stats,
    check_release_collision, collection_counts_by_platform, count_collection,
    count_companies_search, count_enriched_releases, count_media_search, count_releases_for_work,
    count_releases_search, count_works_search, find_collection_entry, find_media_by_crc32,
    find_media_by_md5, find_media_by_serial, find_media_by_sha1, find_media_by_tag,
    find_reconcilable_works, find_release_by_serial, find_works_by_tag, get_company_name,
    get_disagreement, get_media_by_id, get_platform_by_id, get_platform_display_name,
    get_release_by_id, get_work_by_id, list_collection, list_collection_paged, list_import_logs,
    list_platforms, list_unresolved_disagreements, match_media_by_hash, match_media_by_hashes,
    match_media_by_serial, match_media_by_serials, match_media_ids_by_track_hash,
    media_for_release, platform_media_counts, platform_release_counts, releases_for_platform,
    releases_for_work, releases_missing_asset_type, releases_to_enrich, releases_with_no_assets,
    search_companies, search_media, search_releases, search_releases_filtered,
    search_releases_paged, search_works, works_for_platform,
};
pub use rusqlite::Connection;
pub use schema::{configure_connection, open_database, open_memory};
