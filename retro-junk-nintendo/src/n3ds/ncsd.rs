//! NCSD (CCI) header parsing and analysis for Nintendo 3DS game card dumps.

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;

use retro_junk_core::{
    AnalysisError, AnalysisOptions, ChecksumAlgorithm, ExpectedChecksum, RomIdentification,
};

use super::common::*;
use super::ncch::parse_ncch_header;
use super::{CARD_SEED_SIZE, MEDIA_UNIT, MIN_CCI_SIZE, NCSD_MAGIC};

// ---------------------------------------------------------------------------
// NCSD header
// ---------------------------------------------------------------------------

/// Parsed NCSD header fields.
#[allow(dead_code)]
pub(crate) struct NcsdHeader {
    pub(crate) image_size_mu: u32,
    pub(crate) media_id: u64,
    /// Partition table: (offset_mu, size_mu) for each of 8 partitions.
    pub(crate) partitions: [(u32, u32); 8],
    /// Partition flags byte 5: media type index.
    pub(crate) media_type: u8,
    /// Partition flags byte 4: media platform.
    pub(crate) media_platform: u8,
    /// Card info: writable address at 0x200.
    pub(crate) writable_address: u32,
    /// Card info: title version at 0x310.
    pub(crate) title_version: u16,
    /// Card info: card revision at 0x312.
    pub(crate) card_revision: u16,
    /// Whether the RSA signature (0x000-0x0FF) is all zeros.
    pub(crate) signature_is_zero: bool,
    /// Whether the card seed (0x1000, 16 bytes) is all zeros.
    pub(crate) card_seed_is_zero: bool,
    /// Filled size in bytes from NCSD offset 0x300 (actual content size, not in
    /// media units). Zero if the field is absent or unpopulated.
    pub(crate) filled_size: u64,
}

pub(crate) fn parse_ncsd_header(reader: &mut dyn ReadSeek) -> Result<NcsdHeader, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    // Read first 0x400 bytes for the NCSD header + card info header
    let mut buf = [0u8; 0x400];
    reader.read_exact(&mut buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            AnalysisError::TooSmall {
                expected: MIN_CCI_SIZE,
                actual: 0,
            }
        } else {
            AnalysisError::Io(e)
        }
    })?;

    // Verify magic
    if buf[0x100..0x104] != NCSD_MAGIC {
        return Err(AnalysisError::invalid_format("Missing NCSD magic at 0x100"));
    }

    let signature_is_zero = is_all_zeros(&buf[0x000..0x100]);

    let image_size_mu = read_u32_le(&buf, 0x104);
    let media_id = read_u64_le(&buf, 0x108);

    // Partition table at 0x120: 8 entries of (u32 offset, u32 size)
    let mut partitions = [(0u32, 0u32); 8];
    for (i, partition) in partitions.iter_mut().enumerate() {
        let base = 0x120 + i * 8;
        *partition = (read_u32_le(&buf, base), read_u32_le(&buf, base + 4));
    }

    // Partition flags at 0x188
    let media_platform = buf[0x188 + 4];
    let media_type = buf[0x188 + 5];

    // Card info header at 0x200
    let writable_address = read_u32_le(&buf, 0x200);
    let title_version = read_u16_le(&buf, 0x310);
    let card_revision = read_u16_le(&buf, 0x312);

    // Filled size at NCSD offset 0x300: this is a 4-byte LE value representing
    // the actual used content size in bytes (NOT in media units). Used by
    // trimming tools to know where real data ends.
    let filled_size_raw = read_u32_le(&buf, 0x300);
    let filled_size = filled_size_raw as u64;

    // Card seed at 0x1000
    let card_seed_is_zero = {
        reader.seek(SeekFrom::Start(0x1000))?;
        let mut seed = [0u8; CARD_SEED_SIZE];
        match reader.read_exact(&mut seed) {
            Ok(()) => is_all_zeros(&seed),
            Err(_) => true, // If we can't read it, treat as zero
        }
    };

    Ok(NcsdHeader {
        image_size_mu,
        media_id,
        partitions,
        media_type,
        media_platform,
        writable_address,
        title_version,
        card_revision,
        signature_is_zero,
        card_seed_is_zero,
        filled_size,
    })
}

// ---------------------------------------------------------------------------
// CCI analysis
// ---------------------------------------------------------------------------

pub(crate) fn analyze_cci(
    reader: &mut dyn ReadSeek,
    file_size: u64,
    options: &AnalysisOptions,
) -> Result<RomIdentification, AnalysisError> {
    let ncsd = parse_ncsd_header(reader)?;

    // Partition 0 must exist
    if ncsd.partitions[0].1 == 0 {
        return Err(AnalysisError::invalid_format(
            "NCSD partition 0 has zero size",
        ));
    }

    let partition0_offset = ncsd.partitions[0].0 as u64 * MEDIA_UNIT;
    let ncch = parse_ncch_header(reader, partition0_offset)?;

    let mut id = RomIdentification::new().with_platform("Nintendo 3DS");

    // Format
    id.extra.insert("format".into(), "CCI (NCSD)".into());

    // Product code -> serial number
    if !ncch.product_code.is_empty() {
        id.serial_number = Some(ncch.product_code.clone());
        id.extra
            .insert("product_code".into(), ncch.product_code.clone());
    }

    // Maker code
    if !ncch.maker_code.is_empty() {
        id.maker_code = maker_code_name(&ncch.maker_code)
            .map(|s| s.to_string())
            .or_else(|| Some(ncch.maker_code.clone()));
        id.extra
            .insert("maker_code_raw".into(), ncch.maker_code.clone());
    }

    // Program ID / Title ID
    if ncch.program_id != 0 {
        id.extra
            .insert("title_id".into(), format_title_id(ncch.program_id));
        id.extra.insert(
            "title_type".into(),
            title_type_from_id(ncch.program_id).into(),
        );
    }

    // Regions from product code
    let regions = region_from_product_code(&ncch.product_code);
    id.regions = regions;

    // Title version from card info header
    if ncsd.title_version > 0 {
        let major = ncsd.title_version >> 10;
        let minor = (ncsd.title_version >> 4) & 0x3F;
        let micro = ncsd.title_version & 0xF;
        id.version = Some(format!("v{}.{}.{}", major, minor, micro));
        id.extra.insert(
            "title_version_raw".into(),
            format!("{}", ncsd.title_version),
        );
    } else {
        id.version = Some("v0".into());
    }

    // File size and trimming detection
    //
    // 3DS CCI files are commonly "trimmed": the unused padding between the
    // actual content (`filled_size` at NCSD offset 0x300) and the full game
    // card capacity (`image_size_mu * MEDIA_UNIT`) is stripped. Both trimmed
    // and untrimmed dumps are valid. Only files smaller than `filled_size`
    // are genuinely truncated.
    id.file_size = Some(file_size);
    let image_size = ncsd.image_size_mu as u64 * MEDIA_UNIT;
    let used_size = ncsd.filled_size;

    if used_size > 0 && image_size > 0 {
        if file_size >= used_size && file_size <= image_size {
            // File is between used_size and image_size — perfectly valid
            // (trimmed, partially trimmed, or full dump). Set expected = actual
            // so the CLI size verdict shows OK.
            id.expected_size = Some(file_size);

            if file_size == used_size {
                id.extra.insert("dump_status".into(), "Trimmed".into());
            } else if file_size == image_size {
                id.extra.insert("dump_status".into(), "Untrimmed".into());
            } else {
                id.extra
                    .insert("dump_status".into(), "Partially trimmed".into());
            }
        } else if file_size < used_size {
            // file_size < used_size -> genuinely truncated
            id.expected_size = Some(used_size);
        } else {
            // file_size > image_size -> oversized (shouldn't happen normally)
            id.expected_size = Some(image_size);
        }
    } else if image_size > 0 {
        // No filled_size available; fall back to image_size
        id.expected_size = Some(image_size);
    }

    // Media type
    id.extra
        .insert("media_type".into(), media_type_name(ncsd.media_type).into());

    // Media platform
    if ncsd.media_platform > 0 {
        id.extra.insert(
            "media_platform".into(),
            media_platform_name(ncsd.media_platform).into(),
        );
    }

    // Card type
    match ncsd.media_type {
        1 => {
            id.extra
                .insert("card_type".into(), "Card1 (external save)".into());
        }
        2 => {
            id.extra
                .insert("card_type".into(), "Card2 (on-card save)".into());
            if ncsd.writable_address != 0 && ncsd.writable_address != 0xFFFFFFFF {
                id.extra.insert(
                    "save_offset".into(),
                    format!("0x{:08X}", ncsd.writable_address as u64 * MEDIA_UNIT),
                );
            }
        }
        _ => {}
    }

    // Card revision
    if ncsd.card_revision > 0 {
        id.extra
            .insert("card_revision".into(), format!("{}", ncsd.card_revision));
    }

    // Partition layout
    let partition_count = ncsd.partitions.iter().filter(|p| p.1 > 0).count();
    id.extra
        .insert("partition_count".into(), format!("{}", partition_count));

    // Partition details
    let partition_names = [
        "Main CXI",
        "Manual",
        "Download Play",
        "Partition 3",
        "Partition 4",
        "Partition 5",
        "N3DS Update",
        "Update",
    ];
    for (i, &(off, sz)) in ncsd.partitions.iter().enumerate() {
        if sz > 0 {
            id.extra.insert(
                format!("partition_{}", i),
                format!(
                    "{}: offset 0x{:X}, size {} KB",
                    partition_names[i],
                    off as u64 * MEDIA_UNIT,
                    sz as u64 * MEDIA_UNIT / 1024
                ),
            );
        }
    }

    // NCCH content info
    id.extra.insert(
        "ncch_content_size".into(),
        format!("{} KB", ncch.content_size_mu as u64 * MEDIA_UNIT / 1024),
    );
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
            .insert("encryption".into(), format!("Encrypted ({})", crypto_desc));
    }

    // ExeFS / RomFS presence
    if ncch.exefs_size_mu > 0 {
        id.extra.insert(
            "exefs_size".into(),
            format!("{} KB", ncch.exefs_size_mu as u64 * MEDIA_UNIT / 1024),
        );
    }
    if ncch.romfs_size_mu > 0 {
        id.extra.insert(
            "romfs_size".into(),
            format!(
                "{} MB",
                ncch.romfs_size_mu as u64 * MEDIA_UNIT / (1024 * 1024)
            ),
        );
    }

    // Origin detection (game card vs digital) — heuristic, not definitive
    let origin = detect_cci_origin(&ncsd);
    let origin_str = match origin {
        CciOrigin::GameCard => "Game card dump (likely)",
        CciOrigin::Digital => "Converted from digital/CIA (likely)",
        CciOrigin::Uncertain => "Uncertain (see evidence)",
    };
    id.extra.insert("origin".into(), origin_str.into());

    // Detail the heuristic evidence
    let mut origin_evidence = Vec::new();
    if ncsd.card_seed_is_zero {
        origin_evidence.push("card seed: zeros (digital)");
    } else {
        origin_evidence.push("card seed: present (card)");
    }
    if ncsd.signature_is_zero {
        origin_evidence.push("RSA signature: zeros (digital)");
    } else {
        origin_evidence.push("RSA signature: present (card)");
    }
    match ncsd.media_type {
        0 => origin_evidence.push("media type: Inner Device (digital)"),
        1 => origin_evidence.push("media type: Card1 (card)"),
        2 => origin_evidence.push("media type: Card2 (card)"),
        _ => {}
    }
    id.extra
        .insert("origin_evidence".into(), origin_evidence.join("; "));

    // SHA-256 hash verification (only if not encrypted and not quick mode)
    if !options.quick && ncch.no_crypto {
        // ExHeader hash
        if ncch.exheader_size > 0 {
            let exheader_offset = partition0_offset + 0x200;
            let hash_size = 0x400u64.min(ncch.exheader_size as u64);
            match verify_sha256(reader, exheader_offset, hash_size, &ncch.exheader_hash)? {
                HashResult::Ok => {
                    id.extra
                        .insert("checksum_status:ExHeader SHA-256".into(), "OK".into());
                    id.expected_checksums.push(
                        ExpectedChecksum::new(
                            ChecksumAlgorithm::Sha256,
                            ncch.exheader_hash.to_vec(),
                        )
                        .with_description("ExHeader SHA-256"),
                    );
                }
                HashResult::Mismatch { expected, actual } => {
                    id.extra.insert(
                        "checksum_status:ExHeader SHA-256".into(),
                        format!("MISMATCH (expected {}, got {})", expected, actual),
                    );
                    id.expected_checksums.push(
                        ExpectedChecksum::new(
                            ChecksumAlgorithm::Sha256,
                            ncch.exheader_hash.to_vec(),
                        )
                        .with_description("ExHeader SHA-256"),
                    );
                }
                _ => {}
            }
        }

        // ExeFS superblock hash
        if ncch.exefs_size_mu > 0 && ncch.exefs_hash_region_size_mu > 0 {
            let exefs_offset = partition0_offset + ncch.exefs_offset_mu as u64 * MEDIA_UNIT;
            let hash_region_size = ncch.exefs_hash_region_size_mu as u64 * MEDIA_UNIT;
            match verify_sha256(
                reader,
                exefs_offset,
                hash_region_size,
                &ncch.exefs_superblock_hash,
            )? {
                HashResult::Ok => {
                    id.extra.insert(
                        "checksum_status:ExeFS Superblock SHA-256".into(),
                        "OK".into(),
                    );
                    id.expected_checksums.push(
                        ExpectedChecksum::new(
                            ChecksumAlgorithm::Sha256,
                            ncch.exefs_superblock_hash.to_vec(),
                        )
                        .with_description("ExeFS Superblock SHA-256"),
                    );
                }
                HashResult::Mismatch { expected, actual } => {
                    id.extra.insert(
                        "checksum_status:ExeFS Superblock SHA-256".into(),
                        format!("MISMATCH (expected {}, got {})", expected, actual),
                    );
                    id.expected_checksums.push(
                        ExpectedChecksum::new(
                            ChecksumAlgorithm::Sha256,
                            ncch.exefs_superblock_hash.to_vec(),
                        )
                        .with_description("ExeFS Superblock SHA-256"),
                    );
                }
                _ => {}
            }
        }

        // RomFS superblock hash
        if ncch.romfs_size_mu > 0 && ncch.romfs_hash_region_size_mu > 0 {
            let romfs_offset = partition0_offset + ncch.romfs_offset_mu as u64 * MEDIA_UNIT;
            let hash_region_size = ncch.romfs_hash_region_size_mu as u64 * MEDIA_UNIT;
            match verify_sha256(
                reader,
                romfs_offset,
                hash_region_size,
                &ncch.romfs_superblock_hash,
            )? {
                HashResult::Ok => {
                    id.extra.insert(
                        "checksum_status:RomFS Superblock SHA-256".into(),
                        "OK".into(),
                    );
                    id.expected_checksums.push(
                        ExpectedChecksum::new(
                            ChecksumAlgorithm::Sha256,
                            ncch.romfs_superblock_hash.to_vec(),
                        )
                        .with_description("RomFS Superblock SHA-256"),
                    );
                }
                HashResult::Mismatch { expected, actual } => {
                    id.extra.insert(
                        "checksum_status:RomFS Superblock SHA-256".into(),
                        format!("MISMATCH (expected {}, got {})", expected, actual),
                    );
                    id.expected_checksums.push(
                        ExpectedChecksum::new(
                            ChecksumAlgorithm::Sha256,
                            ncch.romfs_superblock_hash.to_vec(),
                        )
                        .with_description("RomFS Superblock SHA-256"),
                    );
                }
                _ => {}
            }
        }
    } else if !ncch.no_crypto && !options.quick {
        id.extra.insert(
            "checksum_note".into(),
            "Content is encrypted; SHA-256 hashes cannot be verified without decryption keys"
                .into(),
        );
    }

    Ok(id)
}

#[cfg(test)]
#[path = "tests/ncsd_tests.rs"]
mod tests;
