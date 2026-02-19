//! Sega SG-1000 ROM analyzer.
//!
//! Supports:
//! - SG-1000 ROMs (.sg)
//! - SC-3000 software

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, Platform, RomAnalyzer, RomIdentification,
};

/// Analyzer for Sega SG-1000 ROMs.
#[derive(Debug, Default)]
pub struct Sg1000Analyzer;

impl Sg1000Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Sg1000Analyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "SG-1000 ROM analysis not yet implemented",
        ))
    }

    fn analyze_with_progress(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        self.analyze(reader, options)
    }

    fn platform(&self) -> Platform {
        Platform::Sg1000
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["sg", "sc"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sega - SG-1000"]
    }
}
