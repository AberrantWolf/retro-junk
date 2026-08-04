//! NEC PC Engine / TurboGrafx-16 `HuCard` analyzer.
//!
//! `HuCard` dumps (`.pce`) are raw cartridge bytes: unlike the NES or the Mega
//! Drive, the card carries no header, no title string, no serial, and no
//! region byte. There is nothing inside the file to read, so identification
//! happens entirely by hashing the ROM and looking the digest up in the
//! No-Intro database.
//!
//! The one thing a `.pce` file *can* carry is a 512-byte copier header left
//! by old backup units, exactly like the SNES `.smc` case. No-Intro records
//! digests of the cartridge bytes alone, so that header has to be skipped
//! before hashing or every headered dump misses its match.
//!
//! See `.claude/skills/retro-archive/formats/PCEngine.md`.

use retro_junk_core::{
    AnalysisError, AnalysisOptions, Platform, ReadSeek, RomAnalyzer, RomIdentification,
};

/// A `HuCard` is banked in 8 KB units, so a headerless dump is always a whole
/// number of banks. That is what makes a leftover copier header detectable.
const BANK_SIZE: u64 = 8192;

/// Bytes prepended by older backup units ("copier header").
const COPIER_HEADER_SIZE: u64 = 512;

/// Smallest plausible `HuCard`: one 8 KB bank.
const MIN_ROM_SIZE: u64 = BANK_SIZE;

/// Largest commercial `HuCard` is 20 Mbit (2.5 MB, Street Fighter II' Champion
/// Edition). Allow generous headroom for oversized homebrew while still
/// rejecting obviously wrong files.
const MAX_ROM_SIZE: u64 = 8 * 1024 * 1024;

/// Analyzer for NEC PC Engine / TurboGrafx-16 `HuCard` ROMs.
#[derive(Debug, Default)]
pub struct PceAnalyzer;

/// How many bytes at the front of a dump are copier padding rather than
/// cartridge data.
///
/// A clean dump is a whole number of 8 KB banks; a headered dump is that plus
/// 512 bytes. Any other size is neither, so nothing is skipped.
fn copier_header_size(file_size: u64) -> u64 {
    if file_size % BANK_SIZE == COPIER_HEADER_SIZE {
        COPIER_HEADER_SIZE
    } else {
        0
    }
}

impl RomAnalyzer for PceAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Err(AnalysisError::other(
            "PC Engine HuCard identification uses No-Intro hash matching; \
             the cartridge carries no header to read",
        ))
    }

    fn platform(&self) -> Platform {
        Platform::Pce
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["pce"]
    }

    /// With no magic bytes to look for, the only honest check is shape: the
    /// dump must be a whole number of 8 KB banks (optionally behind a copier
    /// header) and within `HuCard` size limits.
    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        let Ok(file_size) = reader.seek(std::io::SeekFrom::End(0)) else {
            return false;
        };
        let _ = reader.seek(std::io::SeekFrom::Start(0));
        let rom_size = file_size - copier_header_size(file_size);
        rom_size.is_multiple_of(BANK_SIZE) && (MIN_ROM_SIZE..=MAX_ROM_SIZE).contains(&rom_size)
    }

    /// Spelled the way `LibRetro`'s mirror of the No-Intro set names the
    /// file (`TurboGrafx 16`, no hyphen) — this string is the download path,
    /// so a prettier spelling is a 404.
    fn dat_names(&self) -> &'static [&'static str] {
        &["NEC - PC Engine - TurboGrafx 16"]
    }

    /// One `GameDataBase` sheet covers the whole card family, `SuperGrafx`
    /// included; the extra rows are harmless here since matching is by hash.
    fn gdb_csv_names(&self) -> &'static [&'static str] {
        &["console_nec_pcengine_turbografx_supergrafx"]
    }

    fn dat_header_size(
        &self,
        _reader: &mut dyn ReadSeek,
        file_size: u64,
    ) -> Result<u64, AnalysisError> {
        Ok(copier_header_size(file_size))
    }
}

#[cfg(test)]
#[path = "tests/pce_tests.rs"]
mod tests;
