//! Nintendo 64 ROM analyzer.
//!
//! Supports:
//! - Big-endian ROMs (.z64)
//! - Little-endian ROMs (.n64)
//! - Byte-swapped ROMs (.v64)

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Nintendo 64 ROMs.
#[derive(Debug, Default)]
pub struct N64Analyzer;

impl N64Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for N64Analyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("N64 ROM analysis not yet implemented"))
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
        "Nintendo 64"
    }

    fn short_name(&self) -> &'static str {
        "n64"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["n64", "nintendo 64", "nintendo64"]
    }

    fn manufacturer(&self) -> &'static str {
        "Nintendo"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["z64", "n64", "v64"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
