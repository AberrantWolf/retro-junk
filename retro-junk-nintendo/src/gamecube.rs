//! Nintendo GameCube disc image analyzer.
//!
//! Supports:
//! - ISO images (.iso)
//! - GCM images (.gcm)
//! - RVZ compressed images (.rvz)
//! - CISO compressed images (.ciso)
//! - NKit images (.nkit.iso, .nkit.gcz)

use retro_junk_core::ReadSeek;

use retro_junk_core::{AnalysisError, AnalysisOptions, Platform, RomAnalyzer, RomIdentification};

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

    fn platform(&self) -> Platform {
        Platform::GameCube
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "gcm", "rvz", "ciso", "gcz"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Nintendo - GameCube"]
    }
}

#[cfg(test)]
#[path = "tests/gamecube_tests.rs"]
mod tests;
