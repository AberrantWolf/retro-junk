//! Sega Master System ROM analyzer.
//!
//! Supports:
//! - Master System ROMs (.sms)
//! - Mark III ROMs

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Sega Master System ROMs.
#[derive(Debug, Default)]
pub struct MasterSystemAnalyzer;

impl MasterSystemAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for MasterSystemAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("Master System ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("Master System ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sega Master System"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["sms"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("Master System ROM detection not yet implemented")
    }
}
