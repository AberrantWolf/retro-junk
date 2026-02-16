//! Sega Master System ROM analyzer.
//!
//! Supports:
//! - Master System ROMs (.sms)
//! - Mark III ROMs

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, Platform, RomAnalyzer, RomIdentification,
};

/// Analyzer for Sega Master System ROMs.
#[derive(Debug, Default)]
pub struct MasterSystemAnalyzer;

impl MasterSystemAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for MasterSystemAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "Master System ROM analysis not yet implemented",
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
        Platform::MasterSystem
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["sms"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_name(&self) -> Option<&'static str> {
        Some("Sega - Master System - Mark III")
    }
}
