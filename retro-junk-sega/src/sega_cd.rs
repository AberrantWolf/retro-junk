//! Sega CD / Mega CD disc image analyzer.
//!
//! Supports:
//! - BIN/CUE images
//! - ISO images
//! - CHD compressed images

use retro_junk_core::ReadSeek;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, ChdExtensionRole, ChdMedia, FileHashes, HashAlgorithms,
    Platform, RomAnalyzer, RomIdentification,
};
use retro_junk_disc::hash::hash_disc_container;

/// Analyzer for Sega CD / Mega CD disc images.
#[derive(Debug, Default)]
pub struct SegaCdAnalyzer;

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

    fn redump_slug(&self) -> Option<&'static str> {
        Some("mcd")
    }

    fn chd_extensions(&self) -> &'static [(&'static str, ChdExtensionRole)] {
        &[("cue", ChdExtensionRole::Source(ChdMedia::Cd))]
    }

    fn compute_container_hashes(
        &self,
        reader: &mut dyn ReadSeek,
        algorithms: HashAlgorithms,
        file_path: Option<&std::path::Path>,
        on_progress: retro_junk_core::HashProgressFn<'_>,
    ) -> Result<Option<FileHashes>, AnalysisError> {
        hash_disc_container(reader, algorithms, file_path, "Sega CD", on_progress)
    }

    fn dat_download_ids(&self) -> &'static [&'static str] {
        &["mcd"]
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Sega - Mega-CD - Sega CD"]
    }

    fn gdb_csv_names(&self) -> &'static [&'static str] {
        &["console_sega_megacd_segacd"]
    }
}

#[cfg(test)]
#[path = "tests/sega_cd_tests.rs"]
mod tests;
