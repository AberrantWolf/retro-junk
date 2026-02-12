//! PlayStation Portable (PSP) disc/ROM analyzer.
//!
//! Supports:
//! - ISO images
//! - CSO compressed images
//! - PBP (EBOOT.PBP format)
//! - DAX compressed images

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for PlayStation Portable disc images.
#[derive(Debug, Default)]
pub struct PspAnalyzer;

impl PspAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for PspAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("PSP disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("PSP disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sony PlayStation Portable"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "cso", "pbp", "dax"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("PSP disc detection not yet implemented")
    }
}
