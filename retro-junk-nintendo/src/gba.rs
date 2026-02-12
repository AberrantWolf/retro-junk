//! Game Boy Advance ROM analyzer.
//!
//! Supports:
//! - GBA ROMs (.gba)
//! - Multiboot ROMs (.mb)

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Game Boy Advance ROMs.
#[derive(Debug, Default)]
pub struct GbaAnalyzer;

impl GbaAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GbaAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("GBA ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("GBA ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Game Boy Advance"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["gba", "mb"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("GBA ROM detection not yet implemented")
    }
}
