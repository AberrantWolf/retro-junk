//! Original Xbox disc image analyzer.
//!
//! Supports:
//! - ISO images
//! - XISO format

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for original Xbox disc images.
#[derive(Debug, Default)]
pub struct XboxAnalyzer;

impl XboxAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for XboxAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("Xbox disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("Xbox disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Microsoft Xbox"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "xiso"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("Xbox disc detection not yet implemented")
    }
}
