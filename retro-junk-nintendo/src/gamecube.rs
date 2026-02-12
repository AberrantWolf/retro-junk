//! Nintendo GameCube disc image analyzer.
//!
//! Supports:
//! - ISO images (.iso)
//! - GCM images (.gcm)
//! - RVZ compressed images (.rvz)
//! - CISO compressed images (.ciso)
//! - NKit images (.nkit.iso, .nkit.gcz)

use std::io::{Read, Seek};
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Nintendo GameCube disc images.
#[derive(Debug, Default)]
pub struct GameCubeAnalyzer;

impl GameCubeAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GameCubeAnalyzer {
    fn analyze<R: Read + Seek>(&self, _reader: R) -> Result<RomIdentification, AnalysisError> {
        todo!("GameCube disc analysis not yet implemented")
    }

    fn analyze_with_progress<R: Read + Seek>(
        &self,
        _reader: R,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        todo!("GameCube disc analysis not yet implemented")
    }

    fn platform_name(&self) -> &'static str {
        "Nintendo GameCube"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "gcm", "rvz", "ciso", "gcz"]
    }

    fn can_handle<R: Read + Seek>(&self, _reader: R) -> bool {
        todo!("GameCube disc detection not yet implemented")
    }
}
