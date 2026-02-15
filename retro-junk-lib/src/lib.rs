use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

pub mod checksum;
pub mod context;
pub mod error;
pub mod n64;
pub mod progress;
pub mod region;

pub use checksum::{ChecksumAlgorithm, ExpectedChecksum};
pub use context::{AnalysisContext, AnalysisOptions, Console};
pub use error::AnalysisError;
pub use progress::AnalysisProgress;
pub use region::Region;

/// Information extracted from analyzing a ROM or disc image.
#[derive(Debug, Clone, Default)]
pub struct RomIdentification {
    /// Serial number (e.g., "SLUS-00123" for PS1, "NUS-NSME-USA" for N64)
    pub serial_number: Option<String>,

    /// Internal name stored in the ROM header
    pub internal_name: Option<String>,

    /// Region(s) the ROM is intended for
    pub regions: Vec<Region>,

    /// Version or revision number
    pub version: Option<String>,

    /// Expected checksums stored in the ROM itself (for self-verification)
    pub expected_checksums: Vec<ExpectedChecksum>,

    /// Actual file size on disk in bytes
    pub file_size: Option<u64>,

    /// Expected file size in bytes, derived from header/metadata.
    /// Compare with `file_size` to detect truncated or padded dumps.
    pub expected_size: Option<u64>,

    /// Platform/console identifier
    pub platform: Option<String>,

    /// Maker/publisher code
    pub maker_code: Option<String>,

    /// Additional platform-specific metadata
    pub extra: std::collections::HashMap<String, String>,
}

impl RomIdentification {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_serial(mut self, serial: impl Into<String>) -> Self {
        self.serial_number = Some(serial.into());
        self
    }

    pub fn with_internal_name(mut self, name: impl Into<String>) -> Self {
        self.internal_name = Some(name.into());
        self
    }

    pub fn with_region(mut self, region: Region) -> Self {
        self.regions.push(region);
        self
    }

    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }
}

/// A reader that implements both Read and Seek.
pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// Trait for analyzing ROM files and disc images.
///
/// Implementors should extract identifying information from the ROM header
/// and any other metadata embedded in the file format.
pub trait RomAnalyzer: Send + Sync {
    /// Analyze a ROM from a reader and extract identification information.
    ///
    /// # Arguments
    /// * `reader` - A reader positioned at the start of the ROM data
    /// * `options` - Analysis options (quick mode, etc.)
    ///
    /// # Returns
    /// * `Ok(RomIdentification)` - Successfully extracted identification info
    /// * `Err(AnalysisError)` - Failed to analyze (invalid format, I/O error, etc.)
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError>;

    /// Analyze a ROM with progress updates sent via channel.
    ///
    /// This is intended for GUI applications that need to display progress
    /// during analysis of large disc images.
    ///
    /// # Arguments
    /// * `reader` - A reader positioned at the start of the ROM data
    /// * `options` - Analysis options (quick mode, etc.)
    /// * `progress_tx` - Channel sender for progress updates
    ///
    /// # Returns
    /// * `Ok(RomIdentification)` - Successfully extracted identification info
    /// * `Err(AnalysisError)` - Failed to analyze
    fn analyze_with_progress(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
        progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError>;

    /// Returns the name of the platform this analyzer handles.
    fn platform_name(&self) -> &'static str;

    /// Returns the short name used for CLI and folder matching.
    fn short_name(&self) -> &'static str;

    /// Returns alternative folder names that should match this console.
    /// These are checked case-insensitively.
    fn folder_names(&self) -> &'static [&'static str];

    /// Returns the manufacturer of this console.
    fn manufacturer(&self) -> &'static str;

    /// Returns file extensions commonly associated with this platform.
    fn file_extensions(&self) -> &'static [&'static str];

    /// Check if the reader contains data this analyzer can handle.
    ///
    /// This performs a quick check (magic bytes, header validation) without
    /// full analysis. Useful for auto-detection of ROM type.
    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool;

    /// Check if this analyzer matches a folder name (case-insensitive).
    fn matches_folder(&self, folder_name: &str) -> bool {
        let folder_lower = folder_name.to_lowercase();
        if self.short_name().to_lowercase() == folder_lower {
            return true;
        }
        self.folder_names()
            .iter()
            .any(|name| name.to_lowercase() == folder_lower)
    }
}
