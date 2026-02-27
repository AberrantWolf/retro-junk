//! NES (Famicom) ROM analyzer.
//!
//! Supports:
//! - iNES format (.nes)
//! - NES 2.0 format
//! - UNIF format (.unf) - detection only
//! - FDS format (.fds) - basic header parsing

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;

use retro_junk_core::util::format_bytes;
use retro_junk_core::{
    AnalysisError, AnalysisOptions, Platform, Region, RomAnalyzer, RomIdentification,
};

/// The 4-byte magic at the start of every iNES / NES 2.0 file.
const INES_MAGIC: [u8; 4] = [0x4E, 0x45, 0x53, 0x1A]; // "NES\x1A"

/// The 4-byte magic at the start of every UNIF file.
const UNIF_MAGIC: [u8; 4] = [0x55, 0x4E, 0x49, 0x46]; // "UNIF"

/// The 4-byte magic used by fwNES-headered FDS images.
const FDS_HEADER_MAGIC: [u8; 4] = [0x46, 0x44, 0x53, 0x1A]; // "FDS\x1A"

/// The 15-byte literal that begins every FDS disk info block.
const FDS_DISK_VERIFY: &[u8; 14] = b"*NINTENDO-HVC*";

/// Size of a single FDS disk side in bytes (65500 bytes in the .fds image format).
const FDS_SIDE_SIZE: u64 = 65500;

/// Detected NES file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NesFormat {
    /// iNES 1.0 (original 16-byte header).
    INes,
    /// NES 2.0 (extended header, backward-compatible with iNES).
    Nes2,
    /// UNIF (Universal NES Image Format).
    Unif,
    /// FDS disk image with fwNES header.
    FdsHeadered,
    /// FDS disk image without header (raw disk sides).
    FdsRaw,
}

impl NesFormat {
    pub fn name(&self) -> &'static str {
        match self {
            Self::INes => "iNES",
            Self::Nes2 => "NES 2.0",
            Self::Unif => "UNIF",
            Self::FdsHeadered => "FDS (headered)",
            Self::FdsRaw => "FDS (raw)",
        }
    }
}

/// Nametable mirroring arrangement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirroring {
    Horizontal,
    Vertical,
    FourScreen,
}

impl Mirroring {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Horizontal => "Horizontal",
            Self::Vertical => "Vertical",
            Self::FourScreen => "Four-screen",
        }
    }
}

/// CPU/PPU timing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TvSystem {
    Ntsc,
    Pal,
    MultiRegion,
    Dendy,
}

impl TvSystem {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ntsc => "NTSC",
            Self::Pal => "PAL",
            Self::MultiRegion => "Multi-region",
            Self::Dendy => "Dendy",
        }
    }

    /// Map a timing mode to region(s).
    pub fn to_regions(&self) -> Vec<Region> {
        match self {
            Self::Ntsc => vec![Region::Usa, Region::Japan],
            Self::Pal => vec![Region::Europe],
            Self::MultiRegion => vec![Region::World],
            Self::Dendy => vec![Region::Unknown], // Dendy was an unofficial clone
        }
    }
}

/// Console type for NES 2.0 extended header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleType {
    Nes,
    VsSystem,
    Playchoice10,
    Extended(u8),
}

impl ConsoleType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Nes => "NES/Famicom",
            Self::VsSystem => "VS. System",
            Self::Playchoice10 => "PlayChoice-10",
            Self::Extended(_) => "Extended",
        }
    }
}

/// Parsed iNES / NES 2.0 header.
#[derive(Debug, Clone)]
pub struct INesHeader {
    /// Detected format variant.
    pub format: NesFormat,
    /// PRG ROM size in bytes.
    pub prg_rom_size: u32,
    /// CHR ROM size in bytes (0 means CHR RAM is used).
    pub chr_rom_size: u32,
    /// Mapper number (0-4095 for NES 2.0, 0-255 for iNES).
    pub mapper: u16,
    /// Submapper number (NES 2.0 only, 0-15).
    pub submapper: Option<u8>,
    /// Nametable mirroring.
    pub mirroring: Mirroring,
    /// Battery-backed PRG RAM or other persistent memory.
    pub has_battery: bool,
    /// 512-byte trainer present before PRG ROM.
    pub has_trainer: bool,
    /// Console type.
    pub console_type: ConsoleType,
    /// TV system / timing mode.
    pub tv_system: TvSystem,
    /// PRG RAM size in bytes (NES 2.0; 0 if not specified).
    pub prg_ram_size: u32,
    /// PRG NVRAM size in bytes (NES 2.0; battery-backed).
    pub prg_nvram_size: u32,
    /// CHR RAM size in bytes (NES 2.0).
    pub chr_ram_size: u32,
    /// CHR NVRAM size in bytes (NES 2.0).
    pub chr_nvram_size: u32,
    /// Number of miscellaneous ROMs (NES 2.0).
    pub misc_roms: u8,
    /// Default expansion device (NES 2.0).
    pub expansion_device: u8,
}

/// Parsed FDS disk info block.
#[derive(Debug, Clone)]
pub struct FdsDiskInfo {
    /// Manufacturer code byte.
    pub manufacturer_code: u8,
    /// 3-character game name code.
    pub game_name: String,
    /// Game type byte.
    pub game_type: u8,
    /// Revision number.
    pub revision: u8,
    /// Side number (0 = Side A, 1 = Side B).
    pub side_number: u8,
    /// Disk number.
    pub disk_number: u8,
    /// Disk type.
    pub disk_type: u8,
    /// Manufacturing date as (year, month, day) in BCD, if valid.
    pub manufacturing_date: Option<(u8, u8, u8)>,
    /// Rewrite date as (year, month, day) in BCD, if valid.
    pub rewrite_date: Option<(u8, u8, u8)>,
    /// Rewrite count.
    pub rewrite_count: u8,
    /// Boot file ID.
    pub boot_file_id: u8,
}

/// Parsed information from any supported NES-family format.
#[derive(Debug, Clone)]
pub enum NesRomInfo {
    INes(INesHeader),
    Fds {
        format: NesFormat,
        disk_count: u8,
        sides: Vec<FdsDiskInfo>,
    },
    Unif {
        revision: u32,
    },
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse an iNES / NES 2.0 header from a 16-byte buffer.
fn parse_ines_header(header: &[u8; 16]) -> Result<INesHeader, AnalysisError> {
    // Byte 6: flags
    let flags6 = header[6];
    let mirroring_bit = flags6 & 0x01;
    let has_battery = flags6 & 0x02 != 0;
    let has_trainer = flags6 & 0x04 != 0;
    let four_screen = flags6 & 0x08 != 0;
    let mapper_lo = (flags6 >> 4) & 0x0F;

    let mirroring = if four_screen {
        Mirroring::FourScreen
    } else if mirroring_bit != 0 {
        Mirroring::Vertical
    } else {
        Mirroring::Horizontal
    };

    // Byte 7: flags
    let flags7 = header[7];
    let console_lo = flags7 & 0x03;
    let nes2_id = (flags7 >> 2) & 0x03;
    let mapper_hi = (flags7 >> 4) & 0x0F;

    let is_nes2 = nes2_id == 2;
    let format = if is_nes2 {
        NesFormat::Nes2
    } else {
        NesFormat::INes
    };

    let console_type = match console_lo {
        0 => ConsoleType::Nes,
        1 => ConsoleType::VsSystem,
        2 => ConsoleType::Playchoice10,
        v => ConsoleType::Extended(v),
    };

    if is_nes2 {
        // NES 2.0 extended parsing
        let mapper_msb = header[8] & 0x0F;
        let submapper = (header[8] >> 4) & 0x0F;

        let mapper = mapper_lo as u16 | ((mapper_hi as u16) << 4) | ((mapper_msb as u16) << 8);

        let prg_rom_msb = header[9] & 0x0F;
        let chr_rom_msb = (header[9] >> 4) & 0x0F;

        let prg_rom_size = if prg_rom_msb == 0x0F {
            // Exponent-multiplier notation
            let exponent = (header[4] >> 2) & 0x3F;
            let multiplier = (header[4] & 0x03) * 2 + 1;
            (1u32 << exponent) * multiplier as u32
        } else {
            let raw = header[4] as u32 | ((prg_rom_msb as u32) << 8);
            raw * 16384
        };

        let chr_rom_size = if chr_rom_msb == 0x0F {
            let exponent = (header[5] >> 2) & 0x3F;
            let multiplier = (header[5] & 0x03) * 2 + 1;
            (1u32 << exponent) * multiplier as u32
        } else {
            let raw = header[5] as u32 | ((chr_rom_msb as u32) << 8);
            raw * 8192
        };

        let prg_ram_shift = header[10] & 0x0F;
        let prg_nvram_shift = (header[10] >> 4) & 0x0F;
        let chr_ram_shift = header[11] & 0x0F;
        let chr_nvram_shift = (header[11] >> 4) & 0x0F;

        let shift_to_size = |s: u8| -> u32 { if s == 0 { 0 } else { 64 << s } };

        let timing = header[12] & 0x03;
        let tv_system = match timing {
            0 => TvSystem::Ntsc,
            1 => TvSystem::Pal,
            2 => TvSystem::MultiRegion,
            3 => TvSystem::Dendy,
            _ => unreachable!(),
        };

        let misc_roms = header[14] & 0x03;
        let expansion_device = header[15] & 0x3F;

        Ok(INesHeader {
            format,
            prg_rom_size,
            chr_rom_size,
            mapper,
            submapper: Some(submapper),
            mirroring,
            has_battery,
            has_trainer,
            console_type,
            tv_system,
            prg_ram_size: shift_to_size(prg_ram_shift),
            prg_nvram_size: shift_to_size(prg_nvram_shift),
            chr_ram_size: shift_to_size(chr_ram_shift),
            chr_nvram_size: shift_to_size(chr_nvram_shift),
            misc_roms,
            expansion_device,
        })
    } else {
        // iNES 1.0
        let mapper = mapper_lo as u16 | ((mapper_hi as u16) << 4);

        let prg_rom_size = header[4] as u32 * 16384;
        let chr_rom_size = header[5] as u32 * 8192;

        // Byte 9 in iNES 1.0: bit 0 = TV system (unofficial)
        let tv_system = if header[9] & 0x01 != 0 {
            TvSystem::Pal
        } else {
            TvSystem::Ntsc
        };

        Ok(INesHeader {
            format,
            prg_rom_size,
            chr_rom_size,
            mapper,
            submapper: None,
            mirroring,
            has_battery,
            has_trainer,
            console_type,
            tv_system,
            prg_ram_size: 0,
            prg_nvram_size: 0,
            chr_ram_size: 0,
            chr_nvram_size: 0,
            misc_roms: 0,
            expansion_device: 0,
        })
    }
}

/// Try to parse an FDS disk info block from a 56-byte buffer.
///
/// The disk info block is 56 bytes starting with block type 0x01,
/// followed by the "*NINTENDO-HVC*" verification string.
fn parse_fds_disk_info(data: &[u8]) -> Result<FdsDiskInfo, AnalysisError> {
    if data.len() < 56 {
        return Err(AnalysisError::corrupted_header(
            "FDS disk info block too short",
        ));
    }

    if data[0] != 0x01 {
        return Err(AnalysisError::corrupted_header(
            "FDS disk info block: wrong block type",
        ));
    }

    if &data[1..15] != FDS_DISK_VERIFY {
        return Err(AnalysisError::corrupted_header(
            "FDS disk info block: verification string mismatch",
        ));
    }

    let manufacturer_code = data[15];

    // Bytes 16-19: Game name (3-letter code + version byte, but we take the
    // printable ASCII portion).
    let game_name_bytes = &data[16..20];
    let game_name: String = game_name_bytes
        .iter()
        .filter(|&&b| (0x20..0x7F).contains(&b))
        .map(|&b| b as char)
        .collect();

    let game_type = data[20];
    let revision = data[21];
    let side_number = data[22];
    let disk_number = data[23];
    let disk_type = data[24];
    let boot_file_id = data[26];

    // BCD date at bytes 31-33 (manufacturing date: year, month, day)
    let manufacturing_date = parse_bcd_date(data[31], data[32], data[33]);

    // Rewrite date at bytes 46-48
    let rewrite_date = if data.len() >= 49 {
        parse_bcd_date(data[46], data[47], data[48])
    } else {
        None
    };

    let rewrite_count = if data.len() >= 42 { data[41] } else { 0 };

    Ok(FdsDiskInfo {
        manufacturer_code,
        game_name,
        game_type,
        revision,
        side_number,
        disk_number,
        disk_type,
        manufacturing_date,
        rewrite_date,
        rewrite_count,
        boot_file_id,
    })
}

/// Parse a BCD-encoded date. Returns None if all zeroes or clearly invalid.
fn parse_bcd_date(year: u8, month: u8, day: u8) -> Option<(u8, u8, u8)> {
    if year == 0 && month == 0 && day == 0 {
        return None;
    }
    Some((year, month, day))
}

/// Format a BCD date as a human-readable string.
fn format_bcd_date(year: u8, month: u8, day: u8) -> String {
    // BCD bytes: e.g. 0x86 means 1986, 0x01 means January
    format!("19{:02x}-{:02x}-{:02x}", year, month, day)
}

/// Look up a human-readable name for common NES mapper numbers.
fn mapper_name(mapper: u16) -> Option<&'static str> {
    match mapper {
        0 => Some("NROM"),
        1 => Some("MMC1 (SxROM)"),
        2 => Some("UxROM"),
        3 => Some("CNROM"),
        4 => Some("MMC3 (TxROM)"),
        5 => Some("MMC5 (ExROM)"),
        7 => Some("AxROM"),
        9 => Some("MMC2 (PxROM)"),
        10 => Some("MMC4 (FxROM)"),
        11 => Some("Color Dreams"),
        16 => Some("Bandai FCG"),
        18 => Some("Jaleco SS 88006"),
        19 => Some("Namco 163"),
        21 => Some("VRC4a/VRC4c"),
        22 => Some("VRC2a"),
        23 => Some("VRC2b/VRC4e"),
        24 => Some("VRC6a"),
        25 => Some("VRC4b/VRC4d"),
        26 => Some("VRC6b"),
        34 => Some("BNROM / NINA-001"),
        48 => Some("Taito TC0190"),
        64 => Some("Tengen RAMBO-1"),
        65 => Some("Irem H3001"),
        66 => Some("GxROM"),
        69 => Some("Sunsoft FME-7"),
        71 => Some("Camerica/Codemasters"),
        73 => Some("VRC3"),
        75 => Some("VRC1"),
        76 => Some("Namco 109 variant"),
        79 => Some("NINA-03/NINA-06"),
        85 => Some("VRC7"),
        86 => Some("Jaleco JF-13"),
        87 => Some("Jaleco/Konami"),
        94 => Some("Senjou no Ookami"),
        95 => Some("Namco 118 variant"),
        105 => Some("NES-EVENT (MMC1)"),
        118 => Some("TxSROM (MMC3)"),
        119 => Some("TQROM (MMC3)"),
        159 => Some("Bandai FCG (EPROM)"),
        206 => Some("DxROM/Namco 118"),
        210 => Some("Namco 175/340"),
        228 => Some("Active Enterprises"),
        232 => Some("Camerica Quattro"),
        _ => None,
    }
}

/// Look up the FDS manufacturer name from a code byte.
fn fds_manufacturer_name(code: u8) -> Option<&'static str> {
    match code {
        0x00 => Some("<Unlicensed>"),
        0x01 => Some("Nintendo"),
        0x08 => Some("Capcom"),
        0x0A => Some("Jaleco"),
        0x18 => Some("Hudson Soft"),
        0x49 => Some("Irem"),
        0x4A => Some("Gakken"),
        0x8B => Some("BulletProof Software"),
        0x99 => Some("Pack-In-Video"),
        0x9B => Some("Tecmo"),
        0x9C => Some("Imagineer"),
        0xA2 => Some("Scorpion Soft"),
        0xA4 => Some("Konami"),
        0xA6 => Some("Kawada Co."),
        0xA7 => Some("Takara"),
        0xA8 => Some("Royal Industries"),
        0xAC => Some("Toei Animation"),
        0xAF => Some("Namco"),
        0xB1 => Some("ASCII Corporation"),
        0xB2 => Some("Bandai"),
        0xB3 => Some("Soft Pro Inc."),
        0xB6 => Some("HAL Laboratory"),
        0xBA => Some("Culture Brain"),
        0xBB => Some("Sunsoft"),
        0xBC => Some("Toshiba EMI"),
        0xC0 => Some("Taito"),
        0xC1 => Some("Sunsoft/Ask"),
        0xC2 => Some("Kemco"),
        0xC3 => Some("Square"),
        0xC4 => Some("Tokuma Shoten"),
        0xC5 => Some("Data East"),
        0xC6 => Some("Tonkin House"),
        0xC7 => Some("East Cube"),
        0xCA => Some("Konami/Ultra/Palcom"),
        0xCB => Some("NTVIC/VAP"),
        0xCC => Some("Use Co."),
        0xCE => Some("Pony Canyon/FCI"),
        0xD1 => Some("Sofel"),
        0xD2 => Some("Bothtec Inc."),
        0xDB => Some("Hiro Co."),
        0xE7 => Some("Athena"),
        0xEB => Some("Atlus"),
        _ => None,
    }
}

/// Look up the NES 2.0 expansion device name.
fn expansion_device_name(id: u8) -> Option<&'static str> {
    match id {
        0x00 => Some("Unspecified"),
        0x01 => Some("Standard NES/Famicom controllers"),
        0x02 => Some("NES Four Score / Satellite"),
        0x03 => Some("Famicom Four Players Adapter"),
        0x04 => Some("VS. System"),
        0x05 => Some("VS. System (reversed inputs)"),
        0x06 => Some("VS. Pinball (Japan)"),
        0x07 => Some("VS. Zapper"),
        0x09 => Some("Zapper"),
        0x0A => Some("Two Zappers"),
        0x0B => Some("Bandai Hyper Shot"),
        0x0C => Some("Power Pad Side A"),
        0x0D => Some("Power Pad Side B"),
        0x0E => Some("Family Trainer Side A"),
        0x0F => Some("Family Trainer Side B"),
        0x10 => Some("Arkanoid Vaus (NES)"),
        0x11 => Some("Arkanoid Vaus (Famicom)"),
        0x12 => Some("Two Vaus + Famicom Data Recorder"),
        0x13 => Some("Konami Hyper Shot"),
        0x14 => Some("Coconuts Pachinko"),
        0x15 => Some("Exciting Boxing Punching Bag"),
        0x16 => Some("Jissen Mahjong"),
        0x17 => Some("Party Tap"),
        0x18 => Some("Oeka Kids Tablet"),
        0x19 => Some("Sunsoft Barcode Battler"),
        0x23 => Some("Multicart (select via $4100)"),
        0x24 => Some("Two SNES controllers"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Analysis helpers
// ---------------------------------------------------------------------------

fn analyze_ines(reader: &mut dyn ReadSeek) -> Result<NesRomInfo, AnalysisError> {
    let mut header = [0u8; 16];
    reader.read_exact(&mut header).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            AnalysisError::TooSmall {
                expected: 16,
                actual: 0,
            }
        } else {
            AnalysisError::Io(e)
        }
    })?;

    if header[0..4] != INES_MAGIC {
        return Err(AnalysisError::invalid_format("Not an iNES file"));
    }

    let parsed = parse_ines_header(&header)?;
    Ok(NesRomInfo::INes(parsed))
}

fn analyze_fds(reader: &mut dyn ReadSeek) -> Result<NesRomInfo, AnalysisError> {
    let total_size = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;

    let mut magic = [0u8; 4];
    reader
        .read_exact(&mut magic)
        .map_err(|_| AnalysisError::TooSmall {
            expected: 4,
            actual: 0,
        })?;

    let (format, data_offset, disk_count_from_header) = if magic == FDS_HEADER_MAGIC {
        // fwNES-headered FDS: 16-byte header, byte 4 = number of disk sides
        let mut rest = [0u8; 12];
        reader
            .read_exact(&mut rest)
            .map_err(|_| AnalysisError::TooSmall {
                expected: 16,
                actual: 4,
            })?;
        let count = rest[0]; // byte 4 of the full 16-byte header
        (NesFormat::FdsHeadered, 16u64, Some(count))
    } else {
        // Headerless: rewind and check for disk info block directly
        reader.seek(SeekFrom::Start(0))?;
        (NesFormat::FdsRaw, 0u64, None)
    };

    // Calculate number of sides from data size
    let data_size = total_size - data_offset;
    let side_count = if let Some(c) = disk_count_from_header {
        c as u64
    } else {
        data_size / FDS_SIDE_SIZE
    };

    if side_count == 0 {
        return Err(AnalysisError::invalid_format(
            "FDS image contains no disk sides",
        ));
    }

    // Parse disk info from each side
    let mut sides = Vec::new();
    for i in 0..side_count.min(8) {
        // cap at 8 sides (4 disks) for sanity
        let side_offset = data_offset + i * FDS_SIDE_SIZE;
        reader.seek(SeekFrom::Start(side_offset))?;

        let mut block = [0u8; 56];
        if reader.read_exact(&mut block).is_err() {
            break;
        }

        match parse_fds_disk_info(&block) {
            Ok(info) => sides.push(info),
            Err(_) => break,
        }
    }

    let disk_count = if sides.is_empty() {
        (side_count as u8).div_ceil(2)
    } else {
        // Derive from max disk_number seen
        sides.iter().map(|s| s.disk_number + 1).max().unwrap_or(1)
    };

    Ok(NesRomInfo::Fds {
        format,
        disk_count,
        sides,
    })
}

fn analyze_unif(reader: &mut dyn ReadSeek) -> Result<NesRomInfo, AnalysisError> {
    let mut header = [0u8; 8];
    reader
        .read_exact(&mut header)
        .map_err(|_| AnalysisError::TooSmall {
            expected: 8,
            actual: 0,
        })?;

    if header[0..4] != UNIF_MAGIC {
        return Err(AnalysisError::invalid_format("Not a UNIF file"));
    }

    // Bytes 4-7: revision number (little-endian u32)
    let revision = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);

    Ok(NesRomInfo::Unif { revision })
}

/// Compute the expected file size for an iNES/NES 2.0 ROM from header fields.
fn ines_expected_size(hdr: &INesHeader) -> u64 {
    let header_size = 16u64;
    let trainer_size = if hdr.has_trainer { 512u64 } else { 0 };
    header_size + trainer_size + hdr.prg_rom_size as u64 + hdr.chr_rom_size as u64
}

/// Compute the expected file size for an FDS image.
fn fds_expected_size(format: NesFormat, side_count: usize) -> u64 {
    let header_size = match format {
        NesFormat::FdsHeadered => 16u64,
        _ => 0u64,
    };
    header_size + (side_count as u64 * FDS_SIDE_SIZE)
}

/// Convert parsed NES ROM info into a generic `RomIdentification`.
fn to_identification(info: &NesRomInfo, file_size: u64) -> RomIdentification {
    let mut id = RomIdentification::new().with_platform(Platform::Nes);
    id.file_size = Some(file_size);

    match info {
        NesRomInfo::INes(hdr) => {
            id.extra.insert("format".into(), hdr.format.name().into());
            id.extra.insert("mapper".into(), hdr.mapper.to_string());
            if let Some(name) = mapper_name(hdr.mapper) {
                id.extra.insert("mapper_name".into(), name.into());
            }
            if let Some(sub) = hdr.submapper
                && sub != 0
            {
                id.extra.insert("submapper".into(), sub.to_string());
            }
            id.extra
                .insert("mirroring".into(), hdr.mirroring.name().into());
            id.extra
                .insert("prg_rom_size".into(), format_bytes(hdr.prg_rom_size as u64));
            id.extra.insert(
                "chr_rom_size".into(),
                if hdr.chr_rom_size > 0 {
                    format_bytes(hdr.chr_rom_size as u64)
                } else {
                    "CHR RAM".into()
                },
            );
            if hdr.has_battery {
                id.extra.insert("battery".into(), "Yes".into());
            }
            if hdr.has_trainer {
                id.extra.insert("trainer".into(), "Yes".into());
            }
            if hdr.console_type != ConsoleType::Nes {
                id.extra
                    .insert("console_type".into(), hdr.console_type.name().into());
            }
            id.extra
                .insert("tv_system".into(), hdr.tv_system.name().into());
            id.regions = hdr.tv_system.to_regions();

            // NES 2.0 specifics
            if hdr.format == NesFormat::Nes2 {
                if hdr.prg_ram_size > 0 {
                    id.extra
                        .insert("prg_ram_size".into(), format_bytes(hdr.prg_ram_size as u64));
                }
                if hdr.prg_nvram_size > 0 {
                    id.extra.insert(
                        "prg_nvram_size".into(),
                        format_bytes(hdr.prg_nvram_size as u64),
                    );
                }
                if hdr.chr_ram_size > 0 {
                    id.extra
                        .insert("chr_ram_size".into(), format_bytes(hdr.chr_ram_size as u64));
                }
                if hdr.chr_nvram_size > 0 {
                    id.extra.insert(
                        "chr_nvram_size".into(),
                        format_bytes(hdr.chr_nvram_size as u64),
                    );
                }
                if hdr.expansion_device != 0 {
                    let dev = if let Some(name) = expansion_device_name(hdr.expansion_device) {
                        name.to_string()
                    } else {
                        format!("Unknown (0x{:02X})", hdr.expansion_device)
                    };
                    id.extra.insert("expansion_device".into(), dev);
                }
                if hdr.misc_roms > 0 {
                    id.extra
                        .insert("misc_roms".into(), hdr.misc_roms.to_string());
                }
            }

            id.expected_size = Some(ines_expected_size(hdr));
        }
        NesRomInfo::Fds {
            format,
            disk_count,
            sides,
        } => {
            id.platform = Some(Platform::Nes);
            id.extra
                .insert("platform_variant".into(), "Famicom Disk System".into());
            id.extra.insert("format".into(), format.name().into());
            id.extra.insert("disk_count".into(), disk_count.to_string());
            id.extra
                .insert("side_count".into(), sides.len().to_string());
            id.regions = vec![Region::Japan]; // FDS was Japan-only
            id.expected_size = Some(fds_expected_size(*format, sides.len()));

            if let Some(first) = sides.first() {
                if !first.game_name.is_empty() {
                    id.internal_name = Some(first.game_name.clone());
                }
                if let Some(name) = fds_manufacturer_name(first.manufacturer_code) {
                    id.maker_code = Some(format!("0x{:02X} ({})", first.manufacturer_code, name));
                } else {
                    id.maker_code = Some(format!("0x{:02X}", first.manufacturer_code));
                }
                if first.revision > 0 {
                    id.version = Some(format!("Rev. {}", first.revision));
                }
                if let Some((y, m, d)) = first.manufacturing_date {
                    id.extra
                        .insert("manufacturing_date".into(), format_bcd_date(y, m, d));
                }
                if let Some((y, m, d)) = first.rewrite_date {
                    id.extra
                        .insert("rewrite_date".into(), format_bcd_date(y, m, d));
                }
                if first.rewrite_count > 0 {
                    id.extra
                        .insert("rewrite_count".into(), first.rewrite_count.to_string());
                }
            }
        }
        NesRomInfo::Unif { revision } => {
            id.extra
                .insert("format".into(), NesFormat::Unif.name().into());
            id.extra
                .insert("unif_revision".into(), revision.to_string());
        }
    }

    id
}

/// Detect which NES-family format a reader contains by peeking at magic bytes.
fn detect_format(reader: &mut dyn ReadSeek) -> Result<NesFormat, AnalysisError> {
    let mut magic = [0u8; 4];
    let bytes_read = reader.read(&mut magic)?;
    reader.seek(SeekFrom::Start(0))?;

    if bytes_read < 4 {
        return Err(AnalysisError::TooSmall {
            expected: 4,
            actual: bytes_read as u64,
        });
    }

    if magic == INES_MAGIC {
        // Peek at byte 7 to distinguish iNES from NES 2.0
        let mut header = [0u8; 16];
        reader.read_exact(&mut header)?;
        reader.seek(SeekFrom::Start(0))?;
        let nes2_id = (header[7] >> 2) & 0x03;
        if nes2_id == 2 {
            return Ok(NesFormat::Nes2);
        }
        return Ok(NesFormat::INes);
    }

    if magic == UNIF_MAGIC {
        return Ok(NesFormat::Unif);
    }

    if magic == FDS_HEADER_MAGIC {
        return Ok(NesFormat::FdsHeadered);
    }

    // Check for headerless FDS (starts with 0x01 + "*NINTENDO-HVC*")
    if magic[0] == 0x01 {
        let mut verify = [0u8; 15];
        let n = reader.read(&mut verify)?;
        reader.seek(SeekFrom::Start(0))?;
        if n >= 15 && &verify[0..14] == FDS_DISK_VERIFY {
            return Ok(NesFormat::FdsRaw);
        }
    }

    Err(AnalysisError::invalid_format(
        "Not a recognized NES/FDS/UNIF file",
    ))
}

// ---------------------------------------------------------------------------
// Analyzer implementation
// ---------------------------------------------------------------------------

/// Analyzer for NES/Famicom ROMs.
#[derive(Debug, Default)]
pub struct NesAnalyzer;

impl NesAnalyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for NesAnalyzer {
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        let format = detect_format(reader)?;

        let info = match format {
            NesFormat::INes | NesFormat::Nes2 => analyze_ines(reader)?,
            NesFormat::FdsHeadered | NesFormat::FdsRaw => analyze_fds(reader)?,
            NesFormat::Unif => analyze_unif(reader)?,
        };

        Ok(to_identification(&info, file_size))
    }

    fn platform(&self) -> Platform {
        Platform::Nes
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["nes", "unf", "unif", "fds"]
    }

    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        detect_format(reader).is_ok()
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Nintendo - Nintendo Entertainment System"]
    }

    fn gdb_csv_names(&self) -> &'static [&'static str] {
        &[
            "console_nintendo_famicom_nes",
            "console_nintendo_famicomdisksystem",
        ]
    }

    fn dat_header_size(
        &self,
        reader: &mut dyn ReadSeek,
        _file_size: u64,
    ) -> Result<u64, AnalysisError> {
        // Detect iNES/NES 2.0 magic; if present, strip the 16-byte header
        let mut magic = [0u8; 4];
        reader.seek(SeekFrom::Start(0))?;
        if reader.read_exact(&mut magic).is_ok() && magic == INES_MAGIC {
            reader.seek(SeekFrom::Start(0))?;
            return Ok(16);
        }
        reader.seek(SeekFrom::Start(0))?;
        Ok(0)
    }
}

#[cfg(test)]
#[path = "tests/nes_tests.rs"]
mod tests;
