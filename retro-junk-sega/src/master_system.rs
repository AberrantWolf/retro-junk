//! Sega Master System ROM analyzer.
//!
//! Supports:
//! - Master System ROMs (.sms)
//! - Mark III ROMs

use retro_junk_core::ReadSeek;

use retro_junk_core::{AnalysisError, AnalysisOptions, Platform, RomAnalyzer, RomIdentification};

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

    fn platform(&self) -> Platform {
        Platform::MasterSystem
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["sms"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sega - Master System - Mark III"]
    }

    fn gdb_csv_names(&self) -> &'static [&'static str] {
        &["console_sega_markIII_mastersystem"]
    }
}
