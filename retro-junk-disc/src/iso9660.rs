//! ISO 9660 filesystem parsing.
//!
//! Reads Primary Volume Descriptors, walks directory trees, and reads files
//! from ISO 9660 disc images in any supported format (ISO, raw BIN, CHD).

use retro_junk_core::AnalysisError;

use crate::format::DiscFormat;
use crate::sector::read_sector_data;

/// Parsed ISO 9660 Primary Volume Descriptor.
#[derive(Debug, Clone)]
pub struct PrimaryVolumeDescriptor {
    /// System identifier (offset 8, 32 bytes). e.g. "PLAYSTATION", "SEGA SEGASATURN"
    pub system_identifier: String,
    /// Volume identifier (offset 40, 32 bytes).
    pub volume_identifier: String,
    /// Volume space size in sectors (offset 80, LE u32).
    pub volume_space_size: u32,
    /// LBA of root directory extent (from root dir record at offset 156).
    pub root_dir_extent_lba: u32,
    /// Size of root directory data in bytes.
    pub root_dir_data_length: u32,
}

/// A parsed ISO 9660 directory record.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DirectoryRecord {
    pub extent_lba: u32,
    pub data_length: u32,
    pub file_flags: u8,
    pub file_identifier: String,
}

/// Maximum file size we'll read from an ISO 9660 filesystem (256 MB).
const MAX_ISO_FILE_SIZE: u32 = 256 * 1024 * 1024;

/// Read and parse the ISO 9660 Primary Volume Descriptor from sector 16.
pub fn read_pvd(
    reader: &mut dyn retro_junk_core::ReadSeek,
    format: DiscFormat,
) -> Result<PrimaryVolumeDescriptor, AnalysisError> {
    let sector_data = read_sector_data(reader, crate::sector::PVD_SECTOR, format)?;
    parse_pvd_data(&sector_data)
}

/// Parse a PVD from raw 2048-byte sector data.
///
/// Used by both direct sector reading and CHD sector reading.
pub fn parse_pvd_data(sector_data: &[u8; 2048]) -> Result<PrimaryVolumeDescriptor, AnalysisError> {
    // Byte 0: type must be 0x01 (Primary Volume Descriptor)
    if sector_data[0] != 0x01 {
        return Err(AnalysisError::invalid_format(format!(
            "Expected PVD type 0x01, got 0x{:02X}",
            sector_data[0]
        )));
    }

    // Bytes 1-5: "CD001"
    if &sector_data[1..6] != b"CD001" {
        return Err(AnalysisError::invalid_format(
            "Missing CD001 signature in PVD",
        ));
    }

    let system_identifier = read_str_a(&sector_data[8..40]);
    let volume_identifier = read_str_a(&sector_data[40..72]);

    // Volume space size: both-endian u32 at offset 80 (LE at 80, BE at 84)
    let volume_space_size = u32::from_le_bytes([
        sector_data[80],
        sector_data[81],
        sector_data[82],
        sector_data[83],
    ]);

    // Root directory record at offset 156, 34 bytes
    let root_record = &sector_data[156..190];
    let root_dir_extent_lba = u32::from_le_bytes([
        root_record[2],
        root_record[3],
        root_record[4],
        root_record[5],
    ]);
    let root_dir_data_length = u32::from_le_bytes([
        root_record[10],
        root_record[11],
        root_record[12],
        root_record[13],
    ]);

    Ok(PrimaryVolumeDescriptor {
        system_identifier,
        volume_identifier,
        volume_space_size,
        root_dir_extent_lba,
        root_dir_data_length,
    })
}

/// Read a padded ISO 9660 string (strip trailing spaces).
pub fn read_str_a(bytes: &[u8]) -> String {
    let s = std::str::from_utf8(bytes).unwrap_or("");
    s.trim_end().to_string()
}

/// Find a file by name in the root directory and return its contents.
pub fn find_file_in_root(
    reader: &mut dyn retro_junk_core::ReadSeek,
    format: DiscFormat,
    pvd: &PrimaryVolumeDescriptor,
    filename: &str,
) -> Result<Vec<u8>, AnalysisError> {
    let target_upper = filename.to_uppercase();

    // Read root directory sectors
    let dir_sectors = (pvd.root_dir_data_length as u64).div_ceil(crate::sector::ISO_SECTOR_SIZE);

    for sector_offset in 0..dir_sectors {
        let sector = pvd.root_dir_extent_lba as u64 + sector_offset;
        let sector_data = read_sector_data(reader, sector, format)?;

        let mut pos = 0;
        while pos < crate::sector::ISO_SECTOR_SIZE as usize {
            let record_len = sector_data[pos] as usize;
            if record_len == 0 {
                break; // No more records in this sector
            }
            if pos + record_len > crate::sector::ISO_SECTOR_SIZE as usize {
                break;
            }

            let record = &sector_data[pos..pos + record_len];
            if let Some(dir_rec) = parse_directory_record(record) {
                // Compare filename (strip ";1" version suffix)
                let id_upper = dir_rec.file_identifier.to_uppercase();
                let id_stripped = id_upper.split(';').next().unwrap_or(&id_upper);

                if id_stripped == target_upper {
                    // Found it — read the file content
                    return read_file_content(reader, format, &dir_rec);
                }
            }

            pos += record_len;
        }
    }

    Err(AnalysisError::other(format!(
        "File '{}' not found in root directory",
        filename
    )))
}

/// Parse a single ISO 9660 directory record.
pub fn parse_directory_record(data: &[u8]) -> Option<DirectoryRecord> {
    if data.len() < 33 {
        return None;
    }
    let record_len = data[0] as usize;
    if record_len < 33 {
        return None;
    }

    let extent_lba = u32::from_le_bytes([data[2], data[3], data[4], data[5]]);
    let data_length = u32::from_le_bytes([data[10], data[11], data[12], data[13]]);
    let file_flags = data[25];
    let id_len = data[32] as usize;

    if 33 + id_len > record_len {
        return None;
    }

    let file_identifier = if id_len == 1 && data[33] == 0x00 {
        ".".to_string()
    } else if id_len == 1 && data[33] == 0x01 {
        "..".to_string()
    } else {
        String::from_utf8_lossy(&data[33..33 + id_len]).to_string()
    };

    Some(DirectoryRecord {
        extent_lba,
        data_length,
        file_flags,
        file_identifier,
    })
}

/// Read the full content of a file given its directory record.
pub fn read_file_content(
    reader: &mut dyn retro_junk_core::ReadSeek,
    format: DiscFormat,
    record: &DirectoryRecord,
) -> Result<Vec<u8>, AnalysisError> {
    if record.data_length > MAX_ISO_FILE_SIZE {
        return Err(AnalysisError::corrupted_header(
            "ISO 9660 file size exceeds safety limit",
        ));
    }
    let mut result = Vec::with_capacity(record.data_length as usize);
    let sectors_needed = (record.data_length as u64).div_ceil(crate::sector::ISO_SECTOR_SIZE);
    let mut remaining = record.data_length as usize;

    for i in 0..sectors_needed {
        let sector = record.extent_lba as u64 + i;
        let sector_data = read_sector_data(reader, sector, format)?;
        let to_copy = remaining.min(crate::sector::ISO_SECTOR_SIZE as usize);
        result.extend_from_slice(&sector_data[..to_copy]);
        remaining -= to_copy;
    }

    Ok(result)
}

#[cfg(test)]
#[path = "tests/iso9660_tests.rs"]
mod tests;
