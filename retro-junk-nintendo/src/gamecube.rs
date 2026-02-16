//! Nintendo GameCube disc image analyzer.
//!
//! Supports:
//! - ISO images (.iso)
//! - GCM images (.gcm)
//! - RVZ compressed images (.rvz)
//! - CISO compressed images (.ciso)
//! - NKit images (.nkit.iso, .nkit.gcz)

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, Platform, RomAnalyzer, RomIdentification,
};

/// Analyzer for Nintendo GameCube disc images.
#[derive(Debug, Default)]
pub struct GameCubeAnalyzer;

impl GameCubeAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GameCubeAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "GameCube disc analysis not yet implemented",
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
        Platform::GameCube
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "gcm", "rvz", "ciso", "gcz"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}

#[cfg(test)]
#[path = "tests/gamecube_tests.rs"]
mod tests;
