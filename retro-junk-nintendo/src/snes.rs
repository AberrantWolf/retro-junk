//! SNES (Super Famicom) ROM analyzer.
//!
//! Supports:
//! - Headered ROMs (.smc, .swc)
//! - Headerless ROMs (.sfc)
//! - LoROM, HiROM, ExHiROM, and SA-1 mappings

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for SNES/Super Famicom ROMs.
#[derive(Debug, Default)]
pub struct SnesAnalyzer;

impl SnesAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for SnesAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("SNES ROM analysis not yet implemented"))
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
        "Super Nintendo Entertainment System"
    }

    fn short_name(&self) -> &'static str {
        "snes"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["snes", "sfc", "super famicom", "super nintendo"]
    }

    fn manufacturer(&self) -> &'static str {
        "Nintendo"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["sfc", "smc", "swc", "fig"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
