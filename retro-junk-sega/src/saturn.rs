//! Sega Saturn disc image analyzer.
//!
//! Supports:
//! - BIN/CUE images
//! - ISO images
//! - CHD compressed images
//! - MDF/MDS images

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Sega Saturn disc images.
#[derive(Debug, Default)]
pub struct SaturnAnalyzer;

impl SaturnAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for SaturnAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("Saturn disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("Saturn disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sega Saturn"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["bin", "cue", "iso", "chd", "mdf", "mds"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("Saturn disc detection not yet implemented")
    }
}
