//! Shared helpers for 3DS ROM analysis.
//!
//! Byte reading, ASCII extraction, alignment, maker code lookup, region
//! detection, title ID formatting, media type names, content type decoding,
//! origin heuristics, and SHA-256 verification.

use retro_junk_core::Region;
use sha2::{Digest, Sha256};
use std::io::SeekFrom;

pub(crate) use retro_junk_core::util::read_ascii;
use retro_junk_core::{
    AnalysisError, ChecksumAlgorithm, ExpectedChecksum, ReadSeek, RomIdentification,
};

// ---------------------------------------------------------------------------
// Byte reading helpers
// ---------------------------------------------------------------------------

pub(crate) fn read_u16_le(buf: &[u8], offset: usize) -> Option<u16> {
    buf.get(offset..offset + 2)
        .map(|s| u16::from_le_bytes(s.try_into().unwrap()))
}

pub(crate) fn read_u32_le(buf: &[u8], offset: usize) -> Option<u32> {
    buf.get(offset..offset + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

pub(crate) fn read_u64_le(buf: &[u8], offset: usize) -> Option<u64> {
    buf.get(offset..offset + 8)
        .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
}

pub(crate) fn read_u16_be(buf: &[u8], offset: usize) -> Option<u16> {
    buf.get(offset..offset + 2)
        .map(|s| u16::from_be_bytes(s.try_into().unwrap()))
}

pub(crate) fn read_u32_be(buf: &[u8], offset: usize) -> Option<u32> {
    buf.get(offset..offset + 4)
        .map(|s| u32::from_be_bytes(s.try_into().unwrap()))
}

pub(crate) fn read_u64_be(buf: &[u8], offset: usize) -> Option<u64> {
    buf.get(offset..offset + 8)
        .map(|s| u64::from_be_bytes(s.try_into().unwrap()))
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
        // P = Europe, D = Germany, F = France, S = Spain, I = Italy, U = Australia (PAL)
        Some('P' | 'D' | 'F' | 'S' | 'I' | 'U') => vec![Region::Europe],
        Some('K') => vec![Region::Korea],
        Some('C') => vec![Region::China],
        // W = World, A = Region-free
        Some('W' | 'A') => vec![Region::World],
        _ => vec![],
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
    format!("{high:08X}{low:08X}")
}

/// Extract the title type from the high 32 bits of a title ID.
pub(crate) fn title_type_from_id(tid: u64) -> &'static str {
    let high = (tid >> 32) as u32;
    match high {
        0x0004_0000 => "Application",
        0x0004_0001 => "System Application",
        0x0004_0002 => "System Data Archive",
        0x0004_0003 => "Shared Data Archive",
        0x0004_0004 => "System Firmware",
        0x0004_0010 => "Application (TWL)",
        0x0004_000E => "Patch/Update",
        0x0004_008C => "DLC",
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
    } else if ncsd.writable_address == 0xFFFF_FFFF && ncsd.media_type == 1 {
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
pub(crate) enum HashResult {
    /// Hash matches.
    Ok,
    /// Hash does not match.
    Mismatch { expected: String, actual: String },
    /// Region is empty (size 0), hash not checked.
    Empty,
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

/// Record the decoded title version (`vMajor.Minor.Micro`) and its raw value.
pub(crate) fn record_title_version(id: &mut RomIdentification, title_version: u16) {
    if title_version > 0 {
        let major = title_version >> 10;
        let minor = (title_version >> 4) & 0x3F;
        let micro = title_version & 0xF;
        id.version = format!("v{major}.{minor}.{micro}");
        id.extra
            .insert("title_version_raw".into(), format!("{title_version}"));
    } else {
        id.version = "v0".into();
    }
}

/// Record identification fields shared by every NCCH-bearing container
/// (CCI and CIA): product code, maker code, regions, content type,
/// encryption status, and ExeFS/RomFS sizes.
pub(crate) fn record_ncch_common(id: &mut RomIdentification, ncch: &super::ncch::NcchHeader) {
    // Product code -> serial number
    if !ncch.product_code.is_empty() {
        id.serial_number.clone_from(&ncch.product_code);
        id.extra
            .insert("product_code".into(), ncch.product_code.clone());
    }

    // Maker code
    if !ncch.maker_code.is_empty() {
        id.maker_code = crate::licensee::maker_code_name(&ncch.maker_code)
            .map_or_else(|| ncch.maker_code.clone(), std::string::ToString::to_string);
        id.extra
            .insert("maker_code_raw".into(), ncch.maker_code.clone());
    }

    // Regions from product code
    id.regions = region_from_product_code(&ncch.product_code);

    // Content type
    id.extra.insert(
        "content_type".into(),
        content_type_description(ncch.content_type_flags).into(),
    );

    // Encryption status
    if ncch.no_crypto {
        id.extra
            .insert("encryption".into(), "None (NoCrypto)".into());
    } else {
        let crypto_desc = match ncch.crypto_method {
            0x00 => "Original (pre-7.0)",
            0x01 => "7.0.0+",
            0x0A => "9.3.0+ (New 3DS)",
            0x0B => "9.6.0+ (New 3DS)",
            _ => "Unknown",
        };
        id.extra
            .insert("encryption".into(), format!("Encrypted ({crypto_desc})"));
    }

    // ExeFS / RomFS presence
    if ncch.exefs_size_mu > 0 {
        id.extra.insert(
            "exefs_size".into(),
            format!(
                "{} KB",
                u64::from(ncch.exefs_size_mu) * super::MEDIA_UNIT / 1024
            ),
        );
    }
    if ncch.romfs_size_mu > 0 {
        id.extra.insert(
            "romfs_size".into(),
            format!(
                "{} MB",
                u64::from(ncch.romfs_size_mu) * super::MEDIA_UNIT / (1024 * 1024)
            ),
        );
    }
}

/// Verify a SHA-256 region and record the outcome on `id`: sets
/// `checksum_status:<label>` in `extra` and pushes the expected checksum
/// (unless the region is empty / the stored hash is all zeros).
pub(crate) fn record_sha256_check(
    id: &mut RomIdentification,
    reader: &mut dyn ReadSeek,
    offset: u64,
    size: u64,
    expected_hash: &[u8; 32],
    label: &str,
) -> Result<(), AnalysisError> {
    match verify_sha256(reader, offset, size, expected_hash)? {
        HashResult::Ok => {
            id.extra
                .insert(format!("checksum_status:{label}"), "OK".into());
        }
        HashResult::Mismatch { expected, actual } => {
            id.extra.insert(
                format!("checksum_status:{label}"),
                format!("MISMATCH (expected {expected}, got {actual})"),
            );
        }
        HashResult::Empty => return Ok(()),
    }
    id.expected_checksums.push(
        ExpectedChecksum::new(ChecksumAlgorithm::Sha256, expected_hash.to_vec())
            .with_description(label),
    );
    Ok(())
}

pub(crate) fn hex_string(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

#[cfg(test)]
#[path = "tests/common_tests.rs"]
mod tests;
