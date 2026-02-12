//! Nintendo 64 ROM analyzer.
//!
//! Supports:
//! - Big-endian ROMs (.z64)
//! - Little-endian ROMs (.n64)
//! - Byte-swapped ROMs (.v64)

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Nintendo 64 ROMs.
#[derive(Debug, Default)]
pub struct N64Analyzer;

impl N64Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for N64Analyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("N64 ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("N64 ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Nintendo 64"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["z64", "n64", "v64"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("N64 ROM detection not yet implemented")
    }
}
