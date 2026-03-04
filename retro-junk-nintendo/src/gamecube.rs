//! Nintendo GameCube disc image analyzer.
//!
//! Supports:
//! - ISO images (.iso)
//! - GCM images (.gcm)
//! - Compressed formats via `nod`: RVZ, WIA, WBFS, CISO, GCZ
//!
//! The GameCube disc header ("boot.bin") occupies bytes 0x0000–0x043F.
//! Detection uses the DVD magic word 0xC2339F3D at offset 0x001C, with
//! verification that the Wii magic at 0x0018 is absent.
//!
//! Compressed format support uses the `nod` crate (by the Dolphin team) to
//! transparently decompress disc containers. The decompressed data is passed
//! to the same `parse_disc_header()` used for raw ISOs.

use std::path::Path;

use retro_junk_core::ReadSeek;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, FileHashes, HashAlgorithms, Platform, RomAnalyzer,
    RomIdentification,
};

use crate::nintendo_disc;

/// Standard GameCube disc size: 1,459,978,240 bytes (1.4 GB mini-DVD).
const GCM_DISC_SIZE: u64 = 1_459_978_240;

/// Analyzer for Nintendo GameCube disc images.
#[derive(Debug, Default)]
pub struct GameCubeAnalyzer;

impl RomAnalyzer for GameCubeAnalyzer {
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = retro_junk_core::util::file_size(reader)?;

        // Detect compressed container (RVZ, WIA, WBFS, CISO, GCZ) or raw ISO
        let (header, format_name) = if nintendo_disc::is_compressed_disc(reader) {
            let path = options.file_path.as_ref().ok_or_else(|| {
                AnalysisError::invalid_format(
                    "Compressed disc format detected but no file path provided",
                )
            })?;
            let (header, format_name, _disc_size) = nintendo_disc::open_compressed_disc(path)?;
            (header, format_name)
        } else {
            (nintendo_disc::parse_disc_header(reader)?, "ISO")
        };

        if !nintendo_disc::is_gamecube(&header) {
            return Err(AnalysisError::invalid_format(
                "Not a GameCube disc (magic word mismatch)",
            ));
        }

        let mut id = nintendo_disc::build_identification(&header, Platform::GameCube);
        id.file_size = Some(file_size);
        id.expected_size = Some(GCM_DISC_SIZE);
        id.extra.insert("format".into(), format_name.into());
        id.extra.insert(
            "detected_extension".into(),
            format_name.to_ascii_lowercase(),
        );

        Ok(id)
    }

    fn platform(&self) -> Platform {
        Platform::GameCube
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "gcm", "rvz", "ciso", "gcz"]
    }

    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        nintendo_disc::check_magic(reader)
            .map(|(gc, _)| gc)
            .unwrap_or(false)
    }

    fn compute_container_hashes(
        &self,
        reader: &mut dyn ReadSeek,
        algorithms: HashAlgorithms,
        file_path: Option<&Path>,
    ) -> Result<Option<FileHashes>, AnalysisError> {
        if !nintendo_disc::is_compressed_disc(reader) {
            return Ok(None);
        }
        let path = file_path.ok_or_else(|| {
            AnalysisError::invalid_format(
                "Compressed GameCube disc detected but no file path provided for hashing",
            )
        })?;
        log::info!("GameCube: hashing compressed disc via nod");
        let hashes = nintendo_disc::hash_compressed_disc(path, algorithms)?;
        Ok(Some(hashes))
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Nintendo - GameCube"]
    }

    fn expects_serial(&self) -> bool {
        true
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        // Serial is the 4-byte game code from the disc header (e.g., "GALE").
        // Return as-is for DAT matching.
        if serial.len() == 4 && serial.is_ascii() {
            Some(serial.to_string())
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "tests/gamecube_tests.rs"]
mod tests;
