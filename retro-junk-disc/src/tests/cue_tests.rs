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
