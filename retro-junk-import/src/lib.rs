//! Import DAT files and other data sources into the game catalog database.
//!
//! This crate owns all ETL logic: parsing DAT entries into catalog entities,
//! merging data from multiple sources, detecting disagreements, and applying
//! overrides.

pub mod dat_import;
pub mod merge;
pub mod progress;

pub use dat_import::{ImportError, ImportStats, dat_source_str, import_dat, log_import};
pub use merge::{apply_overrides, check_field, merge_release_fields};
pub use progress::{ImportProgress, LogProgress, SilentProgress};
