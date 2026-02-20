pub mod cache;
pub mod dat;
pub mod error;
pub mod matcher;

pub use cache::{CacheEntry, CachedDat};
pub use dat::{DatFile, DatGame, DatRom};
pub use error::DatError;
pub use matcher::{DatIndex, FileHashes, MatchMethod, MatchResult, SerialLookupResult};
