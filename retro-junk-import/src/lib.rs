//! Import DAT files and other data sources into the game catalog database.
//!
//! This crate owns all ETL logic: parsing DAT entries into catalog entities,
//! merging data from multiple sources, detecting disagreements, and applying
//! overrides.

pub mod dat_import;
pub mod gdb_import;
pub mod merge;
pub mod progress;
pub mod reconcile;
pub mod scan_import;
pub mod scraper_import;

pub use dat_import::{ImportError, ImportStats, dat_source_str, import_dat, log_import};
pub use gdb_import::{GdbEnrichOptions, GdbEnrichStats, enrich_gdb};
pub use merge::{apply_overrides, check_field, merge_release_fields};
pub use progress::{ImportProgress, LogProgress, SilentProgress};
pub use scan_import::{
    ScanError, ScanOptions, ScanProgress, ScanResult, ScanStats, SilentScanProgress, VerifyStats,
    scan_folder, verify_collection,
};
pub use reconcile::{ReconcileError, ReconcileOptions, ReconcileResult, ReconcileStats, reconcile_works};
pub use scraper_import::{
    EnrichError, EnrichEvent, EnrichOptions, EnrichStats, catalog_region_to_ss, enrich_releases,
    map_game_info, ss_media_type_to_asset_type, ss_region_to_catalog,
};

/// Convert a string to a URL-friendly slug (lowercase, hyphens, no trailing hyphen).
pub(crate) fn slugify(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_separator = false;

    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator && !result.is_empty() {
            result.push('-');
            last_was_separator = true;
        }
    }

    // Trim trailing separator
    if result.ends_with('-') {
        result.pop();
    }

    result
}
