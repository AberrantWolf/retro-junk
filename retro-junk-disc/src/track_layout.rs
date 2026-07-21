//! Per-track byte spans for disc source layouts (cue/bin and GDI track sets).
//!
//! CHD round-trip verification compares track content hashes between the
//! original source files and `chdman extractcd --splitbin` output. A
//! [`TrackSpan`] locates one track's raw bytes inside a source file,
//! independent of whether the source stores one track per file (Redump
//! multi-bin, GDI) or all tracks in a single bin.
//!
//! Track boundaries: within each FILE, track 1 owns everything from byte 0;
//! each subsequent track starts at its first INDEX (00 when present, else
//! 01). Spans always tile the entire file, so round-trip verification covers
//! every byte. The same rule applies to the source cue and to the cue
//! chdman writes on extraction, which is what makes the comparison sound.

use std::fs;
use std::path::{Path, PathBuf};

use retro_junk_core::AnalysisError;

use crate::cue::{CueSheet, CueTrack, sector_size_for_mode};

/// One track's raw bytes: a contiguous range within a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackSpan {
    pub track_number: u8,
    pub file: PathBuf,
    pub byte_offset: u64,
    pub byte_len: u64,
}

/// Absolute frame offset of a track's first index (INDEX 00 when present).
fn first_index_frame(track: &CueTrack) -> Result<u64, AnalysisError> {
    track
        .indexes
        .iter()
        .min_by_key(|i| i.number)
        .map(super::cue::CueIndex::to_sector_offset)
        .ok_or_else(|| {
            AnalysisError::invalid_format(format!(
                "CUE TRACK {:02} has no INDEX lines",
                track.number
            ))
        })
}

/// Compute per-track byte spans for a parsed cue sheet.
///
/// `cue_dir` is the directory containing the cue (FILE references are
/// resolved against it, as players do). Referenced files must exist; their
/// sizes bound the final track of each file.
pub fn cue_track_spans(sheet: &CueSheet, cue_dir: &Path) -> Result<Vec<TrackSpan>, AnalysisError> {
    let mut spans = Vec::new();
    let mut seen_track_numbers = std::collections::HashSet::new();
    let mut previous_track_number = None;

    for file in &sheet.files {
        let path = cue_dir.join(&file.filename);
        let file_size = fs::metadata(&path)
            .map_err(|e| {
                AnalysisError::other(format!("Cannot read size of {}: {e}", path.display()))
            })?
            .len();

        if file.tracks.is_empty() {
            return Err(AnalysisError::invalid_format(format!(
                "CUE FILE \"{}\" has no tracks",
                file.filename
            )));
        }
        for track in &file.tracks {
            if !seen_track_numbers.insert(track.number) {
                return Err(AnalysisError::invalid_format(format!(
                    "CUE declares TRACK {:02} more than once",
                    track.number
                )));
            }
            if previous_track_number.is_some_and(|previous| track.number <= previous) {
                return Err(AnalysisError::invalid_format(
                    "CUE track numbers are not strictly increasing",
                ));
            }
            previous_track_number = Some(track.number);
        }

        // INDEX times are frame counts from the start of the file, so byte
        // positions are only well-defined when all tracks in the file share
        // one sector size (mixed sizes within a single bin cannot be
        // expressed coherently in cue format anyway).
        let sector_size = sector_size_for_mode(&file.tracks[0].mode);
        if let Some(t) = file
            .tracks
            .iter()
            .find(|t| sector_size_for_mode(&t.mode) != sector_size)
        {
            return Err(AnalysisError::unsupported(format!(
                "CUE FILE \"{}\" mixes sector sizes ({} and {})",
                file.filename, file.tracks[0].mode, t.mode
            )));
        }

        let mut starts = Vec::with_capacity(file.tracks.len());
        for track in &file.tracks {
            starts.push(first_index_frame(track)? * sector_size);
        }
        // Track 1 owns everything from byte 0: any bytes before its first
        // INDEX (e.g. a TRACK 01 declaring only INDEX 01 at a nonzero
        // offset) are its implicit pregap region, not uncovered by any
        // span. This makes coverage total by construction — spans always
        // tile the whole file.
        starts[0] = 0;

        for (i, track) in file.tracks.iter().enumerate() {
            let start = starts[i];
            let end = starts.get(i + 1).copied().unwrap_or(file_size);
            if start >= end || end > file_size {
                return Err(AnalysisError::invalid_format(format!(
                    "CUE TRACK {:02} has invalid byte span {start}..{end} in \"{}\" ({file_size} bytes)",
                    track.number, file.filename
                )));
            }
            spans.push(TrackSpan {
                track_number: track.number,
                file: path.clone(),
                byte_offset: start,
                byte_len: end - start,
            });
        }
    }

    Ok(spans)
}

/// One track entry of a GDI (GD-ROM track list) file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GdiTrack {
    pub number: u8,
    /// Starting LBA on the disc (high-density area starts at 45000).
    pub lba: u64,
    /// 4 = data, 0 = audio.
    pub track_type: u32,
    pub sector_size: u64,
    pub filename: String,
    /// Byte offset of the track data within the file (normally 0).
    pub offset: u64,
}

/// Parse GDI content: a track count line, then one
/// `number lba type sector_size filename offset` line per track.
/// Filenames containing spaces are double-quoted.
pub fn parse_gdi(content: &str) -> Result<Vec<GdiTrack>, AnalysisError> {
    let mut lines = content.lines().map(str::trim).filter(|l| !l.is_empty());

    let count: usize = lines
        .next()
        .ok_or_else(|| AnalysisError::invalid_format("GDI file is empty"))?
        .parse()
        .map_err(|_| AnalysisError::invalid_format("GDI first line is not a track count"))?;

    let mut tracks = Vec::new();
    for line in lines {
        let fields = split_gdi_fields(line);
        if fields.len() != 6 {
            return Err(AnalysisError::invalid_format(format!(
                "GDI track line has {} fields, expected 6: {line}",
                fields.len()
            )));
        }
        let parse_err =
            |what: &str| AnalysisError::invalid_format(format!("GDI: invalid {what}: {line}"));
        tracks.push(GdiTrack {
            number: fields[0].parse().map_err(|_| parse_err("track number"))?,
            lba: fields[1].parse().map_err(|_| parse_err("LBA"))?,
            track_type: fields[2].parse().map_err(|_| parse_err("track type"))?,
            sector_size: fields[3].parse().map_err(|_| parse_err("sector size"))?,
            filename: fields[4].clone(),
            offset: fields[5].parse().map_err(|_| parse_err("offset"))?,
        });
    }

    if tracks.len() != count {
        return Err(AnalysisError::invalid_format(format!(
            "GDI declares {count} tracks but lists {}",
            tracks.len()
        )));
    }
    Ok(tracks)
}

/// Split a GDI track line into fields, honoring double-quoted filenames.
fn split_gdi_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut rest = line.trim();
    while !rest.is_empty() {
        if let Some(after_quote) = rest.strip_prefix('"') {
            let end = after_quote.find('"').unwrap_or(after_quote.len());
            fields.push(after_quote[..end].to_string());
            rest = after_quote[end..]
                .strip_prefix('"')
                .unwrap_or("")
                .trim_start();
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            fields.push(rest[..end].to_string());
            rest = rest[end..].trim_start();
        }
    }
    fields
}

/// Compute per-track byte spans for a GDI track set: each track is the
/// whole of its file, minus any declared start offset.
pub fn gdi_track_spans(
    tracks: &[GdiTrack],
    gdi_dir: &Path,
) -> Result<Vec<TrackSpan>, AnalysisError> {
    tracks
        .iter()
        .map(|t| {
            let path = gdi_dir.join(&t.filename);
            let file_size = fs::metadata(&path)
                .map_err(|e| {
                    AnalysisError::other(format!("Cannot read size of {}: {e}", path.display()))
                })?
                .len();
            if t.offset > file_size {
                return Err(AnalysisError::invalid_format(format!(
                    "GDI track {} offset {} is beyond \"{}\" ({file_size} bytes)",
                    t.number, t.offset, t.filename
                )));
            }
            Ok(TrackSpan {
                track_number: t.number,
                file: path,
                byte_offset: t.offset,
                byte_len: file_size - t.offset,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/track_layout_tests.rs"]
mod tests;
