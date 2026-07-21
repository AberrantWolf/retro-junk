//! Track-aware disc hashing for Redump DAT verification.
//!
//! Redump DATs store per-track checksums of raw 2352-byte sector data.
//! These functions handle the various disc container formats (CHD, raw BIN,
//! CUE) to produce hashes that match Redump entries.

use retro_junk_core::{AnalysisError, FileHashes, HashAlgorithms, HashProgressFn, MultiHasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::format::{DiscFormat, detect_disc_format};
use crate::sector::{CD_SYNC_PATTERN, RAW_SECTOR_SIZE, SECTOR_USER_DATA_START};

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
    /// Mode 2 Form 1 sectors (`PlayStation`). User data at offset 24.
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
    on_progress: HashProgressFn<'_>,
) -> Result<Option<FileHashes>, AnalysisError> {
    let format = detect_disc_format(reader)?;

    match format {
        DiscFormat::Chd => {
            log::info!("{platform_name} compute_container_hashes: CHD detected");
            let hashes = hash_chd_raw_sectors(reader, algorithms, on_progress)?;
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
            if let Some((data_size, was_zero_padded)) = detect_data_track_size(reader)? {
                log::info!(
                    "{} compute_container_hashes: raw BIN, hashing Track 1 ({} bytes{})",
                    platform_name,
                    data_size,
                    if was_zero_padded {
                        ", zero-padded audio detected"
                    } else {
                        ""
                    }
                );
                let mut hashes = hash_raw_bin_track1(reader, algorithms, data_size, on_progress)?;
                if was_zero_padded {
                    hashes.warnings.push(ZERO_PADDED_WARNING.to_string());
                }
                Ok(Some(hashes))
            } else {
                // Single-track BIN — let the standard hasher handle it
                Ok(None)
            }
        }
        DiscFormat::Cue => {
            // CUE sheets: hash the referenced BIN, not the CUE text
            if let Some(path) = file_path {
                let hashes = hash_cue_track1(reader, algorithms, path, on_progress)?;
                Ok(Some(hashes))
            } else {
                log::warn!(
                    "{platform_name} compute_container_hashes: CUE without file_path, cannot resolve BIN"
                );
                Ok(None)
            }
        }
        // ISOs: let the standard hasher handle them
        DiscFormat::Iso2048 => Ok(None),
    }
}

/// Hash the largest data track from a CHD disc image, extracting the
/// 2352-byte raw sector data and stripping any subchannel bytes.
///
/// Parses all track metadata to find the largest data track (by frame count).
/// For single-data-track discs (PS1/PS2), this is Track 1. For multi-data-track
/// discs (Saturn with MODE1 boot + MODE2 main), this is typically Track 2.
///
/// Falls back to hashing from sector 0 if no track metadata is found.
///
/// DVD-media CHDs (produced by `chdman createdvd`, e.g. PSP UMD / PS2 DVD
/// dumps) use a 2048-byte `unit_bytes` (one logical ISO 9660 sector per
/// unit) and carry no `CD_ROM_TRACK`/`CD_ROM_TRACK2` metadata at all — see
/// `.claude/skills/retro-archive/formats/CHD.md` for how this was verified
/// against the `chd` crate's header parsing. For those, delegate to
/// [`hash_chd_whole_stream`], which hashes the logical byte stream directly;
/// the CD-specific 2352-byte raw-sector extraction below would read past
/// each unit's bounds (`unit_bytes` < `RAW_SECTOR_SIZE`) and either panic on
/// a hunk-boundary slice or (mid-hunk) silently mix in the next sector's
/// bytes.
pub fn hash_chd_raw_sectors(
    reader: &mut dyn retro_junk_core::ReadSeek,
    algorithms: HashAlgorithms,
    on_progress: HashProgressFn<'_>,
) -> Result<FileHashes, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    let mut chd = chd::Chd::open(reader, None)
        .map_err(|e| AnalysisError::other(format!("Failed to open CHD: {e}")))?;

    if u64::from(chd.header().unit_bytes()) < RAW_SECTOR_SIZE {
        log::info!(
            "CHD hashing: unit_bytes={} < raw CD sector size, treating as DVD-media CHD",
            chd.header().unit_bytes()
        );
        return hash_chd_whole_stream(&mut chd, algorithms, on_progress);
    }

    // Parse all track metadata and select the largest data track.
    let tracks = crate::chd::parse_chd_tracks(&mut chd)?;
    let (start_sector, sectors_to_hash) =
        if let Some(target) = crate::chd::select_largest_data_track(&tracks) {
            log::info!(
                "CHD hashing: selected Track {} ({}, {} frames, starts at sector {})",
                target.track_number,
                target.track_type,
                target.frames,
                target.start_sector
            );
            (target.start_sector, target.frames)
        } else {
            // No track metadata — fall back to all sectors from sector 0
            let logical_bytes = chd.header().logical_bytes();
            let unit_bytes = u64::from(chd.header().unit_bytes());
            let total = (logical_bytes / unit_bytes) as usize;
            log::warn!("CHD: no track metadata found, hashing all {total} sectors from sector 0");
            (0, total)
        };

    let hunk_size = chd.header().hunk_size() as usize;
    let total_hunks = chd.header().hunk_count();
    let unit_bytes = chd.header().unit_bytes() as usize;
    let sectors_per_hunk = hunk_size / unit_bytes;
    let data_size = sectors_to_hash as u64 * RAW_SECTOR_SIZE;

    log::info!(
        "CHD hashing: {sectors_to_hash} sectors ({data_size} bytes) starting at sector {start_sector}, unit_bytes={unit_bytes}"
    );

    let mut hasher = MultiHasher::new(algorithms, data_size, on_progress);

    let mut hunk_buf = chd.get_hunksized_buffer();
    let mut cmp_buf = Vec::new();

    // Track our position in the CHD's global sector space
    let end_sector = start_sector + sectors_to_hash;
    let mut global_sector = 0usize;
    let mut sectors_hashed = 0usize;

    for hunk_num in 0..total_hunks {
        if sectors_hashed >= sectors_to_hash {
            break;
        }

        // Skip hunks entirely before the start sector
        let hunk_end_sector = global_sector + sectors_per_hunk;
        if hunk_end_sector <= start_sector {
            global_sector = hunk_end_sector;
            continue;
        }

        let mut hunk = chd
            .hunk(hunk_num)
            .map_err(|e| AnalysisError::other(format!("Failed to get CHD hunk {hunk_num}: {e}")))?;

        hunk.read_hunk_in(&mut cmp_buf, &mut hunk_buf)
            .map_err(|e| {
                AnalysisError::other(format!("Failed to decompress CHD hunk {hunk_num}: {e}"))
            })?;

        for s in 0..sectors_per_hunk {
            let sector_idx = global_sector + s;
            if sector_idx < start_sector {
                continue;
            }
            if sector_idx >= end_sector {
                break;
            }
            let offset = s * unit_bytes;
            // Hash only the raw 2352-byte sector data, stripping any
            // subchannel bytes that follow when unit_bytes > 2352.
            let raw_sector = &hunk_buf[offset..offset + RAW_SECTOR_SIZE as usize];
            hasher.update(raw_sector);
            sectors_hashed += 1;
        }

        global_sector += sectors_per_hunk;

        // Report progress per hunk (coarser than per-sector, but avoids
        // excessive callback overhead for the many small sectors per hunk).
        hasher.report_progress();
    }

    Ok(hasher.finalize())
}

/// Hash a DVD-media CHD (`chdman createdvd` output) by streaming its decoded
/// logical bytes directly, with no CD-style sync/header/subchannel stripping.
///
/// DVD CHDs store one 2048-byte ISO 9660 sector per unit and have no track
/// structure — the decompressed logical stream *is* the ISO image that
/// Redump hashes, byte for byte. Hunks are padded to `hunk_size`, so this
/// stops at `logical_bytes`, not at the end of the last hunk.
fn hash_chd_whole_stream<F: Read + Seek>(
    chd: &mut chd::Chd<F>,
    algorithms: HashAlgorithms,
    on_progress: HashProgressFn<'_>,
) -> Result<FileHashes, AnalysisError> {
    let logical_bytes = chd.header().logical_bytes();
    let total_hunks = chd.header().hunk_count();

    let mut hasher = MultiHasher::new(algorithms, logical_bytes, on_progress);

    let mut hunk_buf = chd.get_hunksized_buffer();
    let mut cmp_buf = Vec::new();
    let mut remaining = logical_bytes;

    for hunk_num in 0..total_hunks {
        if remaining == 0 {
            break;
        }

        let mut hunk = chd
            .hunk(hunk_num)
            .map_err(|e| AnalysisError::other(format!("Failed to get CHD hunk {hunk_num}: {e}")))?;

        hunk.read_hunk_in(&mut cmp_buf, &mut hunk_buf)
            .map_err(|e| {
                AnalysisError::other(format!("Failed to decompress CHD hunk {hunk_num}: {e}"))
            })?;

        let take = (hunk_buf.len() as u64).min(remaining) as usize;
        hasher.update(&hunk_buf[..take]);
        remaining -= take as u64;

        hasher.report_progress();
    }

    Ok(hasher.finalize())
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
        let mid = lo + (hi - lo).div_ceil(2);
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

/// Warning message for zero-padded audio track detection.
const ZERO_PADDED_WARNING: &str = "Incomplete dump: audio track missing (zero-filled Mode 2 padding detected). Data track intact \u{2014} hash valid.";

/// Check if a sector has the CD sync pattern AND non-zero user data.
///
/// Returns `true` for real data sectors with actual content.
/// Returns `false` for zero-padded filler sectors (sync pattern present but
/// user data is all zeros — a sign of a bad dump where the audio track was
/// written as empty Mode 2 sectors).
fn is_real_data_sector(
    reader: &mut dyn retro_junk_core::ReadSeek,
    sector_index: u64,
) -> Result<bool, AnalysisError> {
    // Read sync pattern + header + subheader + start of user data
    // We check 64 bytes of user data to distinguish real content from zeros.
    const CHECK_SIZE: usize = SECTOR_USER_DATA_START + 64;

    let offset = sector_index * RAW_SECTOR_SIZE;
    reader.seek(SeekFrom::Start(offset))?;

    let mut buf = [0u8; CHECK_SIZE];
    let n = reader.read(&mut buf)?;
    if n < CHECK_SIZE {
        return Ok(false);
    }

    // Must have sync pattern
    if buf[..12] != CD_SYNC_PATTERN {
        return Ok(false);
    }

    // Check if user data region is all zeros (filler sector)
    let user_data = &buf[SECTOR_USER_DATA_START..];
    Ok(user_data.iter().any(|&b| b != 0))
}

/// Find the boundary between real data sectors and zero-padded filler sectors.
///
/// Some bad dumps write audio tracks as Mode 2 sectors with CD sync patterns
/// but all-zero user data. The standard `find_raw_bin_data_track_size()` can't
/// detect this boundary because both sides have sync patterns.
///
/// Uses binary search on `is_real_data_sector()` to find the last sector with
/// non-zero user data.
///
/// Returns `Some(data_size)` if a boundary is found, `None` if the entire file
/// has real data or doesn't start with real data.
pub fn find_zero_padded_track_boundary(
    reader: &mut dyn retro_junk_core::ReadSeek,
) -> Result<Option<u64>, AnalysisError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    if file_size < RAW_SECTOR_SIZE {
        return Ok(None);
    }

    let total_sectors = file_size / RAW_SECTOR_SIZE;

    // Sector 0 must be real data
    if !is_real_data_sector(reader, 0)? {
        return Ok(None);
    }

    // If the last sector also has real data, no boundary to find
    if is_real_data_sector(reader, total_sectors - 1)? {
        return Ok(None);
    }

    // Binary search: find last sector with real (non-zero) data
    let mut lo: u64 = 0;
    let mut hi = total_sectors - 1;

    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if is_real_data_sector(reader, mid)? {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }

    let data_track_size = (lo + 1) * RAW_SECTOR_SIZE;
    log::info!(
        "Raw BIN: zero-padded boundary at sector {} ({} bytes data, {} bytes total)",
        lo + 1,
        data_track_size,
        file_size
    );

    Ok(Some(data_track_size))
}

/// Detect the data track size in a raw BIN file, chaining two detection methods.
///
/// 1. Try sync-pattern boundary detection (fast, handles normal multi-track BINs)
/// 2. Try zero-padded filler detection (handles bad dumps with Mode 2 filler)
///
/// Returns `(data_size, was_zero_padded)` if a boundary is found, or `None`
/// if the entire file appears to be a single data track.
pub fn detect_data_track_size(
    reader: &mut dyn retro_junk_core::ReadSeek,
) -> Result<Option<(u64, bool)>, AnalysisError> {
    // Primary: sync-pattern boundary (audio sectors lack sync pattern)
    if let Some(size) = find_raw_bin_data_track_size(reader)? {
        return Ok(Some((size, false)));
    }

    // Secondary: zero-padded filler detection
    if let Some(size) = find_zero_padded_track_boundary(reader)? {
        return Ok(Some((size, true)));
    }

    Ok(None)
}

/// Determine Track 1 byte size from CUE INDEX entries.
///
/// If the CUE has 2+ tracks for the same file and Track 2 has an INDEX 01
/// entry, the Track 1 size is `track2_index01_sector * sector_size`, where
/// `sector_size` is derived from the tracks' declared mode (see
/// [`crate::cue::sector_size_for_mode`]) rather than assumed to be raw
/// 2352-byte sectors. Returns `None` if the information is insufficient.
#[must_use]
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

    // Derive the sector size from the declared mode instead of assuming raw
    // 2352-byte sectors, so MODE2/2336 and MODE1/2048 single-bin rips get
    // the correct Track 1 boundary.
    let sector_size = crate::cue::sector_size_for_mode(&file.tracks[0].mode);
    if file
        .tracks
        .iter()
        .any(|t| crate::cue::sector_size_for_mode(&t.mode) != sector_size)
    {
        warnings.push(
            "CUE mixes sector sizes within one file; cannot derive Track 1 boundary".to_string(),
        );
        return (None, warnings);
    }

    // Look for Track 2's INDEX 01 to find the boundary
    if let Some(track2) = file.tracks.get(1) {
        if let Some(idx01) = track2.indexes.iter().find(|i| i.number == 1) {
            let track1_size = idx01.to_sector_offset() * sector_size;
            if track1_size > 0 && track1_size <= bin_size {
                return (Some(track1_size), warnings);
            }
            warnings.push(format!(
                "Track 2 INDEX 01 gives size {track1_size} but BIN is {bin_size} bytes"
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
    on_progress: HashProgressFn<'_>,
) -> Result<FileHashes, AnalysisError> {
    // Read CUE text
    reader.seek(SeekFrom::Start(0))?;
    let mut cue_text = String::new();
    reader.read_to_string(&mut cue_text)?;

    let sheet = crate::cue::parse_cue(&cue_text)?;

    let parent = file_path
        .parent()
        .ok_or_else(|| AnalysisError::other("CUE file has no parent directory"))?;

    // Find first data track file (may be None for minimal CUEs without TRACK entries)
    let first_data_file = sheet.files.iter().find(|f| {
        f.tracks
            .iter()
            .any(|t| t.mode.to_uppercase().contains("MODE"))
    });

    let mut warnings = Vec::new();

    let data_file = if let Some(f) = first_data_file {
        f
    } else {
        let f = sheet
            .files
            .first()
            .ok_or_else(|| AnalysisError::invalid_format("CUE has no files"))?;
        warnings.push(
            "CUE has no track metadata — hashing entire BIN. Dump may be incomplete.".to_string(),
        );
        log::warn!(
            "CUE has no track metadata, falling back to first file: {}",
            f.filename
        );
        f
    };

    let bin_path = parent.join(&data_file.filename);
    let mut bin_file = if bin_path.exists() {
        std::fs::File::open(&bin_path)?
    } else {
        // CDRWin CUE: DATAFILE references a virtual name; the actual data
        // lives at the start of the combined BIN referenced by other FILE entries.
        if let Some(real_file) = find_existing_bin_in_cue(&sheet, parent) {
            log::info!(
                "CUE hash: DATAFILE '{}' not found, using '{}'",
                data_file.filename,
                real_file.display()
            );
            std::fs::File::open(&real_file)?
        } else {
            return Err(AnalysisError::other(format!(
                "BIN file not found: {}",
                bin_path.display()
            )));
        }
    };
    let bin_size = bin_file.seek(SeekFrom::End(0))?;
    bin_file.seek(SeekFrom::Start(0))?;

    // Multi-BIN CUE: hash the first data track file entirely. Ignore orphan
    // FILE directives with no TRACK children here: strict whole-disc
    // verification rejects them, but identification should still be able to
    // recover a valid data-track hash from a damaged descriptor.
    let files_with_tracks = sheet
        .files
        .iter()
        .filter(|file| !file.tracks.is_empty())
        .count();
    if files_with_tracks > 1 {
        log::info!(
            "CUE hash: multi-file CUE, hashing first data file '{}' ({} bytes)",
            data_file.filename,
            bin_size
        );
        let mut hashes = hash_raw_bin_track1(&mut bin_file, algorithms, bin_size, on_progress)?;
        hashes.warnings = warnings;
        return Ok(hashes);
    }

    // Single-BIN CUE: try INDEX-based boundary first
    let (index_size, index_warnings) = compute_track1_size_from_cue(&sheet, bin_size);
    warnings.extend(index_warnings);

    if let Some(track1_size) = index_size {
        log::info!(
            "CUE hash: Track 1 size from INDEX = {track1_size} bytes (BIN = {bin_size} bytes)"
        );
        let mut hashes = hash_raw_bin_track1(&mut bin_file, algorithms, track1_size, on_progress)?;
        hashes.warnings = warnings;
        return Ok(hashes);
    }

    // Fall back to chained detection (sync-pattern, then zero-padded filler)
    if let Some((data_size, was_zero_padded)) = detect_data_track_size(&mut bin_file)? {
        log::info!(
            "CUE hash: Track 1 size from {} detection = {} bytes (BIN = {} bytes)",
            if was_zero_padded {
                "zero-padded"
            } else {
                "sync"
            },
            data_size,
            bin_size
        );
        let mut hashes = hash_raw_bin_track1(&mut bin_file, algorithms, data_size, on_progress)?;
        if was_zero_padded {
            warnings.push(ZERO_PADDED_WARNING.to_string());
        }
        hashes.warnings = warnings;
        return Ok(hashes);
    }

    // Final fallback: hash entire BIN
    if sheet.files[0].tracks.len() > 1 {
        warnings.push("Could not determine Track 1 boundary — hashed entire BIN".to_string());
    }
    log::info!("CUE hash: no boundary detected, hashing entire BIN ({bin_size} bytes)");
    let mut hashes = hash_raw_bin_track1(&mut bin_file, algorithms, bin_size, on_progress)?;
    hashes.warnings = warnings;
    Ok(hashes)
}

/// Find the first existing BIN file referenced by any FILE entry in the CUE sheet.
///
/// Used as a fallback when a `CDRWin` DATAFILE references a virtual filename
/// that doesn't exist on disk — the actual data is in the combined BIN.
fn find_existing_bin_in_cue(
    sheet: &crate::cue::CueSheet,
    parent: &Path,
) -> Option<std::path::PathBuf> {
    sheet.files.iter().find_map(|f| {
        let p = parent.join(&f.filename);
        if p.exists() { Some(p) } else { None }
    })
}

/// Hash the first `data_size` bytes of a raw BIN file.
///
/// Shared by all disc-based analyzers for hashing Track 1 of multi-track BIN files.
pub fn hash_raw_bin_track1(
    reader: &mut dyn retro_junk_core::ReadSeek,
    algorithms: HashAlgorithms,
    data_size: u64,
    on_progress: HashProgressFn<'_>,
) -> Result<FileHashes, AnalysisError> {
    reader.seek(SeekFrom::Start(0))?;

    let mut hasher = MultiHasher::new(algorithms, data_size, on_progress);
    let mut buf = vec![0u8; 64 * 1024];
    let mut remaining = data_size;

    while remaining > 0 {
        let to_read = remaining.min(buf.len() as u64) as usize;
        let n = reader.read(&mut buf[..to_read])?;
        if n == 0 {
            break;
        }
        hasher.update_with_progress(&buf[..n]);
        remaining -= n as u64;
    }

    Ok(hasher.finalize())
}

#[cfg(test)]
#[path = "tests/hash_tests.rs"]
mod tests;
