//! Sega SG-1000 ROM analyzer.
//!
//! Supports:
//! - SG-1000 ROMs (.sg)
//! - SC-3000 software

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Sega SG-1000 ROMs.
#[derive(Debug, Default)]
pub struct Sg1000Analyzer;

impl Sg1000Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Sg1000Analyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("SG-1000 ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("SG-1000 ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sega SG-1000"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["sg", "sc"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("SG-1000 ROM detection not yet implemented")
    }
}
