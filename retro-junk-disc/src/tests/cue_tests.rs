use crate::cue::*;
use std::path::Path;

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

#[test]
fn test_parse_cue_with_indexes() {
    let cue = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 00 54:04:50
    INDEX 01 54:04:52
"#;
    let sheet = parse_cue(cue).unwrap();
    assert_eq!(sheet.files.len(), 1);
    assert_eq!(sheet.files[0].tracks.len(), 2);

    let track1 = &sheet.files[0].tracks[0];
    assert_eq!(track1.indexes.len(), 1);
    assert_eq!(track1.indexes[0].number, 1);

    let track2 = &sheet.files[0].tracks[1];
    assert_eq!(track2.indexes.len(), 2);
    assert_eq!(track2.indexes[0].number, 0);
    assert_eq!(track2.indexes[0].minutes, 54);
    assert_eq!(track2.indexes[0].seconds, 4);
    assert_eq!(track2.indexes[0].frames, 50);
    assert_eq!(track2.indexes[1].number, 1);
}

#[test]
fn test_cue_index_to_sector_offset() {
    let index = CueIndex {
        number: 1,
        minutes: 54,
        seconds: 4,
        frames: 52,
    };
    assert_eq!(index.to_sector_offset(), 243352);
}

#[test]
fn test_cue_index_to_sector_offset_zero() {
    let index = CueIndex {
        number: 1,
        minutes: 0,
        seconds: 0,
        frames: 0,
    };
    assert_eq!(index.to_sector_offset(), 0);
}

#[test]
fn test_parse_cue_cdrwin_format() {
    // CDRWin extended format: TRACK before DATAFILE, no track numbers
    let cue = r#"CD_ROM_XA


// Track 1
TRACK MODE2_RAW
NO COPY
DATAFILE "THEBLOCK.bin" 01:32:21 // length in bytes: 16278192


// Track 2
TRACK AUDIO
NO COPY
NO PRE_EMPHASIS
TWO_CHANNEL_AUDIO
SILENCE 00:02:00
FILE "game (Track 1).bin" #16278192 0 00:08:08
START 00:02:00


// Track 3
TRACK AUDIO
NO COPY
NO PRE_EMPHASIS
TWO_CHANNEL_AUDIO
FILE "game (Track 1).bin" #16278192 00:08:08 00:07:64
START 00:00:11
"#;
    let sheet = parse_cue(cue).unwrap();

    // DATAFILE gets the pending Track 1, then two FILE entries for audio tracks
    assert_eq!(sheet.files.len(), 3);

    // First file is from DATAFILE, with pending Track 1 attached.
    // Track 2 (AUDIO) also attaches here since it appears before the next FILE.
    assert_eq!(sheet.files[0].filename, "THEBLOCK.bin");
    assert_eq!(sheet.files[0].file_type, "BINARY");
    assert_eq!(sheet.files[0].tracks.len(), 2);
    assert_eq!(sheet.files[0].tracks[0].number, 1);
    assert_eq!(sheet.files[0].tracks[0].mode, "MODE2_RAW");
    assert_eq!(sheet.files[0].tracks[1].number, 2);
    assert_eq!(sheet.files[0].tracks[1].mode, "AUDIO");

    // Track 3 (AUDIO) appears before its FILE, so it's pending then attached
    assert_eq!(sheet.files[1].filename, "game (Track 1).bin");
    assert_eq!(sheet.files[1].tracks.len(), 1);
    assert_eq!(sheet.files[1].tracks[0].number, 3);
    assert_eq!(sheet.files[1].tracks[0].mode, "AUDIO");

    // No track between last two FILE entries
    assert_eq!(sheet.files[2].filename, "game (Track 1).bin");
    assert_eq!(sheet.files[2].tracks.len(), 0);
}

#[test]
fn test_parse_cue_cdrwin_track_before_datafile() {
    // CDRWin format: TRACK appears before DATAFILE (reversed from standard)
    let cue = "TRACK MODE1_RAW\nDATAFILE \"game.bin\" 01:00:00\n";
    let sheet = parse_cue(cue).unwrap();
    assert_eq!(sheet.files.len(), 1);
    assert_eq!(sheet.files[0].filename, "game.bin");
    assert_eq!(sheet.files[0].file_type, "BINARY");
    // Pending track was attached to the DATAFILE entry
    assert_eq!(sheet.files[0].tracks.len(), 1);
    assert_eq!(sheet.files[0].tracks[0].number, 1);
    assert_eq!(sheet.files[0].tracks[0].mode, "MODE1_RAW");
}

// -- CUE compatibility detection tests --

#[test]
fn test_compat_standard_cue_is_standard() {
    let cue = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
FILE "game (Track 2).bin" BINARY
  TRACK 02 AUDIO
    INDEX 01 00:00:00
"#;
    let report = check_cue_compat(cue);
    assert!(report.is_standard());
    assert!(!report.can_auto_fix());
}

#[test]
fn test_compat_detects_disc_type_header() {
    let cue = "CD_ROM_XA\n\nTRACK MODE2_RAW\nDATAFILE \"game.bin\"\n";
    let report = check_cue_compat(cue);
    assert_eq!(report.disc_type_header.as_deref(), Some("CD_ROM_XA"));
    assert!(!report.is_standard());
}

#[test]
fn test_compat_detects_cdrwin_track_modes() {
    let cue = "TRACK MODE2_RAW\nDATAFILE \"game.bin\"\nTRACK AUDIO\nFILE \"track2.bin\" 0\n";
    let report = check_cue_compat(cue);
    assert_eq!(report.cdwin_track_modes.len(), 1);
    assert_eq!(report.cdwin_track_modes[0].1, "MODE2_RAW");
}

#[test]
fn test_compat_detects_datafile() {
    let cue = "TRACK MODE1_RAW\nDATAFILE \"game.bin\" 01:00:00\n";
    let report = check_cue_compat(cue);
    assert!(report.has_datafile);
}

#[test]
fn test_compat_detects_extra_directives() {
    let cue = "CD_ROM_XA\n\nTRACK MODE2_RAW\nNO COPY\nDATAFILE \"game.bin\"\n";
    let report = check_cue_compat(cue);
    assert!(report.has_extra_directives);
}

#[test]
fn test_compat_detects_comments() {
    let cue = "// This is a CDRWin CUE\nTRACK MODE2_RAW\nDATAFILE \"game.bin\"\n";
    let report = check_cue_compat(cue);
    assert!(report.has_comments);
}

#[test]
fn test_compat_audiofile_with_offset_is_unfixable() {
    let cue = "TRACK AUDIO\nAUDIOFILE \"game.bin\" #16278192 0 00:08:08\n";
    let report = check_cue_compat(cue);
    assert!(report.has_audiofile);
    assert!(report.unfixable_reason.is_some());
    assert!(!report.can_auto_fix());
}

#[test]
fn test_compat_cdrwin_cue_can_auto_fix() {
    let cue = r#"CD_ROM_XA

// Track 1
TRACK MODE2_RAW
NO COPY
DATAFILE "game.bin"
"#;
    let report = check_cue_compat(cue);
    assert!(report.can_auto_fix());
    assert!(report.disc_type_header.is_some());
    assert!(!report.cdwin_track_modes.is_empty());
    assert!(report.has_datafile);
    assert!(report.has_extra_directives);
    assert!(report.has_comments);
}

// -- CUE conversion tests --

#[test]
fn test_convert_simple_cdrwin_cue() {
    let cue = r#"CD_ROM_XA

// Track 1
TRACK MODE2_RAW
NO COPY
DATAFILE "game.bin"
"#;
    let result = convert_cue_to_standard(cue, Path::new("/tmp")).unwrap();
    assert!(result.contains("FILE \"game.bin\" BINARY"));
    assert!(result.contains("TRACK 01 MODE2/2352"));
    assert!(result.contains("INDEX 01 00:00:00"));
    // Should not contain CDRWin-isms
    assert!(!result.contains("CD_ROM_XA"));
    assert!(!result.contains("NO COPY"));
    assert!(!result.contains("DATAFILE"));
    assert!(!result.contains("//"));
    assert!(!result.contains("MODE2_RAW"));
}

#[test]
fn test_convert_multi_track_cdrwin() {
    let cue = r#"CD_ROM_XA

TRACK MODE2_RAW
NO COPY
DATAFILE "game.bin"

TRACK AUDIO
NO COPY
NO PRE_EMPHASIS
TWO_CHANNEL_AUDIO
FILE "game (Track 2).bin" 0
"#;
    let result = convert_cue_to_standard(cue, Path::new("/tmp")).unwrap();
    assert!(result.contains("TRACK 01 MODE2/2352"));
    assert!(result.contains("TRACK 02 AUDIO"));
    assert!(result.contains("FILE \"game.bin\" BINARY"));
    assert!(result.contains("FILE \"game (Track 2).bin\" BINARY"));
}

#[test]
fn test_convert_preserves_standard_cue() {
    let cue = r#"FILE "game (Track 1).bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
FILE "game (Track 2).bin" BINARY
  TRACK 02 AUDIO
    INDEX 00 00:00:00
    INDEX 01 00:02:00
"#;
    let result = convert_cue_to_standard(cue, Path::new("/tmp")).unwrap();
    assert!(result.contains("FILE \"game (Track 1).bin\" BINARY"));
    assert!(result.contains("TRACK 01 MODE2/2352"));
    assert!(result.contains("INDEX 01 00:00:00"));
    assert!(result.contains("FILE \"game (Track 2).bin\" BINARY"));
    assert!(result.contains("TRACK 02 AUDIO"));
    assert!(result.contains("INDEX 01 00:02:00"));
}

#[test]
fn test_convert_audiofile_with_offset_fails() {
    let cue = "TRACK AUDIO\nAUDIOFILE \"game.bin\" #16278192 0 00:08:08\n";
    let result = convert_cue_to_standard(cue, Path::new("/tmp"));
    assert!(result.is_err());
}

#[test]
fn test_convert_roundtrip_parses_equivalently() {
    let cdwin_cue = r#"CD_ROM_XA

TRACK MODE2_RAW
NO COPY
DATAFILE "game.bin"

TRACK AUDIO
NO COPY
FILE "game (Track 2).bin" 0
"#;
    let standard_cue = convert_cue_to_standard(cdwin_cue, Path::new("/tmp")).unwrap();

    // Both should parse successfully
    let cdwin_parsed = parse_cue(cdwin_cue).unwrap();
    let standard_parsed = parse_cue(&standard_cue).unwrap();

    // Same number of files
    assert_eq!(cdwin_parsed.files.len(), standard_parsed.files.len());

    // Same filenames
    for (a, b) in cdwin_parsed.files.iter().zip(standard_parsed.files.iter()) {
        assert_eq!(a.filename, b.filename);
    }
}

#[test]
fn test_convert_mode_mapping() {
    // Verify each CDRWin mode maps correctly
    let cases = [
        ("MODE2_RAW", "MODE2/2352"),
        ("MODE1_RAW", "MODE1/2352"),
        ("MODE2_FORM1", "MODE2/2048"),
        ("MODE2_FORM2", "MODE2/2324"),
        ("MODE2_FORM_MIX", "MODE2/2336"),
    ];
    for (cdwin, standard) in &cases {
        let cue = format!("TRACK {cdwin}\nDATAFILE \"game.bin\"\n");
        let result = convert_cue_to_standard(&cue, Path::new("/tmp")).unwrap();
        assert!(
            result.contains(standard),
            "Expected {standard} in converted CUE for {cdwin}, got: {result}"
        );
    }
}

#[test]
fn test_sectors_to_msf_roundtrip() {
    // Test internal MSF conversion helpers
    let sectors = msf_to_sectors("01:32:21").unwrap();
    let msf = sectors_to_msf(sectors);
    assert_eq!(msf, "01:32:21");
}

#[test]
fn test_msf_to_sectors() {
    // 1 minute 32 seconds 21 frames = (92 * 75) + 21 = 6921 sectors
    let sectors = msf_to_sectors("01:32:21").unwrap();
    assert_eq!(sectors, 6921);
}

#[test]
fn test_compat_summary() {
    let cue = "CD_ROM_XA\n\nTRACK MODE2_RAW\nNO COPY\nDATAFILE \"game.bin\"\n";
    let report = check_cue_compat(cue);
    let summary = report.summary();
    assert!(summary.contains("CD_ROM_XA header"));
    assert!(summary.contains("CDRWin track mode"));
    assert!(summary.contains("DATAFILE"));
}

#[test]
fn test_convert_audiofile_tracks_under_correct_file() {
    // CDRWin format: TRACK before DATAFILE/AUDIOFILE
    // Each track's data must end up under its own FILE in standard CUE
    let cdwin_cue = r#"CD_ROM_XA
TRACK MODE2_RAW
DATAFILE "data.bin" 54:04:52
TRACK AUDIO
AUDIOFILE "audio.bin"
"#;
    let standard = convert_cue_to_standard(cdwin_cue, Path::new("/tmp")).unwrap();
    let parsed = parse_cue(&standard).unwrap();

    assert_eq!(parsed.files.len(), 2);
    // Data track under data.bin
    assert_eq!(parsed.files[0].filename, "data.bin");
    assert_eq!(parsed.files[0].tracks.len(), 1);
    assert_eq!(parsed.files[0].tracks[0].mode, "MODE2/2352");
    // Audio track under audio.bin, not data.bin
    assert_eq!(parsed.files[1].filename, "audio.bin");
    assert_eq!(parsed.files[1].tracks.len(), 1);
    assert_eq!(parsed.files[1].tracks[0].mode, "AUDIO");
}
