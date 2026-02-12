//! Nintendo DS ROM analyzer.
//!
//! Supports:
//! - DS ROMs (.nds)
//! - DSi-enhanced ROMs
//! - DSiWare

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Nintendo DS ROMs.
#[derive(Debug, Default)]
pub struct DsAnalyzer;

impl DsAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for DsAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("DS ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("DS ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Nintendo DS"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["nds", "dsi"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("DS ROM detection not yet implemented")
    }
}
