//! PlayStation 3 disc image analyzer.
//!
//! Supports:
//! - ISO images
//! - Folder/JB format
//! - PKG files

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for PlayStation 3 disc images.
#[derive(Debug, Default)]
pub struct Ps3Analyzer;

impl Ps3Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Ps3Analyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("PS3 disc analysis not yet implemented"))
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
        "Sony PlayStation 3"
    }

    fn short_name(&self) -> &'static str {
        "ps3"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["ps3", "playstation3", "playstation 3"]
    }

    fn manufacturer(&self) -> &'static str {
        "Sony"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "pkg"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
