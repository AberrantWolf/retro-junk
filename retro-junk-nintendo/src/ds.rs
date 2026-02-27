//! Nintendo DS ROM analyzer.
//!
//! Supports:
//! - DS ROMs (.nds)
//! - DSi-enhanced ROMs (.dsi)
//! - DSiWare
//!
//! The NDS cartridge header occupies bytes 0x000–0x1FF (512 bytes). Detection
//! uses the 156-byte Nintendo logo at 0xC0 (identical to GBA) and the logo
//! checksum 0xCF56 at 0x15C. The header CRC-16 covers bytes 0x000–0x15D.

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, ChecksumAlgorithm, ExpectedChecksum, Platform, RomAnalyzer,
    RomIdentification,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum file size: full header is 512 bytes (0x200).
const MIN_FILE_SIZE: u64 = 0x200;

/// Maximum NDS ROM size: 512 MB.
const MAX_ROM_SIZE: u64 = 512 * 1024 * 1024;

/// Expected logo checksum value at 0x15C.
const EXPECTED_LOGO_CHECKSUM: u16 = 0xCF56;

/// Nintendo compressed logo bitmap (156 bytes at offset 0xC0).
/// This is identical to the GBA Nintendo logo.
use crate::constants::{NINTENDO_LOGO_156 as NINTENDO_LOGO, region_from_game_code};

// ---------------------------------------------------------------------------
// CRC-16 (polynomial 0x8005, reflected, init 0xFFFF)
// ---------------------------------------------------------------------------

/// Compute CRC-16 used by the NDS header (polynomial 0x8005, reflected, init 0xFFFF).
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= byte as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xA001; // 0xA001 is reflected 0x8005
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

// ---------------------------------------------------------------------------
// Header struct
// ---------------------------------------------------------------------------

/// Parsed NDS cartridge header (0x000–0x1FF).
struct NdsHeader {
    title: String,
    game_code: String,
    maker_code: String,
    unit_code: u8,
    device_capacity: u8,
    nds_region: u8,
    rom_version: u8,
    arm9_rom_offset: u32,
    arm9_size: u32,
    arm7_rom_offset: u32,
    arm7_size: u32,
    icon_title_offset: u32,
    secure_area_checksum: u16,
    total_used_rom_size: u32,
    logo_checksum: u16,
    header_checksum: u16,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Read a little-endian u16 from a byte slice.
fn read_u16_le(buf: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([buf[offset], buf[offset + 1]])
}

/// Read a little-endian u32 from a byte slice.
fn read_u32_le(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

/// Read and parse the NDS header from the first 512 bytes.
fn parse_header(reader: &mut dyn ReadSeek) -> Result<NdsHeader, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    let mut buf = [0u8; 0x200];
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

    // Title: 12 bytes at 0x000, null-trimmed ASCII
    let title: String = buf[0x000..0x00C]
        .iter()
        .take_while(|&&b| b != 0)
        .filter(|&&b| (0x20..0x7F).contains(&b))
        .map(|&b| b as char)
        .collect();

    // Game code: 4 bytes at 0x00C
    let game_code: String = buf[0x00C..0x010]
        .iter()
        .filter(|&&b| (0x20..0x7F).contains(&b))
        .map(|&b| b as char)
        .collect();

    // Maker code: 2 bytes at 0x010
    let maker_code: String = buf[0x010..0x012]
        .iter()
        .filter(|&&b| (0x20..0x7F).contains(&b))
        .map(|&b| b as char)
        .collect();

    Ok(NdsHeader {
        title,
        game_code,
        maker_code,
        unit_code: buf[0x012],
        device_capacity: buf[0x014],
        nds_region: buf[0x01D],
        rom_version: buf[0x01E],
        arm9_rom_offset: read_u32_le(&buf, 0x020),
        arm9_size: read_u32_le(&buf, 0x02C),
        arm7_rom_offset: read_u32_le(&buf, 0x030),
        arm7_size: read_u32_le(&buf, 0x03C),
        icon_title_offset: read_u32_le(&buf, 0x068),
        secure_area_checksum: read_u16_le(&buf, 0x06C),
        total_used_rom_size: read_u32_le(&buf, 0x080),
        logo_checksum: read_u16_le(&buf, 0x15C),
        header_checksum: read_u16_le(&buf, 0x15E),
    })
}

/// Compute the header CRC-16 over bytes 0x000–0x15D.
fn compute_header_checksum(reader: &mut dyn ReadSeek) -> Result<u16, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut buf = [0u8; 0x15E]; // 0x000..=0x15D = 350 bytes
    reader.read_exact(&mut buf)?;
    Ok(crc16(&buf))
}

/// Secure area state detected from magic bytes at 0x4000.
enum SecureAreaState {
    /// Decrypted dump: first 8 bytes at 0x4000 are E7FFDEFF repeated.
    /// CRC cannot be verified without re-encryption (requires BIOS keys).
    Decrypted,
    /// Encrypted (original cartridge form). CRC can be verified.
    Encrypted { computed_crc: u16 },
    /// No secure area (homebrew: arm9_rom_offset < 0x4000).
    Homebrew,
    /// Not checked (quick mode or file too small).
    Skipped,
}

/// Magic bytes at 0x4000 that indicate a decrypted secure area dump.
/// The BIOS overwrites the "encryObj" ID with 0xE7FFDEFF (an undefined ARM
/// instruction) repeated twice. Stored little-endian in the file.
const DECRYPTED_SECURE_AREA_MAGIC: [u8; 8] = [0xFF, 0xDE, 0xFF, 0xE7, 0xFF, 0xDE, 0xFF, 0xE7];

/// Detect the secure area state and optionally compute its CRC-16.
/// The secure area is the 16 KB block at 0x4000–0x7FFF. The stored CRC at
/// 0x06C is over the *encrypted* form, so it can only be verified if the
/// dump still has the original encrypted data (rare — most dumps are decrypted).
fn detect_secure_area(
    reader: &mut dyn ReadSeek,
    arm9_rom_offset: u32,
) -> Result<SecureAreaState, AnalysisError> {
    // Homebrew ROMs have arm9_rom_offset < 0x4000 → no secure area
    if arm9_rom_offset < 0x4000 {
        return Ok(SecureAreaState::Homebrew);
    }

    // Read the first 8 bytes at 0x4000 to detect encryption state
    reader.seek(SeekFrom::Start(0x4000))?;
    let mut magic = [0u8; 8];
    reader.read_exact(&mut magic)?;

    if magic == DECRYPTED_SECURE_AREA_MAGIC {
        return Ok(SecureAreaState::Decrypted);
    }

    // Encrypted: compute CRC over the full 16 KB secure area (0x4000–0x7FFF)
    reader.seek(SeekFrom::Start(0x4000))?;
    let mut buf = vec![0u8; 0x4000]; // 16 KB
    reader.read_exact(&mut buf)?;
    Ok(SecureAreaState::Encrypted {
        computed_crc: crc16(&buf),
    })
}

// ---------------------------------------------------------------------------
// Lookup functions
// ---------------------------------------------------------------------------

/// Derive region from the 4th character of the game code.
/// Map the unit code byte to a human-readable string.
fn unit_code_name(unit_code: u8) -> &'static str {
    match unit_code {
        0x00 => "NDS",
        0x02 => "NDS+DSi",
        0x03 => "DSi",
        _ => "Unknown",
    }
}

/// Calculate expected ROM size from device capacity byte.
/// Formula: 128 KB << n
fn expected_rom_size_from_capacity(device_capacity: u8) -> u64 {
    131_072u64 << (device_capacity as u64)
}

// ---------------------------------------------------------------------------
// Identification
// ---------------------------------------------------------------------------

/// Convert a parsed NDS header into a RomIdentification.
fn to_identification(
    header: &NdsHeader,
    file_size: u64,
    computed_header_checksum: u16,
    secure_area: SecureAreaState,
) -> RomIdentification {
    let is_dsi = header.unit_code & 0x02 != 0;
    let platform_variant = if header.unit_code == 0x03 {
        Some("Nintendo DSi")
    } else if is_dsi {
        Some("Nintendo DS (DSi Enhanced)")
    } else {
        None
    };

    let mut id = RomIdentification::new().with_platform(Platform::Ds);
    if let Some(variant) = platform_variant {
        id.extra.insert("platform_variant".into(), variant.into());
    }

    // Internal name
    if !header.title.is_empty() {
        id.internal_name = Some(header.title.clone());
    }

    // Serial number: NTR-XXXX for NDS, TWL-XXXX for DSi
    if header.game_code.len() == 4 {
        let prefix = if is_dsi { "TWL" } else { "NTR" };
        id.serial_number = Some(format!("{}-{}", prefix, header.game_code));
    }

    // Maker code
    if header.maker_code.len() == 2 {
        id.maker_code = crate::licensee::maker_code_name(&header.maker_code)
            .map(|s| s.to_string())
            .or_else(|| Some(header.maker_code.clone()));
    }

    // Region from game code
    if header.game_code.len() == 4
        && let Some(region) = region_from_game_code(&header.game_code)
    {
        id.regions.push(region);
    }

    // Version
    id.version = Some(format!("v{}", header.rom_version));

    // File and expected size
    //
    // NDS ROMs are very commonly "trimmed": the unused 0xFF padding between
    // `total_used_rom_size` and the full cartridge chip capacity is stripped.
    // Both trimmed and untrimmed (full capacity) dumps are valid. Only files
    // smaller than total_used_rom_size are truly truncated.
    id.file_size = Some(file_size);
    let used_size = header.total_used_rom_size as u64;
    let chip_capacity = if header.device_capacity <= 20 {
        let cap = expected_rom_size_from_capacity(header.device_capacity);
        if cap <= MAX_ROM_SIZE { Some(cap) } else { None }
    } else {
        None
    };

    if used_size > 0 {
        if file_size >= used_size && chip_capacity.is_some_and(|cap| file_size <= cap) {
            // File is between used_size and chip_capacity — perfectly valid
            // (trimmed, partially trimmed, or full dump). Set expected = actual
            // so the CLI size verdict shows OK.
            id.expected_size = Some(file_size);

            if file_size == used_size {
                id.extra.insert("dump_status".into(), "Trimmed".into());
            } else if chip_capacity.is_some_and(|cap| file_size == cap) {
                id.extra.insert("dump_status".into(), "Untrimmed".into());
            } else {
                id.extra
                    .insert("dump_status".into(), "Partially trimmed".into());
            }
        } else if file_size > used_size && chip_capacity.is_none() {
            // No valid chip capacity to compare; file has all the data it needs
            id.expected_size = Some(file_size);
        } else {
            // file_size < used_size → actually truncated
            // file_size > chip_capacity → oversized (shouldn't happen normally)
            id.expected_size = Some(used_size);
        }
    } else if let Some(cap) = chip_capacity {
        // Fallback: use device capacity if used ROM size is missing
        id.expected_size = Some(cap);
    }

    // Report device (cartridge chip) capacity for informational purposes
    if let Some(cap) = chip_capacity {
        let label = if cap >= 1024 * 1024 {
            format!("{} MB", cap / (1024 * 1024))
        } else {
            format!("{} KB", cap / 1024)
        };
        id.extra.insert("cartridge_capacity".into(), label);
    }

    // Unit code
    id.extra
        .insert("unit_code".into(), unit_code_name(header.unit_code).into());

    // Game code
    if !header.game_code.is_empty() {
        id.extra
            .insert("game_code".into(), header.game_code.clone());
    }

    // ARM9/ARM7 info
    id.extra.insert(
        "arm9_offset".into(),
        format!("0x{:08X}", header.arm9_rom_offset),
    );
    id.extra.insert(
        "arm9_size".into(),
        format!("0x{:X} ({} KB)", header.arm9_size, header.arm9_size / 1024),
    );
    id.extra.insert(
        "arm7_offset".into(),
        format!("0x{:08X}", header.arm7_rom_offset),
    );
    id.extra.insert(
        "arm7_size".into(),
        format!("0x{:X} ({} KB)", header.arm7_size, header.arm7_size / 1024),
    );

    // Total used ROM size
    if header.total_used_rom_size > 0 {
        id.extra.insert(
            "used_rom_size".into(),
            format!(
                "0x{:X} ({} KB)",
                header.total_used_rom_size,
                header.total_used_rom_size / 1024
            ),
        );
    }

    // Icon/title banner
    if header.icon_title_offset > 0 {
        id.extra.insert(
            "banner_offset".into(),
            format!("0x{:08X}", header.icon_title_offset),
        );
    }

    // -- Checksums --

    // Logo checksum
    let logo_status = if header.logo_checksum == EXPECTED_LOGO_CHECKSUM {
        "OK".into()
    } else {
        format!(
            "MISMATCH (expected {:04X}, got {:04X})",
            EXPECTED_LOGO_CHECKSUM, header.logo_checksum
        )
    };
    id.expected_checksums.push(
        ExpectedChecksum::new(
            ChecksumAlgorithm::Crc16,
            header.logo_checksum.to_le_bytes().to_vec(),
        )
        .with_description("Logo CRC-16 (0x15C)"),
    );
    id.extra
        .insert("checksum_status:Logo CRC-16".into(), logo_status);

    // Header checksum
    let header_status = if computed_header_checksum == header.header_checksum {
        "OK".into()
    } else {
        format!(
            "MISMATCH (expected {:04X}, got {:04X})",
            header.header_checksum, computed_header_checksum
        )
    };
    id.expected_checksums.push(
        ExpectedChecksum::new(
            ChecksumAlgorithm::Crc16,
            header.header_checksum.to_le_bytes().to_vec(),
        )
        .with_description("Header CRC-16 (0x15E)"),
    );
    id.extra
        .insert("checksum_status:Header CRC-16".into(), header_status);

    // Secure area checksum
    match &secure_area {
        SecureAreaState::Decrypted => {
            // Decrypted dumps are standard. The stored CRC is over the encrypted
            // form and cannot be verified without BIOS Blowfish keys.
            id.extra.insert(
                "checksum_status:Secure Area CRC-16".into(),
                "OK (decrypted dump, CRC is over encrypted form)".into(),
            );
            id.extra.insert("secure_area".into(), "Decrypted".into());
        }
        SecureAreaState::Encrypted { computed_crc } => {
            let secure_status = if *computed_crc == header.secure_area_checksum {
                "OK".into()
            } else {
                format!(
                    "MISMATCH (expected {:04X}, got {:04X})",
                    header.secure_area_checksum, computed_crc
                )
            };
            id.expected_checksums.push(
                ExpectedChecksum::new(
                    ChecksumAlgorithm::Crc16,
                    header.secure_area_checksum.to_le_bytes().to_vec(),
                )
                .with_description("Secure Area CRC-16 (0x06C)"),
            );
            id.extra
                .insert("checksum_status:Secure Area CRC-16".into(), secure_status);
            id.extra.insert("secure_area".into(), "Encrypted".into());
        }
        SecureAreaState::Homebrew => {
            id.extra
                .insert("secure_area".into(), "None (homebrew)".into());
        }
        SecureAreaState::Skipped => {
            // Quick mode or file too small — don't report
        }
    }

    // NDS region byte
    let nds_region_str = match header.nds_region {
        0x00 => "Normal",
        0x40 => "Korea",
        0x80 => "China",
        _ => "Unknown",
    };
    if header.nds_region != 0x00 {
        id.extra
            .insert("nds_region_lock".into(), nds_region_str.into());
    }

    id
}

// ---------------------------------------------------------------------------
// Analyzer implementation
// ---------------------------------------------------------------------------

/// Analyzer for Nintendo DS ROMs.
#[derive(Debug, Default)]
pub struct DsAnalyzer;

impl DsAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for DsAnalyzer {
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
        let computed_header_checksum = compute_header_checksum(reader)?;

        // Secure area detection: skip in quick mode (requires reading 16 KB)
        let secure_area = if options.quick {
            SecureAreaState::Skipped
        } else if file_size >= 0x8000 {
            detect_secure_area(reader, header.arm9_rom_offset)?
        } else {
            SecureAreaState::Skipped
        };

        Ok(to_identification(
            &header,
            file_size,
            computed_header_checksum,
            secure_area,
        ))
    }

    fn platform(&self) -> Platform {
        Platform::Ds
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["nds", "dsi"]
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

        // Check Nintendo logo at 0xC0
        if reader.seek(SeekFrom::Start(0xC0)).is_err() {
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

        // Check logo checksum at 0x15C
        if reader.seek(SeekFrom::Start(0x15C)).is_err() {
            return false;
        }
        let mut checksum_buf = [0u8; 2];
        if reader.read_exact(&mut checksum_buf).is_err() {
            return false;
        }
        let logo_checksum = u16::from_le_bytes(checksum_buf);
        let _ = reader.seek(SeekFrom::Start(0));

        logo_checksum == EXPECTED_LOGO_CHECKSUM
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &[
            "Nintendo - Nintendo DS",
            "Nintendo - Nintendo DS (Download Play)",
            "Nintendo - Nintendo DSi",
        ]
    }

    fn expects_serial(&self) -> bool {
        true
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        // NTR-XXXX or TWL-XXXX → XXXX
        let parts: Vec<&str> = serial.split('-').collect();
        if parts.len() >= 2 && (parts[0] == "NTR" || parts[0] == "TWL") {
            Some(parts[1].to_string())
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "tests/ds_tests.rs"]
mod tests;
