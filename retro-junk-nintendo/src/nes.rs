//! NES (Famicom) ROM analyzer.
//!
//! Supports:
//! - iNES format (.nes)
//! - NES 2.0 format
//! - UNIF format (.unf)
//! - Raw PRG/CHR dumps

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for NES/Famicom ROMs.
#[derive(Debug, Default)]
pub struct NesAnalyzer;

impl NesAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for NesAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("NES ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("NES ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Nintendo Entertainment System"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["nes", "unf", "unif", "fds"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("NES ROM detection not yet implemented")
    }
}
