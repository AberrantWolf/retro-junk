//! Game Boy / Game Boy Color ROM analyzer.
//!
//! Supports:
//! - Game Boy ROMs (.gb)
//! - Game Boy Color ROMs (.gbc)
//! - Dual-mode ROMs (GB/GBC compatible)
//!
//! GB and GBC share the same header format at 0x0100-0x014F, differing only
//! in the CGB flag byte at 0x0143. Detection uses the 48-byte Nintendo logo
//! at 0x0104, which the boot ROM verifies on real hardware.

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;

use retro_junk_core::util::format_bytes;
use retro_junk_core::{
    AnalysisError, AnalysisOptions, ChecksumAlgorithm, ExpectedChecksum, Platform, Region,
    RomAnalyzer, RomIdentification,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum file size: header ends at 0x014F, so we need at least 0x0150 bytes.
const MIN_FILE_SIZE: u64 = 0x0150;

/// Start of the header (entry point).
const HEADER_START: u64 = 0x0100;

/// Nintendo logo (48 bytes at 0x0104). Used for format detection.
/// The boot ROM compares this against its internal copy.
const NINTENDO_LOGO: [u8; 48] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
    0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
    0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
];

// ---------------------------------------------------------------------------
// Header struct
// ---------------------------------------------------------------------------

/// Parsed Game Boy cartridge header (0x0100-0x014F).
struct GbHeader {
    title: String,
    manufacturer_code: Option<String>,
    cgb_flag: u8,
    new_licensee_code: Option<String>,
    sgb_flag: u8,
    cartridge_type: u8,
    rom_size_code: u8,
    ram_size_code: u8,
    destination_code: u8,
    old_licensee_code: u8,
    version: u8,
    header_checksum: u8,
    global_checksum: u16,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Read and parse the GB header from bytes 0x0100-0x014F.
fn parse_header(reader: &mut dyn ReadSeek) -> Result<GbHeader, AnalysisError> {
    reader.seek(SeekFrom::Start(HEADER_START))?;

    let mut buf = [0u8; 0x50]; // 0x0100..0x014F = 80 bytes
    reader.read_exact(&mut buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            AnalysisError::TooSmall {
                expected: MIN_FILE_SIZE,
                actual: 0,
            }
        } else {
            AnalysisError::Io(e)
        }
    })?;

    // CGB flag at 0x0143 (offset 0x43 from 0x0100)
    let cgb_flag = buf[0x43];

    // Title: if CGB flag is set, title is 11 bytes (0x0134-0x013E), otherwise 16 bytes (0x0134-0x0143)
    let title_bytes = if cgb_flag == 0x80 || cgb_flag == 0xC0 {
        &buf[0x34..0x3F] // 11 bytes
    } else {
        &buf[0x34..0x44] // 16 bytes (includes what would be CGB flag + manufacturer)
    };

    let title: String = title_bytes
        .iter()
        .take_while(|&&b| b != 0)
        .filter(|&&b| (0x20..0x7F).contains(&b))
        .map(|&b| b as char)
        .collect();

    // Manufacturer code (CGB only): 4 bytes at 0x013F-0x0142
    let manufacturer_code = if cgb_flag == 0x80 || cgb_flag == 0xC0 {
        let mfr: String = buf[0x3F..0x43]
            .iter()
            .filter(|&&b| (0x20..0x7F).contains(&b))
            .map(|&b| b as char)
            .collect();
        if mfr.is_empty() { None } else { Some(mfr) }
    } else {
        None
    };

    // New licensee code at 0x0144-0x0145 (ASCII)
    let old_licensee_code = buf[0x4B];
    let new_licensee_code = if old_licensee_code == 0x33 {
        let code: String = buf[0x44..0x46]
            .iter()
            .filter(|&&b| (0x20..0x7F).contains(&b))
            .map(|&b| b as char)
            .collect();
        if code.len() == 2 { Some(code) } else { None }
    } else {
        None
    };

    Ok(GbHeader {
        title,
        manufacturer_code,
        cgb_flag,
        new_licensee_code,
        sgb_flag: buf[0x46],
        cartridge_type: buf[0x47],
        rom_size_code: buf[0x48],
        ram_size_code: buf[0x49],
        destination_code: buf[0x4A],
        old_licensee_code,
        version: buf[0x4C],
        header_checksum: buf[0x4D],
        global_checksum: u16::from_be_bytes([buf[0x4E], buf[0x4F]]),
    })
}

/// Detect CGB mode from the flag byte.
fn detect_cgb_mode(flag: u8) -> Option<&'static str> {
    match flag {
        0x80 => Some("CGB Compatible"),
        0xC0 => Some("CGB Only"),
        _ => None,
    }
}

/// Look up cartridge type name from code byte at 0x0147.
fn cartridge_type_name(code: u8) -> &'static str {
    match code {
        0x00 => "ROM ONLY",
        0x01 => "MBC1",
        0x02 => "MBC1+RAM",
        0x03 => "MBC1+RAM+BATTERY",
        0x05 => "MBC2",
        0x06 => "MBC2+BATTERY",
        0x08 => "ROM+RAM",
        0x09 => "ROM+RAM+BATTERY",
        0x0B => "MMM01",
        0x0C => "MMM01+RAM",
        0x0D => "MMM01+RAM+BATTERY",
        0x0F => "MBC3+TIMER+BATTERY",
        0x10 => "MBC3+TIMER+RAM+BATTERY",
        0x11 => "MBC3",
        0x12 => "MBC3+RAM",
        0x13 => "MBC3+RAM+BATTERY",
        0x19 => "MBC5",
        0x1A => "MBC5+RAM",
        0x1B => "MBC5+RAM+BATTERY",
        0x1C => "MBC5+RUMBLE",
        0x1D => "MBC5+RUMBLE+RAM",
        0x1E => "MBC5+RUMBLE+RAM+BATTERY",
        0x20 => "MBC6",
        0x22 => "MBC7+SENSOR+RUMBLE+RAM+BATTERY",
        0xFC => "POCKET CAMERA",
        0xFD => "BANDAI TAMA5",
        0xFE => "HuC3",
        0xFF => "HuC1+RAM+BATTERY",
        _ => "Unknown",
    }
}

/// Derive ROM size in bytes from the size code at 0x0148.
/// Formula: 32 KB << code, for codes 0x00-0x08.
fn rom_size(code: u8) -> Option<u64> {
    if code <= 0x08 {
        Some(32768u64 << code)
    } else {
        None
    }
}

/// Derive RAM size in bytes from the size code at 0x0149.
fn ram_size(code: u8) -> Option<u64> {
    match code {
        0x00 => Some(0),
        0x01 => Some(0), // Listed in header but unused
        0x02 => Some(8 * 1024),
        0x03 => Some(32 * 1024),
        0x04 => Some(128 * 1024),
        0x05 => Some(64 * 1024),
        _ => None,
    }
}

/// Compute the header checksum (verified by boot ROM).
/// Sum bytes 0x0134 through 0x014C using: x = x - byte - 1 (wrapping).
fn compute_header_checksum(reader: &mut dyn ReadSeek) -> Result<u8, AnalysisError> {
    reader.seek(SeekFrom::Start(0x0134))?;
    let mut buf = [0u8; 25]; // 0x0134..=0x014C = 25 bytes
    reader.read_exact(&mut buf)?;

    let mut x: u8 = 0;
    for &byte in &buf {
        x = x.wrapping_sub(byte).wrapping_sub(1);
    }
    Ok(x)
}

/// Compute the global checksum (sum of all bytes in file except 0x014E-0x014F).
fn compute_global_checksum(reader: &mut dyn ReadSeek) -> Result<u16, AnalysisError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;

    let mut sum: u16 = 0;
    let mut buf = [0u8; 4096];
    let mut pos: u64 = 0;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for (i, &byte) in buf[..n].iter().enumerate() {
            let file_pos = pos + i as u64;
            // Skip the global checksum bytes themselves
            if file_pos == 0x014E || file_pos == 0x014F {
                continue;
            }
            sum = sum.wrapping_add(byte as u16);
        }
        pos += n as u64;
    }

    let _ = file_size; // used only to seek to end initially
    Ok(sum)
}

// ---------------------------------------------------------------------------
// Identification
// ---------------------------------------------------------------------------

/// Convert a parsed GB header into a RomIdentification.
fn to_identification(
    header: &GbHeader,
    file_size: u64,
    computed_header_checksum: u8,
    computed_global_checksum: u16,
) -> RomIdentification {
    let cgb_mode = detect_cgb_mode(header.cgb_flag);
    let is_cgb = cgb_mode.is_some();

    let platform_variant = if header.cgb_flag == 0xC0 || header.cgb_flag == 0x80 {
        Some("Game Boy Color")
    } else {
        None
    };

    let mut id = RomIdentification::new().with_platform(Platform::GameBoy);
    if let Some(variant) = platform_variant {
        id.extra.insert("platform_variant".into(), variant.into());
    }

    // Internal name
    if !header.title.is_empty() {
        id.internal_name = Some(header.title.clone());
    }

    // Version
    id.version = Some(format!("v{}", header.version));

    // Maker/licensee
    let licensee = if header.old_licensee_code == 0x33 {
        header
            .new_licensee_code
            .as_deref()
            .and_then(crate::licensee::maker_code_name)
            .map(|s| s.to_string())
    } else {
        crate::licensee::old_licensee_name(header.old_licensee_code).map(|s| s.to_string())
    };
    id.maker_code = licensee;

    // File size
    id.file_size = Some(file_size);

    // Expected size from ROM size code
    id.expected_size = rom_size(header.rom_size_code);

    // Region
    match header.destination_code {
        0x00 => id.regions.push(Region::Japan),
        _ => id.regions.push(Region::World),
    }

    // Expected checksums
    id.expected_checksums.push(
        ExpectedChecksum::new(
            ChecksumAlgorithm::PlatformSpecific("GB Header".to_string()),
            vec![header.header_checksum],
        )
        .with_description("Header checksum (0x014D)"),
    );
    id.expected_checksums.push(
        ExpectedChecksum::new(
            ChecksumAlgorithm::PlatformSpecific("GB Global".to_string()),
            header.global_checksum.to_be_bytes().to_vec(),
        )
        .with_description("Global checksum (0x014E-0x014F)"),
    );

    // Extra: format
    let format_str = match (header.cgb_flag, is_cgb) {
        (0xC0, true) => "Game Boy Color (Exclusive)",
        (0x80, true) => "Game Boy Color (Compatible)",
        _ => "Game Boy",
    };
    id.extra.insert("format".into(), format_str.into());

    // Extra: cartridge type
    id.extra.insert(
        "cartridge_type".into(),
        cartridge_type_name(header.cartridge_type).into(),
    );

    // Extra: SGB support
    if header.sgb_flag == 0x03 {
        id.extra.insert("sgb".into(), "Yes".into());
    }

    // Extra: RAM size
    if let Some(ram) = ram_size(header.ram_size_code)
        && ram > 0
    {
        id.extra.insert("ram_size".into(), format_bytes(ram));
    }

    // Extra: manufacturer code (CGB only)
    if let Some(ref mfr) = header.manufacturer_code {
        id.extra.insert("manufacturer_code".into(), mfr.clone());
    }

    // Checksum status: header
    let header_status = if computed_header_checksum == header.header_checksum {
        "OK".into()
    } else {
        format!(
            "MISMATCH (expected {:02X}, got {:02X})",
            header.header_checksum, computed_header_checksum
        )
    };
    id.extra
        .insert("checksum_status:GB Header".into(), header_status);

    // Checksum status: global
    let global_status = if computed_global_checksum == header.global_checksum {
        "OK".into()
    } else {
        format!(
            "MISMATCH (expected {:04X}, got {:04X})",
            header.global_checksum, computed_global_checksum
        )
    };
    id.extra
        .insert("checksum_status:GB Global".into(), global_status);

    id
}

// ---------------------------------------------------------------------------
// Analyzer implementation
// ---------------------------------------------------------------------------

/// Analyzer for Game Boy and Game Boy Color ROMs.
#[derive(Debug, Default)]
pub struct GameBoyAnalyzer;

impl GameBoyAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GameBoyAnalyzer {
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        if file_size < MIN_FILE_SIZE {
            return Err(AnalysisError::TooSmall {
                expected: MIN_FILE_SIZE,
                actual: file_size,
            });
        }

        let header = parse_header(reader)?;
        let computed_header = compute_header_checksum(reader)?;
        let computed_global = compute_global_checksum(reader)?;

        Ok(to_identification(
            &header,
            file_size,
            computed_header,
            computed_global,
        ))
    }

    fn platform(&self) -> Platform {
        Platform::GameBoy
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["gb", "gbc", "sgb"]
    }

    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        let file_size = match reader.seek(SeekFrom::End(0)) {
            Ok(s) => s,
            Err(_) => return false,
        };
        if reader.seek(SeekFrom::Start(0)).is_err() {
            return false;
        }
        if file_size < MIN_FILE_SIZE {
            return false;
        }

        // Read the Nintendo logo at 0x0104
        if reader.seek(SeekFrom::Start(0x0104)).is_err() {
            return false;
        }
        let mut logo = [0u8; 48];
        if reader.read_exact(&mut logo).is_err() {
            return false;
        }
        // Reset position
        let _ = reader.seek(SeekFrom::Start(0));

        logo == NINTENDO_LOGO
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Nintendo - Game Boy", "Nintendo - Game Boy Color"]
    }

    fn gdb_csv_names(&self) -> &'static [&'static str] {
        &["console_nintendo_gameboy", "console_nintendo_gameboycolor"]
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        // DMG-XXXX-YYY or CGB-XXXX-YYY → XXXX (YYYY is optional)
        let parts: Vec<&str> = serial.split('-').collect();
        if parts.len() >= 2 && (parts[0] == "DMG" || parts[0] == "CGB") {
            println!("gb/c serial: {}", serial);
            Some(parts[1].to_string())
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "tests/gameboy_tests.rs"]
mod tests;
