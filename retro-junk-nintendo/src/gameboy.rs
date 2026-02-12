//! Game Boy / Game Boy Color ROM analyzer.
//!
//! Supports:
//! - Game Boy ROMs (.gb)
//! - Game Boy Color ROMs (.gbc)
//! - Dual-mode ROMs (GB/GBC compatible)

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Game Boy and Game Boy Color ROMs.
#[derive(Debug, Default)]
pub struct GameBoyAnalyzer;

impl GameBoyAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GameBoyAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("Game Boy ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("Game Boy ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Game Boy / Game Boy Color"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["gb", "gbc", "sgb"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("Game Boy ROM detection not yet implemented")
    }
}
