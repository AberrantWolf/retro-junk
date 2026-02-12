//! Sega CD / Mega CD disc image analyzer.
//!
//! Supports:
//! - BIN/CUE images
//! - ISO images
//! - CHD compressed images

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Sega CD / Mega CD disc images.
#[derive(Debug, Default)]
pub struct SegaCdAnalyzer;

impl SegaCdAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for SegaCdAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("Sega CD disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("Sega CD disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sega CD / Mega CD"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["bin", "cue", "iso", "chd"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("Sega CD disc detection not yet implemented")
    }
}
