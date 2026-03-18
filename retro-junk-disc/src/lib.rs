//! Shared CD-ROM and optical disc utilities.
//!
//! Provides generic disc image handling used by multiple platform analyzers:
//! - Disc format detection (ISO, raw BIN, CUE, CHD)
//! - ISO 9660 filesystem parsing (PVD, directory walking, file reading)
//! - CUE sheet parsing
//! - CHD compressed disc reading
//! - Track-aware hashing for Redump DAT verification

pub mod chd;
pub mod cue;
pub mod format;
pub mod hash;
pub mod iso9660;
pub mod sector;

// Re-export primary types for convenience
pub use cue::{CueFile, CueIndex, CueSheet, CueTrack};
pub use format::{DiscFormat, detect_disc_format};
pub use hash::{SectorMode, hash_disc_container};
pub use iso9660::{DirectoryRecord, PrimaryVolumeDescriptor, find_file_in_root, read_pvd};
pub use sector::{
    CD_SYNC_PATTERN, CHD_MAGIC, ISO_SECTOR_SIZE, MODE1_DATA_OFFSET, MODE2_FORM1_DATA_OFFSET,
    PVD_SECTOR, RAW_SECTOR_SIZE, read_sector_data,
};

/// Test helpers for constructing synthetic disc images.
///
/// Available to downstream crate tests via dev-dependency on `retro-junk-disc`.
pub mod test_helpers;
