//! Sega Dreamcast disc image analyzer.
//!
//! Supports:
//! - GDI images (.gdi)
//! - CDI images (.cdi)
//! - CHD compressed images

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, Platform, RomAnalyzer, RomIdentification,
};

/// Analyzer for Sega Dreamcast disc images.
#[derive(Debug, Default)]
pub struct DreamcastAnalyzer;

impl DreamcastAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for DreamcastAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "Dreamcast disc analysis not yet implemented",
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
        Platform::Dreamcast
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["gdi", "cdi", "chd"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sega - Dreamcast"]
    }

}
