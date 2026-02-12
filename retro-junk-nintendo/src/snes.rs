//! SNES (Super Famicom) ROM analyzer.
//!
//! Supports:
//! - Headered ROMs (.smc, .swc)
//! - Headerless ROMs (.sfc)
//! - LoROM, HiROM, ExHiROM, and SA-1 mappings

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for SNES/Super Famicom ROMs.
#[derive(Debug, Default)]
pub struct SnesAnalyzer;

impl SnesAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for SnesAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("SNES ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("SNES ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Super Nintendo Entertainment System"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["sfc", "smc", "swc", "fig"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("SNES ROM detection not yet implemented")
    }
}
