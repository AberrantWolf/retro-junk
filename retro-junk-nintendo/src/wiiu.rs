//! Nintendo Wii U disc image analyzer.
//!
//! Supports:
//! - WUD images (.wud)
//! - WUX compressed images (.wux)

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Nintendo Wii U disc images.
#[derive(Debug, Default)]
pub struct WiiUAnalyzer;

impl WiiUAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for WiiUAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("Wii U disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("Wii U disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Nintendo Wii U"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["wud", "wux"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("Wii U disc detection not yet implemented")
    }
}
