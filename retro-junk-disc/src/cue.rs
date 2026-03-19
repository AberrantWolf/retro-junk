//! CUE sheet parsing and compatibility detection.

use std::path::Path;

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

// -- CDRWin compatibility detection and conversion --

/// CDRWin disc-type headers that are not part of standard CUE format.
const CDRWIN_DISC_TYPES: &[&str] = &["CD_ROM_XA", "CD_ROM", "CD_DA"];

/// CDRWin directives to strip when converting to standard CUE format.
const CDRWIN_STRIP_DIRECTIVES: &[&str] = &[
    "NO COPY",
    "NO PRE_EMPHASIS",
    "TWO_CHANNEL_AUDIO",
    "SILENCE ",
    "START ",
    "START\n", // START alone on a line (no MSF)
    "ZERO ",
];

/// Detected CDRWin compatibility issues in a CUE sheet.
#[derive(Debug, Clone)]
pub struct CueCompatReport {
    /// CDRWin disc-type header found (e.g., "CD_ROM_XA").
    pub disc_type_header: Option<String>,
    /// Tracks using CDRWin mode syntax: (track number, CDRWin mode).
    pub cdwin_track_modes: Vec<(u8, String)>,
    /// Whether DATAFILE directives are present.
    pub has_datafile: bool,
    /// Whether AUDIOFILE directives are present.
    pub has_audiofile: bool,
    /// Whether extra CDRWin directives (NO COPY, SILENCE, etc.) are present.
    pub has_extra_directives: bool,
    /// Whether `//` comments are present.
    pub has_comments: bool,
    /// If set, auto-conversion is not possible; contains the reason.
    pub unfixable_reason: Option<String>,
}

impl CueCompatReport {
    /// Returns true if the CUE file uses only standard format (no CDRWin-isms).
    pub fn is_standard(&self) -> bool {
        self.disc_type_header.is_none()
            && self.cdwin_track_modes.is_empty()
            && !self.has_datafile
            && !self.has_audiofile
            && !self.has_extra_directives
            && !self.has_comments
    }

    /// Returns true if auto-conversion to standard format is possible.
    pub fn can_auto_fix(&self) -> bool {
        !self.is_standard() && self.unfixable_reason.is_none()
    }

    /// Short human-readable summary of what was detected.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(ref dt) = self.disc_type_header {
            parts.push(format!("{dt} header"));
        }
        if !self.cdwin_track_modes.is_empty() {
            parts.push(format!(
                "{} CDRWin track mode(s)",
                self.cdwin_track_modes.len()
            ));
        }
        if self.has_datafile {
            parts.push("DATAFILE".to_string());
        }
        if self.has_audiofile {
            parts.push("AUDIOFILE".to_string());
        }
        if self.has_extra_directives {
            parts.push("extra directives".to_string());
        }
        if self.has_comments {
            parts.push("comments".to_string());
        }
        parts.join(", ")
    }
}

/// Scan CUE text and report CDRWin compatibility issues.
///
/// This is a lightweight line-by-line scan that does not fully parse the CUE
/// structure. It identifies which CDRWin features are present.
pub fn check_cue_compat(content: &str) -> CueCompatReport {
    let mut report = CueCompatReport {
        disc_type_header: None,
        cdwin_track_modes: Vec::new(),
        has_datafile: false,
        has_audiofile: false,
        has_extra_directives: false,
        has_comments: false,
        unfixable_reason: None,
    };

    let mut auto_track_number: u8 = 0;
    let mut has_audiofile_with_offset = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let upper = trimmed.to_uppercase();

        // Check for // comments
        if trimmed.starts_with("//") {
            report.has_comments = true;
            continue;
        }

        // Check for disc-type headers
        for dt in CDRWIN_DISC_TYPES {
            if upper == *dt {
                report.disc_type_header = Some(dt.to_string());
            }
        }

        // Check for DATAFILE
        if upper.starts_with("DATAFILE ") {
            report.has_datafile = true;
        }

        // Check for AUDIOFILE with byte offset (unfixable)
        if upper.starts_with("AUDIOFILE ") {
            report.has_audiofile = true;
            // AUDIOFILE with #offset is unfixable
            if trimmed.contains('#') {
                has_audiofile_with_offset = true;
            }
        }

        // Check for CDRWin track modes
        if upper.starts_with("TRACK ") {
            auto_track_number += 1;
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            let mode = if parts.len() >= 3 {
                if parts[1].parse::<u8>().is_ok() {
                    parts[2]
                } else {
                    parts[1]
                }
            } else if parts.len() >= 2 {
                parts[1]
            } else {
                continue;
            };

            if convert_track_mode(mode).is_some() {
                let track_num = if parts.len() >= 3 {
                    parts[1].parse::<u8>().unwrap_or(auto_track_number)
                } else {
                    auto_track_number
                };
                report.cdwin_track_modes.push((track_num, mode.to_string()));
            }
        }

        // Check for extra CDRWin directives
        for directive in CDRWIN_STRIP_DIRECTIVES {
            let directive_upper = directive.trim();
            if upper == directive_upper || upper.starts_with(directive_upper) {
                // Make sure it's not a substring match of something else
                if upper == directive_upper || upper.starts_with(&format!("{directive_upper} ")) {
                    report.has_extra_directives = true;
                    break;
                }
            }
        }
    }

    if has_audiofile_with_offset {
        report.unfixable_reason = Some(
            "AUDIOFILE with byte offset (#) cannot be converted to standard CUE without splitting the audio file".to_string()
        );
    }

    report
}

/// Map a CDRWin track mode to its standard CUE equivalent.
/// Returns `None` if the mode is already standard or unknown.
fn convert_track_mode(mode: &str) -> Option<&'static str> {
    match mode.to_uppercase().as_str() {
        "MODE2_RAW" => Some("MODE2/2352"),
        "MODE1_RAW" => Some("MODE1/2352"),
        // Note: bare "MODE1" and "MODE2" are CDRWin-specific (no slash + size)
        "MODE2_FORM1" => Some("MODE2/2048"),
        "MODE2_FORM2" => Some("MODE2/2324"),
        "MODE2_FORM_MIX" => Some("MODE2/2336"),
        _ => None,
    }
}

/// Sector size in bytes for a track mode (CDRWin or standard).
fn sector_size_for_mode(mode: &str) -> u16 {
    match mode.to_uppercase().as_str() {
        "MODE1/2048" | "MODE2_FORM1" => 2048,
        "MODE1/2352" | "MODE1_RAW" => 2352,
        "MODE2/2048" => 2048,
        "MODE2/2324" | "MODE2_FORM2" => 2324,
        "MODE2/2336" | "MODE2" | "MODE2_FORM_MIX" => 2336,
        "MODE2/2352" | "MODE2_RAW" => 2352,
        "AUDIO" => 2352,
        // Default to raw sector size for unknown modes
        _ => 2352,
    }
}

/// Convert an MSF timestamp string "MM:SS:FF" to a sector count.
fn msf_to_sectors(msf: &str) -> Result<u64, AnalysisError> {
    let parts: Vec<&str> = msf.split(':').collect();
    if parts.len() != 3 {
        return Err(AnalysisError::invalid_format(
            "Invalid MSF timestamp in DATAFILE",
        ));
    }
    let minutes: u64 = parts[0]
        .parse()
        .map_err(|_| AnalysisError::invalid_format("Invalid minutes in DATAFILE MSF"))?;
    let seconds: u64 = parts[1]
        .parse()
        .map_err(|_| AnalysisError::invalid_format("Invalid seconds in DATAFILE MSF"))?;
    let frames: u64 = parts[2]
        .parse()
        .map_err(|_| AnalysisError::invalid_format("Invalid frames in DATAFILE MSF"))?;
    Ok((minutes * 60 + seconds) * 75 + frames)
}

/// Convert a sector offset back to MSF "MM:SS:FF" format.
fn sectors_to_msf(sectors: u64) -> String {
    let frames = sectors % 75;
    let total_seconds = sectors / 75;
    let seconds = total_seconds % 60;
    let minutes = total_seconds / 60;
    format!("{:02}:{:02}:{:02}", minutes, seconds, frames)
}

/// Convert a CDRWin-format CUE sheet to standard CUE format.
///
/// `cue_dir` is the directory containing the CUE file, used to resolve BIN
/// file paths when DATAFILE has MSF lengths and sector offsets need computing.
///
/// Returns the converted standard CUE text, or an error if conversion is not
/// possible (e.g., AUDIOFILE with byte offsets).
pub fn convert_cue_to_standard(content: &str, _cue_dir: &Path) -> Result<String, AnalysisError> {
    let mut output_lines: Vec<String> = Vec::new();
    let mut auto_track_number: u8 = 0;
    // Buffer tracks that appear before their FILE/DATAFILE (CDRWin order)
    let mut pending_tracks: Vec<(u8, String, Vec<String>)> = Vec::new(); // (number, mode, index_lines)
    // Track cumulative byte offset within a shared BIN for multi-track DATAFILE
    let mut cumulative_byte_offset: u64 = 0;
    let mut last_datafile: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let upper = trimmed.to_uppercase();

        // Skip comments
        if trimmed.starts_with("//") {
            continue;
        }

        // Skip disc-type headers
        if CDRWIN_DISC_TYPES
            .iter()
            .any(|dt| upper == dt.to_uppercase())
        {
            continue;
        }

        // Skip extra CDRWin directives
        let is_strip_directive = CDRWIN_STRIP_DIRECTIVES.iter().any(|d| {
            let d_upper = d.trim().to_uppercase();
            upper == d_upper || upper.starts_with(&format!("{d_upper} "))
        });
        if is_strip_directive {
            continue;
        }

        if upper.starts_with("AUDIOFILE ") {
            if trimmed.contains('#') {
                return Err(AnalysisError::invalid_format(
                    "AUDIOFILE with byte offset (#) cannot be converted to standard CUE",
                ));
            }
            // Simple AUDIOFILE without offset: convert to FILE ... WAVE
            let (filename, _remainder) = parse_cue_file_line_at(trimmed, 10)?;
            // FILE line must come before its tracks in standard CUE
            output_lines.push(format!("FILE \"{filename}\" WAVE"));
            flush_pending_tracks(&mut output_lines, &mut pending_tracks);
            last_datafile = None;
            cumulative_byte_offset = 0;
        } else if upper.starts_with("DATAFILE ") {
            let (filename, remainder) = parse_cue_file_line_at(trimmed, 9)?;

            // Check if this DATAFILE has an MSF length
            let msf_length = extract_msf_from_remainder(&remainder);

            // If this is a different BIN than the last DATAFILE, reset offset
            if last_datafile.as_deref() != Some(&filename) {
                cumulative_byte_offset = 0;
            }

            // Flush pending tracks, computing INDEX from cumulative offset
            if !pending_tracks.is_empty() {
                // If there are pending tracks and we have a cumulative offset,
                // ensure the first pending track gets the right INDEX
                if cumulative_byte_offset > 0 && !pending_tracks.is_empty() {
                    let mode = &pending_tracks[0].1;
                    let sector_sz = sector_size_for_mode(mode) as u64;
                    let sector_offset = cumulative_byte_offset / sector_sz;
                    let msf = sectors_to_msf(sector_offset);
                    // Only add INDEX if the track doesn't already have one
                    if pending_tracks[0].2.is_empty() {
                        pending_tracks[0].2.push(format!("    INDEX 01 {msf}"));
                    }
                }

                // Only emit FILE line if it's different from the current file
                // being built in output
                let need_file_line = !output_lines
                    .iter()
                    .rev()
                    .any(|l| l.starts_with("FILE ") && l.contains(&format!("\"{filename}\"")));
                if need_file_line {
                    output_lines.push(format!("FILE \"{filename}\" BINARY"));
                }
                flush_pending_tracks_raw(&mut output_lines, &mut pending_tracks);
            } else {
                output_lines.push(format!("FILE \"{filename}\" BINARY"));
            }

            // Advance cumulative offset if MSF length was specified
            if let Some(ref msf) = msf_length {
                if let Ok(sectors) = msf_to_sectors(msf) {
                    // Determine sector size from the most recent track mode
                    let mode = pending_tracks
                        .last()
                        .map(|(_, m, _)| m.as_str())
                        .or_else(|| {
                            // Look at the last TRACK line we emitted
                            output_lines.iter().rev().find_map(|l| {
                                let lt = l.trim();
                                if lt.starts_with("TRACK ") {
                                    lt.split_whitespace().nth(2)
                                } else {
                                    None
                                }
                            })
                        })
                        .unwrap_or("MODE2/2352");
                    let sector_sz = sector_size_for_mode(mode) as u64;
                    cumulative_byte_offset += sectors * sector_sz;
                }
            }

            last_datafile = Some(filename);
        } else if upper.starts_with("FILE ") {
            // Standard FILE directive — pass through as-is, but flush pending tracks first
            flush_pending_tracks(&mut output_lines, &mut pending_tracks);
            let (filename, file_type) = parse_cue_file_line_at(trimmed, 5)?;
            // Strip any CDRWin-style #offset or MSF from FILE lines
            let clean_type = if file_type.starts_with('#') || file_type.is_empty() {
                // CDRWin FILE with #offset — determine type from context
                if upper.contains(".WAV") || upper.contains(".WAVE") {
                    "WAVE"
                } else {
                    "BINARY"
                }
            } else {
                // Check if the file_type itself is a standard type or has extra params
                let first_word = file_type.split_whitespace().next().unwrap_or(&file_type);
                match first_word.to_uppercase().as_str() {
                    "BINARY" | "MOTOROLA" | "AIFF" | "WAVE" | "MP3" => first_word,
                    _ => "BINARY",
                }
            };
            output_lines.push(format!("FILE \"{filename}\" {clean_type}"));
            last_datafile = None;
            cumulative_byte_offset = 0;
        } else if upper.starts_with("TRACK ") {
            auto_track_number += 1;
            let parts: Vec<&str> = trimmed.split_whitespace().collect();

            let (track_num, mode) = if parts.len() >= 3 && parts[1].parse::<u8>().is_ok() {
                (parts[1].parse::<u8>().unwrap(), parts[2].to_string())
            } else if parts.len() >= 2 {
                (auto_track_number, parts[1].to_string())
            } else {
                return Err(AnalysisError::invalid_format("Invalid TRACK line in CUE"));
            };

            // Convert CDRWin mode to standard if needed
            let standard_mode = convert_track_mode(&mode)
                .map(|s| s.to_string())
                .unwrap_or(mode);

            pending_tracks.push((track_num, standard_mode, Vec::new()));
        } else if upper.starts_with("INDEX ") {
            // Attach to last pending track
            if let Some(last) = pending_tracks.last_mut() {
                last.2.push(format!("    {trimmed}"));
            } else {
                // No pending track — just output directly
                output_lines.push(format!("    {trimmed}"));
            }
        } else if upper.starts_with("PREGAP ") || upper.starts_with("POSTGAP ") {
            // Standard CUE directives — keep them
            if let Some(last) = pending_tracks.last_mut() {
                last.2.push(format!("    {trimmed}"));
            } else {
                output_lines.push(format!("    {trimmed}"));
            }
        }
        // All other lines (REM, unknown) are dropped
    }

    // Flush any remaining pending tracks
    flush_pending_tracks(&mut output_lines, &mut pending_tracks);

    if output_lines.is_empty() {
        return Err(AnalysisError::invalid_format(
            "CUE sheet produced no output after conversion",
        ));
    }

    Ok(output_lines.join("\n") + "\n")
}

/// Extract an MSF timestamp from a DATAFILE remainder string.
/// The remainder might look like `01:32:21 // comment` or just `01:32:21`.
fn extract_msf_from_remainder(remainder: &str) -> Option<String> {
    // Strip inline comments
    let clean = if let Some(idx) = remainder.find("//") {
        remainder[..idx].trim()
    } else {
        remainder.trim()
    };

    if clean.is_empty() {
        return None;
    }

    // Check if it looks like MSF (MM:SS:FF)
    let parts: Vec<&str> = clean.split(':').collect();
    if parts.len() == 3 && parts.iter().all(|p| p.parse::<u32>().is_ok()) {
        Some(clean.to_string())
    } else {
        None
    }
}

/// Flush pending tracks into output_lines, ensuring each track gets an INDEX 01
/// if it doesn't already have one.
fn flush_pending_tracks(
    output_lines: &mut Vec<String>,
    pending_tracks: &mut Vec<(u8, String, Vec<String>)>,
) {
    for (num, mode, indexes) in pending_tracks.drain(..) {
        output_lines.push(format!("  TRACK {:02} {mode}", num));
        if indexes.is_empty() {
            output_lines.push("    INDEX 01 00:00:00".to_string());
        } else {
            output_lines.extend(indexes);
        }
    }
}

/// Flush pending tracks into output_lines without adding default INDEX entries.
/// Used when the caller manages INDEX generation from byte offsets.
fn flush_pending_tracks_raw(
    output_lines: &mut Vec<String>,
    pending_tracks: &mut Vec<(u8, String, Vec<String>)>,
) {
    for (num, mode, indexes) in pending_tracks.drain(..) {
        output_lines.push(format!("  TRACK {:02} {mode}", num));
        if indexes.is_empty() {
            output_lines.push("    INDEX 01 00:00:00".to_string());
        } else {
            output_lines.extend(indexes);
        }
    }
}

#[cfg(test)]
#[path = "tests/cue_tests.rs"]
mod tests;
