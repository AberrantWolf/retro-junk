//! Nintendo DS ROM analyzer.
//!
//! Supports:
//! - DS ROMs (.nds)
//! - DSi-enhanced ROMs
//! - DSiWare

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Nintendo DS ROMs.
#[derive(Debug, Default)]
pub struct DsAnalyzer;

impl DsAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for DsAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("DS ROM analysis not yet implemented"))
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
        "Nintendo DS"
    }

    fn short_name(&self) -> &'static str {
        "nds"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["nds", "ds", "nintendo ds"]
    }

    fn manufacturer(&self) -> &'static str {
        "Nintendo"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["nds", "dsi"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
