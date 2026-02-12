//! PlayStation 3 disc image analyzer.
//!
//! Supports:
//! - ISO images
//! - Folder/JB format
//! - PKG files

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for PlayStation 3 disc images.
#[derive(Debug, Default)]
pub struct Ps3Analyzer;

impl Ps3Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Ps3Analyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("PS3 disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("PS3 disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sony PlayStation 3"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "pkg"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("PS3 disc detection not yet implemented")
    }
}
