//! Sega Game Gear ROM analyzer.
//!
//! Supports:
//! - Game Gear ROMs (.gg)

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Sega Game Gear ROMs.
#[derive(Debug, Default)]
pub struct GameGearAnalyzer;

impl GameGearAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GameGearAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("Game Gear ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("Game Gear ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sega Game Gear"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["gg"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("Game Gear ROM detection not yet implemented")
    }
}
