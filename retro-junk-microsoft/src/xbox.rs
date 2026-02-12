//! Original Xbox disc image analyzer.
//!
//! Supports:
//! - ISO images
//! - XISO format

use retro_junk_lib::ReadSeek;
use std::sync::mpsc::Sender;

use retro_junk_lib::{AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification};

/// Analyzer for original Xbox disc images.
#[derive(Debug, Default)]
pub struct XboxAnalyzer;

impl XboxAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for XboxAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other("Xbox disc analysis not yet implemented"))
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
        "Microsoft Xbox"
    }

    fn short_name(&self) -> &'static str {
        "xbox"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["xbox", "xbox1", "ogxbox"]
    }

    fn manufacturer(&self) -> &'static str {
        "Microsoft"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "xiso"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }
}
