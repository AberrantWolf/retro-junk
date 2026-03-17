//! CHD (Compressed Hunks of Data) disc reading.

use retro_junk_core::AnalysisError;
use std::io::SeekFrom;

use crate::iso9660::{PrimaryVolumeDescriptor, parse_directory_record, parse_pvd_data};
use crate::sector::{CHD_CD_SECTOR_SIZE, MODE2_FORM1_DATA_OFFSET};

/// Read 2048 bytes of user data from a given sector in a CHD file.
///
/// CHD CD images store raw 2352-byte sectors plus 96 bytes of subchannel
/// data, for 2448 bytes per sector. This function decompresses the relevant
/// hunk and extracts user data at the Mode 2 Form 1 offset (24 bytes in).
///
/// Use [`read_chd_sector_mode1`] for Mode 1 sectors (offset 16).
pub fn read_chd_sector(
    reader: &mut dyn retro_junk_core::ReadSeek,
    sector: u64,
) -> Result<[u8; 2048], AnalysisError> {
    read_chd_sector_with_offset(reader, sector, MODE2_FORM1_DATA_OFFSET as usize)
}

/// Read 2048 bytes of user data from a Mode 1 sector in a CHD file.
///
/// Mode 1 sectors have user data at offset 16 (12 sync + 4 header).
/// Saturn uses Mode 1 sectors.
pub fn read_chd_sector_mode1(
    reader: &mut dyn retro_junk_core::ReadSeek,
    sector: u64,
) -> Result<[u8; 2048], AnalysisError> {
    read_chd_sector_with_offset(reader, sector, crate::sector::MODE1_DATA_OFFSET as usize)
}

fn read_chd_sector_with_offset(
    reader: &mut dyn retro_junk_core::ReadSeek,
    sector: u64,
    data_offset: usize,
) -> Result<[u8; 2048], AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    let mut chd = chd::Chd::open(reader, None)
        .map_err(|e| AnalysisError::other(format!("Failed to open CHD: {}", e)))?;

    let hunk_size = chd.header().hunk_size() as u64;

    // CHD CD images: each sector is CHD_CD_SECTOR_SIZE bytes (2448)
    let sector_byte_offset = sector * CHD_CD_SECTOR_SIZE as u64;

    // Which hunk contains this offset?
    let hunk_num = sector_byte_offset / hunk_size;
    // Offset within the hunk
    let offset_in_hunk = (sector_byte_offset % hunk_size) as usize;

    let mut hunk_buf = chd.get_hunksized_buffer();
    let mut cmp_buf = Vec::new();

    let mut hunk = chd
        .hunk(hunk_num as u32)
        .map_err(|e| AnalysisError::other(format!("Failed to get CHD hunk {}: {}", hunk_num, e)))?;

    hunk.read_hunk_in(&mut cmp_buf, &mut hunk_buf)
        .map_err(|e| {
            AnalysisError::other(format!("Failed to decompress CHD hunk {}: {}", hunk_num, e))
        })?;

    // Within the raw sector, user data starts at the given offset
    let final_offset = offset_in_hunk + data_offset;
    if final_offset + 2048 > hunk_buf.len() {
        return Err(AnalysisError::corrupted_header(
            "CHD sector data extends beyond hunk boundary",
        ));
    }

    let mut result = [0u8; 2048];
    result.copy_from_slice(&hunk_buf[final_offset..final_offset + 2048]);
    Ok(result)
}

/// Read CHD header metadata for display purposes.
#[allow(dead_code)]
pub struct ChdInfo {
    pub version: u32,
    pub hunk_size: u32,
    pub total_hunks: u32,
    pub logical_size: u64,
}

/// Extract basic CHD file information without full decompression.
pub fn read_chd_info(reader: &mut dyn retro_junk_core::ReadSeek) -> Result<ChdInfo, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    let chd = chd::Chd::open(reader, None)
        .map_err(|e| AnalysisError::other(format!("Failed to open CHD: {}", e)))?;

    let header = chd.header();

    Ok(ChdInfo {
        version: header.version() as u32,
        hunk_size: header.hunk_size(),
        total_hunks: header.hunk_count(),
        logical_size: header.logical_bytes(),
    })
}

/// Read the ISO 9660 PVD from a CHD disc image.
///
/// Reads sector 16, parses as PVD, but does NOT check the system identifier.
/// Callers should validate the system identifier for their platform.
pub fn read_pvd_from_chd(
    reader: &mut dyn retro_junk_core::ReadSeek,
) -> Result<PrimaryVolumeDescriptor, AnalysisError> {
    let pvd_data = read_chd_sector(reader, crate::sector::PVD_SECTOR)?;
    parse_pvd_data(&pvd_data)
}

/// Find and read a file from a CHD disc image's ISO 9660 root directory.
///
/// This is a generic function that reads the PVD, walks the root directory,
/// and reads the specified file. It does NOT check the system identifier —
/// callers should validate that separately.
///
/// Returns both the PVD (for system identifier checking) and the file contents.
pub fn find_file_in_chd(
    reader: &mut dyn retro_junk_core::ReadSeek,
    filename: &str,
) -> Result<(PrimaryVolumeDescriptor, Vec<u8>), AnalysisError> {
    // Read PVD from sector 16
    let pvd_data = read_chd_sector(reader, crate::sector::PVD_SECTOR)?;

    // Verify PVD signature
    if pvd_data[0] != 0x01 || &pvd_data[1..6] != b"CD001" {
        return Err(AnalysisError::invalid_format(
            "CHD: Missing PVD at sector 16",
        ));
    }

    let pvd = parse_pvd_data(&pvd_data)?;

    // Walk root directory to find the file
    let dir_sectors = (pvd.root_dir_data_length as u64).div_ceil(2048);
    let target_upper = filename.to_uppercase();

    for sector_offset in 0..dir_sectors {
        let sector = pvd.root_dir_extent_lba as u64 + sector_offset;
        let sector_data = read_chd_sector(reader, sector)?;

        let mut pos = 0;
        while pos < 2048 {
            let record_len = sector_data[pos] as usize;
            if record_len == 0 {
                break;
            }
            if pos + record_len > 2048 {
                break;
            }

            let record = &sector_data[pos..pos + record_len];
            if let Some(dir_rec) = parse_directory_record(record) {
                let id_upper = dir_rec.file_identifier.to_uppercase();
                let id_stripped = id_upper.split(';').next().unwrap_or(&id_upper);
                if id_stripped == target_upper {
                    // Read the file from CHD
                    let content = read_file_from_chd(reader, &dir_rec)?;
                    return Ok((pvd, content));
                }
            }

            pos += record_len;
        }
    }

    Err(AnalysisError::other(format!(
        "'{}' not found in CHD root directory",
        filename,
    )))
}

/// Maximum file size we'll read from an ISO 9660 filesystem (256 MB).
const MAX_ISO_FILE_SIZE: u32 = 256 * 1024 * 1024;

/// Read file content from a CHD image given a directory record.
pub fn read_file_from_chd(
    reader: &mut dyn retro_junk_core::ReadSeek,
    record: &crate::iso9660::DirectoryRecord,
) -> Result<Vec<u8>, AnalysisError> {
    if record.data_length > MAX_ISO_FILE_SIZE {
        return Err(AnalysisError::corrupted_header(
            "ISO 9660 file size exceeds safety limit",
        ));
    }
    let mut result = Vec::with_capacity(record.data_length as usize);
    let sectors_needed = (record.data_length as u64).div_ceil(2048);
    let mut remaining = record.data_length as usize;

    for i in 0..sectors_needed {
        let sector = record.extent_lba as u64 + i;
        let sector_data = read_chd_sector(reader, sector)?;
        let to_copy = remaining.min(2048);
        result.extend_from_slice(&sector_data[..to_copy]);
        remaining -= to_copy;
    }

    Ok(result)
}

/// Parse CHD track metadata (CHTR or CHT2) to find the number of frames
/// (sectors) in Track 1. Returns `None` if no track metadata is found.
///
/// CHD CD-ROM track metadata is stored as text strings like:
///   `TRACK:1 TYPE:MODE2_RAW SUBTYPE:NONE FRAMES:229020 PREFRAMES:150`
pub fn parse_chd_track1_frames<F: std::io::Read + std::io::Seek>(
    chd: &mut chd::Chd<F>,
) -> Result<Option<usize>, AnalysisError> {
    use chd::metadata::{KnownMetadata, MetadataTag};

    // Collect metadata refs first, then read them.
    let meta_refs: Vec<_> = chd.metadata_refs().collect();

    for meta_ref in &meta_refs {
        let tag = meta_ref.metatag();
        if tag != KnownMetadata::CdRomTrack as u32 && tag != KnownMetadata::CdRomTrack2 as u32 {
            continue;
        }

        // Read the metadata entry — needs mutable borrow to the underlying file
        let meta = meta_ref
            .read(chd.inner())
            .map_err(|e| AnalysisError::other(format!("Failed to read CHD metadata: {}", e)))?;

        let text = String::from_utf8_lossy(&meta.value);

        // Parse "TRACK:N ... FRAMES:N"
        if let Some(track_num) = parse_meta_field(&text, "TRACK") {
            if track_num == "1" {
                if let Some(frames_str) = parse_meta_field(&text, "FRAMES") {
                    let frames: usize = frames_str.parse().map_err(|_| {
                        AnalysisError::other(format!(
                            "Invalid FRAMES value in CHD metadata: {}",
                            frames_str
                        ))
                    })?;
                    log::info!("CHD track metadata: Track 1 has {} frames", frames);
                    return Ok(Some(frames));
                }
            }
        }
    }

    Ok(None)
}

/// Extract a field value from CHD metadata text (e.g., "FRAMES" from
/// `"TRACK:1 TYPE:MODE2_RAW SUBTYPE:NONE FRAMES:229020"`).
pub fn parse_meta_field<'a>(text: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("{}:", field);
    for token in text.split_whitespace() {
        if let Some(value) = token.strip_prefix(&prefix) {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
#[path = "tests/chd_tests.rs"]
mod tests;
