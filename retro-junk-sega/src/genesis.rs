//! Sega Genesis / Mega Drive ROM analyzer.
//!
//! Supports:
//! - Genesis/Mega Drive ROMs (.md, .gen, .bin)
//! - Interleaved ROMs (.smd)

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, ChecksumAlgorithm, ExpectedChecksum,
    Platform, Region, RomAnalyzer, RomIdentification,
};

/// Magic bytes at offset 0x0100 — the system type field always starts with "SEGA".
const SEGA_MAGIC: &[u8; 4] = b"SEGA";

/// Offset of the Genesis ROM header (after 68000 interrupt vectors).
const HEADER_OFFSET: u64 = 0x100;

/// Minimum file size to contain a full header (0x0200 bytes).
const MIN_HEADER_SIZE: u64 = 0x200;

/// Parsed Genesis ROM header (0x0100–0x01FF).
#[derive(Debug, Clone)]
pub struct GenesisHeader {
    /// System type string (e.g. "SEGA MEGA DRIVE", "SEGA GENESIS").
    pub system_type: String,
    /// Copyright / release date (e.g. "(C)SEGA 1991.APR").
    pub copyright: String,
    /// Domestic (Japanese) title.
    pub domestic_title: String,
    /// Overseas (international) title.
    pub overseas_title: String,
    /// Serial number (e.g. "GM 00001009-00").
    pub serial_number: String,
    /// ROM checksum (big-endian u16 at 0x018E).
    pub checksum: u16,
    /// Device support codes.
    pub device_support: String,
    /// ROM start address.
    pub rom_start: u32,
    /// ROM end address (inclusive).
    pub rom_end: u32,
    /// RAM start address.
    pub ram_start: u32,
    /// RAM end address.
    pub ram_end: u32,
    /// Extra memory / SRAM info field.
    pub extra_memory: String,
    /// Region support codes (e.g. "JUE").
    pub region_codes: String,
}

/// Read a fixed-size ASCII string from a buffer slice, trimming trailing spaces and nulls.
fn read_ascii(buf: &[u8]) -> String {
    let s: String = buf
        .iter()
        .map(|&b| {
            if b >= 0x20 && b < 0x7F {
                b as char
            } else {
                ' '
            }
        })
        .collect();
    s.trim().to_string()
}

/// Parse the Genesis header from a 256-byte buffer (offsets 0x0100–0x01FF).
fn parse_header(buf: &[u8; 256]) -> GenesisHeader {
    let system_type = read_ascii(&buf[0x00..0x10]);
    let copyright = read_ascii(&buf[0x10..0x20]);
    let domestic_title = read_ascii(&buf[0x20..0x50]);
    let overseas_title = read_ascii(&buf[0x50..0x80]);
    let serial_number = read_ascii(&buf[0x80..0x8E]);
    let checksum = u16::from_be_bytes([buf[0x8E], buf[0x8F]]);
    let device_support = read_ascii(&buf[0x90..0xA0]);
    let rom_start = u32::from_be_bytes([buf[0xA0], buf[0xA1], buf[0xA2], buf[0xA3]]);
    let rom_end = u32::from_be_bytes([buf[0xA4], buf[0xA5], buf[0xA6], buf[0xA7]]);
    let ram_start = u32::from_be_bytes([buf[0xA8], buf[0xA9], buf[0xAA], buf[0xAB]]);
    let ram_end = u32::from_be_bytes([buf[0xAC], buf[0xAD], buf[0xAE], buf[0xAF]]);
    let extra_memory = read_ascii(&buf[0xB0..0xBC]);
    let region_codes = read_ascii(&buf[0xF0..0xF3]);

    GenesisHeader {
        system_type,
        copyright,
        domestic_title,
        overseas_title,
        serial_number,
        checksum,
        device_support,
        rom_start,
        rom_end,
        ram_start,
        ram_end,
        extra_memory,
        region_codes,
    }
}

/// Decode region codes from the header's region field.
fn decode_regions(region_codes: &str) -> Vec<Region> {
    let mut regions = Vec::new();
    for c in region_codes.chars() {
        match c.to_ascii_uppercase() {
            'J' => regions.push(Region::Japan),
            'U' => regions.push(Region::Usa),
            'E' => regions.push(Region::Europe),
            'A' => {
                // 'A' in Genesis context means Asia (not Australia)
                // We'll map it to Japan as the closest match
                regions.push(Region::Japan);
            }
            _ => {}
        }
    }
    if regions.is_empty() {
        regions.push(Region::Unknown);
    }
    regions
}

/// Compute the additive checksum over ROM data from 0x0200 to `rom_end` (inclusive).
/// Returns the lower 16 bits of the sum of all big-endian u16 words.
///
/// The Genesis checksum only covers data up to the ROM end address declared in the
/// header — any padding beyond that (common in dumped ROMs) is excluded.
fn compute_checksum(reader: &mut dyn ReadSeek, rom_end: u32) -> Result<u16, AnalysisError> {
    let checksum_start = 0x200u64;
    let checksum_end = rom_end as u64 + 1; // exclusive end
    if checksum_end <= checksum_start {
        return Ok(0);
    }
    let len = (checksum_end - checksum_start) as usize;

    reader.seek(SeekFrom::Start(checksum_start))?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;

    let mut sum: u16 = 0;
    // Process pairs of bytes as big-endian u16
    let mut i = 0;
    while i + 1 < buf.len() {
        let word = u16::from_be_bytes([buf[i], buf[i + 1]]);
        sum = sum.wrapping_add(word);
        i += 2;
    }
    // If there's an odd trailing byte, treat it as the high byte of a u16
    if i < buf.len() {
        let word = (buf[i] as u16) << 8;
        sum = sum.wrapping_add(word);
    }

    Ok(sum)
}

/// Analyzer for Sega Genesis / Mega Drive ROMs.
#[derive(Debug, Default)]
pub struct GenesisAnalyzer;

impl GenesisAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GenesisAnalyzer {
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        // Get file size
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        if file_size < MIN_HEADER_SIZE {
            return Err(AnalysisError::TooSmall {
                expected: MIN_HEADER_SIZE,
                actual: file_size,
            });
        }

        // Read header
        reader.seek(SeekFrom::Start(HEADER_OFFSET))?;
        let mut header_buf = [0u8; 256];
        reader.read_exact(&mut header_buf).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                AnalysisError::TooSmall {
                    expected: MIN_HEADER_SIZE,
                    actual: file_size,
                }
            } else {
                AnalysisError::Io(e)
            }
        })?;

        // Verify magic
        if &header_buf[0..4] != SEGA_MAGIC {
            return Err(AnalysisError::invalid_format(
                "Not a Sega Genesis ROM (missing SEGA magic at 0x0100)",
            ));
        }

        let header = parse_header(&header_buf);

        // Build identification
        let mut id = RomIdentification::new().with_platform("Sega Genesis / Mega Drive");
        id.file_size = Some(file_size);

        if !header.serial_number.is_empty() {
            id = id.with_serial(&header.serial_number);
        }
        if !header.domestic_title.is_empty() {
            id = id.with_internal_name(&header.domestic_title);
        }

        // Regions
        id.regions = decode_regions(&header.region_codes);

        // Expected size from ROM end address (inclusive, so +1).
        // Genesis dumps are commonly padded to the next power of 2, so a file
        // larger than rom_end+1 is normal. We only flag truncated files.
        let declared_size = if header.rom_end > 0 {
            header.rom_end as u64 + 1
        } else {
            0
        };
        if declared_size > 0 {
            // Use the file size itself as expected when the file is at least as
            // large as the declared ROM — this avoids false "oversized" reports
            // from power-of-2 padding.  If the file is truncated, report the
            // declared size so the mismatch is visible.
            if file_size >= declared_size {
                id.expected_size = Some(file_size);
            } else {
                id.expected_size = Some(declared_size);
            }
        }

        // Store the header checksum as an expected checksum
        id.expected_checksums.push(
            ExpectedChecksum::new(
                ChecksumAlgorithm::Additive,
                header.checksum.to_be_bytes().to_vec(),
            )
            .with_description("ROM checksum (0x0200 to ROM end)"),
        );

        // Verify checksum — only covers 0x0200..=rom_end per the Genesis spec
        let computed = compute_checksum(reader, header.rom_end)?;
        let checksum_valid = computed == header.checksum;
        id.extra.insert(
            "checksum_status:rom".into(),
            if checksum_valid {
                "Valid".into()
            } else {
                format!(
                    "Invalid (expected 0x{:04X}, computed 0x{:04X})",
                    header.checksum, computed
                )
            },
        );

        // Extra fields
        id.extra
            .insert("system_type".into(), header.system_type.clone());
        if !header.copyright.is_empty() {
            id.extra
                .insert("copyright".into(), header.copyright.clone());
        }
        if !header.overseas_title.is_empty() {
            id.extra
                .insert("overseas_title".into(), header.overseas_title.clone());
        }
        if !header.device_support.is_empty() {
            id.extra
                .insert("device_support".into(), header.device_support.clone());
        }
        id.extra.insert(
            "rom_address_range".into(),
            format!("0x{:08X}-0x{:08X}", header.rom_start, header.rom_end),
        );
        id.extra.insert(
            "ram_address_range".into(),
            format!("0x{:08X}-0x{:08X}", header.ram_start, header.ram_end),
        );
        if !header.region_codes.is_empty() {
            id.extra
                .insert("region_codes".into(), header.region_codes.clone());
        }
        if !header.extra_memory.is_empty() {
            id.extra
                .insert("extra_memory".into(), header.extra_memory.clone());
        }

        Ok(id)
    }

    fn analyze_with_progress(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        self.analyze(reader, options)
    }

    fn platform(&self) -> Platform {
        Platform::Genesis
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["md", "gen", "bin", "smd"]
    }

    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        let result = (|| -> Result<bool, std::io::Error> {
            reader.seek(SeekFrom::Start(HEADER_OFFSET))?;
            let mut magic = [0u8; 4];
            reader.read_exact(&mut magic)?;
            reader.seek(SeekFrom::Start(0))?;
            Ok(&magic == SEGA_MAGIC)
        })();
        // Always rewind on failure too
        let _ = reader.seek(SeekFrom::Start(0));
        result.unwrap_or(false)
    }

    fn dat_name(&self) -> Option<&'static str> {
        Some("Sega - Mega Drive - Genesis")
    }
}

#[cfg(test)]
#[path = "tests/genesis_tests.rs"]
mod tests;
