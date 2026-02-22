//! Import DAT files and other data sources into the game catalog database.
//!
//! This crate owns all ETL logic: parsing DAT entries into catalog entities,
//! merging data from multiple sources, detecting disagreements, and applying
//! overrides.

pub mod dat_import;
pub mod merge;
pub mod progress;
pub mod scraper_import;

pub use dat_import::{ImportError, ImportStats, dat_source_str, import_dat, log_import};
pub use merge::{apply_overrides, check_field, merge_release_fields};
pub use progress::{ImportProgress, LogProgress, SilentProgress};
pub use scraper_import::{
    EnrichError, EnrichOptions, EnrichProgress, EnrichStats, SilentEnrichProgress,
    catalog_region_to_ss, enrich_releases, map_game_info, ss_media_type_to_asset_type,
    ss_region_to_catalog,
};
