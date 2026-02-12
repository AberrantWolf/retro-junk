//! Sega 32X ROM analyzer.
//!
//! Supports:
//! - 32X ROMs (.32x)
//! - Combined Genesis/32X ROMs

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

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

    fn platform_name(&self) -> &'static str {
        "Sega 32X"
    }

    fn short_name(&self) -> &'static str {
        "32x"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["32x", "sega32x", "sega 32x"]
    }

    fn manufacturer(&self) -> &'static str {
        "Sega"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["32x"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
