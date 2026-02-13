//! CIA format parsing and analysis for Nintendo 3DS eShop/installable archives.

use retro_junk_lib::ReadSeek;
use std::io::SeekFrom;

use retro_junk_lib::{
    AnalysisError, AnalysisOptions, ChecksumAlgorithm, ExpectedChecksum, RomIdentification,
};

use super::common::*;
use super::ncch::parse_ncch_header;
use super::{CIA_HEADER_SIZE, MEDIA_UNIT};

// ---------------------------------------------------------------------------
// CIA header
// ---------------------------------------------------------------------------

/// Parsed CIA header.
pub(crate) struct CiaHeader {
    pub(crate) header_size: u32,
    pub(crate) cert_chain_size: u32,
    pub(crate) ticket_size: u32,
    pub(crate) tmd_size: u32,
    pub(crate) meta_size: u32,
    pub(crate) content_size: u64,
}

pub(crate) fn parse_cia_header(reader: &mut dyn ReadSeek) -> Result<CiaHeader, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut buf = [0u8; 0x20];
    reader.read_exact(&mut buf)?;

    let header_size = read_u32_le(&buf, 0x00);
    if header_size != CIA_HEADER_SIZE {
        return Err(AnalysisError::invalid_format(format!(
            "Unexpected CIA header size: 0x{:X}",
            header_size
        )));
    }

    Ok(CiaHeader {
        header_size,
        cert_chain_size: read_u32_le(&buf, 0x08),
        ticket_size: read_u32_le(&buf, 0x0C),
        tmd_size: read_u32_le(&buf, 0x10),
        meta_size: read_u32_le(&buf, 0x14),
        content_size: read_u64_le(&buf, 0x18),
    })
}

// ---------------------------------------------------------------------------
// TMD parsing
// ---------------------------------------------------------------------------

/// Information extracted from the CIA TMD.
pub(crate) struct CiaTmdInfo {
    pub(crate) title_id: u64,
    pub(crate) title_version: u16,
    pub(crate) content_count: u16,
}

/// Determine the size of a TMD/Ticket signature block based on signature type.
fn signature_block_size(sig_type: u32) -> Option<usize> {
    match sig_type {
        0x00010003 => Some(4 + 0x200 + 0x3C), // RSA-4096: type(4) + sig(512) + pad(60)
        0x00010004 => Some(4 + 0x100 + 0x3C), // RSA-2048: type(4) + sig(256) + pad(60)
        0x00010005 => Some(4 + 0x3C + 0x40),  // ECDSA: type(4) + sig(60) + pad(64)
        _ => None,
    }
}

/// Parse title information from the CIA's TMD section.
pub(crate) fn parse_cia_tmd(
    reader: &mut dyn ReadSeek,
    tmd_offset: u64,
    tmd_size: u32,
) -> Result<CiaTmdInfo, AnalysisError> {
    if tmd_size < 8 {
        return Err(AnalysisError::corrupted_header("TMD too small"));
    }

    // Read signature type to determine header offset
    reader.seek(SeekFrom::Start(tmd_offset))?;
    let mut sig_type_buf = [0u8; 4];
    reader.read_exact(&mut sig_type_buf)?;
    let sig_type = read_u32_be(&sig_type_buf, 0);

    let sig_block_size = signature_block_size(sig_type).ok_or_else(|| {
        AnalysisError::invalid_format(format!("Unknown TMD signature type: 0x{:08X}", sig_type))
    })?;

    // TMD header starts after signature block
    let tmd_header_offset = tmd_offset + sig_block_size as u64;
    reader.seek(SeekFrom::Start(tmd_header_offset))?;

    let mut tmd_buf = [0u8; 0xC4];
    reader.read_exact(&mut tmd_buf).map_err(|_| {
        AnalysisError::corrupted_header("TMD header truncated")
    })?;

    let title_id = read_u64_be(&tmd_buf, 0x4C);
    let title_version = read_u16_be(&tmd_buf, 0x9C);
    let content_count = read_u16_be(&tmd_buf, 0x9E);

    Ok(CiaTmdInfo {
        title_id,
        title_version,
        content_count,
    })
}

// ---------------------------------------------------------------------------
// Ticket parsing
// ---------------------------------------------------------------------------

/// Parse title ID from the CIA's Ticket section.
fn parse_cia_ticket_title_id(
    reader: &mut dyn ReadSeek,
    ticket_offset: u64,
) -> Result<u64, AnalysisError> {
    reader.seek(SeekFrom::Start(ticket_offset))?;
    let mut sig_type_buf = [0u8; 4];
    reader.read_exact(&mut sig_type_buf)?;
    let sig_type = read_u32_be(&sig_type_buf, 0);

    let sig_block_size = signature_block_size(sig_type).ok_or_else(|| {
        AnalysisError::invalid_format(format!(
            "Unknown Ticket signature type: 0x{:08X}",
            sig_type
        ))
    })?;

    let ticket_data_offset = ticket_offset + sig_block_size as u64;
    reader.seek(SeekFrom::Start(ticket_data_offset + 0x9C))?;
    let mut tid_buf = [0u8; 8];
    reader.read_exact(&mut tid_buf)?;
    Ok(read_u64_be(&tid_buf, 0))
}

// ---------------------------------------------------------------------------
// Section offset helpers
// ---------------------------------------------------------------------------

/// Calculate the offset of the content section within a CIA.
fn cia_content_offset(cia: &CiaHeader) -> u64 {
    let mut offset = align64(cia.header_size as u64);
    offset += align64(cia.cert_chain_size as u64);
    offset += align64(cia.ticket_size as u64);
    offset += align64(cia.tmd_size as u64);
    offset
}

/// Calculate the offset of the TMD section within a CIA.
fn cia_tmd_offset(cia: &CiaHeader) -> u64 {
    let mut offset = align64(cia.header_size as u64);
    offset += align64(cia.cert_chain_size as u64);
    offset += align64(cia.ticket_size as u64);
    offset
}

/// Calculate the offset of the Ticket section within a CIA.
fn cia_ticket_offset(cia: &CiaHeader) -> u64 {
    let mut offset = align64(cia.header_size as u64);
    offset += align64(cia.cert_chain_size as u64);
    offset
}

// ---------------------------------------------------------------------------
// CIA analysis
// ---------------------------------------------------------------------------

pub(crate) fn analyze_cia(
    reader: &mut dyn ReadSeek,
    file_size: u64,
    options: &AnalysisOptions,
) -> Result<RomIdentification, AnalysisError> {
    let cia = parse_cia_header(reader)?;

    let mut id = RomIdentification::new().with_platform("Nintendo 3DS");

    // Format
    id.extra.insert("format".into(), "CIA".into());
    id.file_size = Some(file_size);

    // Expected size from CIA sections
    let content_offset = cia_content_offset(&cia);
    let expected_size = content_offset + cia.content_size + if cia.meta_size > 0 {
        align64(cia.meta_size as u64)
    } else {
        0
    };
    // CIA files may have trailing alignment; accept anything >= content end
    let content_end = content_offset + cia.content_size;
    if file_size >= content_end {
        id.expected_size = Some(file_size); // OK, no truncation
    } else {
        id.expected_size = Some(expected_size);
    }

    // Parse TMD for title info
    let tmd_offset = cia_tmd_offset(&cia);
    let tmd_info = parse_cia_tmd(reader, tmd_offset, cia.tmd_size)?;

    // Title ID
    if tmd_info.title_id != 0 {
        id.extra
            .insert("title_id".into(), format_title_id(tmd_info.title_id));
        id.extra.insert(
            "title_type".into(),
            title_type_from_id(tmd_info.title_id).into(),
        );
    }

    // Title version
    if tmd_info.title_version > 0 {
        let major = tmd_info.title_version >> 10;
        let minor = (tmd_info.title_version >> 4) & 0x3F;
        let micro = tmd_info.title_version & 0xF;
        id.version = Some(format!("v{}.{}.{}", major, minor, micro));
        id.extra.insert(
            "title_version_raw".into(),
            format!("{}", tmd_info.title_version),
        );
    } else {
        id.version = Some("v0".into());
    }

    // Content count
    id.extra.insert(
        "content_count".into(),
        format!("{}", tmd_info.content_count),
    );

    // Parse ticket for title ID (cross-reference)
    let ticket_offset = cia_ticket_offset(&cia);
    if let Ok(ticket_tid) = parse_cia_ticket_title_id(reader, ticket_offset) {
        if ticket_tid != tmd_info.title_id && ticket_tid != 0 {
            id.extra.insert(
                "ticket_title_id".into(),
                format_title_id(ticket_tid),
            );
        }
    }

    // Try to parse NCCH from content section
    let ncch_result = parse_ncch_header(reader, content_offset);
    if let Ok(ncch) = ncch_result {
        // Product code
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

        // Program ID from NCCH (may differ from TMD title ID for updates/DLC)
        if ncch.program_id != 0 {
            id.extra
                .insert("program_id".into(), format_title_id(ncch.program_id));
        }

        // Regions
        let regions = region_from_product_code(&ncch.product_code);
        id.regions = regions;

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
                .insert("encryption".into(), format!("Encrypted ({})", crypto_desc));
        }

        // ExeFS / RomFS
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

        // Platform
        if ncch.content_platform > 0 {
            id.extra.insert(
                "media_platform".into(),
                media_platform_name(ncch.content_platform).into(),
            );
        }

        // SHA-256 verification for unencrypted content (not quick mode)
        if !options.quick && ncch.no_crypto {
            if ncch.exheader_size > 0 {
                let exheader_offset = content_offset + 0x200;
                let hash_size = 0x400u64.min(ncch.exheader_size as u64);
                match verify_sha256(reader, exheader_offset, hash_size, &ncch.exheader_hash)? {
                    HashResult::Ok => {
                        id.extra.insert(
                            "checksum_status:ExHeader SHA-256".into(),
                            "OK".into(),
                        );
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
        }
    } else {
        // NCCH might be encrypted or have a different structure
        id.extra.insert(
            "ncch_note".into(),
            "Could not parse NCCH content (may be encrypted)".into(),
        );
    }

    // Origin is always digital for CIA
    id.extra
        .insert("origin".into(), "Digital (eShop/CIA)".into());

    // Meta section
    if cia.meta_size > 0 {
        id.extra.insert("has_meta".into(), "Yes".into());
    }

    // CIA section sizes
    id.extra.insert(
        "cia_cert_size".into(),
        format!("{} bytes", cia.cert_chain_size),
    );
    id.extra.insert(
        "cia_ticket_size".into(),
        format!("{} bytes", cia.ticket_size),
    );
    id.extra
        .insert("cia_tmd_size".into(), format!("{} bytes", cia.tmd_size));
    id.extra.insert(
        "cia_content_size".into(),
        format!("{} KB", cia.content_size / 1024),
    );

    Ok(id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use super::super::{NCCH_MAGIC, MEDIA_UNIT};

    /// Build a minimal synthetic CIA file.
    fn make_cia() -> Vec<u8> {
        // CIA header (0x2020 bytes)
        let header_size: u32 = 0x2020;
        let cert_chain_size: u32 = 0x0A00; // typical
        let ticket_size: u32 = 0x0350; // RSA-2048 ticket
        let tmd_size: u32 = 0x0208; // small TMD

        // NCCH content: 64 KB
        let ncch_size: u64 = 0x10000;

        let meta_size: u32 = 0;

        let mut cia = Vec::new();

        // -- CIA Header --
        let mut header = vec![0u8; header_size as usize];
        header[0x00..0x04].copy_from_slice(&header_size.to_le_bytes());
        header[0x04..0x06].copy_from_slice(&0u16.to_le_bytes()); // type
        header[0x06..0x08].copy_from_slice(&0u16.to_le_bytes()); // version
        header[0x08..0x0C].copy_from_slice(&cert_chain_size.to_le_bytes());
        header[0x0C..0x10].copy_from_slice(&ticket_size.to_le_bytes());
        header[0x10..0x14].copy_from_slice(&tmd_size.to_le_bytes());
        header[0x14..0x18].copy_from_slice(&meta_size.to_le_bytes());
        header[0x18..0x20].copy_from_slice(&ncch_size.to_le_bytes());
        // Content index: bit 0 set (content index 0 present)
        header[0x20] = 0x80; // big-endian bit 0 of content index
        cia.extend_from_slice(&header);
        // Align to 64
        cia.resize(align64(cia.len() as u64) as usize, 0);

        // -- Certificate chain (dummy) --
        let cert_end = cia.len() + cert_chain_size as usize;
        cia.resize(cert_end, 0xCC);
        cia.resize(align64(cia.len() as u64) as usize, 0);

        // -- Ticket --
        let mut ticket = vec![0u8; ticket_size as usize];
        // Signature type: RSA-2048 SHA-256 = 0x00010004 (big-endian)
        ticket[0x00..0x04].copy_from_slice(&0x00010004u32.to_be_bytes());
        // Title ID at ticket_data + 0x9C = 0x140 + 0x9C = 0x1DC
        let title_id: u64 = 0x00040000_00ABCDEF;
        ticket[0x1DC..0x1E4].copy_from_slice(&title_id.to_be_bytes());
        cia.extend_from_slice(&ticket);
        cia.resize(align64(cia.len() as u64) as usize, 0);

        // -- TMD --
        let mut tmd = vec![0u8; tmd_size as usize];
        // Signature type: RSA-2048 SHA-256
        tmd[0x00..0x04].copy_from_slice(&0x00010004u32.to_be_bytes());
        // Signature block size = 4 + 256 + 60 = 320 = 0x140
        // TMD header at 0x140:
        let tmd_hdr = 0x140;
        // Title ID at TMD header + 0x4C
        tmd[tmd_hdr + 0x4C..tmd_hdr + 0x54].copy_from_slice(&title_id.to_be_bytes());
        // Title version at TMD header + 0x9C
        let tv: u16 = 0x0410; // v1.1.0
        tmd[tmd_hdr + 0x9C..tmd_hdr + 0x9E].copy_from_slice(&tv.to_be_bytes());
        // Content count at TMD header + 0x9E
        tmd[tmd_hdr + 0x9E..tmd_hdr + 0xA0].copy_from_slice(&1u16.to_be_bytes());
        cia.extend_from_slice(&tmd);
        cia.resize(align64(cia.len() as u64) as usize, 0);

        // -- Content (NCCH) --
        let mut ncch = vec![0u8; ncch_size as usize];

        // NCCH magic
        ncch[0x100..0x104].copy_from_slice(&NCCH_MAGIC);
        ncch[0x104..0x108].copy_from_slice(&((ncch_size / MEDIA_UNIT) as u32).to_le_bytes());
        ncch[0x108..0x110].copy_from_slice(&title_id.to_le_bytes()); // partition ID
        ncch[0x110..0x112].copy_from_slice(b"31"); // maker code
        ncch[0x118..0x120].copy_from_slice(&title_id.to_le_bytes()); // program ID

        // Product code: "CTR-N-ABCJ" (10 bytes + 6 null padding = 16 bytes at 0x150)
        let product = b"CTR-N-ABCJ\0\0\0\0\0\0";
        ncch[0x150..0x160].copy_from_slice(product);

        // Flags: NoCrypto, executable
        ncch[0x188 + 4] = 1; // CTR platform
        ncch[0x188 + 5] = 3; // executable with RomFS
        ncch[0x188 + 7] = 0x04; // NoCrypto

        cia.extend_from_slice(&ncch);
        cia.resize(align64(cia.len() as u64) as usize, 0);

        cia
    }

    #[test]
    fn test_cia_basic_analysis() {
        let cia = make_cia();
        let file_size = cia.len() as u64;
        let options = AnalysisOptions::default();
        let result = analyze_cia(&mut Cursor::new(cia), file_size, &options).unwrap();

        assert_eq!(result.platform.as_deref(), Some("Nintendo 3DS"));
        assert_eq!(result.serial_number.as_deref(), Some("CTR-N-ABCJ"));
        assert_eq!(result.maker_code.as_deref(), Some("Nintendo"));
        assert_eq!(result.extra.get("format").unwrap(), "CIA");
        assert_eq!(result.extra.get("origin").unwrap(), "Digital (eShop/CIA)");
    }

    #[test]
    fn test_cia_title_id() {
        let cia = make_cia();
        let file_size = cia.len() as u64;
        let options = AnalysisOptions::default();
        let result = analyze_cia(&mut Cursor::new(cia), file_size, &options).unwrap();

        assert_eq!(
            result.extra.get("title_id").unwrap(),
            "0004000000ABCDEF"
        );
        assert_eq!(result.extra.get("title_type").unwrap(), "Application");
    }

    #[test]
    fn test_cia_title_version() {
        let cia = make_cia();
        let file_size = cia.len() as u64;
        let options = AnalysisOptions::default();
        let result = analyze_cia(&mut Cursor::new(cia), file_size, &options).unwrap();

        assert_eq!(result.version.as_deref(), Some("v1.1.0"));
    }

    #[test]
    fn test_cia_regions() {
        let cia = make_cia();
        let file_size = cia.len() as u64;
        let options = AnalysisOptions::default();
        let result = analyze_cia(&mut Cursor::new(cia), file_size, &options).unwrap();

        assert_eq!(result.regions, vec![retro_junk_lib::Region::Japan]);
    }

    #[test]
    fn test_cia_content_count() {
        let cia = make_cia();
        let file_size = cia.len() as u64;
        let options = AnalysisOptions::default();
        let result = analyze_cia(&mut Cursor::new(cia), file_size, &options).unwrap();

        assert_eq!(result.extra.get("content_count").unwrap(), "1");
    }
}
