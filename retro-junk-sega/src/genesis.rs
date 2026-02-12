//! Sega Genesis / Mega Drive ROM analyzer.
//!
//! Supports:
//! - Genesis/Mega Drive ROMs (.md, .gen, .bin)
//! - Interleaved ROMs (.smd)

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Sega Genesis / Mega Drive ROMs.
#[derive(Debug, Default)]
pub struct GenesisAnalyzer;

impl GenesisAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GenesisAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("Genesis ROM analysis not yet implemented"))
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
        "Sega Genesis / Mega Drive"
    }

    fn short_name(&self) -> &'static str {
        "genesis"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["genesis", "megadrive", "mega drive", "md"]
    }

    fn manufacturer(&self) -> &'static str {
        "Sega"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["md", "gen", "bin", "smd"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
