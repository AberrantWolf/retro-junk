//! PlayStation 2 disc image analyzer.
//!
//! Supports:
//! - ISO images
//! - BIN/CUE images
//! - CHD compressed images
//! - CSO/ZSO compressed images

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for PlayStation 2 disc images.
#[derive(Debug, Default)]
pub struct Ps2Analyzer;

impl Ps2Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Ps2Analyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("PS2 disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("PS2 disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sony PlayStation 2"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "bin", "cue", "img", "chd", "cso", "zso"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("PS2 disc detection not yet implemented")
    }
}
