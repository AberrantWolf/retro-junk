//! Sega Dreamcast disc image analyzer.
//!
//! Supports:
//! - GDI images (.gdi)
//! - CDI images (.cdi)
//! - CHD compressed images

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Sega Dreamcast disc images.
#[derive(Debug, Default)]
pub struct DreamcastAnalyzer;

impl DreamcastAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for DreamcastAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("Dreamcast disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("Dreamcast disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sega Dreamcast"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["gdi", "cdi", "chd"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("Dreamcast disc detection not yet implemented")
    }
}
