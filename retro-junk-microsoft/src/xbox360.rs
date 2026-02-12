//! Xbox 360 disc image analyzer.
//!
//! Supports:
//! - ISO images
//! - GOD (Games on Demand) format
//! - XEX executables

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Xbox 360 disc images.
#[derive(Debug, Default)]
pub struct Xbox360Analyzer;

impl Xbox360Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Xbox360Analyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("Xbox 360 disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("Xbox 360 disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Microsoft Xbox 360"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "xex"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("Xbox 360 disc detection not yet implemented")
    }
}
