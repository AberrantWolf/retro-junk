//! Sega Genesis / Mega Drive ROM analyzer.
//!
//! Supports:
//! - Genesis/Mega Drive ROMs (.md, .gen, .bin)
//! - Interleaved ROMs (.smd)

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, ChecksumAlgorithm, ExpectedChecksum, Region,
    RomAnalyzer, RomIdentification,
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
    let s: String = buf.iter().map(|&b| {
        if b >= 0x20 && b < 0x7F { b as char } else { ' ' }
    }).collect();
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

    fn platform_name(&self) -> &'static str {
        "Sega Genesis / Mega Drive"
    }

    fn short_name(&self) -> &'static str {
        "genesis"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["genesis", "megadrive", "mega drive", "md"]
    }

    fn manufacturer(&self) -> &'static str {
        "Sega"
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
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Build a minimal valid Genesis ROM with the given header fields.
    fn make_genesis_rom(
        system_type: &str,
        domestic_title: &str,
        overseas_title: &str,
        serial: &str,
        region_codes: &str,
    ) -> Vec<u8> {
        // Total ROM: 0x0200 header area + 0x0200 data = 0x0400 bytes
        let mut rom = vec![0u8; 0x0400];

        // 68000 vectors: initial SP at 0x00, initial PC at 0x04
        // (doesn't matter for analysis, but let's set something)
        rom[0x00..0x04].copy_from_slice(&0x00FF_FFFEu32.to_be_bytes()); // SP
        rom[0x04..0x08].copy_from_slice(&0x0000_0200u32.to_be_bytes()); // PC

        // Write header fields (padded to their field sizes with spaces)
        write_field(&mut rom, 0x100, 16, system_type);
        write_field(&mut rom, 0x110, 16, "(C)SEGA 1991.JAN");
        write_field(&mut rom, 0x120, 48, domestic_title);
        write_field(&mut rom, 0x150, 48, overseas_title);
        write_field(&mut rom, 0x180, 14, serial);
        // Device support
        write_field(&mut rom, 0x190, 16, "J");
        // ROM start/end addresses
        rom[0x1A0..0x1A4].copy_from_slice(&0x0000_0000u32.to_be_bytes());
        rom[0x1A4..0x1A8].copy_from_slice(&0x0000_03FFu32.to_be_bytes()); // ROM end = 0x3FF
        // RAM start/end
        rom[0x1A8..0x1AC].copy_from_slice(&0x00FF_0000u32.to_be_bytes());
        rom[0x1AC..0x1B0].copy_from_slice(&0x00FF_FFFFu32.to_be_bytes());
        // Region codes at 0x1F0
        write_field(&mut rom, 0x1F0, 3, region_codes);

        // Fill ROM data area (0x0200..0x0400) with some data
        for i in 0x200..0x400 {
            rom[i] = (i & 0xFF) as u8;
        }

        // Compute and write checksum
        let mut sum: u16 = 0;
        let mut i = 0x200;
        while i + 1 < rom.len() {
            let word = u16::from_be_bytes([rom[i], rom[i + 1]]);
            sum = sum.wrapping_add(word);
            i += 2;
        }
        rom[0x18E..0x190].copy_from_slice(&sum.to_be_bytes());

        rom
    }

    /// Write a string into a fixed-size field, padding with spaces.
    fn write_field(rom: &mut [u8], offset: usize, size: usize, value: &str) {
        let bytes = value.as_bytes();
        let len = bytes.len().min(size);
        rom[offset..offset + len].copy_from_slice(&bytes[..len]);
        for b in &mut rom[offset + len..offset + size] {
            *b = b' ';
        }
    }

    #[test]
    fn test_can_handle_valid() {
        let rom = make_genesis_rom("SEGA MEGA DRIVE", "TEST GAME", "TEST GAME", "GM 00001009-00", "JUE");
        let analyzer = GenesisAnalyzer::new();
        assert!(analyzer.can_handle(&mut Cursor::new(rom)));
    }

    #[test]
    fn test_can_handle_invalid() {
        let data = vec![0xFFu8; 0x200];
        let analyzer = GenesisAnalyzer::new();
        assert!(!analyzer.can_handle(&mut Cursor::new(data)));
    }

    #[test]
    fn test_can_handle_too_small() {
        let data = vec![0u8; 16];
        let analyzer = GenesisAnalyzer::new();
        assert!(!analyzer.can_handle(&mut Cursor::new(data)));
    }

    #[test]
    fn test_header_fields() {
        let rom = make_genesis_rom(
            "SEGA MEGA DRIVE",
            "SONIC THE HEDGEHOG",
            "SONIC THE HEDGEHOG",
            "GM 00001009-00",
            "JUE",
        );
        let analyzer = GenesisAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.internal_name.as_deref(), Some("SONIC THE HEDGEHOG"));
        assert_eq!(result.serial_number.as_deref(), Some("GM 00001009-00"));
        assert_eq!(result.extra.get("system_type").unwrap(), "SEGA MEGA DRIVE");
        assert_eq!(result.extra.get("overseas_title").unwrap(), "SONIC THE HEDGEHOG");
        assert_eq!(result.platform.as_deref(), Some("Sega Genesis / Mega Drive"));
    }

    #[test]
    fn test_region_decode_multi() {
        let rom = make_genesis_rom("SEGA GENESIS", "TEST", "TEST", "GM 00000000-00", "JUE");
        let analyzer = GenesisAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert!(result.regions.contains(&Region::Japan));
        assert!(result.regions.contains(&Region::Usa));
        assert!(result.regions.contains(&Region::Europe));
        assert_eq!(result.regions.len(), 3);
    }

    #[test]
    fn test_region_decode_single() {
        let rom = make_genesis_rom("SEGA GENESIS", "TEST", "TEST", "GM 00000000-00", "U");
        let analyzer = GenesisAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.regions, vec![Region::Usa]);
    }

    #[test]
    fn test_checksum_valid() {
        let rom = make_genesis_rom("SEGA MEGA DRIVE", "TEST", "TEST", "GM 00000000-00", "J");
        let analyzer = GenesisAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.extra.get("checksum_status:rom").unwrap(), "Valid");
    }

    #[test]
    fn test_checksum_invalid() {
        let mut rom = make_genesis_rom("SEGA MEGA DRIVE", "TEST", "TEST", "GM 00000000-00", "J");
        // Corrupt the stored checksum
        rom[0x18E] = 0xFF;
        rom[0x18F] = 0xFF;

        let analyzer = GenesisAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        let status = result.extra.get("checksum_status:rom").unwrap();
        assert!(status.starts_with("Invalid"), "expected Invalid, got: {status}");
    }

    #[test]
    fn test_expected_size_exact() {
        let rom = make_genesis_rom("SEGA MEGA DRIVE", "TEST", "TEST", "GM 00000000-00", "J");
        let analyzer = GenesisAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        // ROM end = 0x3FF, file is exactly 0x400 — expected should match file
        assert_eq!(result.expected_size, Some(0x0400));
        assert_eq!(result.file_size, Some(0x0400));
    }

    #[test]
    fn test_padded_rom_not_oversized() {
        let mut rom = make_genesis_rom("SEGA MEGA DRIVE", "TEST", "TEST", "GM 00000000-00", "J");
        // Pad to a larger power-of-2 size (simulates a real dump)
        rom.resize(0x80000, 0x00); // 512 KB
        let analyzer = GenesisAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        // Padded ROM: expected_size should equal file_size (no false oversized report)
        assert_eq!(result.file_size, Some(0x80000));
        assert_eq!(result.expected_size, result.file_size);
        // Checksum should still be valid (only covers 0x0200..=0x03FF)
        assert_eq!(result.extra.get("checksum_status:rom").unwrap(), "Valid");
    }

    #[test]
    fn test_too_small_rom() {
        let data = vec![0u8; 0x100]; // Too small for header
        let analyzer = GenesisAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(data), &options);
        assert!(result.is_err());
    }

    #[test]
    fn test_address_ranges() {
        let rom = make_genesis_rom("SEGA MEGA DRIVE", "TEST", "TEST", "GM 00000000-00", "J");
        let analyzer = GenesisAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(
            result.extra.get("rom_address_range").unwrap(),
            "0x00000000-0x000003FF"
        );
        assert_eq!(
            result.extra.get("ram_address_range").unwrap(),
            "0x00FF0000-0x00FFFFFF"
        );
    }
}
