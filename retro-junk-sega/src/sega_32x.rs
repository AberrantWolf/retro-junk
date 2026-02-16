//! Sega 32X ROM analyzer.
//!
//! Supports:
//! - 32X ROMs (.32x)
//! - Combined Genesis/32X ROMs

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, Platform, RomAnalyzer, RomIdentification,
};

/// Analyzer for Sega 32X ROMs.
#[derive(Debug, Default)]
pub struct Sega32xAnalyzer;

impl Sega32xAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Sega32xAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("32X ROM analysis not yet implemented"))
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
        Platform::Sega32x
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["32x"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_name(&self) -> Option<&'static str> {
        Some("Sega - 32X")
    }
}
