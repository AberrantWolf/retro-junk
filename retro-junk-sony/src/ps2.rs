//! PlayStation 2 disc image analyzer.
//!
//! Supports:
//! - ISO images
//! - BIN/CUE images
//! - CHD compressed images
//! - CSO/ZSO compressed images

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification,
};

/// Analyzer for PlayStation 2 disc images.
#[derive(Debug, Default)]
pub struct Ps2Analyzer;

impl Ps2Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Ps2Analyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "PS2 disc analysis not yet implemented",
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
        "Sony PlayStation 2"
    }

    fn short_name(&self) -> &'static str {
        "ps2"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["ps2", "playstation2", "playstation 2"]
    }

    fn manufacturer(&self) -> &'static str {
        "Sony"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "bin", "cue", "img", "chd", "cso", "zso"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
