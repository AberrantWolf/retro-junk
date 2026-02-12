//! Xbox 360 disc image analyzer.
//!
//! Supports:
//! - ISO images
//! - GOD (Games on Demand) format
//! - XEX executables

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for Xbox 360 disc images.
#[derive(Debug, Default)]
pub struct Xbox360Analyzer;

impl Xbox360Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for Xbox360Analyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("Xbox 360 disc analysis not yet implemented"))
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
        "Microsoft Xbox 360"
    }

    fn short_name(&self) -> &'static str {
        "xbox360"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["xbox360", "xbox 360", "x360"]
    }

    fn manufacturer(&self) -> &'static str {
        "Microsoft"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "xex"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
