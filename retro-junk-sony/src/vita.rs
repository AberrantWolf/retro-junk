//! PlayStation Vita ROM analyzer.
//!
//! Supports:
//! - VPK files
//! - Game card dumps

use retro_junk_core::ReadSeek;

use retro_junk_core::{AnalysisError, AnalysisOptions, Platform, RomAnalyzer, RomIdentification};

/// Analyzer for PlayStation Vita ROMs.
#[derive(Debug, Default)]
pub struct VitaAnalyzer;

impl VitaAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for VitaAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "PS Vita ROM analysis not yet implemented",
        ))
    }

    fn platform(&self) -> Platform {
        Platform::Vita
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["vpk"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sony - PlayStation Vita", "Sony - PlayStation Vita (PSN)"]
    }
}
