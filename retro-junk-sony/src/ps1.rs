//! PlayStation (PS1/PSX) disc image analyzer.
//!
//! Supports:
//! - BIN/CUE images
//! - ISO images
//! - CHD compressed images
//! - PBP (PlayStation Portable eboot format)
//! - ECM compressed images

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for PlayStation disc images.
#[derive(Debug, Default)]
pub struct Ps1Analyzer;

impl Ps1Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Ps1Analyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("PS1 disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("PS1 disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sony PlayStation"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["bin", "cue", "iso", "img", "chd", "pbp", "ecm"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("PS1 disc detection not yet implemented")
    }
}
