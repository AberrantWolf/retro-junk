//! Sega Saturn disc image analyzer.
//!
//! Supports:
//! - BIN/CUE images
//! - ISO images
//! - CHD compressed images
//! - MDF/MDS images

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Sega Saturn disc images.
#[derive(Debug, Default)]
pub struct SaturnAnalyzer;

impl SaturnAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for SaturnAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("Saturn disc analysis not yet implemented"))
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
        "Sega Saturn"
    }

    fn short_name(&self) -> &'static str {
        "saturn"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["saturn", "sega saturn"]
    }

    fn manufacturer(&self) -> &'static str {
        "Sega"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["bin", "cue", "iso", "chd", "mdf", "mds"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
