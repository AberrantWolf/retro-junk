//! Sega Genesis / Mega Drive ROM analyzer.
//!
//! Supports:
//! - Genesis/Mega Drive ROMs (.md, .gen, .bin)
//! - Interleaved ROMs (.smd)

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Sega Genesis / Mega Drive ROMs.
#[derive(Debug, Default)]
pub struct GenesisAnalyzer;

impl GenesisAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GenesisAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("Genesis ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("Genesis ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sega Genesis / Mega Drive"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["md", "gen", "bin", "smd"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("Genesis ROM detection not yet implemented")
    }
}
