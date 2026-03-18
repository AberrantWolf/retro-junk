//! Nintendo Wii U disc image analyzer.
//!
//! Supports:
//! - WUD images (.wud)
//! - WUX compressed images (.wux)

use retro_junk_core::ReadSeek;

use retro_junk_core::{AnalysisError, AnalysisOptions, Platform, RomAnalyzer, RomIdentification};

/// Analyzer for Nintendo Wii U disc images.
#[derive(Debug, Default)]
pub struct WiiUAnalyzer;

impl RomAnalyzer for WiiUAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "Wii U disc analysis not yet implemented",
        ))
    }

    fn platform(&self) -> Platform {
        Platform::WiiU
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["wud", "wux"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    // No DAT support — Redump has no Wii U entries, and the LibRetro
    // "Nintendo - Wii U (Digital)" DAT was not a real Redump dataset.
}
