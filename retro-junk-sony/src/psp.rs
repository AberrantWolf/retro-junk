//! PlayStation Portable (PSP) disc/ROM analyzer.
//!
//! Supports:
//! - ISO images
//! - CSO compressed images
//! - PBP (EBOOT.PBP format)
//! - DAX compressed images

use retro_junk_core::ReadSeek;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, ChdExtensionRole, ChdMedia, FileHashes, HashAlgorithms,
    Platform, RomAnalyzer, RomIdentification,
};
use retro_junk_disc::hash::hash_disc_container;

/// Analyzer for PlayStation Portable disc images.
#[derive(Debug, Default)]
pub struct PspAnalyzer;

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

    fn platform(&self) -> Platform {
        Platform::Psp
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "cso", "pbp", "dax", "chd"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn redump_slug(&self) -> Option<&'static str> {
        Some("psp")
    }

    fn chd_extensions(&self) -> &'static [(&'static str, ChdExtensionRole)] {
        // UMDs are DVD-family media (PPSSPP supports CHD since 1.12).
        // cso/dax are already-compressed containers chdman cannot read.
        &[
            ("iso", ChdExtensionRole::Source(ChdMedia::Dvd)),
            ("cso", ChdExtensionRole::Unconvertible),
            ("dax", ChdExtensionRole::Unconvertible),
        ]
    }

    fn compute_container_hashes(
        &self,
        reader: &mut dyn ReadSeek,
        algorithms: HashAlgorithms,
        file_path: Option<&std::path::Path>,
        on_progress: retro_junk_core::HashProgressFn<'_>,
    ) -> Result<Option<FileHashes>, AnalysisError> {
        hash_disc_container(reader, algorithms, file_path, "PSP", on_progress)
    }

    fn dat_download_ids(&self) -> &'static [&'static str] {
        &["psp"]
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sony - PlayStation Portable"]
    }
}

#[cfg(test)]
#[path = "tests/psp_tests.rs"]
mod tests;
