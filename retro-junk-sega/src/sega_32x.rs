//! Sega 32X ROM analyzer.
//!
//! Supports:
//! - 32X ROMs (.32x)
//! - Combined Genesis/32X ROMs

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Sega 32X ROMs.
#[derive(Debug, Default)]
pub struct Sega32xAnalyzer;

impl Sega32xAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Sega32xAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("32X ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("32X ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sega 32X"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["32x"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("32X ROM detection not yet implemented")
    }
}
