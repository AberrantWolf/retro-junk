//! Sega CD / Mega CD disc image analyzer.
//!
//! Supports:
//! - BIN/CUE images
//! - ISO images
//! - CHD compressed images

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification,
};

/// Analyzer for Sega CD / Mega CD disc images.
#[derive(Debug, Default)]
pub struct SegaCdAnalyzer;

impl SegaCdAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for SegaCdAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "Sega CD disc analysis not yet implemented",
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
        "Sega CD / Mega CD"
    }

    fn short_name(&self) -> &'static str {
        "segacd"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["segacd", "sega cd", "megacd", "mega cd"]
    }

    fn manufacturer(&self) -> &'static str {
        "Sega"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["bin", "cue", "iso", "chd"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
