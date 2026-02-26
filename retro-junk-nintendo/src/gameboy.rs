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
use std::sync::mpsc::Sender;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, ChecksumAlgorithm, ExpectedChecksum,
    Platform, Region, RomAnalyzer, RomIdentification,
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

/// Look up old licensee name from code byte at 0x014B.
fn old_licensee_name(code: u8) -> Option<&'static str> {
    match code {
        0x00 => Some("None"),
        0x01 => Some("Nintendo"),
        0x08 => Some("Capcom"),
        0x09 => Some("Hot-B"),
        0x0A => Some("Jaleco"),
        0x0B => Some("Coconuts Japan"),
        0x0C => Some("Elite Systems"),
        0x13 => Some("EA (Electronic Arts)"),
        0x18 => Some("Hudson Soft"),
        0x19 => Some("ITC Entertainment"),
        0x1A => Some("Yanoman"),
        0x1D => Some("Japan Clary"),
        0x1F => Some("Virgin Interactive"),
        0x24 => Some("PCM Complete"),
        0x25 => Some("San-X"),
        0x28 => Some("Kemco (Kotobuki Systems)"),
        0x29 => Some("SETA Corporation"),
        0x30 => Some("Infogrames"),
        0x31 => Some("Nintendo"),
        0x32 => Some("Bandai"),
        // 0x33 => use new licensee code
        0x34 => Some("Konami"),
        0x35 => Some("HectorSoft"),
        0x38 => Some("Capcom"),
        0x39 => Some("Banpresto"),
        0x3C => Some("Entertainment i"),
        0x3E => Some("Gremlin"),
        0x41 => Some("Ubisoft"),
        0x42 => Some("Atlus"),
        0x44 => Some("Malibu"),
        0x46 => Some("Angel"),
        0x47 => Some("Spectrum Holobyte"),
        0x49 => Some("Irem"),
        0x4A => Some("Virgin Interactive"),
        0x4D => Some("Malibu"),
        0x4F => Some("U.S. Gold"),
        0x50 => Some("Absolute"),
        0x51 => Some("Acclaim"),
        0x52 => Some("Activision"),
        0x53 => Some("American Sammy"),
        0x54 => Some("GameTek"),
        0x55 => Some("Park Place"),
        0x56 => Some("LJN"),
        0x57 => Some("Matchbox"),
        0x59 => Some("Milton Bradley"),
        0x5A => Some("Mindscape"),
        0x5B => Some("Romstar"),
        0x5C => Some("Naxat Soft"),
        0x5D => Some("Tradewest"),
        0x60 => Some("Titus Interactive"),
        0x61 => Some("Virgin Interactive"),
        0x67 => Some("Ocean Interactive"),
        0x69 => Some("EA (Electronic Arts)"),
        0x6E => Some("Elite Systems"),
        0x6F => Some("Electro Brain"),
        0x70 => Some("Infogrames"),
        0x71 => Some("Interplay"),
        0x72 => Some("Broderbund"),
        0x73 => Some("Sculptured Software"),
        0x75 => Some("The Sales Curve"),
        0x78 => Some("THQ"),
        0x79 => Some("Accolade"),
        0x7A => Some("Triffix Entertainment"),
        0x7C => Some("Microprose"),
        0x7F => Some("Kemco"),
        0x80 => Some("Misawa Entertainment"),
        0x83 => Some("Lozc"),
        0x86 => Some("Tokuma Shoten"),
        0x8B => Some("Bullet-Proof Software"),
        0x8C => Some("Vic Tokai"),
        0x8E => Some("Ape"),
        0x8F => Some("I'Max"),
        0x91 => Some("Chunsoft"),
        0x92 => Some("Video System"),
        0x93 => Some("Tsubaraya Productions"),
        0x95 => Some("Varie"),
        0x96 => Some("Yonezawa/s'pal"),
        0x97 => Some("Kaneko"),
        0x99 => Some("Arc"),
        0x9A => Some("Nihon Bussan"),
        0x9B => Some("Tecmo"),
        0x9C => Some("Imagineer"),
        0x9D => Some("Banpresto"),
        0x9F => Some("Nova"),
        0xA1 => Some("Hori Electric"),
        0xA2 => Some("Bandai"),
        0xA4 => Some("Konami"),
        0xA6 => Some("Kawada"),
        0xA7 => Some("Takara"),
        0xA9 => Some("Technos Japan"),
        0xAA => Some("Broderbund"),
        0xAC => Some("Toei Animation"),
        0xAD => Some("Toho"),
        0xAF => Some("Namco"),
        0xB0 => Some("Acclaim"),
        0xB1 => Some("ASCII/Nexsoft"),
        0xB2 => Some("Bandai"),
        0xB4 => Some("Square Enix"),
        0xB6 => Some("HAL Laboratory"),
        0xB7 => Some("SNK"),
        0xB9 => Some("Pony Canyon"),
        0xBA => Some("Culture Brain"),
        0xBB => Some("Sunsoft"),
        0xBD => Some("Sony Imagesoft"),
        0xBF => Some("Sammy"),
        0xC0 => Some("Taito"),
        0xC2 => Some("Kemco"),
        0xC3 => Some("Square"),
        0xC4 => Some("Tokuma Shoten"),
        0xC5 => Some("Data East"),
        0xC6 => Some("Tonkinhouse"),
        0xC8 => Some("Koei"),
        0xC9 => Some("UFL"),
        0xCA => Some("Ultra"),
        0xCB => Some("Vap"),
        0xCC => Some("Use Corporation"),
        0xCD => Some("Meldac"),
        0xCE => Some("Pony Canyon"),
        0xCF => Some("Angel"),
        0xD0 => Some("Taito"),
        0xD1 => Some("Sofel"),
        0xD2 => Some("Quest"),
        0xD3 => Some("Sigma Enterprises"),
        0xD4 => Some("ASK Kodansha"),
        0xD6 => Some("Naxat Soft"),
        0xD7 => Some("Copya System"),
        0xD9 => Some("Banpresto"),
        0xDA => Some("Tomy"),
        0xDB => Some("LJN"),
        0xDD => Some("NCS"),
        0xDE => Some("Human"),
        0xDF => Some("Altron"),
        0xE0 => Some("Jaleco"),
        0xE1 => Some("Towa Chiki"),
        0xE2 => Some("Yutaka"),
        0xE3 => Some("Varie"),
        0xE5 => Some("Epoch"),
        0xE7 => Some("Athena"),
        0xE8 => Some("Asmik Ace"),
        0xE9 => Some("Natsume"),
        0xEA => Some("King Records"),
        0xEB => Some("Atlus"),
        0xEC => Some("Epic/Sony Records"),
        0xEE => Some("IGS"),
        0xF0 => Some("A Wave"),
        0xF3 => Some("Extreme Entertainment"),
        0xFF => Some("LJN"),
        _ => None,
    }
}

/// Look up new licensee name from 2-character ASCII code at 0x0144-0x0145.
fn new_licensee_name(code: &str) -> Option<&'static str> {
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

/// Format a byte count as a human-readable size string.
fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0".into();
    }
    if bytes >= 1024 * 1024 && bytes.is_multiple_of(1024 * 1024) {
        format!("{} MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 && bytes.is_multiple_of(1024) {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} bytes", bytes)
    }
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

    let platform = if header.cgb_flag == 0xC0 || header.cgb_flag == 0x80 {
        "Game Boy Color"
    } else {
        "Game Boy"
    };

    let mut id = RomIdentification::new().with_platform(platform);

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
            .and_then(new_licensee_name)
            .map(|s| s.to_string())
    } else {
        old_licensee_name(header.old_licensee_code).map(|s| s.to_string())
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
        id.extra.insert("ram_size".into(), format_size(ram));
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

    fn analyze_with_progress(
        &self,
        reader: &mut dyn ReadSeek,
        options: &AnalysisOptions,
        _progress_tx: Sender<AnalysisProgress>,
    ) -> Result<RomIdentification, AnalysisError> {
        // GB files are small enough that progress reporting is unnecessary.
        self.analyze(reader, options)
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
