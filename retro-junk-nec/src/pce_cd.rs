//! NEC PC Engine CD-ROM² / TurboGrafx-CD analyzer.
//!
//! These discs carry no usable header of their own, so identification is done
//! by hashing every track and matching the set against Redump.

use retro_junk_core::{
    AnalysisError, AnalysisOptions, ChdExtensionRole, ChdMedia, DatSource, FileHashes,
    HashAlgorithms, HashProgressFn, Platform, ReadSeek, RomAnalyzer, RomIdentification,
};

/// Redump-backed PC Engine CD / TurboGrafx-CD analyzer.
#[derive(Debug, Default)]
pub struct PceCdAnalyzer;

impl RomAnalyzer for PceCdAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "PC Engine CD identification uses complete Redump track hashes",
        ))
    }

    fn platform(&self) -> Platform {
        Platform::PceCd
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["bin", "cue", "iso", "chd"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        false
    }

    fn dat_source(&self) -> DatSource {
        DatSource::Redump
    }

    fn redump_slug(&self) -> Option<&'static str> {
        Some("pce")
    }

    fn dat_download_ids(&self) -> &'static [&'static str] {
        &["pce"]
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["NEC - PC Engine CD & TurboGrafx CD"]
    }

    fn gdb_csv_names(&self) -> &'static [&'static str] {
        &["console_nec_cdrom2"]
    }

    fn chd_extensions(&self) -> &'static [(&'static str, ChdExtensionRole)] {
        &[("cue", ChdExtensionRole::Source(ChdMedia::Cd))]
    }

    fn compute_container_hashes(
        &self,
        reader: &mut dyn ReadSeek,
        algorithms: HashAlgorithms,
        file_path: Option<&std::path::Path>,
        on_progress: HashProgressFn<'_>,
    ) -> Result<Option<FileHashes>, AnalysisError> {
        retro_junk_disc::hash::hash_disc_container(
            reader,
            algorithms,
            file_path,
            "PC Engine CD",
            on_progress,
        )
    }
}
