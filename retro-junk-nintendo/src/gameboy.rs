//! Game Boy / Game Boy Color ROM analyzer.
//!
//! Supports:
//! - Game Boy ROMs (.gb)
//! - Game Boy Color ROMs (.gbc)
//! - Dual-mode ROMs (GB/GBC compatible)

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Game Boy and Game Boy Color ROMs.
#[derive(Debug, Default)]
pub struct GameBoyAnalyzer;

impl GameBoyAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GameBoyAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("Game Boy ROM analysis not yet implemented"))
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
        "Game Boy / Game Boy Color"
    }

    fn short_name(&self) -> &'static str {
        "gb"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["gb", "gbc", "gameboy", "game boy"]
    }

    fn manufacturer(&self) -> &'static str {
        "Nintendo"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["gb", "gbc", "sgb"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
