//! Nintendo Wii disc image analyzer.
//!
//! Supports:
//! - ISO images (.iso)
//! - WBFS images (.wbfs)
//! - RVZ compressed images (.rvz)
//! - CISO compressed images (.ciso)
//! - NKit images (.nkit.iso)
//! - WIA images (.wia)

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Nintendo Wii disc images.
#[derive(Debug, Default)]
pub struct WiiAnalyzer;

impl WiiAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for WiiAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("Wii disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("Wii disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Nintendo Wii"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "wbfs", "rvz", "ciso", "wia"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("Wii disc detection not yet implemented")
    }
}
