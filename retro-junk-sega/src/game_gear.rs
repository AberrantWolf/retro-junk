//! Sega Game Gear ROM analyzer.
//!
//! Supports:
//! - Game Gear ROMs (.gg)

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, Platform, RomAnalyzer, RomIdentification,
};

/// Analyzer for Sega Game Gear ROMs.
#[derive(Debug, Default)]
pub struct GameGearAnalyzer;

impl GameGearAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GameGearAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "Game Gear ROM analysis not yet implemented",
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
        Platform::GameGear
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["gg"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sega - Game Gear"]
    }

    fn gdb_csv_names(&self) -> &'static [&'static str] {
        &["console_sega_gamegear"]
    }
}
