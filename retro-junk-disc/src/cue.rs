//! CUE sheet parsing.

use retro_junk_core::AnalysisError;

/// A parsed CUE sheet.
#[derive(Debug, Clone)]
pub struct CueSheet {
    pub files: Vec<CueFile>,
}

/// A FILE entry in a CUE sheet.
#[derive(Debug, Clone)]
pub struct CueFile {
    pub filename: String,
    pub tracks: Vec<CueTrack>,
}

/// A TRACK entry in a CUE sheet.
#[derive(Debug, Clone)]
pub struct CueTrack {
    pub number: u8,
    pub mode: String,
    pub indexes: Vec<CueIndex>,
    /// Frames of pregap declared via a PREGAP directive (gap data NOT stored
    /// in the file). 0 when absent. In-file pregaps use INDEX 00 instead.
    pub pregap_frames: u64,
    /// Frames declared via POSTGAP (not stored in the file). 0 when absent.
    pub postgap_frames: u64,
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
    #[must_use]
    pub fn to_sector_offset(&self) -> u64 {
        (u64::from(self.minutes) * 60 + u64::from(self.seconds)) * 75 + u64::from(self.frames)
    }
}

/// Parse a CUE sheet from its text content.
///
/// Supports both standard CUE format (`FILE`/`TRACK <num> <mode>`) and
/// `CDRWin` extended format (`DATAFILE`/`TRACK <mode>` without track numbers).
///
/// In `CDRWin` format, `TRACK` lines may appear *before* their `DATAFILE`/`FILE`
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

        // Detect the directive by its first whitespace-delimited token
        // (rather than a literal-space prefix) so tab-separated cue sheets
        // parse identically to space-separated ones.
        let (token, rest) = split_first_token(line);
        let keyword = token.to_uppercase();

        match keyword.as_str() {
            "FILE" | "DATAFILE" | "AUDIOFILE" => {
                // Save previous file entry
                if let Some(f) = current_file.take() {
                    files.push(f);
                }

                let (filename, _file_type) = parse_cue_file_line_at(rest)?;
                let mut new_file = CueFile {
                    filename,
                    tracks: Vec::new(),
                };
                // Attach any pending tracks (CDRWin: TRACK before DATAFILE)
                if !pending_tracks.is_empty() {
                    new_file.tracks.append(&mut pending_tracks);
                }
                current_file = Some(new_file);
            }
            "TRACK" => {
                auto_track_number += 1;
                let (number, mode) = parse_cue_track_line(line, auto_track_number)?;
                let track = CueTrack {
                    number,
                    mode,
                    indexes: Vec::new(),
                    pregap_frames: 0,
                    postgap_frames: 0,
                };
                if let Some(ref mut f) = current_file {
                    f.tracks.push(track);
                } else {
                    // CDRWin: TRACK appears before its DATAFILE/FILE
                    pending_tracks.push(track);
                }
            }
            "INDEX" => {
                // A cue that lies about its indexes must fail loudly: a
                // silently dropped INDEX line now feeds destructive
                // verify-then-delete logic downstream.
                let index = parse_cue_index_line(line)?;
                if let Some(ref mut f) = current_file
                    && let Some(ref mut track) = f.tracks.last_mut()
                {
                    track.indexes.push(index);
                } else if let Some(ref mut track) = pending_tracks.last_mut() {
                    track.indexes.push(index);
                }
            }
            "PREGAP" => {
                let frames = msf_to_sectors(rest)?;
                let track = current_file
                    .as_mut()
                    .and_then(|f| f.tracks.last_mut())
                    .or_else(|| pending_tracks.last_mut())
                    .ok_or_else(|| {
                        AnalysisError::invalid_format(format!(
                            "PREGAP directive with no current TRACK: {line}"
                        ))
                    })?;
                track.pregap_frames = frames;
            }
            "POSTGAP" => {
                let frames = msf_to_sectors(rest)?;
                let track = current_file
                    .as_mut()
                    .and_then(|f| f.tracks.last_mut())
                    .or_else(|| pending_tracks.last_mut())
                    .ok_or_else(|| {
                        AnalysisError::invalid_format(format!(
                            "POSTGAP directive with no current TRACK: {line}"
                        ))
                    })?;
                track.postgap_frames = frames;
            }
            // Ignore REM, CD_ROM_XA, NO COPY, etc.
            _ => {}
        }
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

/// Split a trimmed CUE line into its first whitespace-delimited token
/// (verbatim, not case-changed) and the trimmed remainder after it. Tabs and
/// runs of spaces are both accepted as separators.
fn split_first_token(line: &str) -> (&str, &str) {
    match line.find(char::is_whitespace) {
        Some(idx) => (&line[..idx], line[idx..].trim_start()),
        None => (line, ""),
    }
}

/// Parse the remainder of a FILE/DATAFILE/AUDIOFILE line (after the keyword):
/// `"filename.bin" BINARY` or `"filename.bin" 01:32:21`.
fn parse_cue_file_line_at(rest: &str) -> Result<(String, String), AnalysisError> {
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
/// `CDRWin` format: `TRACK MODE2_RAW` or `TRACK AUDIO` (2 parts, no track number)
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
        return Err(AnalysisError::invalid_format(format!(
            "Invalid INDEX line in CUE: {line}"
        )));
    }

    let number: u8 = parts[1].parse().map_err(|_| {
        AnalysisError::invalid_format(format!("Invalid index number in CUE: {line}"))
    })?;

    let (minutes, seconds, frames) = parse_msf(parts[2]).map_err(|_| {
        AnalysisError::invalid_format(format!("Invalid MSF timestamp in CUE INDEX: {line}"))
    })?;

    Ok(CueIndex {
        number,
        minutes,
        seconds,
        frames,
    })
}

/// Parse an "MM:SS:FF" timestamp string into its (minutes, seconds, frames)
/// components. Shared by [`parse_cue_index_line`] and [`msf_to_sectors`].
fn parse_msf(s: &str) -> Result<(u32, u32, u32), AnalysisError> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return Err(AnalysisError::invalid_format(format!(
            "Invalid MSF timestamp: {s}"
        )));
    }

    let minutes: u32 = parts[0]
        .parse()
        .map_err(|_| AnalysisError::invalid_format(format!("Invalid minutes in MSF: {s}")))?;
    let seconds: u32 = parts[1]
        .parse()
        .map_err(|_| AnalysisError::invalid_format(format!("Invalid seconds in MSF: {s}")))?;
    let frames: u32 = parts[2]
        .parse()
        .map_err(|_| AnalysisError::invalid_format(format!("Invalid frames in MSF: {s}")))?;

    Ok((minutes, seconds, frames))
}

/// Sector size in bytes for a cue TRACK mode string (standard or `CDRWin`).
///
/// Standard modes carry the size after the slash (`MODE1/2352`, `MODE2/2336`);
/// `CDRWin` modes are looked up by name. Unknown modes default to raw (2352).
///
/// `CDRWin` bare mode names (`MODE1`, `MODE2`, `MODE2_FORM1`, `MODE2_FORM2`,
/// `MODE2_FORM_MIX`, `MODE1_RAW`, `MODE2_RAW`) are CDRWin/cdrdao TOC-format
/// knowledge; see `.claude/skills/retro-archive/formats/CUE.md` for sourcing.
#[must_use]
pub fn sector_size_for_mode(mode: &str) -> u64 {
    // A slash suffix is a standard-CUE explicit size (`MODE1/2352`); when
    // present but not numeric (e.g. a malformed `MODE2/abc`), fall back to
    // a name lookup on the part before the slash rather than the size we
    // couldn't parse.
    let name = match mode.rsplit_once('/') {
        Some((prefix, size)) => {
            if let Ok(n) = size.trim().parse::<u64>() {
                return n;
            }
            prefix
        }
        None => mode,
    };
    match name.to_uppercase().as_str() {
        "MODE1" | "MODE2_FORM1" => 2048, // CDRWin cooked
        "MODE2_FORM2" => 2324,
        "MODE2" | "MODE2_FORM_MIX" => 2336,
        // MODE1_RAW, MODE2_RAW, AUDIO, and anything unrecognized: raw 2352.
        _ => 2352,
    }
}

/// Convert an MSF timestamp string "MM:SS:FF" to a sector count.
fn msf_to_sectors(msf: &str) -> Result<u64, AnalysisError> {
    let (minutes, seconds, frames) = parse_msf(msf)?;
    Ok((u64::from(minutes) * 60 + u64::from(seconds)) * 75 + u64::from(frames))
}

#[cfg(test)]
#[path = "tests/cue_tests.rs"]
mod tests;
