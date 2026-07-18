//! CD-ROM sector constants and low-level sector reading.

use retro_junk_core::AnalysisError;
use std::io::SeekFrom;

use crate::format::DiscFormat;

/// CD sync pattern at the start of every raw (2352-byte) sector.
pub const CD_SYNC_PATTERN: [u8; 12] = [
    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
];

/// Standard ISO 9660 sector size (user data only).
pub const ISO_SECTOR_SIZE: u64 = 2048;

/// Raw CD sector size (sync + header + subheader + data + EDC + ECC).
pub const RAW_SECTOR_SIZE: u64 = 2352;

/// Offset to user data within a Mode 1 raw sector.
/// 12 (sync) + 4 (header) = 16.
pub const MODE1_DATA_OFFSET: u64 = 16;

/// Offset to user data within a Mode 2 Form 1 raw sector.
/// 12 (sync) + 4 (header) + 8 (subheader) = 24.
pub const MODE2_FORM1_DATA_OFFSET: u64 = 24;

/// ISO 9660 Primary Volume Descriptor is always at sector 16.
pub const PVD_SECTOR: u64 = 16;

/// Offset to the mode byte within a raw sector header.
/// 12 (sync) + 3 (MSF) = 15.
pub const SECTOR_MODE_OFFSET: usize = 15;

/// Offset to the submode byte within a Mode 2 sector subheader.
/// 12 (sync) + 4 (header) + 2 (file/channel) = 18.
pub const SECTOR_SUBMODE_OFFSET: usize = 18;

/// Start of user data within a Mode 2 Form 1/Form 2 sector.
/// 12 (sync) + 4 (header) + 8 (subheader) = 24.
pub const SECTOR_USER_DATA_START: usize = 24;

/// CHD file magic bytes.
pub const CHD_MAGIC: &[u8; 8] = b"MComprHD";

/// CD sector size within CHD: raw sector (2352) + subchannel (96) = 2448.
pub const CHD_CD_SECTOR_SIZE: u32 = 2448;

/// Read 2048 bytes of user data from a given sector number.
///
/// For ISO images, reads directly at `sector * 2048`.
/// For raw BIN images, reads user data at the appropriate offset within
/// the 2352-byte sector, using Mode 2 Form 1 layout (offset 24) by default.
///
/// Use [`read_sector_data_mode1`] for Mode 1 sectors (offset 16).
pub fn read_sector_data(
    reader: &mut dyn retro_junk_core::ReadSeek,
    sector: u64,
    format: DiscFormat,
) -> Result<[u8; 2048], AnalysisError> {
    read_sector_data_with_offset(reader, sector, format, MODE2_FORM1_DATA_OFFSET)
}

/// Read 2048 bytes of user data from a Mode 1 raw sector.
///
/// Mode 1 sectors have user data at offset 16 (12 sync + 4 header),
/// unlike Mode 2 Form 1 which has an 8-byte subheader (offset 24).
/// Saturn uses Mode 1 sectors.
pub fn read_sector_data_mode1(
    reader: &mut dyn retro_junk_core::ReadSeek,
    sector: u64,
    format: DiscFormat,
) -> Result<[u8; 2048], AnalysisError> {
    read_sector_data_with_offset(reader, sector, format, MODE1_DATA_OFFSET)
}

fn read_sector_data_with_offset(
    reader: &mut dyn retro_junk_core::ReadSeek,
    sector: u64,
    format: DiscFormat,
    raw_data_offset: u64,
) -> Result<[u8; 2048], AnalysisError> {
    let offset = match format {
        DiscFormat::Iso2048 => sector * ISO_SECTOR_SIZE,
        DiscFormat::RawSector2352 => sector * RAW_SECTOR_SIZE + raw_data_offset,
        _ => {
            return Err(AnalysisError::unsupported(
                "Cannot read sectors directly from CUE/CHD format",
            ));
        }
    };

    reader.seek(SeekFrom::Start(offset))?;
    let mut data = [0u8; 2048];
    reader.read_exact(&mut data).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            AnalysisError::corrupted_header(format!("Sector {sector} is beyond end of image"))
        } else {
            AnalysisError::Io(e)
        }
    })?;
    Ok(data)
}
