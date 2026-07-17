//! NCCH partition header parsing for Nintendo 3DS.

use retro_junk_core::{AnalysisError, ReadSeek};
use std::io::SeekFrom;

use super::NCCH_MAGIC;
use super::common::{read_ascii, read_u32_le, read_u64_le};

// ---------------------------------------------------------------------------
// NCCH header
// ---------------------------------------------------------------------------

/// Parsed NCCH partition header fields.
pub(crate) struct NcchHeader {
    pub(crate) content_size_mu: u32,
    pub(crate) maker_code: String,
    pub(crate) program_id: u64,
    pub(crate) product_code: String,
    pub(crate) exheader_hash: [u8; 32],
    pub(crate) exheader_size: u32,
    /// NCCH flags[7] bit 2: content is not encrypted.
    pub(crate) no_crypto: bool,
    /// NCCH flags[4]: content platform (1=Old3DS, 2=New3DS).
    pub(crate) content_platform: u8,
    /// NCCH flags[5]: form type + content type.
    pub(crate) content_type_flags: u8,
    /// NCCH flags[3]: crypto method.
    pub(crate) crypto_method: u8,
    pub(crate) exefs_offset_mu: u32,
    pub(crate) exefs_size_mu: u32,
    pub(crate) exefs_hash_region_size_mu: u32,
    pub(crate) romfs_offset_mu: u32,
    pub(crate) romfs_size_mu: u32,
    pub(crate) romfs_hash_region_size_mu: u32,
    pub(crate) exefs_superblock_hash: [u8; 32],
    pub(crate) romfs_superblock_hash: [u8; 32],
}

/// Read and parse an NCCH header at the given absolute file offset.
pub(crate) fn parse_ncch_header(
    reader: &mut dyn ReadSeek,
    offset: u64,
) -> Result<NcchHeader, AnalysisError> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut buf = [0u8; 0x200];
    reader.read_exact(&mut buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            AnalysisError::corrupted_header("NCCH header truncated")
        } else {
            AnalysisError::Io(e)
        }
    })?;

    if buf[0x100..0x104] != NCCH_MAGIC {
        return Err(AnalysisError::invalid_format(format!(
            "Missing NCCH magic at 0x{:X}",
            offset + 0x100
        )));
    }

    let trunc = || AnalysisError::corrupted_header("NCCH header data truncated");
    let content_size_mu = read_u32_le(&buf, 0x104).ok_or_else(trunc)?;
    let maker_code = read_ascii(&buf[0x110..0x112]);
    let program_id = read_u64_le(&buf, 0x118).ok_or_else(trunc)?;
    let product_code = read_ascii(&buf[0x150..0x160]);

    let mut exheader_hash = [0u8; 32];
    exheader_hash.copy_from_slice(&buf[0x160..0x180]);
    let exheader_size = read_u32_le(&buf, 0x180).ok_or_else(trunc)?;

    let flags = &buf[0x188..0x190];
    let crypto_method = flags[3];
    let content_platform = flags[4];
    let content_type_flags = flags[5];
    let no_crypto = flags[7] & 0x04 != 0;

    let exefs_offset_mu = read_u32_le(&buf, 0x1A0).ok_or_else(trunc)?;
    let exefs_size_mu = read_u32_le(&buf, 0x1A4).ok_or_else(trunc)?;
    let exefs_hash_region_size_mu = read_u32_le(&buf, 0x1A8).ok_or_else(trunc)?;
    let romfs_offset_mu = read_u32_le(&buf, 0x1B0).ok_or_else(trunc)?;
    let romfs_size_mu = read_u32_le(&buf, 0x1B4).ok_or_else(trunc)?;
    let romfs_hash_region_size_mu = read_u32_le(&buf, 0x1B8).ok_or_else(trunc)?;

    let mut exefs_superblock_hash = [0u8; 32];
    exefs_superblock_hash.copy_from_slice(&buf[0x1C0..0x1E0]);
    let mut romfs_superblock_hash = [0u8; 32];
    romfs_superblock_hash.copy_from_slice(&buf[0x1E0..0x200]);

    Ok(NcchHeader {
        content_size_mu,
        maker_code,
        program_id,
        product_code,
        exheader_hash,
        exheader_size,
        no_crypto,
        content_platform,
        content_type_flags,
        crypto_method,
        exefs_offset_mu,
        exefs_size_mu,
        exefs_hash_region_size_mu,
        romfs_offset_mu,
        romfs_size_mu,
        romfs_hash_region_size_mu,
        exefs_superblock_hash,
        romfs_superblock_hash,
    })
}
