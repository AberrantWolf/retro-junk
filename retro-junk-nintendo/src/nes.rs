//! NES (Famicom) ROM analyzer.
//!
//! Supports:
//! - iNES format (.nes)
//! - NES 2.0 format
//! - UNIF format (.unf)
//! - Raw PRG/CHR dumps

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for NES/Famicom ROMs.
#[derive(Debug, Default)]
pub struct NesAnalyzer;

impl NesAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for NesAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("NES ROM analysis not yet implemented"))
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
        "Nintendo Entertainment System"
    }

    fn short_name(&self) -> &'static str {
        "nes"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["nes", "famicom", "fc"]
    }

    fn manufacturer(&self) -> &'static str {
        "Nintendo"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["nes", "unf", "unif", "fds"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
