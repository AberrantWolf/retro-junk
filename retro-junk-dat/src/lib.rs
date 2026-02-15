pub mod cache;
pub mod dat;
pub mod error;
pub mod hasher;
pub mod matcher;
pub mod rename;
pub mod systems;

pub use cache::{CacheEntry, CachedDat};
pub use dat::{DatFile, DatGame, DatRom};
pub use error::DatError;
pub use hasher::FileHashes;
pub use matcher::{DatIndex, MatchMethod, MatchResult};
pub use rename::{
    execute_renames, format_match_method, plan_renames, MatchDiscrepancy, RenameAction,
    RenameOptions, RenamePlan, RenameProgress, RenameSummary,
};
pub use systems::{ByteOrderRule, HeaderDetect, SystemMapping};
