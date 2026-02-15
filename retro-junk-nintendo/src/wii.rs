//! Nintendo Wii disc image analyzer.
//!
//! Supports:
//! - ISO images (.iso)
//! - WBFS images (.wbfs)
//! - RVZ compressed images (.rvz)
//! - CISO compressed images (.ciso)
//! - NKit images (.nkit.iso)
//! - WIA images (.wia)

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification,
};

/// Analyzer for Nintendo Wii disc images.
#[derive(Debug, Default)]
pub struct WiiAnalyzer;

impl WiiAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for WiiAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "Wii disc analysis not yet implemented",
        ))
    }

    fn analyze_with_progress(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        self.analyze(reader, options)
    }

    fn platform_name(&self) -> &'static str {
        "Nintendo Wii"
    }

    fn short_name(&self) -> &'static str {
        "wii"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["wii"]
    }

    fn manufacturer(&self) -> &'static str {
        "Nintendo"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "wbfs", "rvz", "ciso", "wia"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
