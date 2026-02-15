//! PlayStation (PS1/PSX) disc image analyzer.
//!
//! Supports:
//! - BIN/CUE images
//! - ISO images
//! - CHD compressed images
//! - PBP (PlayStation Portable eboot format)
//! - ECM compressed images

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for PlayStation disc images.
#[derive(Debug, Default)]
pub struct Ps1Analyzer;

impl Ps1Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Ps1Analyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("PS1 disc analysis not yet implemented"))
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
        "Sony PlayStation"
    }

    fn short_name(&self) -> &'static str {
        "ps1"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["ps1", "psx", "playstation", "playstation1"]
    }

    fn manufacturer(&self) -> &'static str {
        "Sony"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["bin", "cue", "iso", "img", "chd", "pbp", "ecm"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
