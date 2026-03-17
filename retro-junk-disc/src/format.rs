//! Disc image format detection.

use retro_junk_core::AnalysisError;
use std::io::{Read, SeekFrom};

use crate::sector::{CD_SYNC_PATTERN, CHD_MAGIC, ISO_SECTOR_SIZE, PVD_SECTOR};

/// Detected disc image format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscFormat {
    /// Standard 2048 byte/sector ISO image.
    Iso2048,
    /// Raw 2352 byte/sector BIN image.
    RawSector2352,
    /// CUE sheet (text file referencing BIN tracks).
    Cue,
    /// MAME Compressed Hunks of Data.
    Chd,
}

impl DiscFormat {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Iso2048 => "ISO 9660",
            Self::RawSector2352 => "Raw BIN (2352)",
            Self::Cue => "CUE Sheet",
            Self::Chd => "CHD",
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Self::Iso2048 => "iso",
            Self::RawSector2352 => "bin",
            Self::Cue => "cue",
            Self::Chd => "chd",
        }
    }
}

/// Detect the disc image format by examining the reader content.
pub fn detect_disc_format(
    reader: &mut dyn retro_junk_core::ReadSeek,
) -> Result<DiscFormat, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    let mut buf = [0u8; 16];
    let bytes_read = reader.read(&mut buf)?;
    reader.seek(SeekFrom::Start(0))?;

    if bytes_read < 12 {
        return Err(AnalysisError::TooSmall {
            expected: 12,
            actual: bytes_read as u64,
        });
    }

    // Check CHD magic
    if bytes_read >= 8 && buf[..8] == *CHD_MAGIC {
        return Ok(DiscFormat::Chd);
    }

    // Check raw sector sync pattern
    if buf[..12] == CD_SYNC_PATTERN {
        return Ok(DiscFormat::RawSector2352);
    }

    // Check for CUE sheet: scan for common CUE keywords in what looks like text
    if looks_like_cue(reader)? {
        return Ok(DiscFormat::Cue);
    }

    // Check for ISO 9660 PVD at sector 16
    let pvd_offset = PVD_SECTOR * ISO_SECTOR_SIZE + 1; // +1 to skip type byte
    reader.seek(SeekFrom::Start(pvd_offset))?;
    let mut cd001 = [0u8; 5];
    if reader.read_exact(&mut cd001).is_ok() && &cd001 == b"CD001" {
        reader.seek(SeekFrom::Start(0))?;
        return Ok(DiscFormat::Iso2048);
    }

    reader.seek(SeekFrom::Start(0))?;
    Err(AnalysisError::invalid_format(
        "Not a recognized disc format",
    ))
}

/// Check if reader content looks like a CUE sheet.
fn looks_like_cue(reader: &mut dyn retro_junk_core::ReadSeek) -> Result<bool, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut buf = [0u8; 512];
    let n = reader.read(&mut buf)?;
    reader.seek(SeekFrom::Start(0))?;

    if n == 0 {
        return Ok(false);
    }

    // CUE files are text; check for non-text bytes (ignoring common whitespace)
    let slice = &buf[..n];
    let has_binary = slice
        .iter()
        .any(|&b| b < 0x09 || (b > 0x0D && b < 0x20 && b != 0x1A));
    if has_binary {
        return Ok(false);
    }

    let text = String::from_utf8_lossy(slice).to_uppercase();
    let has_file = text.contains("FILE ");
    let has_track = text.contains("TRACK ");
    Ok(has_file && has_track)
}
