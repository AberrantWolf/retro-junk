//! Game Boy Advance ROM analyzer.
//!
//! Supports:
//! - GBA ROMs (.gba)
//! - Multiboot ROMs (.mb)
//!
//! The GBA cartridge header occupies bytes 0x00–0xBF (192 bytes). Detection
//! uses the 156-byte compressed Nintendo logo at 0x04 and the fixed value
//! 0x96 at 0xB2. The complement checksum covers bytes 0xA0–0xBC.

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, ChecksumAlgorithm, ExpectedChecksum, Platform, RomAnalyzer,
    RomIdentification,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum file size: full header is 192 bytes (0x00–0xBF).
const MIN_FILE_SIZE: u64 = 0xC0;

/// Maximum GBA ROM size: 32 MB.
const MAX_ROM_SIZE: u64 = 32 * 1024 * 1024;

/// Fixed value that must appear at offset 0xB2.
const FIXED_VALUE: u8 = 0x96;

/// Nintendo compressed logo bitmap (156 bytes at offset 0x04).
use crate::constants::{NINTENDO_LOGO_156 as NINTENDO_LOGO, region_from_game_code};

// ---------------------------------------------------------------------------
// Header struct
// ---------------------------------------------------------------------------

/// Parsed GBA cartridge header (0x00–0xBF).
struct GbaHeader {
    title: String,
    game_code: String,
    maker_code: String,
    fixed_value: u8,
    #[allow(dead_code)]
    main_unit_code: u8,
    device_type: u8,
    software_version: u8,
    header_checksum: u8,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Read and parse the GBA header from the first 192 bytes.
fn parse_header(reader: &mut dyn ReadSeek) -> Result<GbaHeader, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    let mut buf = [0u8; 0xC0];
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

    // Title: 12 bytes at 0xA0, null-trimmed ASCII
    let title: String = buf[0xA0..0xAC]
        .iter()
        .take_while(|&&b| b != 0)
        .filter(|&&b| (0x20..0x7F).contains(&b))
        .map(|&b| b as char)
        .collect();

    // Game code: 4 bytes at 0xAC
    let game_code: String = buf[0xAC..0xB0]
        .iter()
        .filter(|&&b| (0x20..0x7F).contains(&b))
        .map(|&b| b as char)
        .collect();

    // Maker code: 2 bytes at 0xB0
    let maker_code: String = buf[0xB0..0xB2]
        .iter()
        .filter(|&&b| (0x20..0x7F).contains(&b))
        .map(|&b| b as char)
        .collect();

    Ok(GbaHeader {
        title,
        game_code,
        maker_code,
        fixed_value: buf[0xB2],
        main_unit_code: buf[0xB3],
        device_type: buf[0xB4],
        software_version: buf[0xBC],
        header_checksum: buf[0xBD],
    })
}

/// Compute the GBA header complement checksum.
/// Sum bytes 0xA0–0xBC, then negate and subtract 0x19.
fn compute_header_checksum(reader: &mut dyn ReadSeek) -> Result<u8, AnalysisError> {
    reader.seek(SeekFrom::Start(0xA0))?;
    let mut buf = [0u8; 29]; // 0xA0..=0xBC = 29 bytes
    reader.read_exact(&mut buf)?;

    let mut sum: u8 = 0;
    for &byte in &buf {
        sum = sum.wrapping_add(byte);
    }
    Ok((-(sum as i8)).wrapping_sub(0x19) as u8)
}

// ---------------------------------------------------------------------------
// Lookup functions
// ---------------------------------------------------------------------------

/// Derive region from the 4th character of the game code.
/// Round file size up to the nearest power of 2, capped at 32 MB.
/// GBA has no ROM size field in the header so we infer from file size.
fn expected_rom_size(file_size: u64) -> Option<u64> {
    if file_size == 0 {
        return None;
    }
    let mut size = 1u64;
    while size < file_size && size < MAX_ROM_SIZE {
        size <<= 1;
    }
    Some(size.min(MAX_ROM_SIZE))
}

/// Scan ROM data for save type magic strings.
/// Returns the detected save type, or None.
fn detect_save_type(reader: &mut dyn ReadSeek) -> Result<Option<&'static str>, AnalysisError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;

    // Read the entire ROM into memory for scanning
    let read_size = file_size.min(MAX_ROM_SIZE) as usize;
    let mut data = vec![0u8; read_size];
    reader.read_exact(&mut data)?;

    let patterns: &[(&[u8], &str)] = &[
        (b"EEPROM_V", "EEPROM"),
        (b"SRAM_V", "SRAM"),
        (b"FLASH_V", "Flash"),
        (b"FLASH512_V", "Flash 512K"),
        (b"FLASH1M_V", "Flash 1M"),
    ];

    // Check more specific patterns first (Flash1M/Flash512 before Flash)
    for &(pattern, name) in patterns.iter().rev() {
        if data.windows(pattern.len()).any(|w| w == pattern) {
            return Ok(Some(name));
        }
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Identification
// ---------------------------------------------------------------------------

/// Convert a parsed GBA header into a RomIdentification.
fn to_identification(
    header: &GbaHeader,
    file_size: u64,
    computed_checksum: u8,
    save_type: Option<&str>,
) -> RomIdentification {
    let mut id = RomIdentification::new().with_platform(Platform::Gba);

    // Internal name
    if !header.title.is_empty() {
        id.internal_name = Some(header.title.clone());
    }

    // Serial number: AGB-XXXX format
    if header.game_code.len() == 4 {
        id.serial_number = Some(format!("AGB-{}", header.game_code));
    }

    // Maker code
    let maker = if header.maker_code.len() == 2 {
        crate::licensee::maker_code_name(&header.maker_code)
            .map(|s| s.to_string())
            .or_else(|| Some(header.maker_code.clone()))
    } else {
        None
    };
    id.maker_code = maker;

    // Region from game code
    if header.game_code.len() == 4
        && let Some(region) = region_from_game_code(&header.game_code)
    {
        id.regions.push(region);
    }

    // Version
    id.version = Some(format!("v{}", header.software_version));

    // File and expected size
    id.file_size = Some(file_size);
    id.expected_size = expected_rom_size(file_size);

    // Expected checksums
    id.expected_checksums.push(
        ExpectedChecksum::new(
            ChecksumAlgorithm::PlatformSpecific("GBA Complement".to_string()),
            vec![header.header_checksum],
        )
        .with_description("Header complement check (0xBD)"),
    );

    // Checksum status
    let checksum_status = if computed_checksum == header.header_checksum {
        "OK".into()
    } else {
        format!(
            "MISMATCH (expected {:02X}, got {:02X})",
            header.header_checksum, computed_checksum
        )
    };
    id.extra
        .insert("checksum_status:GBA Complement".into(), checksum_status);

    // Fixed value validation
    if header.fixed_value != FIXED_VALUE {
        id.extra.insert(
            "fixed_value".into(),
            format!(
                "INVALID (expected {:02X}, got {:02X})",
                FIXED_VALUE, header.fixed_value
            ),
        );
    }

    // Device type
    if header.device_type != 0 {
        id.extra
            .insert("device_type".into(), format!("{:02X}", header.device_type));
    }

    // Save type
    if let Some(save) = save_type {
        id.extra.insert("save_type".into(), save.into());
    }

    // Raw game code
    if !header.game_code.is_empty() {
        id.extra
            .insert("game_code".into(), header.game_code.clone());
    }

    id
}

// ---------------------------------------------------------------------------
// Analyzer implementation
// ---------------------------------------------------------------------------

/// Analyzer for Game Boy Advance ROMs.
#[derive(Debug, Default)]
pub struct GbaAnalyzer;

impl GbaAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for GbaAnalyzer {
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
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
        let computed_checksum = compute_header_checksum(reader)?;

        let save_type = if options.quick {
            None
        } else {
            detect_save_type(reader)?
        };

        Ok(to_identification(
            &header,
            file_size,
            computed_checksum,
            save_type,
        ))
    }

    fn platform(&self) -> Platform {
        Platform::Gba
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["gba", "mb"]
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

        // Check Nintendo logo at 0x04
        if reader.seek(SeekFrom::Start(0x04)).is_err() {
            return false;
        }
        let mut logo = [0u8; 156];
        if reader.read_exact(&mut logo).is_err() {
            return false;
        }
        if logo != NINTENDO_LOGO {
            let _ = reader.seek(SeekFrom::Start(0));
            return false;
        }

        // Check fixed value at 0xB2
        if reader.seek(SeekFrom::Start(0xB2)).is_err() {
            return false;
        }
        let mut fixed = [0u8; 1];
        if reader.read_exact(&mut fixed).is_err() {
            return false;
        }
        let _ = reader.seek(SeekFrom::Start(0));

        fixed[0] == FIXED_VALUE
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Nintendo - Game Boy Advance"]
    }

    fn expects_serial(&self) -> bool {
        true
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        // AGB-XXXX-YYY → XXXX
        let parts: Vec<&str> = serial.split('-').collect();
        if parts.len() >= 2 && parts[0] == "AGB" {
            Some(parts[1].to_string())
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "tests/gba_tests.rs"]
mod tests;
