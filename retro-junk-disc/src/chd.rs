//! CHD (Compressed Hunks of Data) disc reading.

use retro_junk_core::AnalysisError;
use std::io::SeekFrom;

use crate::iso9660::{PrimaryVolumeDescriptor, parse_directory_record, parse_pvd_data};
use crate::sector::MODE2_FORM1_DATA_OFFSET;

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
        .map_err(|e| AnalysisError::other(format!("Failed to open CHD: {e}")))?;

    let hunk_size = u64::from(chd.header().hunk_size());

    // Use the CHD header's unit_bytes for the actual sector stride.
    // CHDs without subchannel data (SUBTYPE:NONE) use 2352 bytes per sector,
    // not the 2448 assumed by the CHD_CD_SECTOR_SIZE constant.
    let unit_bytes = u64::from(chd.header().unit_bytes());
    let sector_byte_offset = sector * unit_bytes;

    // Which hunk contains this offset?
    let hunk_num = sector_byte_offset / hunk_size;
    // Offset within the hunk
    let offset_in_hunk = (sector_byte_offset % hunk_size) as usize;

    let mut hunk_buf = chd.get_hunksized_buffer();
    let mut cmp_buf = Vec::new();

    let mut hunk = chd
        .hunk(hunk_num as u32)
        .map_err(|e| AnalysisError::other(format!("Failed to get CHD hunk {hunk_num}: {e}")))?;

    hunk.read_hunk_in(&mut cmp_buf, &mut hunk_buf)
        .map_err(|e| {
            AnalysisError::other(format!("Failed to decompress CHD hunk {hunk_num}: {e}"))
        })?;

    // Within the raw sector, user data starts at the given offset
    let final_offset = offset_in_hunk + data_offset;
    if final_offset + crate::sector::ISO_SECTOR_SIZE as usize > hunk_buf.len() {
        return Err(AnalysisError::corrupted_header(
            "CHD sector data extends beyond hunk boundary",
        ));
    }

    let mut result = [0u8; 2048];
    result.copy_from_slice(&hunk_buf[final_offset..final_offset + 2048]);
    Ok(result)
}

/// Read CHD header metadata for display purposes.
pub struct ChdInfo {
    pub version: u32,
    pub hunk_size: u32,
    pub logical_size: u64,
}

/// Extract basic CHD file information without full decompression.
pub fn read_chd_info(reader: &mut dyn retro_junk_core::ReadSeek) -> Result<ChdInfo, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    let chd = chd::Chd::open(reader, None)
        .map_err(|e| AnalysisError::other(format!("Failed to open CHD: {e}")))?;

    let header = chd.header();

    Ok(ChdInfo {
        version: header.version() as u32,
        hunk_size: header.hunk_size(),
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
    let dir_sectors = u64::from(pvd.root_dir_data_length).div_ceil(crate::sector::ISO_SECTOR_SIZE);
    let target_upper = filename.to_uppercase();

    for sector_offset in 0..dir_sectors {
        let sector = u64::from(pvd.root_dir_extent_lba) + sector_offset;
        let sector_data = read_chd_sector(reader, sector)?;

        let mut pos = 0;
        while pos < crate::sector::ISO_SECTOR_SIZE as usize {
            let record_len = sector_data[pos] as usize;
            if record_len == 0 {
                break;
            }
            if pos + record_len > crate::sector::ISO_SECTOR_SIZE as usize {
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
        "'{filename}' not found in CHD root directory",
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
    let sectors_needed = u64::from(record.data_length).div_ceil(crate::sector::ISO_SECTOR_SIZE);
    let mut remaining = record.data_length as usize;

    for i in 0..sectors_needed {
        let sector = u64::from(record.extent_lba) + i;
        let sector_data = read_chd_sector(reader, sector)?;
        let to_copy = remaining.min(crate::sector::ISO_SECTOR_SIZE as usize);
        result.extend_from_slice(&sector_data[..to_copy]);
        remaining -= to_copy;
    }

    Ok(result)
}

/// Parsed CHD track metadata entry.
#[derive(Debug, Clone)]
pub struct ChdTrackInfo {
    /// Track number (1-based).
    pub track_number: u32,
    /// Track type string (e.g., "`MODE1_RAW`", "`MODE2_RAW`", "AUDIO").
    pub track_type: String,
    /// Number of data frames (sectors) in this track.
    pub frames: usize,
    /// Sector offset where this track starts in the CHD's linear sector space.
    /// Computed by summing the frames of all preceding tracks.
    pub start_sector: usize,
}

impl ChdTrackInfo {
    /// Returns true if this is a data track (MODE1 or MODE2, not AUDIO).
    #[must_use]
    pub fn is_data(&self) -> bool {
        self.track_type.contains("MODE")
    }
}

/// Parse all CHD track metadata entries.
///
/// CHD CD-ROM track metadata is stored as text strings like:
///   `TRACK:1 TYPE:MODE1_RAW SUBTYPE:NONE FRAMES:19560 PREGAP:0 ...`
///
/// Returns tracks sorted by track number with computed `start_sector` offsets.
pub fn parse_chd_tracks<F: std::io::Read + std::io::Seek>(
    chd: &mut chd::Chd<F>,
) -> Result<Vec<ChdTrackInfo>, AnalysisError> {
    use chd::metadata::{KnownMetadata, MetadataTag};

    let meta_refs: Vec<_> = chd.metadata_refs().collect();
    let mut tracks = Vec::new();

    for meta_ref in &meta_refs {
        let tag = meta_ref.metatag();
        if tag != KnownMetadata::CdRomTrack as u32 && tag != KnownMetadata::CdRomTrack2 as u32 {
            continue;
        }

        let meta = meta_ref
            .read(chd.inner())
            .map_err(|e| AnalysisError::other(format!("Failed to read CHD metadata: {e}")))?;

        let text = String::from_utf8_lossy(&meta.value);

        if let Some(track_num_str) = parse_meta_field(&text, "TRACK")
            && let Ok(track_number) = track_num_str.parse::<u32>()
            && let Some(frames_str) = parse_meta_field(&text, "FRAMES")
            && let Ok(frames) = frames_str.parse::<usize>()
        {
            let track_type = parse_meta_field(&text, "TYPE")
                .unwrap_or("UNKNOWN")
                .to_string();

            tracks.push(ChdTrackInfo {
                track_number,
                track_type,
                frames,
                start_sector: 0, // computed below
            });
        }
    }

    // Sort by track number and compute cumulative sector offsets
    tracks.sort_by_key(|t| t.track_number);
    let mut offset = 0usize;
    for track in &mut tracks {
        track.start_sector = offset;
        // CD CHDs pad each track to a four-frame boundary internally. FRAMES
        // is the true track length; the padding belongs to no track and must
        // not shift the next track's hashes.
        offset += track.frames.next_multiple_of(4);
    }

    Ok(tracks)
}

/// Select the largest data track from parsed CHD track metadata.
///
/// Returns the track with the most frames among data tracks (those whose
/// TYPE contains "MODE"). This handles both single-data-track discs (PS1/PS2
/// where Track 1 is the only data track) and multi-data-track discs (Saturn
/// where Track 2 is often the largest data track).
#[must_use]
pub fn select_largest_data_track(tracks: &[ChdTrackInfo]) -> Option<&ChdTrackInfo> {
    tracks
        .iter()
        .filter(|t| t.is_data())
        .max_by_key(|t| t.frames)
}

/// Parse CHD track metadata (CHTR or CHT2) to find the number of frames
/// (sectors) in Track 1. Returns `None` if no track metadata is found.
///
/// Prefer [`parse_chd_tracks`] + [`select_largest_data_track`] for hash
/// matching, as some discs (Saturn) store the main data in Track 2.
pub fn parse_chd_track1_frames<F: std::io::Read + std::io::Seek>(
    chd: &mut chd::Chd<F>,
) -> Result<Option<usize>, AnalysisError> {
    let tracks = parse_chd_tracks(chd)?;
    if let Some(track) = tracks.iter().find(|t| t.track_number == 1) {
        log::info!("CHD track metadata: Track 1 has {} frames", track.frames);
        Ok(Some(track.frames))
    } else {
        Ok(None)
    }
}

/// Extract a field value from CHD metadata text (e.g., "FRAMES" from
/// `"TRACK:1 TYPE:MODE2_RAW SUBTYPE:NONE FRAMES:229020"`).
#[must_use]
pub fn parse_meta_field<'a>(text: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("{field}:");
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
