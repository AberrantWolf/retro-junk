//! CIA format parsing and analysis for Nintendo 3DS eShop/installable archives.

use retro_junk_core::ReadSeek;
use std::io::SeekFrom;

use retro_junk_core::{AnalysisError, AnalysisOptions, RomIdentification};

use super::CIA_HEADER_SIZE;
use super::common::{
    align64, format_title_id, media_platform_name, read_u16_be, read_u32_be, read_u32_le,
    read_u64_be, read_u64_le, record_ncch_common, record_sha256_check, record_title_version,
    title_type_from_id,
};
use super::ncch::parse_ncch_header;

// ---------------------------------------------------------------------------
// CIA header
// ---------------------------------------------------------------------------

/// Parsed CIA header.
// Field names mirror the canonical CIA spec section names; stripping the `size` suffix would lose that mapping.
#[allow(clippy::struct_field_names)]
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

    let trunc = || AnalysisError::corrupted_header("CIA header data truncated");
    let header_size = read_u32_le(&buf, 0x00).ok_or_else(trunc)?;
    if header_size != CIA_HEADER_SIZE {
        return Err(AnalysisError::invalid_format(format!(
            "Unexpected CIA header size: 0x{header_size:X}"
        )));
    }

    Ok(CiaHeader {
        header_size,
        cert_chain_size: read_u32_le(&buf, 0x08).ok_or_else(trunc)?,
        ticket_size: read_u32_le(&buf, 0x0C).ok_or_else(trunc)?,
        tmd_size: read_u32_le(&buf, 0x10).ok_or_else(trunc)?,
        meta_size: read_u32_le(&buf, 0x14).ok_or_else(trunc)?,
        content_size: read_u64_le(&buf, 0x18).ok_or_else(trunc)?,
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
        0x0001_0003 => Some(4 + 0x200 + 0x3C), // RSA-4096: type(4) + sig(512) + pad(60)
        0x0001_0004 => Some(4 + 0x100 + 0x3C), // RSA-2048: type(4) + sig(256) + pad(60)
        0x0001_0005 => Some(4 + 0x3C + 0x40),  // ECDSA: type(4) + sig(60) + pad(64)
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
    let sig_type = read_u32_be(&sig_type_buf, 0)
        .ok_or_else(|| AnalysisError::corrupted_header("TMD signature type truncated"))?;

    let sig_block_size = signature_block_size(sig_type).ok_or_else(|| {
        AnalysisError::invalid_format(format!("Unknown TMD signature type: 0x{sig_type:08X}"))
    })?;

    // TMD header starts after signature block
    let tmd_header_offset = tmd_offset + sig_block_size as u64;
    reader.seek(SeekFrom::Start(tmd_header_offset))?;

    let mut tmd_buf = [0u8; 0xC4];
    reader
        .read_exact(&mut tmd_buf)
        .map_err(|_| AnalysisError::corrupted_header("TMD header truncated"))?;

    let trunc = || AnalysisError::corrupted_header("TMD header data truncated");
    let title_id = read_u64_be(&tmd_buf, 0x4C).ok_or_else(trunc)?;
    let title_version = read_u16_be(&tmd_buf, 0x9C).ok_or_else(trunc)?;
    let content_count = read_u16_be(&tmd_buf, 0x9E).ok_or_else(trunc)?;

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
    let sig_type = read_u32_be(&sig_type_buf, 0)
        .ok_or_else(|| AnalysisError::corrupted_header("Ticket signature type truncated"))?;

    let sig_block_size = signature_block_size(sig_type).ok_or_else(|| {
        AnalysisError::invalid_format(format!("Unknown Ticket signature type: 0x{sig_type:08X}"))
    })?;

    let ticket_data_offset = ticket_offset + sig_block_size as u64;
    reader.seek(SeekFrom::Start(ticket_data_offset + 0x9C))?;
    let mut tid_buf = [0u8; 8];
    reader.read_exact(&mut tid_buf)?;
    read_u64_be(&tid_buf, 0)
        .ok_or_else(|| AnalysisError::corrupted_header("Ticket title ID truncated"))
}

// ---------------------------------------------------------------------------
// Section offset helpers
// ---------------------------------------------------------------------------

/// Calculate the offset of the content section within a CIA.
fn cia_content_offset(cia: &CiaHeader) -> u64 {
    let mut offset = align64(u64::from(cia.header_size));
    offset += align64(u64::from(cia.cert_chain_size));
    offset += align64(u64::from(cia.ticket_size));
    offset += align64(u64::from(cia.tmd_size));
    offset
}

/// Calculate the offset of the TMD section within a CIA.
fn cia_tmd_offset(cia: &CiaHeader) -> u64 {
    let mut offset = align64(u64::from(cia.header_size));
    offset += align64(u64::from(cia.cert_chain_size));
    offset += align64(u64::from(cia.ticket_size));
    offset
}

/// Calculate the offset of the Ticket section within a CIA.
fn cia_ticket_offset(cia: &CiaHeader) -> u64 {
    let mut offset = align64(u64::from(cia.header_size));
    offset += align64(u64::from(cia.cert_chain_size));
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

    let mut id = RomIdentification::new();

    // Format
    id.extra.insert("format".into(), "CIA".into());
    id.file_size = file_size;

    // Expected size from CIA sections
    let content_offset = cia_content_offset(&cia);
    let expected_size = content_offset
        + cia.content_size
        + if cia.meta_size > 0 {
            align64(u64::from(cia.meta_size))
        } else {
            0
        };
    // CIA files may have trailing alignment; accept anything >= content end
    let content_end = content_offset + cia.content_size;
    if file_size >= content_end {
        id.expected_size = file_size; // OK, no truncation
    } else {
        id.expected_size = expected_size;
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
    record_title_version(&mut id, tmd_info.title_version);

    // Content count
    id.extra.insert(
        "content_count".into(),
        format!("{}", tmd_info.content_count),
    );

    // Parse ticket for title ID (cross-reference)
    let ticket_offset = cia_ticket_offset(&cia);
    if let Ok(ticket_tid) = parse_cia_ticket_title_id(reader, ticket_offset)
        && ticket_tid != tmd_info.title_id
        && ticket_tid != 0
    {
        id.extra
            .insert("ticket_title_id".into(), format_title_id(ticket_tid));
    }

    // Try to parse NCCH from content section
    let ncch_result = parse_ncch_header(reader, content_offset);
    if let Ok(ncch) = ncch_result {
        // Shared NCCH-derived fields (product code, maker, regions, content
        // type, encryption, ExeFS/RomFS sizes)
        record_ncch_common(&mut id, &ncch);

        // Program ID from NCCH (may differ from TMD title ID for updates/DLC)
        if ncch.program_id != 0 {
            id.extra
                .insert("program_id".into(), format_title_id(ncch.program_id));
        }

        // Platform
        if ncch.content_platform > 0 {
            id.extra.insert(
                "media_platform".into(),
                media_platform_name(ncch.content_platform).into(),
            );
        }

        // SHA-256 verification for unencrypted content (not quick mode)
        if !options.quick && ncch.no_crypto && ncch.exheader_size > 0 {
            let exheader_offset = content_offset + 0x200;
            let hash_size = 0x400u64.min(u64::from(ncch.exheader_size));
            record_sha256_check(
                &mut id,
                reader,
                exheader_offset,
                hash_size,
                &ncch.exheader_hash,
                "ExHeader SHA-256",
            )?;
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

#[cfg(test)]
#[path = "tests/cia_tests.rs"]
mod tests;
