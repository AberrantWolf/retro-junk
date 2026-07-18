//! Serial-based DAT matching for the rename workflow.

use std::fs;
use std::path::Path;

use retro_junk_core::{AnalysisOptions, RomAnalyzer};
use retro_junk_dat::matcher::{DatIndex, MatchResult, SerialLookupResult};

/// Internal result from serial matching, carrying diagnostic information.
pub struct SerialMatchOutcome {
    pub result: Option<MatchResult>,
    /// Full serial from ROM header (e.g., "NUS-NSME-USA"); empty when none found.
    pub full_serial: String,
    /// Extracted game code used for DAT lookup (e.g., "NSME"); empty when none.
    pub game_code: String,
    /// When serial matched multiple games, the candidate names; empty otherwise.
    pub ambiguous_candidates: Vec<String>,
    /// Detected file format extension from analyzer (e.g., "iso", "chd", "rvz");
    /// empty when undetected.
    pub detected_extension: String,
}

/// Quick-analyze a disc/ROM file to extract its serial, then look it up in the DAT.
///
/// Returns a [`SerialMatchOutcome`] with diagnostic info regardless of success,
/// so the caller can generate appropriate warnings. Uses quick analysis mode
/// to minimize I/O (reads only enough data to extract the serial).
pub fn serial_lookup(
    file_path: &Path,
    analyzer: &dyn RomAnalyzer,
    index: &DatIndex,
) -> SerialMatchOutcome {
    let analysis_options = AnalysisOptions::new().quick(true).file_path(file_path);
    let no_match = SerialMatchOutcome {
        result: None,
        full_serial: String::new(),
        game_code: String::new(),
        ambiguous_candidates: Vec::new(),
        detected_extension: String::new(),
    };

    let Ok(mut file) = fs::File::open(file_path) else {
        return no_match;
    };
    let Ok(info) = analyzer.analyze(&mut file, &analysis_options) else {
        return no_match;
    };

    let detected_extension = info
        .extra
        .get("detected_extension")
        .cloned()
        .unwrap_or_default();

    let serial = info.serial_number;
    if serial.is_empty() {
        return SerialMatchOutcome {
            detected_extension,
            ..no_match
        };
    }

    let game_code = analyzer.extract_dat_game_code(&serial);
    let lookup = index.match_by_serial(&serial, game_code.as_deref());
    let game_code = game_code.unwrap_or_default();

    match lookup {
        SerialLookupResult::Match(result) => SerialMatchOutcome {
            result: Some(result),
            full_serial: serial,
            game_code,
            ambiguous_candidates: Vec::new(),
            detected_extension,
        },
        SerialLookupResult::Ambiguous { candidates } => SerialMatchOutcome {
            result: None,
            full_serial: serial,
            game_code,
            ambiguous_candidates: candidates,
            detected_extension,
        },
        SerialLookupResult::NotFound => SerialMatchOutcome {
            result: None,
            full_serial: serial,
            game_code,
            ambiguous_candidates: Vec::new(),
            detected_extension,
        },
    }
}
