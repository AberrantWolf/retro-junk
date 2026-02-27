//! Original Xbox disc image analyzer.
//!
//! Supports:
//! - ISO images
//! - XISO format

use retro_junk_core::ReadSeek;

use retro_junk_core::{AnalysisError, AnalysisOptions, Platform, RomAnalyzer, RomIdentification};

/// Analyzer for original Xbox disc images.
#[derive(Debug, Default)]
pub struct XboxAnalyzer;

impl XboxAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for XboxAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "Xbox disc analysis not yet implemented",
        ))
    }

    fn platform(&self) -> Platform {
        Platform::Xbox
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "xiso"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Microsoft - Xbox"]
    }
}
