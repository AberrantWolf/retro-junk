//! PlayStation Vita ROM analyzer.
//!
//! Supports:
//! - VPK files
//! - Game card dumps

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for PlayStation Vita ROMs.
#[derive(Debug, Default)]
pub struct VitaAnalyzer;

impl VitaAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for VitaAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("PS Vita ROM analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("PS Vita ROM analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Sony PlayStation Vita"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["vpk"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("PS Vita ROM detection not yet implemented")
    }
}
