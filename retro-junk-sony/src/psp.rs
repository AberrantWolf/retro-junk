//! PlayStation Portable (PSP) disc/ROM analyzer.
//!
//! Supports:
//! - ISO images
//! - CSO compressed images
//! - PBP (EBOOT.PBP format)
//! - DAX compressed images

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification,
};

/// Analyzer for PlayStation Portable disc images.
#[derive(Debug, Default)]
pub struct PspAnalyzer;

impl PspAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for PspAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "PSP disc analysis not yet implemented",
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

    fn platform_name(&self) -> &'static str {
        "Sony PlayStation Portable"
    }

    fn short_name(&self) -> &'static str {
        "psp"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["psp", "playstation portable"]
    }

    fn manufacturer(&self) -> &'static str {
        "Sony"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "cso", "pbp", "dax"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
