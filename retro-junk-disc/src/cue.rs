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
        } else if upper.starts_with("TRACK ")
            && let Some(ref mut f) = current_file
        {
            let (number, mode) = parse_cue_track_line(line)?;
            f.tracks.push(CueTrack {
                number,
                mode,
                indexes: Vec::new(),
            });
        } else if upper.starts_with("INDEX ")
            && let Some(ref mut f) = current_file
            && let Some(ref mut track) = f.tracks.last_mut()
        {
            if let Ok(index) = parse_cue_index_line(line) {
                track.indexes.push(index);
            }
        }
        // Ignore PREGAP, POSTGAP, REM, etc.
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
