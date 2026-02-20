use std::io::{Read, Seek};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

pub mod checksum;
pub mod disc;
pub mod error;
pub mod platform;
pub mod progress;
pub mod region;

pub use checksum::{ChecksumAlgorithm, ExpectedChecksum};
pub use error::AnalysisError;
pub use platform::{Platform, PlatformParseError};
pub use progress::AnalysisProgress;
pub use region::Region;

/// Options that control how ROM analysis is performed.
#[derive(Debug, Clone, Default)]
pub struct AnalysisOptions {
    /// Quick mode: read as little data as possible.
    /// Useful for network shares or slow storage.
    pub quick: bool,

    /// Path to the file being analyzed. Used by disc-based analyzers
    /// (e.g., CUE sheets) to resolve relative file references.
    pub file_path: Option<PathBuf>,
}

impl AnalysisOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn quick(mut self, quick: bool) -> Self {
        self.quick = quick;
        self
    }

    pub fn file_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.file_path = Some(path.into());
        self
    }
}

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

/// The source database for DAT files.
///
/// Both sources use the LibRetro enhanced DAT repository on GitHub:
/// - No-Intro DATs for cartridge-based consoles (`metadat/no-intro/`)
/// - Redump DATs for disc-based consoles (`metadat/redump/`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatSource {
    /// No-Intro DATs (cartridge-based consoles: NES, SNES, N64, GB, GBA, etc.)
    NoIntro,
    /// Redump DATs (disc-based consoles: PS1, PS2, GameCube, Saturn, etc.)
    Redump,
}

impl DatSource {
    /// Returns the base URL for downloading DATs from this source.
    pub fn base_url(&self) -> &'static str {
        match self {
            DatSource::NoIntro => {
                "https://raw.githubusercontent.com/libretro/libretro-database/master/metadat/no-intro/"
            }
            DatSource::Redump => {
                "https://raw.githubusercontent.com/libretro/libretro-database/master/metadat/redump/"
            }
        }
    }

    /// Returns a human-readable name for this source.
    pub fn display_name(&self) -> &'static str {
        match self {
            DatSource::NoIntro => "No-Intro",
            DatSource::Redump => "Redump",
        }
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

    /// Returns the platform this analyzer handles.
    fn platform(&self) -> Platform;

    /// Returns the full name of the platform this analyzer handles.
    fn platform_name(&self) -> &'static str {
        self.platform().display_name()
    }

    /// Returns the short name used for CLI and folder matching.
    fn short_name(&self) -> &'static str {
        self.platform().short_name()
    }

    /// Returns alternative folder names that should match this console.
    /// These are checked case-insensitively.
    fn folder_names(&self) -> &'static [&'static str] {
        self.platform().aliases()
    }

    /// Returns the manufacturer of this console.
    fn manufacturer(&self) -> &'static str {
        self.platform().manufacturer()
    }

    /// Returns file extensions commonly associated with this platform.
    fn file_extensions(&self) -> &'static [&'static str];

    /// Check if the reader contains data this analyzer can handle.
    ///
    /// This performs a quick check (magic bytes, header validation) without
    /// full analysis. Useful for auto-detection of ROM type.
    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool;

    /// Check if this analyzer matches a folder name (case-insensitive).
    fn matches_folder(&self, folder_name: &str) -> bool {
        folder_name.parse::<Platform>().ok() == Some(self.platform())
    }

    // -- DAT support methods (override in platform analyzers) --

    /// Returns the DAT source for this platform (No-Intro or Redump).
    ///
    /// Cartridge-based consoles default to `DatSource::NoIntro`.
    /// Disc-based consoles should override this to return `DatSource::Redump`.
    fn dat_source(&self) -> DatSource {
        DatSource::NoIntro
    }

    /// Returns the DAT names for this platform.
    ///
    /// Most consoles return a single name, but some need multiple DATs
    /// (e.g., base + DLC, color variants). All are merged into one index.
    /// The DAT source (No-Intro vs Redump) is determined by `dat_source()`.
    ///
    /// Example: `&["Nintendo - Nintendo Entertainment System"]`
    fn dat_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// Returns download identifiers for DAT files.
    ///
    /// For No-Intro, this is the same as `dat_names()` (the DAT name IS the download path).
    /// For Redump, this returns system IDs (e.g., "psx") used in the redump.org URL path.
    fn dat_download_ids(&self) -> &'static [&'static str] {
        self.dat_names()
    }

    /// Returns true if this platform has DAT support (i.e., `dat_names()` is non-empty).
    fn has_dat_support(&self) -> bool {
        !self.dat_names().is_empty()
    }

    /// Returns the number of header bytes to skip before hashing for DAT matching.
    ///
    /// Override this for platforms with format headers (e.g., 16-byte iNES header,
    /// 512-byte SNES copier header). The default returns 0 (no header to skip).
    fn dat_header_size(
        &self,
        _reader: &mut dyn ReadSeek,
        _file_size: u64,
    ) -> Result<u64, AnalysisError> {
        Ok(0)
    }

    /// Returns a closure that normalizes each chunk of ROM data before hashing.
    ///
    /// Override this for platforms with byte-order variants (e.g., N64 ROMs exist
    /// in big-endian, byte-swapped, and little-endian formats). The closure is
    /// called on each 64KB buffer before feeding it to the hasher.
    ///
    /// The `header_offset` parameter is the number of bytes skipped (from
    /// `dat_header_size`), so the normalizer can detect the format from the
    /// first bytes of the actual ROM data.
    ///
    /// Returns `None` if no normalization is needed (the default).
    fn dat_chunk_normalizer(
        &self,
        _reader: &mut dyn ReadSeek,
        _header_offset: u64,
    ) -> Result<Option<Box<dyn FnMut(&mut [u8])>>, AnalysisError> {
        Ok(None)
    }

    /// Extract the core game code from a serial number for DAT matching.
    ///
    /// Different sources use different serial formats:
    /// - ROM headers (analyzers): `NUS-NSME-USA` (prefix-code-region)
    /// - LibRetro DATs: `NSME` (just the 4-char game code)
    ///
    /// Override this to extract the inner game code from your platform's
    /// serial format. Returns `None` if the serial doesn't match the
    /// expected pattern.
    fn extract_dat_game_code(&self, _serial: &str) -> Option<String> {
        None
    }

    /// Whether ROMs for this platform normally contain an extractable serial number.
    ///
    /// When true, failure to extract a serial during matching is reported as a
    /// diagnostic warning rather than silently falling back to hash matching.
    fn expects_serial(&self) -> bool {
        false
    }

    // -- Scraper support methods (override in platform analyzers) --

    /// Extract a serial number adapted for ScreenScraper API lookups.
    ///
    /// ScreenScraper may need a different serial format than NoIntro DATs.
    /// By default this delegates to `extract_dat_game_code()`, which works
    /// for most platforms. Override per-console when ScreenScraper needs
    /// a different format.
    ///
    /// Returns `None` if no adaptation is needed (use the raw serial as-is).
    fn extract_scraper_serial(&self, serial: &str) -> Option<String> {
        self.extract_dat_game_code(serial)
    }
}
