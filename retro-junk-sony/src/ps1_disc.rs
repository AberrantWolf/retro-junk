//! PS1 disc parsing utilities.
//!
//! Handles ISO 9660 filesystem parsing, CD sector formats, SYSTEM.CNF extraction,
//! serial/region detection, CUE sheet parsing, and CHD disc reading.

use std::io::SeekFrom;

use retro_junk_core::{AnalysisError, Region};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// CD sync pattern at the start of every raw (2352-byte) sector.
const CD_SYNC_PATTERN: [u8; 12] = [
    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
];

/// Standard ISO 9660 sector size (user data only).
const ISO_SECTOR_SIZE: u64 = 2048;

/// Raw CD sector size (sync + header + subheader + data + EDC + ECC).
const RAW_SECTOR_SIZE: u64 = 2352;

/// Offset to user data within a Mode 2 Form 1 raw sector.
/// 12 (sync) + 4 (header) + 8 (subheader) = 24.
const MODE2_FORM1_DATA_OFFSET: u64 = 24;

/// ISO 9660 Primary Volume Descriptor is always at sector 16.
const PVD_SECTOR: u64 = 16;

/// CHD file magic bytes.
const CHD_MAGIC: &[u8; 8] = b"MComprHD";

/// CD sector size within CHD: raw sector (2352) + subchannel (96) = 2448.
const CHD_CD_SECTOR_SIZE: u32 = 2448;

// ---------------------------------------------------------------------------
// Disc format detection
// ---------------------------------------------------------------------------

/// Detected disc image format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscFormat {
    /// Standard 2048 byte/sector ISO image.
    Iso2048,
    /// Raw 2352 byte/sector BIN image.
    RawSector2352,
    /// CUE sheet (text file referencing BIN tracks).
    Cue,
    /// MAME Compressed Hunks of Data.
    Chd,
}

impl DiscFormat {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Iso2048 => "ISO 9660",
            Self::RawSector2352 => "Raw BIN (2352)",
            Self::Cue => "CUE Sheet",
            Self::Chd => "CHD",
        }
    }
}

/// Detect the disc image format by examining the reader content.
pub fn detect_disc_format(
    reader: &mut dyn retro_junk_core::ReadSeek,
) -> Result<DiscFormat, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    let mut buf = [0u8; 16];
    let bytes_read = reader.read(&mut buf)?;
    reader.seek(SeekFrom::Start(0))?;

    if bytes_read < 12 {
        return Err(AnalysisError::TooSmall {
            expected: 12,
            actual: bytes_read as u64,
        });
    }

    // Check CHD magic
    if bytes_read >= 8 && buf[..8] == *CHD_MAGIC {
        return Ok(DiscFormat::Chd);
    }

    // Check raw sector sync pattern
    if buf[..12] == CD_SYNC_PATTERN {
        return Ok(DiscFormat::RawSector2352);
    }

    // Check for CUE sheet: scan for common CUE keywords in what looks like text
    if looks_like_cue(reader)? {
        return Ok(DiscFormat::Cue);
    }

    // Check for ISO 9660 PVD at sector 16
    let pvd_offset = PVD_SECTOR * ISO_SECTOR_SIZE + 1; // +1 to skip type byte
    reader.seek(SeekFrom::Start(pvd_offset))?;
    let mut cd001 = [0u8; 5];
    if reader.read_exact(&mut cd001).is_ok() && &cd001 == b"CD001" {
        reader.seek(SeekFrom::Start(0))?;
        return Ok(DiscFormat::Iso2048);
    }

    reader.seek(SeekFrom::Start(0))?;
    Err(AnalysisError::invalid_format(
        "Not a recognized PS1 disc format",
    ))
}

/// Check if reader content looks like a CUE sheet.
fn looks_like_cue(reader: &mut dyn retro_junk_core::ReadSeek) -> Result<bool, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;
    let mut buf = [0u8; 512];
    let n = reader.read(&mut buf)?;
    reader.seek(SeekFrom::Start(0))?;

    if n == 0 {
        return Ok(false);
    }

    // CUE files are text; check for non-text bytes (ignoring common whitespace)
    let slice = &buf[..n];
    let has_binary = slice
        .iter()
        .any(|&b| b < 0x09 || (b > 0x0D && b < 0x20 && b != 0x1A));
    if has_binary {
        return Ok(false);
    }

    let text = String::from_utf8_lossy(slice).to_uppercase();
    let has_file = text.contains("FILE ");
    let has_track = text.contains("TRACK ");
    Ok(has_file && has_track)
}

// ---------------------------------------------------------------------------
// Sector reading
// ---------------------------------------------------------------------------

/// Read 2048 bytes of user data from a given sector number.
pub fn read_sector_data(
    reader: &mut dyn retro_junk_core::ReadSeek,
    sector: u64,
    format: DiscFormat,
) -> Result<[u8; 2048], AnalysisError> {
    let offset = match format {
        DiscFormat::Iso2048 => sector * ISO_SECTOR_SIZE,
        DiscFormat::RawSector2352 => sector * RAW_SECTOR_SIZE + MODE2_FORM1_DATA_OFFSET,
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
            AnalysisError::corrupted_header(format!("Sector {} is beyond end of image", sector))
        } else {
            AnalysisError::Io(e)
        }
    })?;
    Ok(data)
}

// ---------------------------------------------------------------------------
// ISO 9660 Primary Volume Descriptor
// ---------------------------------------------------------------------------

/// Parsed ISO 9660 Primary Volume Descriptor.
#[derive(Debug, Clone)]
pub struct PrimaryVolumeDescriptor {
    /// System identifier (offset 8, 32 bytes). e.g. "PLAYSTATION"
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

/// Read and parse the ISO 9660 Primary Volume Descriptor from sector 16.
pub fn read_pvd(
    reader: &mut dyn retro_junk_core::ReadSeek,
    format: DiscFormat,
) -> Result<PrimaryVolumeDescriptor, AnalysisError> {
    let sector_data = read_sector_data(reader, PVD_SECTOR, format)?;

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
fn read_str_a(bytes: &[u8]) -> String {
    let s = std::str::from_utf8(bytes).unwrap_or("");
    s.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// ISO 9660 directory parsing
// ---------------------------------------------------------------------------

/// A parsed ISO 9660 directory record.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DirectoryRecord {
    pub extent_lba: u32,
    pub data_length: u32,
    pub file_flags: u8,
    pub file_identifier: String,
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
    let dir_sectors = (pvd.root_dir_data_length as u64 + 2047) / 2048;

    for sector_offset in 0..dir_sectors {
        let sector = pvd.root_dir_extent_lba as u64 + sector_offset;
        let sector_data = read_sector_data(reader, sector, format)?;

        let mut pos = 0;
        while pos < 2048 {
            let record_len = sector_data[pos] as usize;
            if record_len == 0 {
                break; // No more records in this sector
            }
            if pos + record_len > 2048 {
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
fn parse_directory_record(data: &[u8]) -> Option<DirectoryRecord> {
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
fn read_file_content(
    reader: &mut dyn retro_junk_core::ReadSeek,
    format: DiscFormat,
    record: &DirectoryRecord,
) -> Result<Vec<u8>, AnalysisError> {
    let mut result = Vec::with_capacity(record.data_length as usize);
    let sectors_needed = (record.data_length as u64 + 2047) / 2048;
    let mut remaining = record.data_length as usize;

    for i in 0..sectors_needed {
        let sector = record.extent_lba as u64 + i;
        let sector_data = read_sector_data(reader, sector, format)?;
        let to_copy = remaining.min(2048);
        result.extend_from_slice(&sector_data[..to_copy]);
        remaining -= to_copy;
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// SYSTEM.CNF parsing
// ---------------------------------------------------------------------------

/// Parsed SYSTEM.CNF contents.
#[derive(Debug, Clone)]
pub struct SystemCnf {
    /// Boot executable path, e.g. "cdrom:\SLUS_012.34;1"
    pub boot_path: String,
    /// Video mode from VMODE key, if present.
    pub vmode: Option<String>,
}

/// Parse the contents of a SYSTEM.CNF file.
pub fn parse_system_cnf(content: &str) -> Result<SystemCnf, AnalysisError> {
    let mut boot_path = None;
    let mut vmode = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_uppercase();
            let value = value.trim();

            match key.as_str() {
                "BOOT" | "BOOT2" => {
                    if boot_path.is_none() {
                        boot_path = Some(value.to_string());
                    }
                }
                "VMODE" => {
                    vmode = Some(value.to_string());
                }
                _ => {}
            }
        }
    }

    match boot_path {
        Some(path) => Ok(SystemCnf {
            boot_path: path,
            vmode,
        }),
        None => Err(AnalysisError::corrupted_header(
            "SYSTEM.CNF missing BOOT= line",
        )),
    }
}

// ---------------------------------------------------------------------------
// Serial extraction and region mapping
// ---------------------------------------------------------------------------

/// Extract a normalized serial from a SYSTEM.CNF boot path.
///
/// Input: `"cdrom:\SLUS_012.34;1"` or `"cdrom:\\SLUS_012.34;1"` or `"cdrom:SLUS_006.91;1"`
/// Output: `"SLUS-01234"`
pub fn extract_serial(boot_path: &str) -> Option<String> {
    // Find the filename part (after last \, /, or : to handle all SYSTEM.CNF variants)
    // Some games use "cdrom:\SLUS_012.34;1", others use "cdrom:SLUS_006.91;1"
    let filename = boot_path
        .rsplit(|c: char| c == '\\' || c == '/' || c == ':')
        .next()?;

    // Strip version suffix (";1")
    let filename = filename.split(';').next().unwrap_or(filename);

    // Match pattern like "SLUS_012.34" or "SLUS_01234" or "SCUS_012.34"
    let filename = filename.trim();
    if filename.len() < 8 {
        return None;
    }

    let prefix = &filename[..4];
    if !is_ps1_serial_prefix(prefix) {
        return None;
    }

    // Extract digits after the prefix+separator
    let rest = &filename[4..];
    let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();

    if digits.len() >= 5 {
        Some(format!("{}-{}", prefix.to_uppercase(), digits))
    } else {
        None
    }
}

/// Check if a 4-character prefix is a known PS1 serial prefix.
fn is_ps1_serial_prefix(prefix: &str) -> bool {
    let upper = prefix.to_uppercase();
    matches!(
        upper.as_str(),
        "SLUS"
            | "SCUS"
            | "SLPS"
            | "SCPS"
            | "SLPM"
            | "SLES"
            | "SCES"
            | "SCED"
            | "SLKA"
            | "SCKA"
            | "PAPX"
            | "PCPX"
            | "SIPS"
    )
}

/// Map a PS1 serial prefix to a region.
pub fn serial_to_region(serial: &str) -> Option<Region> {
    if serial.len() < 4 {
        return None;
    }
    let prefix = serial[..4].to_uppercase();
    match prefix.as_str() {
        "SLUS" | "SCUS" => Some(Region::Usa),
        "SLPS" | "SCPS" | "SLPM" | "SIPS" => Some(Region::Japan),
        "SLES" | "SCES" | "SCED" => Some(Region::Europe),
        "SLKA" | "SCKA" => Some(Region::Korea),
        "PAPX" | "PCPX" => Some(Region::Japan), // dev/promo discs, usually Japanese
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// CUE sheet parsing
// ---------------------------------------------------------------------------

/// A parsed CUE sheet.
#[derive(Debug, Clone)]
pub struct CueSheet {
    pub files: Vec<CueFile>,
}

/// A FILE entry in a CUE sheet.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CueFile {
    pub filename: String,
    pub file_type: String,
    pub tracks: Vec<CueTrack>,
}

/// A TRACK entry in a CUE sheet.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CueTrack {
    pub number: u8,
    pub mode: String,
}

/// Parse a CUE sheet from its text content.
pub fn parse_cue(content: &str) -> Result<CueSheet, AnalysisError> {
    let mut files = Vec::new();
    let mut current_file: Option<CueFile> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let upper = line.to_uppercase();

        if upper.starts_with("FILE ") {
            // Save previous file entry
            if let Some(f) = current_file.take() {
                files.push(f);
            }

            // Parse: FILE "filename" TYPE
            let (filename, file_type) = parse_cue_file_line(line)?;
            current_file = Some(CueFile {
                filename,
                file_type,
                tracks: Vec::new(),
            });
        } else if upper.starts_with("TRACK ") {
            if let Some(ref mut f) = current_file {
                let (number, mode) = parse_cue_track_line(line)?;
                f.tracks.push(CueTrack { number, mode });
            }
        }
        // Ignore INDEX, PREGAP, POSTGAP, REM, etc.
    }

    if let Some(f) = current_file.take() {
        files.push(f);
    }

    if files.is_empty() {
        return Err(AnalysisError::invalid_format(
            "CUE sheet contains no FILE entries",
        ));
    }

    Ok(CueSheet { files })
}

/// Parse a FILE line: `FILE "filename.bin" BINARY`
fn parse_cue_file_line(line: &str) -> Result<(String, String), AnalysisError> {
    let rest = &line[5..]; // skip "FILE "

    let (filename, remainder) = if rest.starts_with('"') {
        // Quoted filename
        let end_quote = rest[1..]
            .find('"')
            .ok_or_else(|| AnalysisError::invalid_format("Unterminated quote in CUE FILE line"))?;
        let filename = rest[1..1 + end_quote].to_string();
        let remainder = rest[2 + end_quote..].trim().to_string();
        (filename, remainder)
    } else {
        // Unquoted filename (space-delimited)
        let mut parts = rest.splitn(2, ' ');
        let filename = parts.next().unwrap_or("").to_string();
        let remainder = parts.next().unwrap_or("").trim().to_string();
        (filename, remainder)
    };

    Ok((filename, remainder))
}

/// Parse a TRACK line: `TRACK 01 MODE2/2352`
fn parse_cue_track_line(line: &str) -> Result<(u8, String), AnalysisError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(AnalysisError::invalid_format("Invalid TRACK line in CUE"));
    }

    let number: u8 = parts[1]
        .parse()
        .map_err(|_| AnalysisError::invalid_format("Invalid track number in CUE"))?;
    let mode = parts[2].to_string();

    Ok((number, mode))
}

// ---------------------------------------------------------------------------
// CHD disc reading
// ---------------------------------------------------------------------------

/// Read the PVD sector (sector 16) from a CHD file.
///
/// CHD CD images store raw 2352-byte sectors. We need to decompress the
/// appropriate hunk and extract the user data from sector 16.
pub fn read_chd_sector(
    reader: &mut dyn retro_junk_core::ReadSeek,
    sector: u64,
) -> Result<[u8; 2048], AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    let mut chd = chd::Chd::open(reader, None)
        .map_err(|e| AnalysisError::other(format!("Failed to open CHD: {}", e)))?;

    let hunk_size = chd.header().hunk_size() as u64;

    // CHD CD images: each sector is CHD_CD_SECTOR_SIZE bytes (2448)
    // The byte offset of our target sector within the logical data:
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

    // Within the raw sector (2352 bytes), user data starts at offset 24
    // (12 sync + 4 header + 8 subheader)
    let data_offset = offset_in_hunk + MODE2_FORM1_DATA_OFFSET as usize;
    if data_offset + 2048 > hunk_buf.len() {
        return Err(AnalysisError::corrupted_header(
            "CHD sector data extends beyond hunk boundary",
        ));
    }

    let mut result = [0u8; 2048];
    result.copy_from_slice(&hunk_buf[data_offset..data_offset + 2048]);
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

/// Find and read SYSTEM.CNF from a CHD disc image.
pub fn read_system_cnf_from_chd(
    reader: &mut dyn retro_junk_core::ReadSeek,
) -> Result<Vec<u8>, AnalysisError> {
    // Read PVD from sector 16
    let pvd_data = read_chd_sector(reader, PVD_SECTOR)?;

    // Verify PVD
    if pvd_data[0] != 0x01 || &pvd_data[1..6] != b"CD001" {
        return Err(AnalysisError::invalid_format(
            "CHD: Missing PVD at sector 16",
        ));
    }

    let system_id = read_str_a(&pvd_data[8..40]);
    if !system_id.starts_with("PLAYSTATION") {
        return Err(AnalysisError::invalid_format(format!(
            "Not a PlayStation disc (system ID: '{}')",
            system_id,
        )));
    }

    // Parse root directory record from PVD
    let root_record = &pvd_data[156..190];
    let root_lba = u32::from_le_bytes([
        root_record[2],
        root_record[3],
        root_record[4],
        root_record[5],
    ]);
    let root_size = u32::from_le_bytes([
        root_record[10],
        root_record[11],
        root_record[12],
        root_record[13],
    ]);

    // Walk root directory to find SYSTEM.CNF
    let dir_sectors = (root_size as u64 + 2047) / 2048;

    for sector_offset in 0..dir_sectors {
        let sector = root_lba as u64 + sector_offset;
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
                if id_stripped == "SYSTEM.CNF" {
                    // Read the file
                    return read_file_from_chd(reader, &dir_rec);
                }
            }

            pos += record_len;
        }
    }

    Err(AnalysisError::other(
        "SYSTEM.CNF not found in CHD root directory",
    ))
}

/// Read file content from a CHD image given a directory record.
fn read_file_from_chd(
    reader: &mut dyn retro_junk_core::ReadSeek,
    record: &DirectoryRecord,
) -> Result<Vec<u8>, AnalysisError> {
    let mut result = Vec::with_capacity(record.data_length as usize);
    let sectors_needed = (record.data_length as u64 + 2047) / 2048;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // -- Test helpers --

    /// Build a minimal 2048-byte PVD sector with a given system identifier.
    fn make_pvd_sector(system_id: &str) -> [u8; 2048] {
        let mut sector = [0u8; 2048];
        sector[0] = 0x01; // PVD type
        sector[1..6].copy_from_slice(b"CD001"); // standard identifier
        sector[6] = 0x01; // version

        // System identifier at offset 8, 32 bytes padded with spaces
        let id_bytes = system_id.as_bytes();
        let len = id_bytes.len().min(32);
        sector[8..8 + len].copy_from_slice(&id_bytes[..len]);
        for i in len..32 {
            sector[8 + i] = b' ';
        }

        // Volume identifier at offset 40, 32 bytes
        let vol = b"TEST_VOLUME";
        sector[40..40 + vol.len()].copy_from_slice(vol);
        for i in vol.len()..32 {
            sector[40 + i] = b' ';
        }

        // Volume space size at offset 80 (LE) — say 200 sectors
        sector[80..84].copy_from_slice(&200u32.to_le_bytes());
        sector[84..88].copy_from_slice(&200u32.to_be_bytes());

        // Root directory record at offset 156 (34 bytes)
        sector[156] = 34; // record length
        // extent LBA at record+2 (LE) — sector 18
        sector[158..162].copy_from_slice(&18u32.to_le_bytes());
        // data length at record+10 (LE) — 2048 bytes (1 sector)
        sector[166..170].copy_from_slice(&2048u32.to_le_bytes());

        sector
    }

    /// Build a minimal ISO: 16 sectors of padding + PVD at sector 16.
    fn make_iso(system_id: &str) -> Vec<u8> {
        let mut data = vec![0u8; 16 * 2048]; // 16 empty sectors
        let pvd = make_pvd_sector(system_id);
        data.extend_from_slice(&pvd);
        data
    }

    /// Wrap 2048 bytes of user data into a raw 2352-byte Mode 2 Form 1 sector.
    fn make_raw_sector(user_data: &[u8; 2048]) -> [u8; 2352] {
        let mut sector = [0u8; 2352];
        // 12 bytes sync
        sector[0..12].copy_from_slice(&CD_SYNC_PATTERN);
        // 4 bytes header (MSF + mode) — just set mode to 2
        sector[15] = 0x02;
        // 8 bytes subheader — zeros are fine
        // 2048 bytes user data at offset 24
        sector[24..24 + 2048].copy_from_slice(user_data);
        // Remaining bytes (EDC/ECC) left as zero
        sector
    }

    /// Build a raw BIN: 16 raw empty sectors + raw PVD sector.
    fn make_raw_bin(system_id: &str) -> Vec<u8> {
        let empty_user = [0u8; 2048];
        let mut data = Vec::new();
        for _ in 0..16 {
            data.extend_from_slice(&make_raw_sector(&empty_user));
        }
        let pvd = make_pvd_sector(system_id);
        data.extend_from_slice(&make_raw_sector(&pvd));
        data
    }

    /// Build a directory record for a file.
    fn make_dir_record(filename: &str, extent_lba: u32, data_length: u32) -> Vec<u8> {
        let id_bytes = filename.as_bytes();
        let id_len = id_bytes.len();
        let record_len = 33 + id_len + (id_len % 2); // pad to even
        let mut record = vec![0u8; record_len];
        record[0] = record_len as u8;
        record[2..6].copy_from_slice(&extent_lba.to_le_bytes());
        record[10..14].copy_from_slice(&data_length.to_le_bytes());
        record[25] = 0; // file flags (regular file)
        record[32] = id_len as u8;
        record[33..33 + id_len].copy_from_slice(id_bytes);
        record
    }

    /// Build a full ISO with a root directory containing SYSTEM.CNF.
    fn make_iso_with_system_cnf(serial: &str) -> Vec<u8> {
        let system_cnf_content = format!("BOOT = cdrom:\\{};1\r\nVMODE = NTSC\r\n", serial);
        let cnf_bytes = system_cnf_content.as_bytes();

        // Layout:
        // Sectors 0-15: empty padding
        // Sector 16: PVD (root dir at sector 18, 1 sector)
        // Sector 17: empty (VD terminator)
        // Sector 18: root directory (with "." ".." and "SYSTEM.CNF;1" entries)
        // Sector 19: SYSTEM.CNF content

        let mut data = vec![0u8; 16 * 2048]; // sectors 0-15

        // Sector 16: PVD
        let mut pvd = make_pvd_sector("PLAYSTATION");
        // Point root dir to sector 18, size 2048
        pvd[158..162].copy_from_slice(&18u32.to_le_bytes());
        pvd[166..170].copy_from_slice(&2048u32.to_le_bytes());
        data.extend_from_slice(&pvd);

        // Sector 17: empty (VD set terminator would go here)
        data.extend_from_slice(&[0u8; 2048]);

        // Sector 18: root directory
        let mut dir_sector = [0u8; 2048];
        let mut pos = 0;

        // "." entry (current directory)
        let dot_record = make_dir_record("\0", 18, 2048);
        dir_sector[pos..pos + dot_record.len()].copy_from_slice(&dot_record);
        pos += dot_record.len();

        // ".." entry (parent directory)
        let dotdot_record = make_dir_record("\x01", 18, 2048);
        dir_sector[pos..pos + dotdot_record.len()].copy_from_slice(&dotdot_record);
        pos += dotdot_record.len();

        // SYSTEM.CNF entry pointing to sector 19
        let cnf_record = make_dir_record("SYSTEM.CNF;1", 19, cnf_bytes.len() as u32);
        dir_sector[pos..pos + cnf_record.len()].copy_from_slice(&cnf_record);

        data.extend_from_slice(&dir_sector);

        // Sector 19: SYSTEM.CNF content
        let mut cnf_sector = [0u8; 2048];
        cnf_sector[..cnf_bytes.len()].copy_from_slice(cnf_bytes);
        data.extend_from_slice(&cnf_sector);

        data
    }

    // -- Format detection tests --

    #[test]
    fn test_detect_iso_format() {
        let data = make_iso("PLAYSTATION");
        let mut cursor = Cursor::new(data);
        assert_eq!(
            detect_disc_format(&mut cursor).unwrap(),
            DiscFormat::Iso2048
        );
    }

    #[test]
    fn test_detect_raw_bin_format() {
        let data = make_raw_bin("PLAYSTATION");
        let mut cursor = Cursor::new(data);
        assert_eq!(
            detect_disc_format(&mut cursor).unwrap(),
            DiscFormat::RawSector2352
        );
    }

    #[test]
    fn test_detect_chd_magic() {
        let mut data = vec![0u8; 64];
        data[..8].copy_from_slice(CHD_MAGIC);
        let mut cursor = Cursor::new(data);
        assert_eq!(detect_disc_format(&mut cursor).unwrap(), DiscFormat::Chd);
    }

    #[test]
    fn test_detect_cue_text() {
        let cue = b"FILE \"game.bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n";
        let mut cursor = Cursor::new(cue.to_vec());
        assert_eq!(detect_disc_format(&mut cursor).unwrap(), DiscFormat::Cue);
    }

    #[test]
    fn test_detect_invalid_data() {
        let data = vec![
            0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let mut cursor = Cursor::new(data);
        assert!(detect_disc_format(&mut cursor).is_err());
    }

    // -- PVD parsing tests --

    #[test]
    fn test_read_pvd_iso() {
        let data = make_iso("PLAYSTATION");
        let mut cursor = Cursor::new(data);
        let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
        assert_eq!(pvd.system_identifier, "PLAYSTATION");
        assert_eq!(pvd.volume_identifier, "TEST_VOLUME");
        assert_eq!(pvd.volume_space_size, 200);
    }

    #[test]
    fn test_read_pvd_raw_bin() {
        let data = make_raw_bin("PLAYSTATION");
        let mut cursor = Cursor::new(data);
        let pvd = read_pvd(&mut cursor, DiscFormat::RawSector2352).unwrap();
        assert_eq!(pvd.system_identifier, "PLAYSTATION");
        assert_eq!(pvd.volume_identifier, "TEST_VOLUME");
    }

    #[test]
    fn test_pvd_non_playstation() {
        let data = make_iso("SOME_OTHER_SYS");
        let mut cursor = Cursor::new(data);
        let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
        assert_eq!(pvd.system_identifier, "SOME_OTHER_SYS");
    }

    // -- SYSTEM.CNF parsing tests --

    #[test]
    fn test_parse_system_cnf_standard() {
        let cnf = "BOOT = cdrom:\\SLUS_012.34;1\r\nVMODE = NTSC\r\n";
        let result = parse_system_cnf(cnf).unwrap();
        assert_eq!(result.boot_path, "cdrom:\\SLUS_012.34;1");
        assert_eq!(result.vmode.as_deref(), Some("NTSC"));
    }

    #[test]
    fn test_parse_system_cnf_boot2() {
        let cnf = "BOOT2 = cdrom0:\\SLPS_123.45;1\r\n";
        let result = parse_system_cnf(cnf).unwrap();
        assert_eq!(result.boot_path, "cdrom0:\\SLPS_123.45;1");
        assert_eq!(result.vmode, None);
    }

    #[test]
    fn test_parse_system_cnf_missing_boot() {
        let cnf = "VMODE = PAL\r\n";
        assert!(parse_system_cnf(cnf).is_err());
    }

    // -- Serial extraction tests --

    #[test]
    fn test_extract_serial_slus() {
        assert_eq!(
            extract_serial("cdrom:\\SLUS_012.34;1"),
            Some("SLUS-01234".to_string())
        );
    }

    #[test]
    fn test_extract_serial_sles() {
        assert_eq!(
            extract_serial("cdrom:\\SLES_567.89;1"),
            Some("SLES-56789".to_string())
        );
    }

    #[test]
    fn test_extract_serial_scps() {
        assert_eq!(
            extract_serial("cdrom:\\SCPS_100.01;1"),
            Some("SCPS-10001".to_string())
        );
    }

    #[test]
    fn test_extract_serial_double_backslash() {
        assert_eq!(
            extract_serial("cdrom:\\\\SLUS_012.34;1"),
            Some("SLUS-01234".to_string())
        );
    }

    #[test]
    fn test_extract_serial_no_version() {
        assert_eq!(
            extract_serial("cdrom:\\SLPS_000.01"),
            Some("SLPS-00001".to_string())
        );
    }

    #[test]
    fn test_extract_serial_no_backslash() {
        // Some games use "cdrom:FILENAME" with no path separator
        assert_eq!(
            extract_serial("cdrom:SLUS_006.91;1"),
            Some("SLUS-00691".to_string())
        );
    }

    #[test]
    fn test_extract_serial_invalid() {
        assert_eq!(extract_serial("cdrom:\\BOOT.EXE;1"), None);
    }

    // -- Region mapping tests --

    #[test]
    fn test_serial_to_region() {
        assert_eq!(serial_to_region("SLUS-01234"), Some(Region::Usa));
        assert_eq!(serial_to_region("SCUS-94900"), Some(Region::Usa));
        assert_eq!(serial_to_region("SLES-01234"), Some(Region::Europe));
        assert_eq!(serial_to_region("SCES-01234"), Some(Region::Europe));
        assert_eq!(serial_to_region("SLPS-01234"), Some(Region::Japan));
        assert_eq!(serial_to_region("SCPS-01234"), Some(Region::Japan));
        assert_eq!(serial_to_region("SLPM-01234"), Some(Region::Japan));
        assert_eq!(serial_to_region("SLKA-01234"), Some(Region::Korea));
        assert_eq!(serial_to_region("XXXX-01234"), None);
    }

    // -- CUE parsing tests --

    #[test]
    fn test_parse_cue_single_track() {
        let cue = "FILE \"game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n";
        let sheet = parse_cue(cue).unwrap();
        assert_eq!(sheet.files.len(), 1);
        assert_eq!(sheet.files[0].filename, "game.bin");
        assert_eq!(sheet.files[0].file_type, "BINARY");
        assert_eq!(sheet.files[0].tracks.len(), 1);
        assert_eq!(sheet.files[0].tracks[0].number, 1);
        assert_eq!(sheet.files[0].tracks[0].mode, "MODE2/2352");
    }

    #[test]
    fn test_parse_cue_multi_track() {
        let cue = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 00 45:00:00
    INDEX 01 45:02:00
  TRACK 03 AUDIO
    INDEX 00 50:30:00
    INDEX 01 50:32:00
"#;
        let sheet = parse_cue(cue).unwrap();
        assert_eq!(sheet.files.len(), 1);
        assert_eq!(sheet.files[0].tracks.len(), 3);
        assert_eq!(sheet.files[0].tracks[0].mode, "MODE2/2352");
        assert_eq!(sheet.files[0].tracks[1].mode, "AUDIO");
        assert_eq!(sheet.files[0].tracks[2].number, 3);
    }

    #[test]
    fn test_parse_cue_multiple_files() {
        let cue = r#"FILE "game (Track 1).bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
FILE "game (Track 2).bin" BINARY
  TRACK 02 AUDIO
    INDEX 01 00:00:00
"#;
        let sheet = parse_cue(cue).unwrap();
        assert_eq!(sheet.files.len(), 2);
        assert_eq!(sheet.files[0].filename, "game (Track 1).bin");
        assert_eq!(sheet.files[1].filename, "game (Track 2).bin");
    }

    // -- Full ISO analysis tests --

    #[test]
    fn test_find_system_cnf_in_iso() {
        let data = make_iso_with_system_cnf("SLUS_012.34");
        let mut cursor = Cursor::new(data);
        let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
        let content =
            find_file_in_root(&mut cursor, DiscFormat::Iso2048, &pvd, "SYSTEM.CNF").unwrap();
        let text = String::from_utf8_lossy(&content);
        assert!(text.contains("SLUS_012.34"));
    }

    #[test]
    fn test_full_iso_serial_extraction() {
        let data = make_iso_with_system_cnf("SLUS_012.34");
        let mut cursor = Cursor::new(data);
        let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
        let content =
            find_file_in_root(&mut cursor, DiscFormat::Iso2048, &pvd, "SYSTEM.CNF").unwrap();
        let text = String::from_utf8_lossy(&content);
        let cnf = parse_system_cnf(&text).unwrap();
        let serial = extract_serial(&cnf.boot_path).unwrap();
        assert_eq!(serial, "SLUS-01234");
    }

    #[test]
    fn test_file_not_found_in_root() {
        let data = make_iso_with_system_cnf("SLUS_012.34");
        let mut cursor = Cursor::new(data);
        let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
        assert!(find_file_in_root(&mut cursor, DiscFormat::Iso2048, &pvd, "NONEXIST.TXT").is_err());
    }
}
