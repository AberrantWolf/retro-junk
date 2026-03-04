//! Nintendo Wii disc image analyzer.
//!
//! Supports:
//! - ISO images (.iso)
//! - Compressed formats via `nod`: WBFS, RVZ, WIA, CISO, GCZ
//!
//! The Wii disc header shares the same layout as GameCube ("boot.bin",
//! 0x0000–0x043F). Detection uses the Wii magic word 0x5D1C9EA3 at
//! offset 0x0018.
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

/// DVD-5 capacity threshold (4.7 GB).
/// Files larger than this are likely dual-layer (DVD-9).
const DVD5_SIZE_THRESHOLD: u64 = 4_700_000_000;

/// Analyzer for Nintendo Wii disc images.
#[derive(Debug, Default)]
pub struct WiiAnalyzer;

impl RomAnalyzer for WiiAnalyzer {
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = retro_junk_core::util::file_size(reader)?;

        // Detect compressed container (RVZ, WIA, WBFS, CISO, GCZ) or raw ISO.
        // For compressed formats, use the uncompressed disc size for DVD layer detection.
        let (header, format_name, layer_size) = if nintendo_disc::is_compressed_disc(reader) {
            let path = options.file_path.as_ref().ok_or_else(|| {
                AnalysisError::invalid_format(
                    "Compressed disc format detected but no file path provided",
                )
            })?;
            let (header, format_name, disc_size) = nintendo_disc::open_compressed_disc(path)?;
            (header, format_name, disc_size)
        } else {
            (nintendo_disc::parse_disc_header(reader)?, "ISO", file_size)
        };

        if !nintendo_disc::is_wii(&header) {
            return Err(AnalysisError::invalid_format(
                "Not a Wii disc (magic word mismatch)",
            ));
        }

        let mut id = nintendo_disc::build_identification(&header, Platform::Wii);
        id.file_size = Some(file_size);
        id.extra.insert("format".into(), format_name.into());
        id.extra.insert(
            "detected_extension".into(),
            format_name.to_ascii_lowercase(),
        );

        // Detect DVD layer type from uncompressed disc size
        let layer = if layer_size > DVD5_SIZE_THRESHOLD {
            "DVD-9"
        } else {
            "DVD-5"
        };
        id.extra.insert("dvd_layer".into(), layer.into());

        Ok(id)
    }

    fn platform(&self) -> Platform {
        Platform::Wii
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["iso", "wbfs", "rvz", "ciso", "wia"]
    }

    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        nintendo_disc::check_magic(reader)
            .map(|(_, wii)| wii)
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
                "Compressed Wii disc detected but no file path provided for hashing",
            )
        })?;
        log::info!("Wii: hashing compressed disc via nod");
        let hashes = nintendo_disc::hash_compressed_disc(path, algorithms)?;
        Ok(Some(hashes))
    }

    fn dat_source(&self) -> retro_junk_core::DatSource {
        retro_junk_core::DatSource::Redump
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Nintendo - Wii"]
    }

    fn expects_serial(&self) -> bool {
        true
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        // Serial is the 4-byte game code from the disc header (e.g., "RSBE").
        // Return as-is for DAT matching.
        if serial.len() == 4 && serial.is_ascii() {
            Some(serial.to_string())
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "tests/wii_tests.rs"]
mod tests;
