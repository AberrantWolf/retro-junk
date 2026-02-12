//! Nintendo 3DS ROM analyzer.
//!
//! Supports:
//! - 3DS ROMs (.3ds)
//! - CIA files (.cia)
//! - CCI files (.cci)

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Nintendo 3DS ROMs.
#[derive(Debug, Default)]
pub struct N3dsAnalyzer;

impl N3dsAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for N3dsAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("3DS ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("3DS ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Nintendo 3DS"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["3ds", "cia", "cci"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("3DS ROM detection not yet implemented")
    }
}
