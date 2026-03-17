//! Track-aware disc hashing for Redump DAT verification.
//!
//! Redump DATs store per-track checksums of raw 2352-byte sector data.
//! These functions handle the various disc container formats (CHD, raw BIN,
//! CUE) to produce hashes that match Redump entries.

use retro_junk_core::{AnalysisError, FileHashes, HashAlgorithms};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::format::{DiscFormat, detect_disc_format};
use crate::sector::{CD_SYNC_PATTERN, CHD_CD_SECTOR_SIZE, RAW_SECTOR_SIZE};

/// Sector mode for raw disc images.
///
/// Different disc systems use different CD-ROM sector modes:
/// - Mode 1: 12 sync + 4 header + 2048 data + 288 ECC/EDC (used by Saturn, Sega CD)
/// - Mode 2 Form 1: 12 sync + 4 header + 8 subheader + 2048 data + 280 ECC/EDC (used by PS1, PS2)
///
/// This affects how user data is extracted from raw sectors, but NOT how
/// raw sectors are hashed for Redump (which always hashes the full 2352 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorMode {
    /// Mode 1 sectors (Saturn, Sega CD). User data at offset 16.
    Mode1,
    /// Mode 2 Form 1 sectors (PlayStation). User data at offset 24.
    Mode2Form1,
}

/// Standard disc container hashing dispatch.
///
/// Handles CHD, multi-track BIN, and CUE containers. Returns `Ok(None)` for
/// plain ISOs (which should use the standard streaming hasher).
///
/// This is the canonical implementation — all disc-based analyzers (PS1, PS2,
/// Saturn, Sega CD, etc.) should delegate to this function instead of
/// reimplementing the same dispatch logic.
pub fn hash_disc_container(
    reader: &mut dyn retro_junk_core::ReadSeek,
    algorithms: HashAlgorithms,
    file_path: Option<&Path>,
    platform_name: &str,
) -> Result<Option<FileHashes>, AnalysisError> {
    let format = detect_disc_format(reader)?;

    match format {
        DiscFormat::Chd => {
            log::info!("{} compute_container_hashes: CHD detected", platform_name);
            let hashes = hash_chd_raw_sectors(reader, algorithms)?;
            log::info!(
                "{} compute_container_hashes: done, crc32={}, data_size={}",
                platform_name,
                hashes.crc32,
                hashes.data_size
            );
            Ok(Some(hashes))
        }
        DiscFormat::RawSector2352 => {
            // Multi-track BIN files contain data + audio tracks concatenated.
            // Redump DATs hash only Track 1 (data), so detect the boundary.
            if let Some(data_size) = find_raw_bin_data_track_size(reader)? {
                log::info!(
                    "{} compute_container_hashes: raw BIN, hashing Track 1 ({} bytes)",
                    platform_name,
                    data_size
                );
                let hashes = hash_raw_bin_track1(reader, algorithms, data_size)?;
                Ok(Some(hashes))
            } else {
                // Single-track BIN — let the standard hasher handle it
                Ok(None)
            }
        }
        DiscFormat::Cue => {
            // CUE sheets: hash the referenced BIN, not the CUE text
            if let Some(path) = file_path {
                let hashes = hash_cue_track1(reader, algorithms, path)?;
                Ok(Some(hashes))
            } else {
                log::warn!(
                    "{} compute_container_hashes: CUE without file_path, cannot resolve BIN",
                    platform_name
                );
                Ok(None)
            }
        }
        // ISOs: let the standard hasher handle them
        _ => Ok(None),
    }
}

/// Hash Track 1 (data track) raw sectors from a CHD disc image, extracting
/// the 2352-byte raw sector data and stripping the 96-byte subchannel from
/// each 2448-byte CHD sector. Only Track 1 is hashed because Redump/LibRetro
/// DAT entries contain per-track hashes, and the data track is Track 1.
pub fn hash_chd_raw_sectors(
    reader: &mut dyn retro_junk_core::ReadSeek,
    algorithms: HashAlgorithms,
) -> Result<FileHashes, AnalysisError> {
    use sha1::Digest;

    reader.seek(SeekFrom::Start(0))?;

    let mut chd = chd::Chd::open(reader, None)
        .map_err(|e| AnalysisError::other(format!("Failed to open CHD: {}", e)))?;

    // Parse track metadata to find Track 1's sector count.
    let track1_frames = crate::chd::parse_chd_track1_frames(&mut chd)?;

    let hunk_size = chd.header().hunk_size() as usize;
    let logical_bytes = chd.header().logical_bytes();
    let total_disc_sectors = logical_bytes / CHD_CD_SECTOR_SIZE as u64;
    let sectors_per_hunk = hunk_size / CHD_CD_SECTOR_SIZE as usize;
    let total_hunks = chd.header().hunk_count();

    // Hash only Track 1 sectors. Fall back to all sectors if metadata unavailable.
    let sectors_to_hash = track1_frames.unwrap_or_else(|| {
        log::warn!(
            "CHD: no track metadata found, hashing all {} sectors",
            total_disc_sectors
        );
        total_disc_sectors as usize
    });
    let data_size = sectors_to_hash as u64 * RAW_SECTOR_SIZE;

    log::info!(
        "CHD hashing: track1={} sectors ({} bytes), total_disc={} sectors",
        sectors_to_hash,
        data_size,
        total_disc_sectors
    );

    let mut crc = if algorithms.crc32() {
        Some(crc32fast::Hasher::new())
    } else {
        None
    };
    let mut sha = if algorithms.sha1() {
        Some(sha1::Sha1::new())
    } else {
        None
    };
    let mut md5_ctx = if algorithms.md5() {
        Some(md5::Context::new())
    } else {
        None
    };

    let mut hunk_buf = chd.get_hunksized_buffer();
    let mut cmp_buf = Vec::new();
    let mut sectors_remaining = sectors_to_hash;

    for hunk_num in 0..total_hunks {
        if sectors_remaining == 0 {
            break;
        }

        let mut hunk = chd.hunk(hunk_num).map_err(|e| {
            AnalysisError::other(format!("Failed to get CHD hunk {}: {}", hunk_num, e))
        })?;

        hunk.read_hunk_in(&mut cmp_buf, &mut hunk_buf)
            .map_err(|e| {
                AnalysisError::other(format!("Failed to decompress CHD hunk {}: {}", hunk_num, e))
            })?;

        let sectors_in_hunk = sectors_remaining.min(sectors_per_hunk);

        for s in 0..sectors_in_hunk {
            let offset = s * CHD_CD_SECTOR_SIZE as usize;
            let raw_sector = &hunk_buf[offset..offset + RAW_SECTOR_SIZE as usize];

            if let Some(ref mut h) = crc {
                h.update(raw_sector);
            }
            if let Some(ref mut h) = sha {
                h.update(raw_sector);
            }
            if let Some(ref mut h) = md5_ctx {
                h.consume(raw_sector);
            }
        }

        sectors_remaining -= sectors_in_hunk;
    }

    Ok(FileHashes {
        crc32: crc
            .map(|h| format!("{:08x}", h.finalize()))
            .unwrap_or_default(),
        sha1: sha.map(|h| format!("{:x}", h.finalize())),
        md5: md5_ctx.map(|h| format!("{:x}", h.compute())),
        data_size,
        warnings: vec![],
    })
}

/// Find the byte length of the data track (Track 1) in a raw 2352-byte sector BIN file.
///
/// Data sectors have the 12-byte CD sync pattern at the start; audio sectors do not.
/// Uses binary search to efficiently find the boundary between data and audio.
/// Returns `None` if the file doesn't start with raw sectors or is entirely data.
pub fn find_raw_bin_data_track_size(
    reader: &mut dyn retro_junk_core::ReadSeek,
) -> Result<Option<u64>, AnalysisError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    if file_size < RAW_SECTOR_SIZE {
        return Ok(None);
    }

    let total_sectors = file_size / RAW_SECTOR_SIZE;

    // Verify sector 0 is a data sector
    if !is_data_sector(reader, 0)? {
        return Ok(None);
    }

    // If the last sector is also data, the whole file is the data track
    if is_data_sector(reader, total_sectors - 1)? {
        return Ok(None); // no trimming needed — single-track or data-only
    }

    // Binary search for the boundary: last data sector
    let mut lo: u64 = 0;
    let mut hi = total_sectors - 1;

    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if is_data_sector(reader, mid)? {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    // lo is the last data sector index; data track size = (lo + 1) sectors
    let data_track_size = (lo + 1) * RAW_SECTOR_SIZE;
    log::info!(
        "Raw BIN: data track = {} sectors ({} bytes), total = {} sectors ({} bytes)",
        lo + 1,
        data_track_size,
        total_sectors,
        file_size
    );

    Ok(Some(data_track_size))
}

/// Check if a sector at the given index has the CD sync pattern (i.e., is a data sector).
fn is_data_sector(
    reader: &mut dyn retro_junk_core::ReadSeek,
    sector_index: u64,
) -> Result<bool, AnalysisError> {
    let offset = sector_index * RAW_SECTOR_SIZE;
    reader.seek(SeekFrom::Start(offset))?;
    let mut sync = [0u8; 12];
    let n = reader.read(&mut sync)?;
    Ok(n == 12 && sync == CD_SYNC_PATTERN)
}

/// Determine Track 1 byte size from CUE INDEX entries.
///
/// If the CUE has 2+ tracks for the same file and Track 2 has an INDEX 01
/// entry, the Track 1 size is `track2_index01_sector * 2352`.
/// Returns `None` if the information is insufficient.
pub fn compute_track1_size_from_cue(
    sheet: &crate::cue::CueSheet,
    bin_size: u64,
) -> (Option<u64>, Vec<String>) {
    let mut warnings = Vec::new();

    // Only relevant for single-file CUEs (multi-file CUEs hash per-file)
    if sheet.files.len() != 1 {
        return (None, warnings);
    }

    let file = &sheet.files[0];
    if file.tracks.len() < 2 {
        // Single track — hash entire BIN
        return (None, warnings);
    }

    // Look for Track 2's INDEX 01 to find the boundary
    if let Some(track2) = file.tracks.get(1) {
        if let Some(idx01) = track2.indexes.iter().find(|i| i.number == 1) {
            let track1_size = idx01.to_sector_offset() * RAW_SECTOR_SIZE;
            if track1_size > 0 && track1_size <= bin_size {
                return (Some(track1_size), warnings);
            }
            warnings.push(format!(
                "Track 2 INDEX 01 gives size {} but BIN is {} bytes",
                track1_size, bin_size
            ));
        } else {
            warnings.push("CUE has multiple tracks but Track 2 has no INDEX 01 entry".to_string());
        }
    }

    (None, warnings)
}

/// Hash Track 1 of a CUE-referenced BIN file.
///
/// Reads the CUE text from `reader`, resolves the BIN path relative to
/// `file_path`, and hashes Track 1 data. Always returns `Ok(Some(hashes))`
/// on success — never `Ok(None)`, which would cause the caller to hash
/// the CUE text itself.
pub fn hash_cue_track1(
    reader: &mut dyn retro_junk_core::ReadSeek,
    algorithms: HashAlgorithms,
    file_path: &Path,
) -> Result<FileHashes, AnalysisError> {
    // Read CUE text
    reader.seek(SeekFrom::Start(0))?;
    let mut cue_text = String::new();
    reader.read_to_string(&mut cue_text)?;

    let sheet = crate::cue::parse_cue(&cue_text)?;

    let parent = file_path
        .parent()
        .ok_or_else(|| AnalysisError::other("CUE file has no parent directory"))?;

    // Find first data track file
    let first_data_file = sheet
        .files
        .iter()
        .find(|f| {
            f.tracks
                .iter()
                .any(|t| t.mode.to_uppercase().contains("MODE"))
        })
        .ok_or_else(|| AnalysisError::invalid_format("CUE has no data tracks"))?;

    let bin_path = parent.join(&first_data_file.filename);
    if !bin_path.exists() {
        return Err(AnalysisError::other(format!(
            "BIN file not found: {}",
            bin_path.display()
        )));
    }

    let mut bin_file = std::fs::File::open(&bin_path)?;
    let bin_size = bin_file.seek(SeekFrom::End(0))?;
    bin_file.seek(SeekFrom::Start(0))?;

    let mut warnings = Vec::new();

    // Multi-BIN CUE: hash first data track file entirely
    if sheet.files.len() > 1 {
        log::info!(
            "CUE hash: multi-file CUE, hashing first data file '{}' ({} bytes)",
            first_data_file.filename,
            bin_size
        );
        let mut hashes = hash_raw_bin_track1(&mut bin_file, algorithms, bin_size)?;
        hashes.warnings = warnings;
        return Ok(hashes);
    }

    // Single-BIN CUE: try INDEX-based boundary first
    let (index_size, index_warnings) = compute_track1_size_from_cue(&sheet, bin_size);
    warnings.extend(index_warnings);

    if let Some(track1_size) = index_size {
        log::info!(
            "CUE hash: Track 1 size from INDEX = {} bytes (BIN = {} bytes)",
            track1_size,
            bin_size
        );
        let mut hashes = hash_raw_bin_track1(&mut bin_file, algorithms, track1_size)?;
        hashes.warnings = warnings;
        return Ok(hashes);
    }

    // Fall back to sync-pattern detection
    if let Some(data_size) = find_raw_bin_data_track_size(&mut bin_file)? {
        log::info!(
            "CUE hash: Track 1 size from sync detection = {} bytes (BIN = {} bytes)",
            data_size,
            bin_size
        );
        let mut hashes = hash_raw_bin_track1(&mut bin_file, algorithms, data_size)?;
        hashes.warnings = warnings;
        return Ok(hashes);
    }

    // Final fallback: hash entire BIN
    if sheet.files[0].tracks.len() > 1 {
        warnings.push("Could not determine Track 1 boundary — hashed entire BIN".to_string());
    }
    log::info!(
        "CUE hash: no boundary detected, hashing entire BIN ({} bytes)",
        bin_size
    );
    let mut hashes = hash_raw_bin_track1(&mut bin_file, algorithms, bin_size)?;
    hashes.warnings = warnings;
    Ok(hashes)
}

/// Hash the first `data_size` bytes of a raw BIN file.
///
/// Shared by all disc-based analyzers for hashing Track 1 of multi-track BIN files.
pub fn hash_raw_bin_track1(
    reader: &mut dyn retro_junk_core::ReadSeek,
    algorithms: HashAlgorithms,
    data_size: u64,
) -> Result<FileHashes, AnalysisError> {
    use sha1::Digest;

    reader.seek(SeekFrom::Start(0))?;

    let mut crc = if algorithms.crc32() {
        Some(crc32fast::Hasher::new())
    } else {
        None
    };
    let mut sha = if algorithms.sha1() {
        Some(sha1::Sha1::new())
    } else {
        None
    };
    let mut md5_ctx = if algorithms.md5() {
        Some(md5::Context::new())
    } else {
        None
    };

    let mut buf = [0u8; 64 * 1024];
    let mut remaining = data_size;

    while remaining > 0 {
        let to_read = remaining.min(buf.len() as u64) as usize;
        let n = reader.read(&mut buf[..to_read])?;
        if n == 0 {
            break;
        }
        if let Some(ref mut h) = crc {
            h.update(&buf[..n]);
        }
        if let Some(ref mut h) = sha {
            h.update(&buf[..n]);
        }
        if let Some(ref mut h) = md5_ctx {
            h.consume(&buf[..n]);
        }
        remaining -= n as u64;
    }

    Ok(FileHashes {
        crc32: crc
            .map(|h| format!("{:08x}", h.finalize()))
            .unwrap_or_default(),
        sha1: sha.map(|h| format!("{:x}", h.finalize())),
        md5: md5_ctx.map(|h| format!("{:x}", h.compute())),
        data_size,
        warnings: vec![],
    })
}

#[cfg(test)]
#[path = "tests/hash_tests.rs"]
mod tests;
