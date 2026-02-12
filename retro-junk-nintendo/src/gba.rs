//! Game Boy Advance ROM analyzer.
//!
//! Supports:
//! - GBA ROMs (.gba)
//! - Multiboot ROMs (.mb)

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Game Boy Advance ROMs.
#[derive(Debug, Default)]
pub struct GbaAnalyzer;

impl GbaAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GbaAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("GBA ROM analysis not yet implemented"))
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
        "Game Boy Advance"
    }

    fn short_name(&self) -> &'static str {
        "gba"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["gba", "game boy advance", "gameboy advance"]
    }

    fn manufacturer(&self) -> &'static str {
        "Nintendo"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["gba", "mb"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
