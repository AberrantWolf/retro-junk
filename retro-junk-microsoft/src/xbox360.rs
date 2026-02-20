//! Xbox 360 disc image analyzer.
//!
//! Supports:
//! - ISO images
//! - GOD (Games on Demand) format
//! - XEX executables

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, Platform, RomAnalyzer, RomIdentification,
};

/// Analyzer for Xbox 360 disc images.
#[derive(Debug, Default)]
pub struct Xbox360Analyzer;

impl Xbox360Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Xbox360Analyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "Xbox 360 disc analysis not yet implemented",
        ))
    }

    fn analyze_with_progress(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        self.analyze(reader, options)
    }

    fn platform(&self) -> Platform {
        Platform::Xbox360
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "xex"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Microsoft - Xbox 360"]
    }

}
