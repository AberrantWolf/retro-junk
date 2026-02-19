//! Nintendo Wii U disc image analyzer.
//!
//! Supports:
//! - WUD images (.wud)
//! - WUX compressed images (.wux)

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, Platform, RomAnalyzer, RomIdentification,
};

/// Analyzer for Nintendo Wii U disc images.
#[derive(Debug, Default)]
pub struct WiiUAnalyzer;

impl WiiUAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for WiiUAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "Wii U disc analysis not yet implemented",
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
        Platform::WiiU
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["wud", "wux"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Nintendo - Wii U (Digital)"]
    }
}
