//! Sega Dreamcast disc image analyzer.
//!
//! Supports:
//! - GDI images (.gdi)
//! - CDI images (.cdi)
//! - CHD compressed images

use retro_junk_core::ReadSeek;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, ChdExtensionRole, ChdMedia, FileHashes, HashAlgorithms,
    Platform, RomAnalyzer, RomIdentification,
};
use retro_junk_disc::hash::hash_disc_container;

/// Analyzer for Sega Dreamcast disc images.
#[derive(Debug, Default)]
pub struct DreamcastAnalyzer;

impl RomAnalyzer for DreamcastAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "Dreamcast disc analysis not yet implemented",
        ))
    }

    fn platform(&self) -> Platform {
        Platform::Dreamcast
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["gdi", "cdi", "chd"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false // Not yet implemented
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn redump_slug(&self) -> Option<&'static str> {
        Some("dc")
    }

    fn chd_extensions(&self) -> &'static [(&'static str, ChdExtensionRole)] {
        // GD-ROM dumps as .gdi track sets round-trip losslessly through
        // chdman createcd/extractcd. .cdi (DiscJuggler) is not supported
        // as chdman input.
        &[
            ("gdi", ChdExtensionRole::Source(ChdMedia::Cd)),
            ("cdi", ChdExtensionRole::Unconvertible),
        ]
    }

    fn compute_container_hashes(
        &self,
        reader: &mut dyn ReadSeek,
        algorithms: HashAlgorithms,
        file_path: Option<&std::path::Path>,
        on_progress: retro_junk_core::HashProgressFn<'_>,
    ) -> Result<Option<FileHashes>, AnalysisError> {
        hash_disc_container(reader, algorithms, file_path, "Dreamcast", on_progress)
    }

    fn dat_download_ids(&self) -> &'static [&'static str] {
        &["dc"]
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sega - Dreamcast"]
    }
}

#[cfg(test)]
#[path = "tests/dreamcast_tests.rs"]
mod tests;
