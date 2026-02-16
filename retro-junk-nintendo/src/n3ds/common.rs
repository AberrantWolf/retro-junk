//! Shared helpers for 3DS ROM analysis.
//!
//! Byte reading, ASCII extraction, alignment, maker code lookup, region
//! detection, title ID formatting, media type names, content type decoding,
//! origin heuristics, and SHA-256 verification.

use retro_junk_core::Region;
use sha2::{Digest, Sha256};
use std::io::SeekFrom;

use retro_junk_core::{AnalysisError, ReadSeek};

// ---------------------------------------------------------------------------
// Byte reading helpers
// ---------------------------------------------------------------------------

pub(crate) fn read_u16_le(buf: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([buf[offset], buf[offset + 1]])
}

pub(crate) fn read_u32_le(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

pub(crate) fn read_u64_le(buf: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
        buf[offset + 4],
        buf[offset + 5],
        buf[offset + 6],
        buf[offset + 7],
    ])
}

pub(crate) fn read_u16_be(buf: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([buf[offset], buf[offset + 1]])
}

pub(crate) fn read_u32_be(buf: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

pub(crate) fn read_u64_be(buf: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
        buf[offset + 4],
        buf[offset + 5],
        buf[offset + 6],
        buf[offset + 7],
    ])
}

/// Read a null-terminated ASCII string from a byte slice, filtering to printable chars.
pub(crate) fn read_ascii(buf: &[u8]) -> String {
    buf.iter()
        .take_while(|&&b| b != 0)
        .filter(|&&b| b >= 0x20 && b < 0x7F)
        .map(|&b| b as char)
        .collect()
}

/// Align a value up to a 64-byte boundary.
pub(crate) fn align64(val: u64) -> u64 {
    (val + 63) & !63
}

/// Check if a byte slice is all zeros.
pub(crate) fn is_all_zeros(buf: &[u8]) -> bool {
    buf.iter().all(|&b| b == 0)
}

// ---------------------------------------------------------------------------
// Region detection
// ---------------------------------------------------------------------------

/// Derive region from the last character of a product code like "CTR-P-ABCE".
pub(crate) fn region_from_product_code(product_code: &str) -> Vec<Region> {
    // The game ID is the last 4 chars; region is the last char of that
    let region_char = if product_code.contains('-') {
        // Format: CTR-P-ABCE -> last char 'E'
        product_code.chars().last()
    } else if product_code.len() >= 4 {
        // Just a raw code like "ABCE"
        product_code.chars().last()
    } else {
        None
    };

    match region_char {
        Some('J') => vec![Region::Japan],
        Some('E') => vec![Region::Usa],
        Some('P') => vec![Region::Europe],
        Some('D') => vec![Region::Europe], // Germany
        Some('F') => vec![Region::Europe], // France
        Some('S') => vec![Region::Europe], // Spain
        Some('I') => vec![Region::Europe], // Italy
        Some('U') => vec![Region::Europe], // Australia (PAL)
        Some('K') => vec![Region::Korea],
        Some('C') => vec![Region::China],
        Some('W') => vec![Region::World],
        Some('A') => vec![Region::World], // Region-free
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Maker code lookup
// ---------------------------------------------------------------------------

pub(crate) fn maker_code_name(code: &str) -> Option<&'static str> {
    match code {
        "00" => Some("None"),
        "01" => Some("Nintendo R&D1"),
        "08" => Some("Capcom"),
        "13" => Some("EA (Electronic Arts)"),
        "18" => Some("Hudson Soft"),
        "20" => Some("kss"),
        "24" => Some("PCM Complete"),
        "28" => Some("Kemco Japan"),
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
        "49" => Some("irem"),
        "50" => Some("Absolute"),
        "51" => Some("Acclaim"),
        "52" => Some("Activision"),
        "53" => Some("American sammy"),
        "54" => Some("Konami"),
        "56" => Some("LJN"),
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
        "75" => Some("sci"),
        "78" => Some("THQ"),
        "79" => Some("Accolade"),
        "86" => Some("Tokuma Shoten"),
        "91" => Some("Chunsoft"),
        "92" => Some("Video system"),
        "97" => Some("Kaneko"),
        "A4" => Some("Konami (Yu-Gi-Oh!)"),
        "GR" => Some("Grasshopper Manufacture"),
        "GT" => Some("GUST"),
        "HB" => Some("Happinet"),
        "KA" => Some("Kadokawa"),
        "MR" => Some("Marvelous"),
        "NB" => Some("Bandai Namco"),
        "QH" => Some("D3 Publisher"),
        "SQ" => Some("Square Enix"),
        "XB" => Some("XSEED"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Media type / platform names
// ---------------------------------------------------------------------------

pub(crate) fn media_type_name(media_type: u8) -> &'static str {
    match media_type {
        0 => "Inner Device",
        1 => "Card1",
        2 => "Card2",
        3 => "Extended Device",
        _ => "Unknown",
    }
}

pub(crate) fn media_platform_name(platform: u8) -> &'static str {
    match platform {
        1 => "Old 3DS (CTR)",
        2 => "New 3DS",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------------------
// Content type decoding
// ---------------------------------------------------------------------------

pub(crate) fn content_type_description(flags: u8) -> &'static str {
    let form_type = flags & 0x03;
    let content_category = (flags >> 2) & 0x3F;

    match (form_type, content_category) {
        (1, 0) => "Simple content",
        (2, 0) => "Executable (no RomFS)",
        (3, 0) => "Executable",
        (_, 1) => "System update",
        (_, 2) => "Manual",
        (_, 3) => "Download Play child",
        (_, 4) => "Trial",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------------------
// Title ID formatting
// ---------------------------------------------------------------------------

/// Format a 3DS title ID as a hex string with high/low halves separated.
pub(crate) fn format_title_id(tid: u64) -> String {
    let high = (tid >> 32) as u32;
    let low = tid as u32;
    format!("{:08X}{:08X}", high, low)
}

/// Extract the title type from the high 32 bits of a title ID.
pub(crate) fn title_type_from_id(tid: u64) -> &'static str {
    let high = (tid >> 32) as u32;
    match high {
        0x00040000 => "Application",
        0x00040001 => "System Application",
        0x00040002 => "System Data Archive",
        0x00040003 => "Shared Data Archive",
        0x00040004 => "System Firmware",
        0x00040010 => "Application (TWL)",
        0x0004000E => "Patch/Update",
        0x0004008C => "DLC",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------------------
// Origin detection
// ---------------------------------------------------------------------------

use super::ncsd::NcsdHeader;

/// Heuristic determination of whether a CCI originated from a physical game card
/// or was converted from a CIA/digital title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CciOrigin {
    /// Likely authentic game card dump.
    GameCard,
    /// Likely converted from CIA / digital origin.
    Digital,
    /// Cannot determine with confidence.
    Uncertain,
}

pub(crate) fn detect_cci_origin(ncsd: &NcsdHeader) -> CciOrigin {
    let mut card_score: i32 = 0;
    let mut digital_score: i32 = 0;

    // Card seed: zeros -> digital origin (strong signal)
    if ncsd.card_seed_is_zero {
        digital_score += 3;
    } else {
        card_score += 3;
    }

    // RSA signature: zeros -> not authentic
    if ncsd.signature_is_zero {
        digital_score += 2;
    } else {
        card_score += 2;
    }

    // Media type: Inner Device (0) -> digital
    match ncsd.media_type {
        0 => digital_score += 2,
        1 | 2 => card_score += 1,
        _ => {}
    }

    // Writable address: 0x00000000 is suspicious for a real card
    // Card1 should be 0xFFFFFFFF, Card2 should be non-zero
    if ncsd.writable_address == 0 && ncsd.media_type != 2 {
        digital_score += 1;
    } else if ncsd.writable_address == 0xFFFFFFFF && ncsd.media_type == 1 {
        card_score += 1;
    }

    // Count non-empty partitions: game cards typically have 2+
    let partition_count = ncsd.partitions.iter().filter(|p| p.1 > 0).count();
    if partition_count >= 3 {
        card_score += 1;
    } else if partition_count <= 1 {
        digital_score += 1;
    }

    if card_score > digital_score + 2 {
        CciOrigin::GameCard
    } else if digital_score > card_score + 2 {
        CciOrigin::Digital
    } else if digital_score > card_score {
        CciOrigin::Digital
    } else if card_score > digital_score {
        CciOrigin::GameCard
    } else {
        CciOrigin::Uncertain
    }
}

// ---------------------------------------------------------------------------
// SHA-256 verification
// ---------------------------------------------------------------------------

/// Result of a SHA-256 hash verification.
#[allow(dead_code)]
pub(crate) enum HashResult {
    /// Hash matches.
    Ok,
    /// Hash does not match.
    Mismatch { expected: String, actual: String },
    /// Region is empty (size 0), hash not checked.
    Empty,
    /// Content is encrypted, cannot verify.
    Encrypted,
    /// Skipped (quick mode or other reason).
    Skipped,
}

/// Verify a SHA-256 hash by reading `size` bytes from `offset`.
pub(crate) fn verify_sha256(
    reader: &mut dyn ReadSeek,
    offset: u64,
    size: u64,
    expected: &[u8; 32],
) -> Result<HashResult, AnalysisError> {
    if size == 0 {
        return Ok(HashResult::Empty);
    }
    if is_all_zeros(expected) {
        return Ok(HashResult::Empty);
    }

    reader.seek(SeekFrom::Start(offset))?;
    let mut hasher = Sha256::new();
    let mut remaining = size;
    let mut buf = vec![0u8; 0x10000]; // 64 KB read buffer

    while remaining > 0 {
        let to_read = remaining.min(buf.len() as u64) as usize;
        reader.read_exact(&mut buf[..to_read]).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                AnalysisError::corrupted_header("Data truncated during hash verification")
            } else {
                AnalysisError::Io(e)
            }
        })?;
        hasher.update(&buf[..to_read]);
        remaining -= to_read as u64;
    }

    let actual = hasher.finalize();
    if actual.as_slice() == expected {
        Ok(HashResult::Ok)
    } else {
        Ok(HashResult::Mismatch {
            expected: hex_string(expected),
            actual: hex_string(actual.as_slice()),
        })
    }
}

pub(crate) fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
#[path = "tests/common_tests.rs"]
mod tests;
