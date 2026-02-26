//! Sega CD / Mega CD disc image analyzer.
//!
//! Supports:
//! - BIN/CUE images
//! - ISO images
//! - CHD compressed images

use retro_junk_core::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, Platform, RomAnalyzer, RomIdentification,
};

/// Analyzer for Sega CD / Mega CD disc images.
#[derive(Debug, Default)]
pub struct SegaCdAnalyzer;

impl SegaCdAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for SegaCdAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "Sega CD disc analysis not yet implemented",
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
        Platform::SegaCd
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["bin", "cue", "iso", "chd"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sega - Mega-CD - Sega CD"]
    }

    fn gdb_csv_names(&self) -> &'static [&'static str] {
        &["console_sega_megacd_segacd"]
    }
}
