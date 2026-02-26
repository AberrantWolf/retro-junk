//! Nintendo 64 ROM analyzer.
//!
//! Supports:
//! - Big-endian ROMs (.z64)
//! - Byte-swapped ROMs (.v64)
//! - Little-endian ROMs (.n64)
//!
//! Detects CIC variant from boot code and uses the correct checksum algorithm
//! for CIC-6101/6102, 6103, 6105, and 6106.

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;
use std::sync::mpsc::Sender;

use crate::n64_byteorder::{N64Format, detect_n64_format, normalize_to_big_endian};
use retro_junk_core::{
    AnalysisError, AnalysisOptions, AnalysisProgress, ChecksumAlgorithm, ExpectedChecksum,
    Platform, Region, RomAnalyzer, RomIdentification,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const HEADER_SIZE: u64 = 0x40;
const BOOT_CODE_START: u64 = 0x40;
const BOOT_CODE_END: u64 = 0x1000;
const BOOT_CODE_SIZE: usize = (BOOT_CODE_END - BOOT_CODE_START) as usize; // 4032 bytes
const MIN_CRC_SIZE: u64 = 0x101000;

const CRC_START: u64 = 0x1000;
const CRC_END: u64 = 0x101000;

// ---------------------------------------------------------------------------
// CIC variant detection and seeds
// ---------------------------------------------------------------------------

/// CIC lockout chip variants. Each has a different seed and potentially
/// different checksum algorithm behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CicVariant {
    Cic6101,
    Cic6102,
    Cic6103,
    Cic6105,
    Cic6106,
    Unknown,
}

impl CicVariant {
    fn seed(self) -> u32 {
        match self {
            CicVariant::Cic6101 | CicVariant::Cic6102 => 0xF8CA4DDC,
            CicVariant::Cic6103 => 0xA3886759,
            CicVariant::Cic6105 => 0xDF26F436,
            CicVariant::Cic6106 => 0x1FEA617A,
            CicVariant::Unknown => 0xF8CA4DDC, // default to 6102
        }
    }

    fn name(self) -> &'static str {
        match self {
            CicVariant::Cic6101 => "6101",
            CicVariant::Cic6102 => "6102",
            CicVariant::Cic6103 => "6103",
            CicVariant::Cic6105 => "6105",
            CicVariant::Cic6106 => "6106",
            CicVariant::Unknown => "unknown",
        }
    }
}

/// Detect the CIC variant by computing CRC32-IEEE of the IPL3 boot code
/// (bytes 0x40-0x1000) and matching against known values.
/// Boot code must already be normalized to big-endian.
fn detect_cic(boot_code: &[u8]) -> CicVariant {
    let crc = crc32fast::hash(boot_code);
    match crc {
        0x6170A4A1 => CicVariant::Cic6101,
        0x90BB6CB5 => CicVariant::Cic6102,
        0x0B050EE0 => CicVariant::Cic6103,
        0x98BC2C86 => CicVariant::Cic6105,
        0xACC8580A => CicVariant::Cic6106,
        _ => CicVariant::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Local alias — maps the shared `N64Format` to context-appropriate display names.
type RomFormat = N64Format;

fn rom_format_name(format: RomFormat) -> &'static str {
    match format {
        N64Format::Z64 => "z64 (big-endian)",
        N64Format::V64 => "v64 (byte-swapped)",
        N64Format::N64 => "n64 (little-endian)",
    }
}

struct N64Header {
    format: RomFormat,
    cic: CicVariant,
    clock_rate: u32,
    boot_address: u32,
    #[allow(dead_code)]
    libultra_version: u32,
    crc1: u32,
    crc2: u32,
    title: String,
    category_code: u8,
    game_id: [u8; 2],
    destination_code: u8,
    rom_version: u8,
}

// ---------------------------------------------------------------------------
// Header parsing
// ---------------------------------------------------------------------------

/// Read and parse the N64 header. Also reads the boot code (0x40-0x1000)
/// to detect the CIC variant.
fn parse_header(reader: &mut dyn ReadSeek) -> Result<N64Header, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    // Read header + boot code together (0x1000 bytes total)
    let mut buf = [0u8; BOOT_CODE_END as usize];
    reader.read_exact(&mut buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            AnalysisError::TooSmall {
                expected: HEADER_SIZE,
                actual: 0,
            }
        } else {
            AnalysisError::Io(e)
        }
    })?;

    let format = detect_n64_format(&buf).ok_or_else(|| {
        AnalysisError::invalid_format(format!(
            "unrecognized N64 magic bytes: [{:02X}, {:02X}, {:02X}, {:02X}] \
             (expected z64=[80,37,12,40], v64=[37,80,40,12], n64=[40,12,37,80])",
            buf[0], buf[1], buf[2], buf[3]
        ))
    })?;

    normalize_to_big_endian(&mut buf, format);

    // Detect CIC from boot code (bytes 0x40-0x1000, already big-endian)
    let cic = detect_cic(&buf[BOOT_CODE_START as usize..BOOT_CODE_END as usize]);

    let clock_rate = u32::from_be_bytes([buf[0x04], buf[0x05], buf[0x06], buf[0x07]]);
    let boot_address = u32::from_be_bytes([buf[0x08], buf[0x09], buf[0x0A], buf[0x0B]]);
    let libultra_version = u32::from_be_bytes([buf[0x0C], buf[0x0D], buf[0x0E], buf[0x0F]]);
    let crc1 = u32::from_be_bytes([buf[0x10], buf[0x11], buf[0x12], buf[0x13]]);
    let crc2 = u32::from_be_bytes([buf[0x14], buf[0x15], buf[0x16], buf[0x17]]);

    let title: String = buf[0x20..0x34]
        .iter()
        .map(|&b| {
            if b >= 0x20 && b < 0x7F {
                b as char
            } else {
                ' '
            }
        })
        .collect::<String>()
        .trim()
        .to_string();

    let category_code = buf[0x3B];
    let game_id = [buf[0x3C], buf[0x3D]];
    let destination_code = buf[0x3E];
    let rom_version = buf[0x3F];

    Ok(N64Header {
        format,
        cic,
        clock_rate,
        boot_address,
        libultra_version,
        crc1,
        crc2,
        title,
        category_code,
        game_id,
        destination_code,
        rom_version,
    })
}

// ---------------------------------------------------------------------------
// CRC computation
// ---------------------------------------------------------------------------

/// Compute the N64 CRC checksum pair using the correct algorithm for the
/// detected CIC variant. The boot_code parameter is needed for CIC-6105
/// which reads from it during computation.
fn compute_n64_crc(
    reader: &mut dyn ReadSeek,
    format: RomFormat,
    cic: CicVariant,
) -> Result<(u32, u32), AnalysisError> {
    // Read boot code (needed for CIC-6105 t1 update)
    let mut boot_code = [0u8; BOOT_CODE_SIZE];
    reader.seek(SeekFrom::Start(BOOT_CODE_START))?;
    reader.read_exact(&mut boot_code)?;
    normalize_to_big_endian(&mut boot_code, format);

    // Read CRC data region
    let size = (CRC_END - CRC_START) as usize;
    let mut data = vec![0u8; size];
    reader.seek(SeekFrom::Start(CRC_START))?;
    reader.read_exact(&mut data)?;
    normalize_to_big_endian(&mut data, format);

    let seed = cic.seed();
    let mut t1: u32 = seed;
    let mut t2: u32 = seed;
    let mut t3: u32 = seed;
    let mut t4: u32 = seed;
    let mut t5: u32 = seed;
    let mut t6: u32 = seed;

    for (i, chunk) in data.chunks_exact(4).enumerate() {
        let d = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);

        // t6 += d, with overflow counter in t4
        let k1 = t6.wrapping_add(d);
        if k1 < t6 {
            t4 = t4.wrapping_add(1);
        }
        t6 = k1;

        // t3: unconditional XOR
        t3 ^= d;

        // r = rotate_left(d, d & 0x1F); t5 += r
        let r = d.rotate_left(d & 0x1F);
        t5 = t5.wrapping_add(r);

        // t2: conditional XOR
        if d < t2 {
            t2 ^= r;
        } else {
            t2 ^= t6 ^ d;
        }

        // t1: CIC-6105 reads from boot code, others use t5
        if cic == CicVariant::Cic6105 {
            let byte_offset = (i * 4) & 0xFF;
            let boot_offset = 0x0710 + byte_offset;
            let b = u32::from_be_bytes([
                boot_code[boot_offset],
                boot_code[boot_offset + 1],
                boot_code[boot_offset + 2],
                boot_code[boot_offset + 3],
            ]);
            t1 = t1.wrapping_add(b ^ d);
        } else {
            t1 = t1.wrapping_add(d ^ t5);
        }
    }

    // Final combination differs by CIC variant
    let (crc1, crc2) = match cic {
        CicVariant::Cic6103 => ((t6 ^ t4).wrapping_add(t3), (t5 ^ t2).wrapping_add(t1)),
        CicVariant::Cic6106 => (
            (t6.wrapping_mul(t4)).wrapping_add(t3),
            (t5.wrapping_mul(t2)).wrapping_add(t1),
        ),
        _ => (t6 ^ t4 ^ t3, t5 ^ t2 ^ t1),
    };

    Ok((crc1, crc2))
}

// ---------------------------------------------------------------------------
// Region and serial helpers
// ---------------------------------------------------------------------------

fn region_from_destination(code: u8) -> Region {
    match code {
        b'E' | b'N' => Region::Usa,
        b'J' => Region::Japan,
        b'P' | b'D' | b'F' | b'S' | b'I' | b'H' | b'W' | b'X' | b'Y' | b'L' => Region::Europe,
        b'U' => Region::Australia,
        b'A' => Region::World,
        b'B' => Region::Brazil,
        b'K' => Region::Korea,
        b'C' => Region::China,
        _ => Region::Unknown,
    }
}

fn region_suffix(region: &Region) -> &'static str {
    match region {
        Region::Usa => "USA",
        Region::Japan => "JPN",
        Region::Europe => "EUR",
        Region::Australia => "AUS",
        Region::World => "ALL",
        Region::Brazil => "BRA",
        Region::Korea => "KOR",
        Region::China => "CHN",
        _ => "UNK",
    }
}

fn build_serial(header: &N64Header, region: &Region) -> Option<String> {
    let cat = header.category_code;
    let id0 = header.game_id[0];
    let id1 = header.game_id[1];
    let dest = header.destination_code;

    if cat < 0x20 || cat >= 0x7F || id0 < 0x20 || id0 >= 0x7F || id1 < 0x20 || id1 >= 0x7F {
        return None;
    }

    Some(format!(
        "NUS-{}{}{}{}-{}",
        cat as char,
        id0 as char,
        id1 as char,
        dest as char,
        region_suffix(region)
    ))
}

// ---------------------------------------------------------------------------
// Identification builder
// ---------------------------------------------------------------------------

fn to_identification(
    header: &N64Header,
    file_size: u64,
    crc_result: Option<(u32, u32)>,
) -> RomIdentification {
    let mut id = RomIdentification::new().with_platform("Nintendo 64");

    if !header.title.is_empty() {
        id.internal_name = Some(header.title.clone());
    }

    let region = region_from_destination(header.destination_code);
    if region != Region::Unknown {
        id.regions.push(region);
    }

    if let Some(serial) = build_serial(header, &region) {
        id.serial_number = Some(serial);
    }

    id.version = Some(format!("v1.{}", header.rom_version));
    id.file_size = Some(file_size);

    id.extra
        .insert("format".into(), rom_format_name(header.format).into());
    id.extra.insert(
        "boot_address".into(),
        format!("0x{:08X}", header.boot_address),
    );
    id.extra
        .insert("clock_rate".into(), format!("0x{:08X}", header.clock_rate));
    id.extra.insert(
        "category_code".into(),
        if header.category_code >= 0x20 && header.category_code < 0x7F {
            format!("{}", header.category_code as char)
        } else {
            format!("0x{:02X}", header.category_code)
        },
    );
    id.extra.insert("cic".into(), header.cic.name().into());

    // Expected checksums (from header)
    let mut crc_bytes = Vec::with_capacity(8);
    crc_bytes.extend_from_slice(&header.crc1.to_be_bytes());
    crc_bytes.extend_from_slice(&header.crc2.to_be_bytes());
    id.expected_checksums.push(
        ExpectedChecksum::new(ChecksumAlgorithm::PlatformSpecific("N64 CRC".to_string()), crc_bytes)
            .with_description("CRC1+CRC2 from header (0x10-0x17)"),
    );

    // Checksum status
    let checksum_status = match crc_result {
        Some((computed_crc1, computed_crc2)) => {
            let crc1_ok = computed_crc1 == header.crc1;
            let crc2_ok = computed_crc2 == header.crc2;
            match (crc1_ok, crc2_ok) {
                (true, true) => "OK".into(),
                (false, true) => format!(
                    "CRC1 MISMATCH (header={:08X}, computed={:08X})",
                    header.crc1, computed_crc1
                ),
                (true, false) => format!(
                    "CRC2 MISMATCH (header={:08X}, computed={:08X})",
                    header.crc2, computed_crc2
                ),
                (false, false) => format!(
                    "CRC1 MISMATCH (header={:08X}, computed={:08X}); \
                     CRC2 MISMATCH (header={:08X}, computed={:08X})",
                    header.crc1, computed_crc1, header.crc2, computed_crc2
                ),
            }
        }
        None => "SKIPPED (quick mode or file too small for CRC range)".into(),
    };
    id.extra
        .insert("checksum_status:N64 CRC".into(), checksum_status);

    id
}

// ---------------------------------------------------------------------------
// Analyzer implementation
// ---------------------------------------------------------------------------

/// Analyzer for Nintendo 64 ROMs.
#[derive(Debug, Default)]
pub struct N64Analyzer;

impl N64Analyzer {
    pub fn new() -> Self {
        Self
    }
}

impl RomAnalyzer for N64Analyzer {
    fn analyze(
        &self,
        reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        if file_size < BOOT_CODE_END {
            return Err(AnalysisError::TooSmall {
                expected: BOOT_CODE_END,
                actual: file_size,
            });
        }

        let header = parse_header(reader)?;

        let crc_result = if file_size >= MIN_CRC_SIZE {
            Some(compute_n64_crc(reader, header.format, header.cic)?)
        } else {
            None
        };

        Ok(to_identification(&header, file_size, crc_result))
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
        Platform::N64
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["z64", "n64", "v64"]
    }

    fn can_handle(&self, reader: &mut dyn ReadSeek) -> bool {
        let file_size = match reader.seek(SeekFrom::End(0)) {
            Ok(s) => s,
            Err(_) => return false,
        };
        if reader.seek(SeekFrom::Start(0)).is_err() {
            return false;
        }
        if file_size < HEADER_SIZE {
            return false;
        }

        let mut magic = [0u8; 4];
        if reader.read_exact(&mut magic).is_err() {
            let _ = reader.seek(SeekFrom::Start(0));
            return false;
        }
        let _ = reader.seek(SeekFrom::Start(0));

        detect_n64_format(&magic).is_some()
    }

    fn dat_names(&self) -> &'static [&'static str] {
        &["Nintendo - Nintendo 64"]
    }

    fn gdb_csv_names(&self) -> &'static [&'static str] {
        &["console_nintendo_nintendo64"]
    }

    fn dat_chunk_normalizer(
        &self,
        reader: &mut dyn ReadSeek,
        header_offset: u64,
    ) -> Result<Option<Box<dyn FnMut(&mut [u8])>>, AnalysisError> {
        reader.seek(SeekFrom::Start(header_offset))?;
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        reader.seek(SeekFrom::Start(header_offset))?;

        match detect_n64_format(&magic) {
            Some(N64Format::Z64) | None => Ok(None),
            Some(fmt) => Ok(Some(Box::new(move |buf: &mut [u8]| {
                normalize_to_big_endian(buf, fmt);
            }))),
        }
    }

    fn expects_serial(&self) -> bool {
        true
    }

    fn extract_dat_game_code(&self, serial: &str) -> Option<String> {
        // NUS-XXXX-YYY → XXXX
        let parts: Vec<&str> = serial.split('-').collect();
        if parts.len() >= 3 && parts[0] == "NUS" {
            Some(parts[1].to_string())
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "tests/n64_tests.rs"]
mod tests;
