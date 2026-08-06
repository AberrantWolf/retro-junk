//! Import DAT files and other data sources into the game catalog database.
//!
//! This crate owns all ETL logic: parsing DAT entries into catalog entities,
//! merging data from multiple sources, detecting disagreements, and applying
//! overrides.

pub mod companies;
pub mod dat_import;
pub mod gdb_import;
pub mod merge;
pub mod progress;
pub mod reconcile;
pub mod scan_import;
pub mod scraper_import;

pub use dat_import::{ImportError, ImportStats, dat_source_str, import_dat, log_import};
pub use gdb_import::{GdbEnrichOptions, GdbEnrichStats, enrich_gdb};
pub use merge::{
    FieldRef, ReleaseFieldValues, SourcedValue, apply_overrides, check_field, merge_release_fields,
};
pub use progress::{ImportProgress, LogProgress, SilentProgress};
pub use reconcile::{
    ReconcileError, ReconcileOptions, ReconcileResult, ReconcileStats, reconcile_works,
};
pub use scan_import::{
    ScanError, ScanOptions, ScanProgress, ScanResult, ScanStats, SilentScanProgress, VerifyStats,
    scan_folder, verify_collection,
};
pub use scraper_import::{
    EnrichError, EnrichEvent, EnrichOptions, EnrichStats, catalog_region_to_ss, enrich_releases,
    map_game_info, ss_media_type_to_asset_type, ss_region_to_catalog,
};
