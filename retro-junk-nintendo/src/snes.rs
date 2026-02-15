//! SNES (Super Famicom) ROM analyzer.
//!
//! Supports:
//! - Headered ROMs (.smc, .swc) with 512-byte copier header
//! - Headerless ROMs (.sfc)
//! - LoROM, HiROM, ExHiROM, SA-1, and S-DD1 mappings
//!
//! SNES ROMs have no magic bytes. Detection uses a heuristic scoring system
//! that evaluates candidate header locations and picks the best match.

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, ChecksumAlgorithm, ExpectedChecksum, Region,
    RomAnalyzer, RomIdentification,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Size of the optional copier header (SMC/SWC/fig).
const COPIER_HEADER_SIZE: u64 = 512;

/// Minimum file size: smallest valid LoROM is 32 KB.
const MIN_FILE_SIZE: u64 = 0x8000;

/// Header base offsets within the ROM data (relative to start of ROM, after
/// any copier header has been stripped).
const LOROM_HEADER_BASE: u64 = 0x7FB0;
const HIROM_HEADER_BASE: u64 = 0xFFB0;
const EXHIROM_HEADER_BASE: u64 = 0x40FFB0;

/// Field offsets within the 48-byte header region (base + offset).
const OFF_TITLE: usize = 0x10; // 21 bytes
const OFF_MAP_MODE: usize = 0x25;
const OFF_ROM_TYPE: usize = 0x26;
const OFF_ROM_SIZE: usize = 0x27;
const OFF_RAM_SIZE: usize = 0x28;
const OFF_COUNTRY: usize = 0x29;
const OFF_DEVELOPER_ID: usize = 0x2A;
const OFF_VERSION: usize = 0x2B;
const OFF_COMPLEMENT: usize = 0x2C; // 2 bytes, little-endian
const OFF_CHECKSUM: usize = 0x2E;   // 2 bytes, little-endian

/// Extended header fields (at base + 0x00..0x0F, valid when developer_id == 0x33).
const OFF_EXT_MAKER_CODE: usize = 0x00; // 2 bytes ASCII
const OFF_EXT_GAME_CODE: usize = 0x02;  // 4 bytes ASCII
const OFF_EXT_EXPANSION_RAM: usize = 0x0D;
const OFF_EXT_SPECIAL_VERSION: usize = 0x0E;
const OFF_EXT_CARTRIDGE_SUBTYPE: usize = 0x0F;

/// Minimum heuristic score to accept a header candidate. A single matching
/// field (score 1) is not sufficient -- we require at least two independent
/// indicators to avoid false positives on random data.
const MIN_SCORE_THRESHOLD: i32 = 2;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// SNES memory mapping mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnesMapping {
    LoRom,
    HiRom,
    SA1,
    SDD1,
    ExHiRom,
    Unknown(u8),
}

impl SnesMapping {
    pub fn name(&self) -> &'static str {
        match self {
            Self::LoRom => "LoROM",
            Self::HiRom => "HiROM",
            Self::SA1 => "SA-1",
            Self::SDD1 => "S-DD1",
            Self::ExHiRom => "ExHiROM",
            Self::Unknown(_) => "Unknown",
        }
    }

    /// Decode the map mode byte into a mapping variant.
    fn from_byte(byte: u8) -> Self {
        match byte & 0x2F {
            0x20 => Self::LoRom,
            0x21 => Self::HiRom,
            0x23 => Self::SA1,
            0x25 => Self::ExHiRom,
            _ => {
                // S-DD1 uses map mode 0x2A in some dumps
                if byte & 0x0F == 0x0A {
                    Self::SDD1
                } else {
                    Self::Unknown(byte)
                }
            }
        }
    }
}

/// SNES CPU clock speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnesSpeed {
    Slow,
    Fast,
}

impl SnesSpeed {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Slow => "SlowROM (2.68 MHz)",
            Self::Fast => "FastROM (3.58 MHz)",
        }
    }

    fn from_byte(byte: u8) -> Self {
        if byte & 0x10 != 0 {
            Self::Fast
        } else {
            Self::Slow
        }
    }
}

// ---------------------------------------------------------------------------
// SnesHeader
// ---------------------------------------------------------------------------

/// Parsed SNES ROM header.
#[derive(Debug, Clone)]
pub struct SnesHeader {
    /// Internal title (up to 21 characters).
    pub title: String,
    /// Memory mapping mode.
    pub mapping: SnesMapping,
    /// CPU clock speed.
    pub speed: SnesSpeed,
    /// ROM type / chipset byte.
    pub rom_type: u8,
    /// ROM size in bytes (decoded from header).
    pub rom_size: u64,
    /// SRAM size in bytes (0 if no SRAM).
    pub ram_size: u64,
    /// Country/destination code.
    pub country: u8,
    /// Developer/licensee ID.
    pub developer_id: u8,
    /// Mask ROM version number.
    pub version: u8,
    /// Stored checksum (little-endian u16).
    pub checksum: u16,
    /// Stored checksum complement (little-endian u16).
    pub checksum_complement: u16,
    /// Whether a 512-byte copier header was detected.
    pub has_copier_header: bool,
    /// Offset of the header within the file (including copier header if present).
    pub header_offset: u64,

    // Extended header fields (valid when developer_id == 0x33)
    /// 2-character maker code (extended header).
    pub maker_code: Option<String>,
    /// 4-character game code (extended header).
    pub game_code: Option<String>,
    /// Expansion RAM size in bytes (extended header).
    pub expansion_ram_size: Option<u64>,
    /// Special version byte (extended header).
    pub special_version: Option<u8>,
    /// Cartridge sub-type byte (extended header).
    pub cartridge_subtype: Option<u8>,
}

// ---------------------------------------------------------------------------
// Detection (heuristic scoring)
// ---------------------------------------------------------------------------

/// Returns true if the file size suggests a 512-byte copier header is present.
fn detect_copier_header(file_size: u64) -> bool {
    file_size % 1024 == 512
}

/// Score a candidate header location. Higher score = more likely to be the real header.
fn score_header_at(reader: &mut dyn ReadSeek, offset: u64) -> i32 {
    let mut buf = [0u8; 0x30]; // 48 bytes covers the full header region
    if reader.seek(SeekFrom::Start(offset)).is_err() {
        return -100;
    }
    if reader.read_exact(&mut buf).is_err() {
        return -100;
    }

    let mut score: i32 = 0;

    // Checksum + complement should equal 0xFFFF
    let complement = u16::from_le_bytes([buf[OFF_COMPLEMENT], buf[OFF_COMPLEMENT + 1]]);
    let checksum = u16::from_le_bytes([buf[OFF_CHECKSUM], buf[OFF_CHECKSUM + 1]]);
    if checksum.wrapping_add(complement) == 0xFFFF {
        score += 4;
    }

    // Title characters should be printable ASCII or null padding
    let title_bytes = &buf[OFF_TITLE..OFF_TITLE + 21];
    let valid_title_chars = title_bytes
        .iter()
        .all(|&b| b == 0x00 || (0x20..=0x7E).contains(&b));
    if valid_title_chars {
        score += 2;
    }

    // ROM size code should be in reasonable range (8 KB to 8 MB)
    let rom_size_code = buf[OFF_ROM_SIZE];
    if (0x07..=0x0D).contains(&rom_size_code) {
        score += 2;
    }

    // Map mode bits should match expected mapping for this offset's location
    let map_mode = buf[OFF_MAP_MODE];
    let mapping = SnesMapping::from_byte(map_mode);
    // LoROM candidates should declare LoROM/SA1, HiROM candidates should declare HiROM, etc.
    let rom_offset_no_copier = offset & !0x1FF; // strip copier influence
    let is_lorom_offset = (rom_offset_no_copier & 0xFFFF) == LOROM_HEADER_BASE
        || (rom_offset_no_copier == LOROM_HEADER_BASE + COPIER_HEADER_SIZE);
    let is_hirom_offset = (rom_offset_no_copier & 0xFFFF) == (HIROM_HEADER_BASE & 0xFFFF)
        || (rom_offset_no_copier == HIROM_HEADER_BASE + COPIER_HEADER_SIZE);

    match mapping {
        SnesMapping::LoRom | SnesMapping::SA1 if is_lorom_offset => score += 3,
        SnesMapping::HiRom | SnesMapping::SDD1 if is_hirom_offset => score += 3,
        SnesMapping::ExHiRom if offset > 0x400000 => score += 3,
        _ => {}
    }

    // Country code in valid range
    if buf[OFF_COUNTRY] <= 0x14 {
        score += 1;
    }

    // Developer ID is non-zero (licensed game)
    if buf[OFF_DEVELOPER_ID] != 0x00 {
        score += 1;
    }

    // RAM size code in reasonable range
    if buf[OFF_RAM_SIZE] <= 0x07 {
        score += 1;
    }

    // ROM type is a recognized chipset
    if is_known_chipset(buf[OFF_ROM_TYPE]) {
        score += 1;
    }

    score
}

/// Returns true if the ROM type byte corresponds to a recognized chipset.
fn is_known_chipset(rom_type: u8) -> bool {
    matches!(
        rom_type,
        0x00 | 0x01 | 0x02 | 0x03 | 0x04 | 0x05 | 0x06 |
        0x13 | 0x14 | 0x15 | 0x16 |
        0x23 | 0x25 | 0x26 |
        0x33 | 0x34 | 0x35 | 0x36 |
        0x43 | 0x45 |
        0x55 |
        0xE3 | 0xE5 |
        0xF3 | 0xF5 | 0xF6 | 0xF9
    )
}

/// Detect the mapping mode by trying all candidate header locations and scoring them.
/// Returns `(file_offset_of_header_base, has_copier_header)`.
fn detect_mapping(
    reader: &mut dyn ReadSeek,
    file_size: u64,
) -> Result<(u64, bool), AnalysisError> {
    let has_copier = detect_copier_header(file_size);
    let copier_offset = if has_copier { COPIER_HEADER_SIZE } else { 0 };
    let rom_size = file_size - copier_offset;

    let mut candidates: Vec<(u64, i32)> = Vec::new();

    // Always try LoROM and HiROM
    if rom_size > LOROM_HEADER_BASE + 0x30 {
        let offset = copier_offset + LOROM_HEADER_BASE;
        let s = score_header_at(reader, offset);
        candidates.push((offset, s));
    }

    if rom_size > HIROM_HEADER_BASE + 0x30 {
        let offset = copier_offset + HIROM_HEADER_BASE;
        let s = score_header_at(reader, offset);
        candidates.push((offset, s));
    }

    // Try ExHiROM only for large files (> 4 MB)
    if rom_size > 0x400000 && rom_size > EXHIROM_HEADER_BASE + 0x30 {
        let offset = copier_offset + EXHIROM_HEADER_BASE;
        let s = score_header_at(reader, offset);
        candidates.push((offset, s));
    }

    // Pick the highest-scoring candidate
    candidates.sort_by(|a, b| b.1.cmp(&a.1));

    if let Some(&(offset, score)) = candidates.first() {
        if score >= MIN_SCORE_THRESHOLD {
            return Ok((offset, has_copier));
        }
    }

    Err(AnalysisError::invalid_format(
        "No valid SNES header found at any candidate location",
    ))
}

// ---------------------------------------------------------------------------
// Header parsing
// ---------------------------------------------------------------------------

/// Parse the SNES header from the given file offset.
fn parse_header(
    reader: &mut dyn ReadSeek,
    offset: u64,
    has_copier: bool,
) -> Result<SnesHeader, AnalysisError> {
    let mut buf = [0u8; 0x30]; // 48 bytes: extended header (0x00-0x0F) + main header (0x10-0x2F)
    reader.seek(SeekFrom::Start(offset))?;
    reader.read_exact(&mut buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            AnalysisError::corrupted_header("SNES header truncated")
        } else {
            AnalysisError::Io(e)
        }
    })?;

    // Title: 21 bytes at offset 0x10, trim trailing spaces and nulls
    let title_bytes = &buf[OFF_TITLE..OFF_TITLE + 21];
    let title: String = title_bytes
        .iter()
        .map(|&b| {
            if (0x20..=0x7E).contains(&b) {
                b as char
            } else {
                ' '
            }
        })
        .collect::<String>()
        .trim()
        .to_string();

    let map_mode = buf[OFF_MAP_MODE];
    let mapping = SnesMapping::from_byte(map_mode);
    let speed = SnesSpeed::from_byte(map_mode);

    let rom_type = buf[OFF_ROM_TYPE];

    // ROM size: (1 << code) KB
    let rom_size_code = buf[OFF_ROM_SIZE];
    let rom_size = if rom_size_code > 0 && rom_size_code <= 0x0D {
        (1u64 << rom_size_code as u64) * 1024
    } else {
        0
    };

    // RAM size: same encoding, 0 means no SRAM
    let ram_size_code = buf[OFF_RAM_SIZE];
    let ram_size = if ram_size_code > 0 && ram_size_code <= 0x08 {
        (1u64 << ram_size_code as u64) * 1024
    } else {
        0
    };

    let country = buf[OFF_COUNTRY];
    let developer_id = buf[OFF_DEVELOPER_ID];
    let version = buf[OFF_VERSION];

    let checksum_complement = u16::from_le_bytes([buf[OFF_COMPLEMENT], buf[OFF_COMPLEMENT + 1]]);
    let checksum = u16::from_le_bytes([buf[OFF_CHECKSUM], buf[OFF_CHECKSUM + 1]]);

    // Extended header: only valid when developer_id == 0x33
    let (maker_code, game_code, expansion_ram_size, special_version, cartridge_subtype) =
        if developer_id == 0x33 {
            let maker = String::from_utf8_lossy(&buf[OFF_EXT_MAKER_CODE..OFF_EXT_MAKER_CODE + 2])
                .trim()
                .to_string();
            let game = String::from_utf8_lossy(&buf[OFF_EXT_GAME_CODE..OFF_EXT_GAME_CODE + 4])
                .trim()
                .to_string();
            let exp_ram_code = buf[OFF_EXT_EXPANSION_RAM];
            let exp_ram = if exp_ram_code > 0 {
                Some((1u64 << exp_ram_code as u64) * 1024)
            } else {
                None
            };
            (
                if maker.is_empty() { None } else { Some(maker) },
                if game.is_empty() { None } else { Some(game) },
                exp_ram,
                Some(buf[OFF_EXT_SPECIAL_VERSION]),
                Some(buf[OFF_EXT_CARTRIDGE_SUBTYPE]),
            )
        } else {
            (None, None, None, None, None)
        };

    Ok(SnesHeader {
        title,
        mapping,
        speed,
        rom_type,
        rom_size,
        ram_size,
        country,
        developer_id,
        version,
        checksum,
        checksum_complement,
        has_copier_header: has_copier,
        header_offset: offset,
        maker_code,
        game_code,
        expansion_ram_size,
        special_version,
        cartridge_subtype,
    })
}

// ---------------------------------------------------------------------------
// Checksum computation
// ---------------------------------------------------------------------------

/// Compute the SNES internal checksum: wrapping 16-bit sum of all ROM bytes
/// (excluding any copier header).
///
/// For non-power-of-2 ROM sizes, the remainder after the largest power-of-2
/// block is mirrored (repeated) to fill the gap up to the next power of 2.
fn compute_snes_checksum(
    reader: &mut dyn ReadSeek,
    has_copier: bool,
) -> Result<u16, AnalysisError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    let copier_offset = if has_copier { COPIER_HEADER_SIZE } else { 0 };
    let rom_size = file_size - copier_offset;

    if rom_size == 0 {
        return Err(AnalysisError::invalid_format("ROM has zero size"));
    }

    // Read entire ROM data (after copier header)
    reader.seek(SeekFrom::Start(copier_offset))?;
    let mut rom_data = vec![0u8; rom_size as usize];
    reader.read_exact(&mut rom_data)?;

    // Sum ALL bytes as-is (don't zero the checksum fields). The SNES checksum
    // is defined as the 16-bit wrapping sum of every ROM byte. This works
    // because checksum + complement = 0xFFFF, so those 4 bytes always
    // contribute exactly 0x01FE to the sum regardless of their values —
    // the sum is invariant after writing the final checksum/complement.

    // Find the largest power of 2 <= rom_size
    let mut power = 1u64;
    while power * 2 <= rom_size {
        power *= 2;
    }

    let mut sum: u16 = 0;

    if power == rom_size {
        // Power-of-2 size: simple sum
        for &byte in &rom_data {
            sum = sum.wrapping_add(byte as u16);
        }
    } else {
        // Non-power-of-2: sum the base block, then mirror the remainder
        let base = &rom_data[..power as usize];
        let remainder = &rom_data[power as usize..];
        let remainder_len = remainder.len();

        // Sum the base block
        for &byte in base {
            sum = sum.wrapping_add(byte as u16);
        }

        // Mirror the remainder to fill (power - remainder_len) bytes
        // i.e., repeat the remainder enough times to equal `power` total bytes
        // for the second "half"
        let mirror_total = power as usize;
        for i in 0..mirror_total {
            sum = sum.wrapping_add(remainder[i % remainder_len] as u16);
        }
    }

    Ok(sum)
}

// ---------------------------------------------------------------------------
// Lookup tables
// ---------------------------------------------------------------------------

/// Map country code to human-readable name.
fn country_name(code: u8) -> &'static str {
    match code {
        0x00 => "Japan",
        0x01 => "USA",
        0x02 => "Europe",
        0x03 => "Sweden/Scandinavia",
        0x04 => "Finland",
        0x05 => "Denmark",
        0x06 => "France",
        0x07 => "Netherlands",
        0x08 => "Spain",
        0x09 => "Germany",
        0x0A => "Italy",
        0x0B => "China",
        0x0C => "Indonesia",
        0x0D => "South Korea",
        0x0E => "International",
        0x0F => "Canada",
        0x10 => "Brazil",
        0x11 => "Australia",
        _ => "Unknown",
    }
}

/// Map country code to Region enum.
fn country_to_region(code: u8) -> Region {
    match code {
        0x00 => Region::Japan,
        0x01 => Region::Usa,
        0x02..=0x0A => Region::Europe,
        0x0B => Region::China,
        0x0D => Region::Korea,
        0x0E => Region::World,
        0x0F => Region::Usa, // Canada
        0x10 => Region::Brazil,
        0x11 => Region::Australia,
        _ => Region::Unknown,
    }
}

/// Decode the ROM type byte into a chipset description.
fn chipset_name(rom_type: u8) -> &'static str {
    match rom_type {
        0x00 => "ROM only",
        0x01 => "ROM + RAM",
        0x02 => "ROM + RAM + Battery",
        0x03 => "ROM + DSP",
        0x04 => "ROM + DSP + RAM",
        0x05 => "ROM + DSP + RAM + Battery",
        0x06 => "ROM + FX (SuperFX)",
        0x13 => "ROM + SuperFX",
        0x14 => "ROM + SuperFX + RAM",
        0x15 => "ROM + SuperFX + RAM + Battery",
        0x16 => "ROM + SuperFX + Battery",
        0x23 => "ROM + OBC1",
        0x25 => "ROM + OBC1 + RAM + Battery",
        0x26 => "ROM + OBC1 + RAM",
        0x33 => "ROM + SA-1",
        0x34 => "ROM + SA-1 + RAM",
        0x35 => "ROM + SA-1 + RAM + Battery",
        0x36 => "ROM + SA-1 + Battery",
        0x43 => "ROM + S-DD1",
        0x45 => "ROM + S-DD1 + RAM + Battery",
        0x55 => "ROM + S-RTC + RAM + Battery",
        0xE3 => "ROM + Other (Game Boy data)",
        0xE5 => "ROM + Other + RAM + Battery",
        0xF3 => "ROM + Custom chip",
        0xF5 => "ROM + Custom chip + RAM + Battery",
        0xF6 => "ROM + Custom chip + Battery",
        0xF9 => "ROM + SPC7110 + RAM + Battery",
        _ => "Unknown chipset",
    }
}

/// Extract coprocessor info from the ROM type byte (high nibble).
fn coprocessor_name(rom_type: u8) -> Option<&'static str> {
    match rom_type >> 4 {
        0x0 => {
            // Check specific low nibble for DSP
            if rom_type >= 0x03 && rom_type <= 0x05 {
                Some("DSP")
            } else {
                None
            }
        }
        0x1 => Some("SuperFX"),
        0x2 => Some("OBC1"),
        0x3 => Some("SA-1"),
        0x4 => Some("S-DD1"),
        0x5 => Some("S-RTC"),
        0xE => Some("Other"),
        0xF => {
            if rom_type == 0xF9 {
                Some("SPC7110")
            } else {
                Some("Custom")
            }
        }
        _ => None,
    }
}

/// Look up old-style developer/licensee ID (1 byte, when developer_id != 0x33).
fn old_maker_name(id: u8) -> Option<&'static str> {
    match id {
        0x01 => Some("Nintendo"),
        0x08 => Some("Capcom"),
        0x0A => Some("Jaleco"),
        0x0B => Some("Coconuts Japan"),
        0x18 => Some("Hudson Soft"),
        0x1D => Some("Banpresto"),
        0x28 => Some("Kemco Japan"),
        0x30 => Some("Infogrames"),
        0x31 => Some("Nintendo"),
        0x33 => Some("Ocean/Acclaim"),
        0x34 => Some("Konami"),
        0x35 => Some("HectorSoft"),
        0x38 => Some("Capcom"),
        0x41 => Some("Ubisoft"),
        0x42 => Some("Atlus"),
        0x44 => Some("Malibu"),
        0x46 => Some("Angel"),
        0x4A => Some("Virgin Interactive"),
        0x4D => Some("Tradewest"),
        0x4F => Some("U.S. Gold"),
        0x50 => Some("Absolute"),
        0x51 => Some("Acclaim"),
        0x52 => Some("Activision"),
        0x53 => Some("American Sammy"),
        0x54 => Some("GameTek"),
        0x56 => Some("Majesco"),
        0x5A => Some("Mindscape"),
        0x60 => Some("Titus"),
        0x61 => Some("Virgin Interactive"),
        0x67 => Some("Ocean"),
        0x69 => Some("Electronic Arts"),
        0x6E => Some("Elite Systems"),
        0x6F => Some("Electro Brain"),
        0x70 => Some("Infogrames"),
        0x71 => Some("Interplay"),
        0x72 => Some("Broderbund"),
        0x75 => Some("The Sales Curve"),
        0x78 => Some("THQ"),
        0x79 => Some("Accolade"),
        0x7F => Some("Kemco"),
        0x80 => Some("Misawa Entertainment"),
        0x83 => Some("LOZC"),
        0x86 => Some("Tokuma Shoten"),
        0x8B => Some("Bullet-Proof Software"),
        0x8C => Some("Vic Tokai"),
        0x8E => Some("Character Soft"),
        0x91 => Some("Chunsoft"),
        0x93 => Some("Banpresto"),
        0x95 => Some("Varie"),
        0x97 => Some("Kaneko"),
        0x99 => Some("Pack-In-Video"),
        0x9A => Some("Nichibutsu"),
        0x9B => Some("Tecmo"),
        0x9C => Some("Imagineer"),
        0xA0 => Some("Telenet"),
        0xA4 => Some("Konami"),
        0xA7 => Some("Takara"),
        0xAA => Some("Culture Brain"),
        0xAC => Some("Toei Animation"),
        0xAF => Some("Namco"),
        0xB0 => Some("Acclaim"),
        0xB1 => Some("ASCII / Nexoft"),
        0xB2 => Some("Bandai"),
        0xB4 => Some("Enix"),
        0xB6 => Some("HAL Laboratory"),
        0xBA => Some("Culture Brain"),
        0xBB => Some("Sunsoft"),
        0xBD => Some("Sony Imagesoft"),
        0xBF => Some("Sammy"),
        0xC0 => Some("Taito"),
        0xC2 => Some("Kemco"),
        0xC3 => Some("Square"),
        0xC4 => Some("Tokuma Shoten"),
        0xC5 => Some("Data East"),
        0xC6 => Some("Tonkin House"),
        0xC8 => Some("Koei"),
        0xCA => Some("Konami"),
        0xCB => Some("Vapinc / NTVIC"),
        0xCC => Some("Use Corporation"),
        0xCE => Some("Pony Canyon"),
        0xD0 => Some("Taito"),
        0xD1 => Some("Sofel"),
        0xD2 => Some("Bothtec"),
        0xD6 => Some("Naxat Soft"),
        0xD9 => Some("Banpresto"),
        0xDA => Some("Tomy"),
        0xDB => Some("Hiro"),
        0xDD => Some("NCS"),
        0xDE => Some("Human"),
        0xDF => Some("Altron"),
        0xE1 => Some("Towa Chiki"),
        0xE2 => Some("Yutaka"),
        0xE5 => Some("Epoch"),
        0xE7 => Some("Athena"),
        0xE8 => Some("Asmik"),
        0xE9 => Some("Natsume"),
        0xEA => Some("King Records"),
        0xEB => Some("Atlus"),
        0xEC => Some("Epic/Sony Records"),
        0xEE => Some("IGS"),
        0xF0 => Some("A-Wave"),
        _ => None,
    }
}

/// Look up new-style 2-character maker code (when developer_id == 0x33).
fn new_maker_name(code: &str) -> Option<&'static str> {
    match code {
        "01" => Some("Nintendo"),
        "08" => Some("Capcom"),
        "0A" => Some("Jaleco"),
        "18" => Some("Hudson Soft"),
        "1P" => Some("Creatures"),
        "20" => Some("Destination Software / KSS"),
        "28" => Some("Kemco Japan"),
        "34" => Some("Konami"),
        "38" => Some("Capcom"),
        "41" => Some("Ubisoft"),
        "42" => Some("Atlus"),
        "47" => Some("Spectrum Holobyte"),
        "51" => Some("Acclaim"),
        "52" => Some("Activision"),
        "53" => Some("American Sammy"),
        "54" => Some("GameTek"),
        "5D" => Some("Midway"),
        "5G" => Some("Majesco"),
        "64" => Some("LucasArts"),
        "69" => Some("Electronic Arts"),
        "6E" => Some("Elite Systems"),
        "6S" => Some("TDK Mediactive"),
        "70" => Some("Infogrames"),
        "71" => Some("Interplay"),
        "72" => Some("Broderbund"),
        "75" => Some("The Sales Curve"),
        "78" => Some("THQ"),
        "79" => Some("Accolade"),
        "7D" => Some("Vivendi"),
        "8P" => Some("Sega"),
        "99" => Some("Pack-In-Video"),
        "9B" => Some("Tecmo"),
        "A4" => Some("Konami"),
        "AF" => Some("Namco"),
        "B0" => Some("Acclaim"),
        "B1" => Some("ASCII"),
        "B2" => Some("Bandai"),
        "B4" => Some("Enix"),
        "B6" => Some("HAL Laboratory"),
        "BB" => Some("Sunsoft"),
        "C0" => Some("Taito"),
        "C3" => Some("Square"),
        "C5" => Some("Data East"),
        "C8" => Some("Koei"),
        "D1" => Some("Sofel"),
        "E5" => Some("Epoch"),
        "E7" => Some("Athena"),
        "E9" => Some("Natsume"),
        "EB" => Some("Atlus"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Conversion to RomIdentification
// ---------------------------------------------------------------------------

/// Format a byte count as a human-readable size string.
fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0".into();
    }
    if bytes >= 1024 * 1024 && bytes % (1024 * 1024) == 0 {
        format!("{} MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 && bytes % 1024 == 0 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Convert a parsed SnesHeader into a RomIdentification.
fn to_identification(
    header: &SnesHeader,
    file_size: u64,
    computed_checksum: Option<u16>,
) -> RomIdentification {
    let mut id = RomIdentification::new()
        .with_platform("Super Nintendo Entertainment System");

    // Internal name
    if !header.title.is_empty() {
        id = id.with_internal_name(&header.title);
    }

    // Serial / game code
    if let Some(ref game_code) = header.game_code {
        id.serial_number = Some(game_code.clone());
    }

    // Version
    id.version = Some(format!("1.{}", header.version));

    // Maker code
    if header.developer_id == 0x33 {
        if let Some(ref maker) = header.maker_code {
            if let Some(name) = new_maker_name(maker) {
                id.maker_code = Some(format!("{} ({})", maker, name));
            } else {
                id.maker_code = Some(maker.clone());
            }
        }
    } else if let Some(name) = old_maker_name(header.developer_id) {
        id.maker_code = Some(format!("0x{:02X} ({})", header.developer_id, name));
    } else if header.developer_id != 0 {
        id.maker_code = Some(format!("0x{:02X}", header.developer_id));
    }

    // File sizes
    id.file_size = Some(file_size);
    if header.rom_size > 0 {
        let copier = if header.has_copier_header {
            COPIER_HEADER_SIZE
        } else {
            0
        };
        id.expected_size = Some(header.rom_size + copier);
    }

    // Region
    id.regions = vec![country_to_region(header.country)];

    // Expected checksum
    id.expected_checksums.push(
        ExpectedChecksum::new(
            ChecksumAlgorithm::PlatformSpecific("SNES Internal"),
            header.checksum.to_le_bytes().to_vec(),
        )
        .with_description(format!("0x{:04X}", header.checksum)),
    );

    // Extra fields
    let format_name = if header.has_copier_header {
        "SMC (copier header)"
    } else {
        "SFC (headerless)"
    };
    id.extra.insert("format".into(), format_name.into());
    id.extra.insert("mapping".into(), header.mapping.name().into());
    id.extra.insert("speed".into(), header.speed.name().into());
    id.extra.insert("chipset".into(), chipset_name(header.rom_type).into());

    if let Some(copro) = coprocessor_name(header.rom_type) {
        id.extra.insert("coprocessor".into(), copro.into());
    }

    if header.rom_size > 0 {
        id.extra.insert("rom_size".into(), format_size(header.rom_size));
    }

    if header.ram_size > 0 {
        id.extra.insert("sram_size".into(), format_size(header.ram_size));
    }

    id.extra.insert("country".into(), country_name(header.country).into());

    if header.has_copier_header {
        id.extra.insert("copier_header".into(), "Yes".into());
    }

    // Checksum complement validation
    let complement_valid = header.checksum.wrapping_add(header.checksum_complement) == 0xFFFF;
    id.extra.insert(
        "checksum_complement_valid".into(),
        if complement_valid { "Yes" } else { "No" }.into(),
    );

    // Computed checksum status
    if let Some(computed) = computed_checksum {
        if computed == header.checksum {
            id.extra.insert(
                "checksum_status:SNES Internal".into(),
                "OK".into(),
            );
        } else {
            id.extra.insert(
                "checksum_status:SNES Internal".into(),
                format!(
                    "MISMATCH (expected 0x{:04X}, computed 0x{:04X})",
                    header.checksum, computed
                ),
            );
        }
    }

    // Extended header fields
    if let Some(ref maker) = header.maker_code {
        id.extra.insert("maker_code_raw".into(), maker.clone());
    }
    if let Some(ref game_code) = header.game_code {
        id.extra.insert("game_code".into(), game_code.clone());
    }
    if let Some(exp_ram) = header.expansion_ram_size {
        id.extra.insert("expansion_ram".into(), format_size(exp_ram));
    }

    id
}

// ---------------------------------------------------------------------------
// Analyzer implementation
// ---------------------------------------------------------------------------

/// Analyzer for SNES/Super Famicom ROMs.
#[derive(Debug, Default)]
pub struct SnesAnalyzer;

impl SnesAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for SnesAnalyzer {
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

        let (header_offset, has_copier) = detect_mapping(reader, file_size)?;
        let header = parse_header(reader, header_offset, has_copier)?;

        // Compute checksum unless in quick mode
        let computed_checksum = if !options.quick {
            compute_snes_checksum(reader, has_copier).ok()
        } else {
            None
        };

        Ok(to_identification(&header, file_size, computed_checksum))
    }

    fn analyze_with_progress(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        // SNES ROMs are small enough that progress reporting is unnecessary.
        self.analyze(reader, options)
    }

    fn platform_name(&self) -> &'static str {
        "Super Nintendo Entertainment System"
    }

    fn short_name(&self) -> &'static str {
        "snes"
    }

    fn folder_names(&self) -> &'static [&'static str] {
        &["snes", "sfc", "super famicom", "super nintendo"]
    }

    fn manufacturer(&self) -> &'static str {
        "Nintendo"
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["sfc", "smc", "swc", "fig"]
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

        detect_mapping(reader, file_size).is_ok()
    }

    fn dat_name(&self) -> Option<&'static str> {
        Some("Nintendo - Super Nintendo Entertainment System")
    }

    fn dat_header_size(
        &self,
        _reader: &mut dyn ReadSeek,
        file_size: u64,
    ) -> Result<u64, AnalysisError> {
        // SNES copier headers: if file_size % 1024 == 512, skip 512 bytes
        if file_size % 1024 == 512 {
            Ok(512)
        } else {
            Ok(0)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Build a synthetic 256 KB LoROM with a valid header and checksums.
    fn make_snes_rom() -> Vec<u8> {
        let size: usize = 256 * 1024; // 256 KB
        let mut rom = vec![0u8; size];

        let base = LOROM_HEADER_BASE as usize;

        // Title: "TEST ROM" padded with spaces
        let title = b"TEST ROM             ";
        rom[base + OFF_TITLE..base + OFF_TITLE + 21].copy_from_slice(title);

        // Map mode: LoROM, SlowROM
        rom[base + OFF_MAP_MODE] = 0x20;

        // ROM type: ROM only
        rom[base + OFF_ROM_TYPE] = 0x00;

        // ROM size: 2^18 = 256 KB, code = 0x09 (2^9 * 1024 = 512 KB... wait)
        // Actually: 1 << code KB. For 256 KB: 256 = 1 << 8, but header stores it as
        // the power: so for 256 KB we want code such that (1 << code) * 1024 = 256*1024
        // That means 1 << code = 256, code = 8.
        rom[base + OFF_ROM_SIZE] = 0x08;

        // RAM size: 0 (no SRAM)
        rom[base + OFF_RAM_SIZE] = 0x00;

        // Country: USA
        rom[base + OFF_COUNTRY] = 0x01;

        // Developer ID: Nintendo (0x01)
        rom[base + OFF_DEVELOPER_ID] = 0x01;

        // Version: 0
        rom[base + OFF_VERSION] = 0x00;

        // Compute and set checksums
        recompute_snes_checksums(&mut rom, base);

        rom
    }

    /// Build a synthetic 1 MB HiROM with a valid header and checksums.
    fn make_snes_hirom() -> Vec<u8> {
        let size: usize = 1024 * 1024; // 1 MB
        let mut rom = vec![0u8; size];

        let base = HIROM_HEADER_BASE as usize;

        // Title
        let title = b"HIROM TEST           ";
        rom[base + OFF_TITLE..base + OFF_TITLE + 21].copy_from_slice(title);

        // Map mode: HiROM, SlowROM
        rom[base + OFF_MAP_MODE] = 0x21;

        // ROM type: ROM + RAM + Battery
        rom[base + OFF_ROM_TYPE] = 0x02;

        // ROM size: 1 MB = 1 << 10 * 1024, code = 0x0A
        rom[base + OFF_ROM_SIZE] = 0x0A;

        // RAM size: 8 KB = 1 << 3 * 1024, code = 0x03
        rom[base + OFF_RAM_SIZE] = 0x03;

        // Country: Japan
        rom[base + OFF_COUNTRY] = 0x00;

        // Developer ID: Square (0xC3)
        rom[base + OFF_DEVELOPER_ID] = 0xC3;

        // Version: 1
        rom[base + OFF_VERSION] = 0x01;

        recompute_snes_checksums(&mut rom, base);

        rom
    }

    /// Prepend a 512-byte copier header (all zeros) to a ROM.
    fn add_copier_header(rom: &[u8]) -> Vec<u8> {
        let mut result = vec![0u8; COPIER_HEADER_SIZE as usize];
        result.extend_from_slice(rom);
        result
    }

    /// Recompute the SNES checksum and complement for a ROM in memory.
    fn recompute_snes_checksums(rom: &mut [u8], header_base: usize) {
        // Initialize: complement = 0xFFFF, checksum = 0x0000
        // These 4 bytes contribute 0x01FE to the sum, which is invariant —
        // no matter what checksum/complement pair we write later (as long as
        // they sum to 0xFFFF), the byte contribution stays 0x01FE.
        rom[header_base + OFF_COMPLEMENT] = 0xFF;
        rom[header_base + OFF_COMPLEMENT + 1] = 0xFF;
        rom[header_base + OFF_CHECKSUM] = 0;
        rom[header_base + OFF_CHECKSUM + 1] = 0;

        // Compute wrapping 16-bit sum
        let rom_size = rom.len() as u64;
        let mut power = 1u64;
        while power * 2 <= rom_size {
            power *= 2;
        }

        let mut sum: u16 = 0;
        if power == rom_size {
            for &byte in rom.iter() {
                sum = sum.wrapping_add(byte as u16);
            }
        } else {
            let base_data = &rom[..power as usize];
            let remainder = &rom[power as usize..];
            let remainder_len = remainder.len();

            for &byte in base_data {
                sum = sum.wrapping_add(byte as u16);
            }
            let mirror_total = power as usize;
            for i in 0..mirror_total {
                sum = sum.wrapping_add(remainder[i % remainder_len] as u16);
            }
        }

        let complement = sum ^ 0xFFFF;

        rom[header_base + OFF_COMPLEMENT] = complement as u8;
        rom[header_base + OFF_COMPLEMENT + 1] = (complement >> 8) as u8;
        rom[header_base + OFF_CHECKSUM] = sum as u8;
        rom[header_base + OFF_CHECKSUM + 1] = (sum >> 8) as u8;
    }

    // -- can_handle tests --

    #[test]
    fn test_can_handle_lorom() {
        let rom = make_snes_rom();
        let analyzer = SnesAnalyzer::new();
        assert!(analyzer.can_handle(&mut Cursor::new(rom)));
    }

    #[test]
    fn test_can_handle_hirom() {
        let rom = make_snes_hirom();
        let analyzer = SnesAnalyzer::new();
        assert!(analyzer.can_handle(&mut Cursor::new(rom)));
    }

    #[test]
    fn test_can_handle_too_small() {
        let data = vec![0u8; 100];
        let analyzer = SnesAnalyzer::new();
        assert!(!analyzer.can_handle(&mut Cursor::new(data)));
    }

    #[test]
    fn test_can_handle_garbage() {
        // Random-ish garbage that's large enough but has no valid header
        let mut data = vec![0xFFu8; 256 * 1024];
        // Ensure checksum fields are garbage (don't accidentally sum to 0xFFFF)
        data[LOROM_HEADER_BASE as usize + OFF_COMPLEMENT] = 0xDE;
        data[LOROM_HEADER_BASE as usize + OFF_CHECKSUM] = 0xAD;
        data[HIROM_HEADER_BASE as usize + OFF_COMPLEMENT] = 0xBE;
        data[HIROM_HEADER_BASE as usize + OFF_CHECKSUM] = 0xEF;
        let analyzer = SnesAnalyzer::new();
        // With all 0xFF bytes, the title check fails (0xFF is not valid ASCII)
        // and mapping checks fail -- should not be detected
        assert!(!analyzer.can_handle(&mut Cursor::new(data)));
    }

    // -- Basic analysis tests --

    #[test]
    fn test_analyze_lorom() {
        let rom = make_snes_rom();
        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(
            result.platform.as_deref(),
            Some("Super Nintendo Entertainment System")
        );
        assert_eq!(result.internal_name.as_deref(), Some("TEST ROM"));
        assert_eq!(result.extra.get("mapping").unwrap(), "LoROM");
        assert_eq!(result.extra.get("format").unwrap(), "SFC (headerless)");
        assert_eq!(result.extra.get("country").unwrap(), "USA");
        assert_eq!(result.regions, vec![Region::Usa]);
        assert_eq!(result.version.as_deref(), Some("1.0"));
        assert_eq!(result.file_size, Some(256 * 1024));
    }

    #[test]
    fn test_analyze_hirom() {
        let rom = make_snes_hirom();
        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.internal_name.as_deref(), Some("HIROM TEST"));
        assert_eq!(result.extra.get("mapping").unwrap(), "HiROM");
        assert_eq!(result.extra.get("chipset").unwrap(), "ROM + RAM + Battery");
        assert_eq!(result.extra.get("country").unwrap(), "Japan");
        assert_eq!(result.regions, vec![Region::Japan]);
        assert_eq!(result.version.as_deref(), Some("1.1"));
        assert!(result.extra.get("sram_size").is_some());
    }

    #[test]
    fn test_analyze_with_copier_header() {
        let rom = make_snes_rom();
        let rom_with_copier = add_copier_header(&rom);
        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer
            .analyze(&mut Cursor::new(rom_with_copier), &options)
            .unwrap();

        assert_eq!(result.internal_name.as_deref(), Some("TEST ROM"));
        assert_eq!(
            result.extra.get("format").unwrap(),
            "SMC (copier header)"
        );
        assert_eq!(result.extra.get("copier_header").unwrap(), "Yes");
    }

    #[test]
    fn test_analyze_fastrom() {
        let mut rom = make_snes_rom();
        let base = LOROM_HEADER_BASE as usize;
        // Set FastROM bit
        rom[base + OFF_MAP_MODE] = 0x30; // LoROM + FastROM
        recompute_snes_checksums(&mut rom, base);

        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.extra.get("speed").unwrap(), "FastROM (3.58 MHz)");
    }

    #[test]
    fn test_analyze_extended_header() {
        let mut rom = make_snes_rom();
        let base = LOROM_HEADER_BASE as usize;

        // Set developer_id to 0x33 to enable extended header
        rom[base + OFF_DEVELOPER_ID] = 0x33;

        // Set maker code "01" (Nintendo)
        rom[base + OFF_EXT_MAKER_CODE] = b'0';
        rom[base + OFF_EXT_MAKER_CODE + 1] = b'1';

        // Set game code "ABCD"
        rom[base + OFF_EXT_GAME_CODE] = b'A';
        rom[base + OFF_EXT_GAME_CODE + 1] = b'B';
        rom[base + OFF_EXT_GAME_CODE + 2] = b'C';
        rom[base + OFF_EXT_GAME_CODE + 3] = b'D';

        recompute_snes_checksums(&mut rom, base);

        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.serial_number.as_deref(), Some("ABCD"));
        assert_eq!(result.extra.get("game_code").unwrap(), "ABCD");
        assert_eq!(result.extra.get("maker_code_raw").unwrap(), "01");
        assert_eq!(result.maker_code.as_deref(), Some("01 (Nintendo)"));
    }

    // -- Checksum tests --

    #[test]
    fn test_checksum_valid() {
        let rom = make_snes_rom();
        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(
            result.extra.get("checksum_complement_valid").unwrap(),
            "Yes"
        );
        assert_eq!(
            result.extra.get("checksum_status:SNES Internal").unwrap(),
            "OK"
        );
    }

    #[test]
    fn test_checksum_mismatch() {
        let mut rom = make_snes_rom();
        // Corrupt a byte outside the header to change the actual checksum
        rom[0] = 0xFF;

        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        let status = result
            .extra
            .get("checksum_status:SNES Internal")
            .unwrap();
        assert!(status.starts_with("MISMATCH"));
    }

    #[test]
    fn test_checksum_complement() {
        let rom = make_snes_rom();
        let base = LOROM_HEADER_BASE as usize;

        let complement = u16::from_le_bytes([rom[base + OFF_COMPLEMENT], rom[base + OFF_COMPLEMENT + 1]]);
        let checksum = u16::from_le_bytes([rom[base + OFF_CHECKSUM], rom[base + OFF_CHECKSUM + 1]]);
        assert_eq!(checksum.wrapping_add(complement), 0xFFFF);
    }

    #[test]
    fn test_quick_mode_skips_checksum() {
        let rom = make_snes_rom();
        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::new().quick(true);
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        // In quick mode, no checksum_status field should be present
        assert!(!result.extra.contains_key("checksum_status:SNES Internal"));
    }

    // -- Lookup table tests --

    #[test]
    fn test_country_names() {
        assert_eq!(country_name(0x00), "Japan");
        assert_eq!(country_name(0x01), "USA");
        assert_eq!(country_name(0x02), "Europe");
        assert_eq!(country_name(0x11), "Australia");
        assert_eq!(country_name(0xFF), "Unknown");
    }

    #[test]
    fn test_country_to_region_mapping() {
        assert_eq!(country_to_region(0x00), Region::Japan);
        assert_eq!(country_to_region(0x01), Region::Usa);
        assert_eq!(country_to_region(0x02), Region::Europe);
        assert_eq!(country_to_region(0x0B), Region::China);
        assert_eq!(country_to_region(0x0D), Region::Korea);
        assert_eq!(country_to_region(0x0E), Region::World);
        assert_eq!(country_to_region(0x10), Region::Brazil);
        assert_eq!(country_to_region(0x11), Region::Australia);
    }

    #[test]
    fn test_chipset_names() {
        assert_eq!(chipset_name(0x00), "ROM only");
        assert_eq!(chipset_name(0x02), "ROM + RAM + Battery");
        assert_eq!(chipset_name(0x15), "ROM + SuperFX + RAM + Battery");
        assert_eq!(chipset_name(0x33), "ROM + SA-1");
        assert_eq!(chipset_name(0x43), "ROM + S-DD1");
        assert_eq!(chipset_name(0xFF), "Unknown chipset");
    }

    #[test]
    fn test_coprocessor_names() {
        assert_eq!(coprocessor_name(0x03), Some("DSP"));
        assert_eq!(coprocessor_name(0x15), Some("SuperFX"));
        assert_eq!(coprocessor_name(0x33), Some("SA-1"));
        assert_eq!(coprocessor_name(0x43), Some("S-DD1"));
        assert_eq!(coprocessor_name(0x00), None);
        assert_eq!(coprocessor_name(0x01), None);
    }

    // -- Metadata tests --

    #[test]
    fn test_sram_detection() {
        let rom = make_snes_hirom();
        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.extra.get("sram_size").unwrap(), "8 KB");
    }

    #[test]
    fn test_version_number() {
        let mut rom = make_snes_rom();
        let base = LOROM_HEADER_BASE as usize;
        rom[base + OFF_VERSION] = 3;
        recompute_snes_checksums(&mut rom, base);

        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.version.as_deref(), Some("1.3"));
    }

    #[test]
    fn test_game_code_extraction() {
        let mut rom = make_snes_rom();
        let base = LOROM_HEADER_BASE as usize;

        rom[base + OFF_DEVELOPER_ID] = 0x33;
        rom[base + OFF_EXT_GAME_CODE] = b'S';
        rom[base + OFF_EXT_GAME_CODE + 1] = b'M';
        rom[base + OFF_EXT_GAME_CODE + 2] = b'W';
        rom[base + OFF_EXT_GAME_CODE + 3] = b'J';
        recompute_snes_checksums(&mut rom, base);

        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.serial_number.as_deref(), Some("SMWJ"));
        assert_eq!(result.extra.get("game_code").unwrap(), "SMWJ");
    }

    #[test]
    fn test_old_maker_name_lookup() {
        assert_eq!(old_maker_name(0x01), Some("Nintendo"));
        assert_eq!(old_maker_name(0xC3), Some("Square"));
        assert_eq!(old_maker_name(0x08), Some("Capcom"));
        assert_eq!(old_maker_name(0xFF), None);
    }

    #[test]
    fn test_new_maker_name_lookup() {
        assert_eq!(new_maker_name("01"), Some("Nintendo"));
        assert_eq!(new_maker_name("C3"), Some("Square"));
        assert_eq!(new_maker_name("69"), Some("Electronic Arts"));
        assert_eq!(new_maker_name("ZZ"), None);
    }

    // -- Edge case tests --

    #[test]
    fn test_scoring_prefers_correct_mapping() {
        // For a LoROM, the LoROM offset should score higher than HiROM offset
        let rom = make_snes_rom();
        let lo_score = score_header_at(
            &mut Cursor::new(&rom),
            LOROM_HEADER_BASE,
        );
        let hi_score = score_header_at(
            &mut Cursor::new(&rom),
            HIROM_HEADER_BASE,
        );
        assert!(
            lo_score > hi_score,
            "LoROM score ({}) should be higher than HiROM score ({})",
            lo_score,
            hi_score
        );
    }

    #[test]
    fn test_copier_header_detection() {
        assert!(detect_copier_header(256 * 1024 + 512));
        assert!(!detect_copier_header(256 * 1024));
        assert!(detect_copier_header(512 * 1024 + 512));
        assert!(!detect_copier_header(512 * 1024));
    }

    #[test]
    fn test_detect_mapping_lorom() {
        let rom = make_snes_rom();
        let (offset, has_copier) = detect_mapping(&mut Cursor::new(&rom), rom.len() as u64).unwrap();
        assert_eq!(offset, LOROM_HEADER_BASE);
        assert!(!has_copier);
    }

    #[test]
    fn test_detect_mapping_hirom() {
        let rom = make_snes_hirom();
        let (offset, has_copier) = detect_mapping(&mut Cursor::new(&rom), rom.len() as u64).unwrap();
        assert_eq!(offset, HIROM_HEADER_BASE);
        assert!(!has_copier);
    }

    #[test]
    fn test_detect_mapping_with_copier() {
        let rom = make_snes_rom();
        let rom_with_copier = add_copier_header(&rom);
        let (offset, has_copier) =
            detect_mapping(&mut Cursor::new(&rom_with_copier), rom_with_copier.len() as u64)
                .unwrap();
        assert_eq!(offset, COPIER_HEADER_SIZE + LOROM_HEADER_BASE);
        assert!(has_copier);
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0");
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(8192), "8 KB");
        assert_eq!(format_size(262144), "256 KB");
        assert_eq!(format_size(1048576), "1 MB");
        assert_eq!(format_size(4194304), "4 MB");
    }

    #[test]
    fn test_expected_checksums_present() {
        let rom = make_snes_rom();
        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(result.expected_checksums.len(), 1);
        assert_eq!(
            result.expected_checksums[0].algorithm,
            ChecksumAlgorithm::PlatformSpecific("SNES Internal")
        );
    }

    #[test]
    fn test_hirom_checksum_valid() {
        let rom = make_snes_hirom();
        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

        assert_eq!(
            result.extra.get("checksum_status:SNES Internal").unwrap(),
            "OK"
        );
    }

    #[test]
    fn test_copier_header_checksum_valid() {
        let rom = make_snes_rom();
        let rom_with_copier = add_copier_header(&rom);
        let analyzer = SnesAnalyzer::new();
        let options = AnalysisOptions::default();
        let result = analyzer
            .analyze(&mut Cursor::new(rom_with_copier), &options)
            .unwrap();

        assert_eq!(
            result.extra.get("checksum_status:SNES Internal").unwrap(),
            "OK"
        );
    }
}
