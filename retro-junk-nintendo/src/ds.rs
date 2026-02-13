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

use retro_junk_lib::ReadSeek;
use std::io::SeekFrom;
use std::sync::mpsc::Sender;

use retro_junk_lib::{
    AnalysisError, AnalysisOptions, AnalysisProgress, ChecksumAlgorithm, ExpectedChecksum, Region,
    RomAnalyzer, RomIdentification,
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
const NINTENDO_LOGO: [u8; 156] = [
    0x24, 0xFF, 0xAE, 0x51, 0x69, 0x9A, 0xA2, 0x21, 0x3D, 0x84, 0x82, 0x0A, 0x84, 0xE4, 0x09,
    0xAD, 0x11, 0x24, 0x8B, 0x98, 0xC0, 0x81, 0x7F, 0x21, 0xA3, 0x52, 0xBE, 0x19, 0x93, 0x09,
    0xCE, 0x20, 0x10, 0x46, 0x4A, 0x4A, 0xF8, 0x27, 0x31, 0xEC, 0x58, 0xC7, 0xE8, 0x33, 0x82,
    0xE3, 0xCE, 0xBF, 0x85, 0xF4, 0xDF, 0x94, 0xCE, 0x4B, 0x09, 0xC1, 0x94, 0x56, 0x8A, 0xC0,
    0x13, 0x72, 0xA7, 0xFC, 0x9F, 0x84, 0x4D, 0x73, 0xA3, 0xCA, 0x9A, 0x61, 0x58, 0x97, 0xA3,
    0x27, 0xFC, 0x03, 0x98, 0x76, 0x23, 0x1D, 0xC7, 0x61, 0x03, 0x04, 0xAE, 0x56, 0xBF, 0x38,
    0x84, 0x00, 0x40, 0xA7, 0x0E, 0xFD, 0xFF, 0x52, 0xFE, 0x03, 0x6F, 0x95, 0x30, 0xF1, 0x97,
    0xFB, 0xC0, 0x85, 0x60, 0xD6, 0x80, 0x25, 0xA9, 0x63, 0xBE, 0x03, 0x01, 0x4E, 0x38, 0xE2,
    0xF9, 0xA2, 0x34, 0xFF, 0xBB, 0x3E, 0x03, 0x44, 0x78, 0x00, 0x90, 0xCB, 0x88, 0x11, 0x3A,
    0x94, 0x65, 0xC0, 0x7C, 0x63, 0x87, 0xF0, 0x3C, 0xAF, 0xD6, 0x25, 0xE4, 0x8B, 0x38, 0x0A,
    0xAC, 0x72, 0x21, 0xD4, 0xF8, 0x07,
];

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
    u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]])
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
        .filter(|&&b| b >= 0x20 && b < 0x7F)
        .map(|&b| b as char)
        .collect();

    // Game code: 4 bytes at 0x00C
    let game_code: String = buf[0x00C..0x010]
        .iter()
        .filter(|&&b| b >= 0x20 && b < 0x7F)
        .map(|&b| b as char)
        .collect();

    // Maker code: 2 bytes at 0x010
    let maker_code: String = buf[0x010..0x012]
        .iter()
        .filter(|&&b| b >= 0x20 && b < 0x7F)
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

/// Look up maker/publisher name from 2-character ASCII code.
fn maker_code_name(code: &str) -> Option<&'static str> {
    match code {
        "00" => Some("None"),
        "01" => Some("Nintendo R&D1"),
        "08" => Some("Capcom"),
        "13" => Some("EA (Electronic Arts)"),
        "18" => Some("Hudson Soft"),
        "19" => Some("b-ai"),
        "20" => Some("kss"),
        "22" => Some("pow"),
        "24" => Some("PCM Complete"),
        "25" => Some("san-x"),
        "28" => Some("Kemco Japan"),
        "29" => Some("seta"),
        "30" => Some("Viacom"),
        "31" => Some("Nintendo"),
        "32" => Some("Bandai"),
        "33" => Some("Ocean/Acclaim"),
        "34" => Some("Konami"),
        "35" => Some("Hector"),
        "37" => Some("Taito"),
        "38" => Some("Hudson"),
        "39" => Some("Banpresto"),
        "41" => Some("Ubi Soft"),
        "42" => Some("Atlus"),
        "44" => Some("Malibu"),
        "46" => Some("angel"),
        "47" => Some("Bullet-Proof"),
        "49" => Some("irem"),
        "50" => Some("Absolute"),
        "51" => Some("Acclaim"),
        "52" => Some("Activision"),
        "53" => Some("American sammy"),
        "54" => Some("Konami"),
        "55" => Some("Hi tech entertainment"),
        "56" => Some("LJN"),
        "57" => Some("Matchbox"),
        "58" => Some("Mattel"),
        "59" => Some("Milton Bradley"),
        "60" => Some("Titus"),
        "61" => Some("Virgin"),
        "64" => Some("LucasArts"),
        "67" => Some("Ocean"),
        "69" => Some("EA (Electronic Arts)"),
        "70" => Some("Infogrames"),
        "71" => Some("Interplay"),
        "72" => Some("Broderbund"),
        "73" => Some("sculptured"),
        "75" => Some("sci"),
        "78" => Some("THQ"),
        "79" => Some("Accolade"),
        "80" => Some("misawa"),
        "83" => Some("lozc"),
        "86" => Some("Tokuma Shoten"),
        "87" => Some("Tsukuda Original"),
        "91" => Some("Chunsoft"),
        "92" => Some("Video system"),
        "93" => Some("Ocean/Acclaim"),
        "95" => Some("Varie"),
        "96" => Some("Yonezawa/s'pal"),
        "97" => Some("Kaneko"),
        "99" => Some("Pack in soft"),
        "A4" => Some("Konami (Yu-Gi-Oh!)"),
        _ => None,
    }
}

/// Derive region from the 4th character of the game code.
fn region_from_game_code(code: &str) -> Option<Region> {
    code.chars().nth(3).and_then(|c| match c {
        'J' => Some(Region::Japan),
        'E' => Some(Region::Usa),
        'P' | 'D' | 'F' | 'S' | 'I' | 'U' => Some(Region::Europe),
        'K' => Some(Region::Korea),
        'C' => Some(Region::China),
        'A' | 'W' => Some(Region::World),
        _ => None,
    })
}

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
    let platform_name = if header.unit_code == 0x03 {
        "Nintendo DSi"
    } else if is_dsi {
        "Nintendo DS (DSi Enhanced)"
    } else {
        "Nintendo DS"
    };

    let mut id = RomIdentification::new().with_platform(platform_name);

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
        id.maker_code = maker_code_name(&header.maker_code)
            .map(|s| s.to_string())
            .or_else(|| Some(header.maker_code.clone()));
    }

    // Region from game code
    if header.game_code.len() == 4 {
        if let Some(region) = region_from_game_code(&header.game_code) {
            id.regions.push(region);
        }
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
            id.extra
                .insert("secure_area".into(), "Decrypted".into());
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
            id.extra
                .insert("secure_area".into(), "Encrypted".into());
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

    fn analyze_with_progress(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        self.analyze(reader, options)
    }

    fn platform_name(&self) -> &'static str {
        "Nintendo DS"
    }

    fn short_name(&self) -> &'static str {
        "nds"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["nds", "ds", "nintendo ds"]
    }

    fn manufacturer(&self) -> &'static str {
        "Nintendo"
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Build a synthetic NDS ROM with a valid header and decrypted secure area.
    /// Size is 0x10000 (64 KB) to include the secure area at 0x4000–0x7FFF.
    fn make_nds_rom() -> Vec<u8> {
        let size: usize = 0x10000; // 64 KB, large enough for secure area
        let mut rom = vec![0u8; size];

        // Title at 0x000: "TESTGAME" (12 bytes, null-padded)
        rom[0x000..0x00C].copy_from_slice(b"TESTGAME\0\0\0\0");

        // Game code at 0x00C: "ADME" (A=NDS, DM=game id, E=USA)
        rom[0x00C..0x010].copy_from_slice(b"ADME");

        // Maker code at 0x010: "01" (Nintendo R&D1)
        rom[0x010..0x012].copy_from_slice(b"01");

        // Unit code at 0x012: NDS only
        rom[0x012] = 0x00;

        // Device capacity at 0x014: 0 = 128 KB
        rom[0x014] = 0x00;

        // NDS region at 0x01D: normal
        rom[0x01D] = 0x00;

        // ROM version at 0x01E
        rom[0x01E] = 0x00;

        // ARM9 ROM offset at 0x020
        rom[0x020..0x024].copy_from_slice(&0x4000u32.to_le_bytes());
        // ARM9 size at 0x02C
        rom[0x02C..0x030].copy_from_slice(&0x1000u32.to_le_bytes());

        // ARM7 ROM offset at 0x030
        rom[0x030..0x034].copy_from_slice(&0x8000u32.to_le_bytes());
        // ARM7 size at 0x03C
        rom[0x03C..0x040].copy_from_slice(&0x800u32.to_le_bytes());

        // Icon/title offset at 0x068: no banner
        rom[0x068..0x06C].copy_from_slice(&0u32.to_le_bytes());

        // Total used ROM size at 0x080
        rom[0x080..0x084].copy_from_slice(&(size as u32).to_le_bytes());

        // ROM header size at 0x084 (always 0x4000)
        rom[0x084..0x088].copy_from_slice(&0x4000u32.to_le_bytes());

        // Nintendo logo at 0xC0
        rom[0xC0..0xC0 + 156].copy_from_slice(&NINTENDO_LOGO);

        // Logo checksum at 0x15C
        let logo_crc = crc16(&rom[0xC0..0x15C]);
        rom[0x15C..0x15E].copy_from_slice(&logo_crc.to_le_bytes());

        // Decrypted secure area magic at 0x4000 (standard for all dumps)
        rom[0x4000..0x4008].copy_from_slice(&DECRYPTED_SECURE_AREA_MAGIC);

        // Secure area CRC at 0x06C — in a real ROM this is over the encrypted
        // form, but we set it to zero since decrypted dumps can't verify it
        rom[0x06C..0x06E].copy_from_slice(&0u16.to_le_bytes());

        // Header checksum at 0x15E: CRC-16 of 0x000–0x15D
        recompute_header_checksum(&mut rom);

        rom
    }

    /// Recompute header CRC-16 for a ROM buffer.
    fn recompute_header_checksum(rom: &mut [u8]) {
        let crc = crc16(&rom[0x000..0x15E]);
        rom[0x15E..0x160].copy_from_slice(&crc.to_le_bytes());
    }

    /// Set up an encrypted secure area with a valid CRC at 0x06C.
    fn setup_encrypted_secure_area(rom: &mut Vec<u8>) {
        if rom.len() >= 0x8000 {
            // Write non-magic bytes at 0x4000 so it looks encrypted
            rom[0x4000..0x4008].copy_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]);
            // Compute CRC over 0x4000–0x7FFF and store at 0x06C
            let crc = crc16(&rom[0x4000..0x8000]);
            rom[0x06C..0x06E].copy_from_slice(&crc.to_le_bytes());
            recompute_header_checksum(rom);
        }
    }

    #[test]
    fn test_crc16_known_value() {
        // The logo CRC should always be 0xCF56 for the standard Nintendo logo
        let crc = crc16(&NINTENDO_LOGO);
        assert_eq!(crc, EXPECTED_LOGO_CHECKSUM);
    }

    #[test]
    fn test_can_handle_valid() {
        let rom = make_nds_rom();
        let analyzer = DsAnalyzer::new();
        assert!(analyzer.can_handle(&mut Cursor::new(rom)));
    }

    #[test]
    fn test_can_handle_too_small() {
        let data = vec![0u8; 0x100]; // Too small
        let analyzer = DsAnalyzer::new();
        assert!(!analyzer.can_handle(&mut Cursor::new(data)));
    }

    #[test]
    fn test_can_handle_bad_logo() {
        let mut rom = make_nds_rom();
        rom[0xC0] = 0xFF; // Corrupt logo
        let analyzer = DsAnalyzer::new();
        assert!(!analyzer.can_handle(&mut Cursor::new(rom)));
    }

    #[test]
    fn test_can_handle_bad_logo_checksum() {
        let mut rom = make_nds_rom();
        rom[0x15C] = 0x00; // Wrong logo checksum
        rom[0x15D] = 0x00;
        let analyzer = DsAnalyzer::new();
        assert!(!analyzer.can_handle(&mut Cursor::new(rom)));
    }

    #[test]
    fn test_basic_analysis() {
        let rom = make_nds_rom();
        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.internal_name.as_deref(), Some("TESTGAME"));
        assert_eq!(result.platform.as_deref(), Some("Nintendo DS"));
        assert_eq!(result.serial_number.as_deref(), Some("NTR-ADME"));
        assert_eq!(result.maker_code.as_deref(), Some("Nintendo R&D1"));
        assert_eq!(result.version.as_deref(), Some("v0"));
        assert_eq!(result.file_size, Some(0x10000));
        assert_eq!(result.expected_size, Some(0x10000)); // file == used_rom_size → OK
        assert_eq!(result.regions, vec![Region::Usa]);
        assert_eq!(result.extra.get("game_code").unwrap(), "ADME");
        assert_eq!(result.extra.get("unit_code").unwrap(), "NDS");
    }

    #[test]
    fn test_checksums_ok() {
        let rom = make_nds_rom();
        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(
            result.extra.get("checksum_status:Logo CRC-16").unwrap(),
            "OK"
        );
        assert_eq!(
            result.extra.get("checksum_status:Header CRC-16").unwrap(),
            "OK"
        );
        // Default test ROM has decrypted secure area
        let sa_status = result
            .extra
            .get("checksum_status:Secure Area CRC-16")
            .unwrap();
        assert!(
            sa_status.starts_with("OK"),
            "Expected OK for decrypted dump, got: {}",
            sa_status
        );
        assert_eq!(result.extra.get("secure_area").unwrap(), "Decrypted");
    }

    #[test]
    fn test_encrypted_secure_area_crc_ok() {
        let mut rom = make_nds_rom();
        setup_encrypted_secure_area(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(
            result
                .extra
                .get("checksum_status:Secure Area CRC-16")
                .unwrap(),
            "OK"
        );
        assert_eq!(result.extra.get("secure_area").unwrap(), "Encrypted");
    }

    #[test]
    fn test_encrypted_secure_area_crc_mismatch() {
        let mut rom = make_nds_rom();
        setup_encrypted_secure_area(&mut rom);
        // Corrupt a byte in the secure area
        rom[0x5000] = 0xFF;

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        let status = result
            .extra
            .get("checksum_status:Secure Area CRC-16")
            .unwrap();
        assert!(
            status.starts_with("MISMATCH"),
            "Expected MISMATCH, got: {}",
            status
        );
    }

    #[test]
    fn test_header_checksum_mismatch() {
        let mut rom = make_nds_rom();
        // Corrupt a byte in the header region without recomputing checksum
        rom[0x000] = b'X';

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        let status = result
            .extra
            .get("checksum_status:Header CRC-16")
            .unwrap();
        assert!(
            status.starts_with("MISMATCH"),
            "Expected MISMATCH, got: {}",
            status
        );
    }

    #[test]
    fn test_quick_mode_skips_secure_area() {
        let rom = make_nds_rom();
        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions { quick: true };
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        // Quick mode doesn't detect secure area state
        assert!(result.extra.get("secure_area").is_none());
    }

    #[test]
    fn test_dsi_enhanced() {
        let mut rom = make_nds_rom();
        rom[0x012] = 0x02; // NDS+DSi
        rom[0x00C] = b'I'; // DSi-enhanced category prefix
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(
            result.platform.as_deref(),
            Some("Nintendo DS (DSi Enhanced)")
        );
        assert_eq!(result.extra.get("unit_code").unwrap(), "NDS+DSi");
        // DSi-enhanced gets TWL prefix
        assert!(result
            .serial_number
            .as_deref()
            .unwrap()
            .starts_with("TWL-"));
    }

    #[test]
    fn test_dsi_only() {
        let mut rom = make_nds_rom();
        rom[0x012] = 0x03; // DSi only
        rom[0x00C] = b'D'; // DSi-exclusive category prefix
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.platform.as_deref(), Some("Nintendo DSi"));
        assert_eq!(result.extra.get("unit_code").unwrap(), "DSi");
        assert!(result
            .serial_number
            .as_deref()
            .unwrap()
            .starts_with("TWL-"));
    }

    #[test]
    fn test_region_japan() {
        let mut rom = make_nds_rom();
        rom[0x00F] = b'J'; // Japan
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.regions, vec![Region::Japan]);
    }

    #[test]
    fn test_region_europe() {
        let mut rom = make_nds_rom();
        rom[0x00F] = b'P'; // Europe/PAL
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.regions, vec![Region::Europe]);
    }

    #[test]
    fn test_region_korea() {
        let mut rom = make_nds_rom();
        rom[0x00F] = b'K'; // Korea
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.regions, vec![Region::Korea]);
    }

    #[test]
    fn test_region_world() {
        let mut rom = make_nds_rom();
        rom[0x00F] = b'W'; // Worldwide
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.regions, vec![Region::World]);
    }

    #[test]
    fn test_region_australia() {
        let mut rom = make_nds_rom();
        rom[0x00F] = b'U'; // Australia → Europe/PAL
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.regions, vec![Region::Europe]);
    }

    #[test]
    fn test_device_capacity() {
        let mut rom = make_nds_rom();
        rom[0x014] = 9; // 64 MB chip
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        // file_size == used_rom_size and both < chip_capacity → trimmed, OK
        assert_eq!(result.expected_size, Some(0x10000));
        assert_eq!(result.extra.get("dump_status").unwrap(), "Trimmed");
        assert_eq!(
            result.extra.get("cartridge_capacity").unwrap(),
            "64 MB"
        );
    }

    #[test]
    fn test_untrimmed_rom() {
        // File size == chip capacity → untrimmed
        let capacity: usize = 128 * 1024; // 128 KB = 128 KB << 0
        let mut rom = make_nds_rom();
        rom.resize(capacity, 0xFF); // pad to full capacity
        rom[0x014] = 0; // device capacity = 128 KB
        // used_rom_size stays at 0x10000 (64 KB), which is < capacity
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.file_size, Some(capacity as u64));
        assert_eq!(result.expected_size, Some(capacity as u64)); // OK, not oversized
        assert_eq!(result.extra.get("dump_status").unwrap(), "Untrimmed");
    }

    #[test]
    fn test_trimmed_rom() {
        // File size == used_rom_size < chip capacity → trimmed
        let mut rom = make_nds_rom();
        rom[0x014] = 9; // 64 MB chip, much larger than 64 KB file
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.file_size, Some(0x10000));
        assert_eq!(result.expected_size, Some(0x10000)); // OK, not truncated
        assert_eq!(result.extra.get("dump_status").unwrap(), "Trimmed");
    }

    #[test]
    fn test_partially_trimmed_rom() {
        // used_rom_size < file_size < chip capacity → partially trimmed
        let mut rom = make_nds_rom();
        rom[0x014] = 1; // 256 KB chip
        rom[0x080..0x084].copy_from_slice(&0x8000u32.to_le_bytes()); // used = 32 KB
        rom.resize(0xC000, 0xFF); // file = 48 KB (between 32 KB and 256 KB)
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.file_size, Some(0xC000));
        assert_eq!(result.expected_size, Some(0xC000)); // OK
        assert_eq!(
            result.extra.get("dump_status").unwrap(),
            "Partially trimmed"
        );
    }

    #[test]
    fn test_actually_truncated_rom() {
        // file_size < used_rom_size → truly truncated
        let mut rom = make_nds_rom();
        rom[0x080..0x084].copy_from_slice(&0x20000u32.to_le_bytes()); // used = 128 KB
        recompute_header_checksum(&mut rom);
        // File is 64 KB but claims to need 128 KB

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions { quick: true };
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.file_size, Some(0x10000));
        assert_eq!(result.expected_size, Some(0x20000)); // shows TRUNCATED
    }

    #[test]
    fn test_rom_version() {
        let mut rom = make_nds_rom();
        rom[0x01E] = 2;
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.version.as_deref(), Some("v2"));
    }

    #[test]
    fn test_title_trimming() {
        let mut rom = make_nds_rom();
        rom[0x000..0x00C].copy_from_slice(b"HI\0\0\0\0\0\0\0\0\0\0");
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.internal_name.as_deref(), Some("HI"));
    }

    #[test]
    fn test_nds_region_korea() {
        let mut rom = make_nds_rom();
        rom[0x01D] = 0x40; // Korea region lock
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.extra.get("nds_region_lock").unwrap(), "Korea");
    }

    #[test]
    fn test_nds_region_china() {
        let mut rom = make_nds_rom();
        rom[0x01D] = 0x80; // China region lock
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.extra.get("nds_region_lock").unwrap(), "China");
    }

    #[test]
    fn test_too_small_file() {
        let data = vec![0u8; 0x100]; // Not enough for header
        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(data), &options);
        assert!(result.is_err());
    }

    #[test]
    fn test_expected_rom_size_calculation() {
        assert_eq!(expected_rom_size_from_capacity(0), 128 * 1024); // 128 KB
        assert_eq!(expected_rom_size_from_capacity(6), 8 * 1024 * 1024); // 8 MB
        assert_eq!(expected_rom_size_from_capacity(7), 16 * 1024 * 1024); // 16 MB
        assert_eq!(expected_rom_size_from_capacity(8), 32 * 1024 * 1024); // 32 MB
        assert_eq!(expected_rom_size_from_capacity(9), 64 * 1024 * 1024); // 64 MB
        assert_eq!(expected_rom_size_from_capacity(10), 128 * 1024 * 1024); // 128 MB
        assert_eq!(expected_rom_size_from_capacity(11), 256 * 1024 * 1024); // 256 MB
        assert_eq!(expected_rom_size_from_capacity(12), 512 * 1024 * 1024); // 512 MB
    }

    #[test]
    fn test_region_from_game_code_function() {
        assert_eq!(region_from_game_code("ADMJ"), Some(Region::Japan));
        assert_eq!(region_from_game_code("ADME"), Some(Region::Usa));
        assert_eq!(region_from_game_code("ADMP"), Some(Region::Europe));
        assert_eq!(region_from_game_code("ADMK"), Some(Region::Korea));
        assert_eq!(region_from_game_code("ADMC"), Some(Region::China));
        assert_eq!(region_from_game_code("ADMU"), Some(Region::Europe)); // Australia → Europe
        assert_eq!(region_from_game_code("ADMA"), Some(Region::World)); // Region-free
        assert_eq!(region_from_game_code("ADMW"), Some(Region::World)); // Worldwide
        assert_eq!(region_from_game_code("ADM"), None); // Too short
    }

    #[test]
    fn test_maker_code_lookup() {
        assert_eq!(maker_code_name("01"), Some("Nintendo R&D1"));
        assert_eq!(maker_code_name("08"), Some("Capcom"));
        assert_eq!(maker_code_name("34"), Some("Konami"));
        assert_eq!(maker_code_name("ZZ"), None);
    }

    #[test]
    fn test_banner_offset_reported() {
        let mut rom = make_nds_rom();
        rom[0x068..0x06C].copy_from_slice(&0x8000u32.to_le_bytes());
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(result.extra.get("banner_offset").unwrap(), "0x00008000");
    }

    #[test]
    fn test_serial_number_format_nds() {
        let rom = make_nds_rom();
        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert!(result
            .serial_number
            .as_deref()
            .unwrap()
            .starts_with("NTR-"));
    }

    #[test]
    fn test_no_secure_area_for_small_file() {
        // File smaller than 0x8000 shouldn't attempt secure area detection
        let mut rom = make_nds_rom();
        rom[0x080..0x084].copy_from_slice(&0x2000u32.to_le_bytes()); // used = 8 KB
        recompute_header_checksum(&mut rom);
        rom.truncate(0x2000); // 8 KB, too small for secure area

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert!(result.extra.get("secure_area").is_none());
    }

    #[test]
    fn test_homebrew_no_secure_area() {
        let mut rom = make_nds_rom();
        // Set arm9_rom_offset < 0x4000 → homebrew, no secure area
        rom[0x020..0x024].copy_from_slice(&0x0200u32.to_le_bytes());
        recompute_header_checksum(&mut rom);

        let analyzer = DsAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
        assert_eq!(
            result.extra.get("secure_area").unwrap(),
            "None (homebrew)"
        );
    }
}
