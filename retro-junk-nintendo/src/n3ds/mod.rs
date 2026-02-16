//! Nintendo 3DS ROM analyzer.
//!
//! Supports:
//! - CCI / NCSD format (.3ds, .cci) — game card dumps
//! - CIA format (.cia) — eShop / installable archives
//!
//! The analyzer extracts metadata from the NCCH partition header (product code,
//! maker code, program ID, title version) and the NCSD header (media type,
//! partition layout, card info). For CCI files, heuristics detect whether the
//! image originated from a physical game card or was converted from a CIA.
//!
//! SHA-256 hashes in the NCCH header can be verified when content is unencrypted
//! (NoCrypto flag set).

mod cia;
mod common;
mod ncch;
pub(crate) mod ncsd;

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, RomAnalyzer, RomIdentification,
};

use common::{read_u16_le, read_u32_le, read_u64_le};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// 1 media unit = 0x200 bytes (512 bytes).
const MEDIA_UNIT: u64 = 0x200;

/// NCSD magic at offset 0x100: "NCSD".
const NCSD_MAGIC: [u8; 4] = [0x4E, 0x43, 0x53, 0x44];

/// NCCH magic at offset 0x100 within a partition: "NCCH".
const NCCH_MAGIC: [u8; 4] = [0x4E, 0x43, 0x43, 0x48];

/// Typical CIA header size field value.
const CIA_HEADER_SIZE: u32 = 0x2020;

/// Minimum CCI file size: NCSD header + NCCH partition 0 header.
const MIN_CCI_SIZE: u64 = 0x4200;

/// Minimum CIA file size: header + some content.
const MIN_CIA_SIZE: u64 = 0x2020 + 64; // header + alignment

/// Size of NCSD initial data card seed region at 0x1000.
const CARD_SEED_SIZE: usize = 16;

// ---------------------------------------------------------------------------
// Format detection
// ---------------------------------------------------------------------------

/// Detected 3DS file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum N3dsFormat {
    /// NCSD / CCI format (.3ds, .cci) — game card dump.
    Cci,
    /// CIA format (.cia) — eShop / installable archive.
    Cia,
}

/// Detect whether the file is CCI (NCSD) or CIA.
fn detect_format(reader: &mut dyn ReadSeek) -> Result<Option<N3dsFormat>, AnalysisError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;

    // Try NCSD: magic "NCSD" at offset 0x100
    if file_size >= MIN_CCI_SIZE {
        reader.seek(SeekFrom::Start(0x100))?;
        let mut magic = [0u8; 4];
        if reader.read_exact(&mut magic).is_ok() && magic == NCSD_MAGIC {
            reader.seek(SeekFrom::Start(0))?;
            return Ok(Some(N3dsFormat::Cci));
        }
    }

    // Try CIA: header size field at offset 0x000 is typically 0x2020
    if file_size >= MIN_CIA_SIZE {
        reader.seek(SeekFrom::Start(0))?;
        let mut header_buf = [0u8; 0x20];
        if reader.read_exact(&mut header_buf).is_ok() {
            let header_size = read_u32_le(&header_buf, 0x00);
            let cia_type = read_u16_le(&header_buf, 0x04);
            let cia_version = read_u16_le(&header_buf, 0x06);
            let cert_size = read_u32_le(&header_buf, 0x08);
            let ticket_size = read_u32_le(&header_buf, 0x0C);
            let tmd_size = read_u32_le(&header_buf, 0x10);
            let content_size = read_u64_le(&header_buf, 0x18);

            if header_size == CIA_HEADER_SIZE
                && cia_type <= 1
                && cia_version <= 1
                && cert_size > 0
                && cert_size < 0x10000
                && ticket_size > 0
                && ticket_size < 0x10000
                && tmd_size > 0
                && tmd_size < 0x100000
                && content_size > 0
            {
                reader.seek(SeekFrom::Start(0))?;
                return Ok(Some(N3dsFormat::Cia));
            }
        }
    }

    reader.seek(SeekFrom::Start(0))?;
    Ok(None)
}

// ---------------------------------------------------------------------------
// Analyzer implementation
// ---------------------------------------------------------------------------

/// Analyzer for Nintendo 3DS ROMs.
#[derive(Debug, Default)]
pub struct N3dsAnalyzer;

impl N3dsAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for N3dsAnalyzer {
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        match detect_format(reader)? {
            Some(N3dsFormat::Cci) => ncsd::analyze_cci(reader, file_size, options),
            Some(N3dsFormat::Cia) => cia::analyze_cia(reader, file_size, options),
            None => Err(AnalysisError::invalid_format(
                "Not a valid 3DS file (no NCSD magic or CIA header found)",
            )),
        }
    }

    fn analyze_with_progress(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
        _progress_tx: std::sync::mpsc::Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        self.analyze(reader, options)
    }

    fn platform_name(&self) -> &'static str {
        "Nintendo 3DS"
    }

    fn short_name(&self) -> &'static str {
        "3ds"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["3ds", "nintendo 3ds", "n3ds"]
    }

    fn manufacturer(&self) -> &'static str {
        "Nintendo"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["3ds", "cia", "cci"]
    }

    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        detect_format(reader).ok().flatten().is_some()
    }

    fn dat_name(&self) -> Option<&'static str> {
        Some("Nintendo - Nintendo 3DS")
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        Some(serial.to_string())
    }
}

#[cfg(test)]
#[path = "tests/mod_tests.rs"]
mod tests;
