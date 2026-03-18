use crate::cue::*;

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
