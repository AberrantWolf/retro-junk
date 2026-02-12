//! Nintendo 3DS ROM analyzer.
//!
//! Supports:
//! - 3DS ROMs (.3ds)
//! - CIA files (.cia)
//! - CCI files (.cci)

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Nintendo 3DS ROMs.
#[derive(Debug, Default)]
pub struct N3dsAnalyzer;

impl N3dsAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for N3dsAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("3DS ROM analysis not yet implemented"))
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
        "Nintendo 3DS"
    }

    fn short_name(&self) -> &'static str {
        "3ds"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["3ds", "nintendo 3ds", "n3ds"]
    }

    fn manufacturer(&self) -> &'static str {
        "Nintendo"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["3ds", "cia", "cci"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
