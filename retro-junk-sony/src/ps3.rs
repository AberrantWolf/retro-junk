//! PlayStation 3 disc image analyzer.
//!
//! Supports:
//! - ISO images
//! - Folder/JB format
//! - PKG files

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, Platform, RomAnalyzer, RomIdentification,
};

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
        Err(AnalysisError::other(
            "PS3 disc analysis not yet implemented",
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

    fn platform(&self) -> Platform {
        Platform::Ps3
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "pkg"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sony - PlayStation 3"]
    }

    fn dat_download_ids(&self) -> &'static [&'static str] {
        &["ps3"]
    }
}
