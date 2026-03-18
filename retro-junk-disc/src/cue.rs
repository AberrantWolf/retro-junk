//! CUE sheet parsing.

use retro_junk_core::AnalysisError;

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
    pub indexes: Vec<CueIndex>,
}

/// An INDEX entry in a CUE sheet track.
#[derive(Debug, Clone)]
pub struct CueIndex {
    pub number: u8,
    pub minutes: u32,
    pub seconds: u32,
    pub frames: u32,
}

impl CueIndex {
    /// Convert MSF (minutes:seconds:frames) to an absolute sector offset.
    /// CD audio uses 75 frames per second.
    pub fn to_sector_offset(&self) -> u64 {
        ((self.minutes * 60 + self.seconds) as u64) * 75 + self.frames as u64
    }
}

/// Parse a CUE sheet from its text content.
///
/// Supports both standard CUE format (`FILE`/`TRACK <num> <mode>`) and
/// CDRWin extended format (`DATAFILE`/`TRACK <mode>` without track numbers).
///
/// In CDRWin format, `TRACK` lines may appear *before* their `DATAFILE`/`FILE`
/// directive (the opposite of standard CUE). Orphan tracks are buffered and
/// attached to the next file entry.
pub fn parse_cue(content: &str) -> Result<CueSheet, AnalysisError> {
    let mut files = Vec::new();
    let mut current_file: Option<CueFile> = None;
    let mut auto_track_number: u8 = 0;
    // Tracks that appeared before any FILE/DATAFILE (CDRWin order)
    let mut pending_tracks: Vec<CueTrack> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        let upper = line.to_uppercase();

        if upper.starts_with("FILE ")
            || upper.starts_with("DATAFILE ")
            || upper.starts_with("AUDIOFILE ")
        {
            // Save previous file entry
            if let Some(f) = current_file.take() {
                files.push(f);
            }

            let is_datafile = upper.starts_with("DATAFILE ");
            let skip_len = if is_datafile {
                9
            } else if upper.starts_with("AUDIOFILE ") {
                10
            } else {
                5
            };
            let (filename, file_type) = parse_cue_file_line_at(line, skip_len)?;
            let mut new_file = CueFile {
                filename,
                file_type: if is_datafile {
                    "BINARY".to_string()
                } else {
                    file_type
                },
                tracks: Vec::new(),
            };
            // Attach any pending tracks (CDRWin: TRACK before DATAFILE)
            if !pending_tracks.is_empty() {
                new_file.tracks.append(&mut pending_tracks);
            }
            current_file = Some(new_file);
        } else if upper.starts_with("TRACK ") {
            auto_track_number += 1;
            let (number, mode) = parse_cue_track_line(line, auto_track_number)?;
            let track = CueTrack {
                number,
                mode,
                indexes: Vec::new(),
            };
            if let Some(ref mut f) = current_file {
                f.tracks.push(track);
            } else {
                // CDRWin: TRACK appears before its DATAFILE/FILE
                pending_tracks.push(track);
            }
        } else if upper.starts_with("INDEX ") {
            // Attach to last track in current_file or pending_tracks
            if let Ok(index) = parse_cue_index_line(line) {
                if let Some(ref mut f) = current_file
                    && let Some(ref mut track) = f.tracks.last_mut()
                {
                    track.indexes.push(index);
                } else if let Some(ref mut track) = pending_tracks.last_mut() {
                    track.indexes.push(index);
                }
            }
        }
        // Ignore PREGAP, POSTGAP, REM, CD_ROM_XA, NO COPY, etc.
    }

    if let Some(f) = current_file.take() {
        files.push(f);
    }

    // If there are still pending tracks with no file, we can't do much
    if files.is_empty() {
        return Err(AnalysisError::invalid_format(
            "CUE sheet contains no FILE entries",
        ));
    }

    Ok(CueSheet { files })
}

/// Parse a FILE/DATAFILE line: `FILE "filename.bin" BINARY` or `DATAFILE "filename.bin" 01:32:21`
///
/// `skip_len` is the number of bytes to skip for the keyword prefix
/// (5 for "FILE ", 9 for "DATAFILE ").
fn parse_cue_file_line_at(line: &str, skip_len: usize) -> Result<(String, String), AnalysisError> {
    let rest = &line[skip_len..];

    let (filename, remainder) = if let Some(after_quote) = rest.strip_prefix('"') {
        // Quoted filename
        let end_quote = after_quote
            .find('"')
            .ok_or_else(|| AnalysisError::invalid_format("Unterminated quote in CUE FILE line"))?;
        let filename = after_quote[..end_quote].to_string();
        let remainder = after_quote[end_quote + 1..].trim().to_string();
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

/// Parse a TRACK line.
///
/// Standard format: `TRACK 01 MODE2/2352` (3 parts)
/// CDRWin format: `TRACK MODE2_RAW` or `TRACK AUDIO` (2 parts, no track number)
///
/// When the track number is omitted, `fallback_number` is used instead.
fn parse_cue_track_line(line: &str, fallback_number: u8) -> Result<(u8, String), AnalysisError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 3 {
        // Standard: TRACK <number> <mode>
        if let Ok(number) = parts[1].parse::<u8>() {
            return Ok((number, parts[2].to_string()));
        }
    }
    if parts.len() >= 2 {
        // CDRWin: TRACK <mode> (no number)
        return Ok((fallback_number, parts[1].to_string()));
    }
    Err(AnalysisError::invalid_format("Invalid TRACK line in CUE"))
}

/// Parse an INDEX line: `INDEX 01 54:04:52`
fn parse_cue_index_line(line: &str) -> Result<CueIndex, AnalysisError> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(AnalysisError::invalid_format("Invalid INDEX line in CUE"));
    }

    let number: u8 = parts[1]
        .parse()
        .map_err(|_| AnalysisError::invalid_format("Invalid index number in CUE"))?;

    let msf_parts: Vec<&str> = parts[2].split(':').collect();
    if msf_parts.len() != 3 {
        return Err(AnalysisError::invalid_format(
            "Invalid MSF timestamp in CUE INDEX",
        ));
    }

    let minutes: u32 = msf_parts[0]
        .parse()
        .map_err(|_| AnalysisError::invalid_format("Invalid minutes in CUE INDEX"))?;
    let seconds: u32 = msf_parts[1]
        .parse()
        .map_err(|_| AnalysisError::invalid_format("Invalid seconds in CUE INDEX"))?;
    let frames: u32 = msf_parts[2]
        .parse()
        .map_err(|_| AnalysisError::invalid_format("Invalid frames in CUE INDEX"))?;

    Ok(CueIndex {
        number,
        minutes,
        seconds,
        frames,
    })
}

#[cfg(test)]
#[path = "tests/cue_tests.rs"]
mod tests;
