use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::cue::parse_cue;
use crate::track_layout::{TrackSpan, cue_track_spans, gdi_track_spans, parse_gdi};

fn write_file(dir: &Path, name: &str, len: usize) {
    fs::write(dir.join(name), vec![0u8; len]).unwrap();
}

/// A5: assert spans exactly tile every file they reference — the sum of
/// `byte_len` per file equals that file's actual size on disk, with no gaps
/// or overlaps.
fn assert_spans_tile_files(spans: &[TrackSpan]) {
    let mut totals: HashMap<&Path, u64> = HashMap::new();
    for s in spans {
        *totals.entry(s.file.as_path()).or_insert(0) += s.byte_len;
    }
    for (file, total) in totals {
        let actual = fs::metadata(file).unwrap().len();
        assert_eq!(
            total,
            actual,
            "spans do not tile {} (summed {total}, file is {actual} bytes)",
            file.display()
        );
    }
}

#[test]
fn multi_bin_cue_spans_are_whole_files() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "t1.bin", 100 * 2352);
    write_file(dir.path(), "t2.bin", 200 * 2352);
    let cue = r#"FILE "t1.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
FILE "t2.bin" BINARY
  TRACK 02 AUDIO
    INDEX 00 00:00:00
    INDEX 01 00:02:00
"#;
    let sheet = parse_cue(cue).unwrap();
    let spans = cue_track_spans(&sheet, dir.path()).unwrap();
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].byte_offset, 0);
    assert_eq!(spans[0].byte_len, 100 * 2352);
    // Track 2's pregap (INDEX 00) is inside its file and belongs to it.
    assert_eq!(spans[1].byte_offset, 0);
    assert_eq!(spans[1].byte_len, 200 * 2352);
    assert_spans_tile_files(&spans);
}

#[test]
fn single_bin_cue_splits_at_first_index_of_each_track() {
    let dir = tempfile::tempdir().unwrap();
    // 100 frames track 1, 150 frames pregap + 50 frames track 2, 30 frames track 3
    write_file(dir.path(), "game.bin", 330 * 2352);
    let cue = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 00 00:01:25
    INDEX 01 00:03:25
  TRACK 03 AUDIO
    INDEX 01 00:04:00
"#;
    let sheet = parse_cue(cue).unwrap();
    let spans = cue_track_spans(&sheet, dir.path()).unwrap();
    assert_eq!(spans.len(), 3);
    // Track 2 starts at its INDEX 00 (frame 100), not INDEX 01 (frame 250).
    assert_eq!(spans[0].byte_len, 100 * 2352);
    assert_eq!(spans[1].byte_offset, 100 * 2352);
    assert_eq!(spans[1].byte_len, 200 * 2352);
    assert_eq!(spans[2].byte_offset, 300 * 2352);
    assert_eq!(spans[2].byte_len, 30 * 2352);
    assert_spans_tile_files(&spans);
}

#[test]
fn mode1_2048_uses_declared_sector_size() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "game.iso", 500 * 2048);
    let cue = r#"FILE "game.iso" BINARY
  TRACK 01 MODE1/2048
    INDEX 01 00:00:00
"#;
    let sheet = parse_cue(cue).unwrap();
    let spans = cue_track_spans(&sheet, dir.path()).unwrap();
    assert_eq!(spans[0].byte_len, 500 * 2048);
    assert_spans_tile_files(&spans);
}

#[test]
fn cdrwin_mode_cue_computes_2048_byte_spans() {
    // TRACK MODE2_FORM1 (CDRWin bare mode name, no slash suffix) must use
    // the CDRWin sector-size table (2048), not fall back to raw 2352 —
    // this is what unifying on cue::sector_size_for_mode fixes (A1).
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "game.bin", 500 * 2048);
    let cue = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2_FORM1
    INDEX 01 00:00:00
"#;
    let sheet = parse_cue(cue).unwrap();
    let spans = cue_track_spans(&sheet, dir.path()).unwrap();
    assert_eq!(spans[0].byte_len, 500 * 2048);
    assert_spans_tile_files(&spans);
}

#[test]
fn first_track_span_always_starts_at_byte_zero() {
    // TRACK 01 declares only INDEX 01 at frame 150 (a 2-second in-file
    // pregap with no INDEX 00). Track 1 must still own bytes 0.., not
    // leave the pregap region uncovered by any span (A5).
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "game.bin", 10 * 2352);
    let cue = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:02:00
"#;
    let sheet = parse_cue(cue).unwrap();
    let spans = cue_track_spans(&sheet, dir.path()).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].byte_offset, 0);
    assert_eq!(spans[0].byte_len, 10 * 2352);
    assert_spans_tile_files(&spans);
}

#[test]
fn cue_index_past_end_of_file_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "game.bin", 50 * 2352);
    let cue = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 01 00:02:00
"#;
    let sheet = parse_cue(cue).unwrap();
    assert!(cue_track_spans(&sheet, dir.path()).is_err());
}

#[test]
fn zero_length_and_duplicate_tracks_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "game.bin", 10 * 2352);
    let zero_length = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 01 00:00:00
"#;
    assert!(cue_track_spans(&parse_cue(zero_length).unwrap(), dir.path()).is_err());

    let duplicate = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
  TRACK 01 AUDIO
    INDEX 01 00:00:05
"#;
    assert!(cue_track_spans(&parse_cue(duplicate).unwrap(), dir.path()).is_err());
}

#[test]
fn missing_referenced_file_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let cue = r#"FILE "missing.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
"#;
    let sheet = parse_cue(cue).unwrap();
    assert!(cue_track_spans(&sheet, dir.path()).is_err());
}

#[test]
fn parse_gdi_standard_layout() {
    let gdi =
        "3\n1 0 4 2352 track01.bin 0\n2 100 0 2352 track02.raw 0\n3 45000 4 2352 track03.bin 0\n";
    let tracks = parse_gdi(gdi).unwrap();
    assert_eq!(tracks.len(), 3);
    assert_eq!(tracks[0].track_type, 4);
    assert_eq!(tracks[1].track_type, 0);
    assert_eq!(tracks[2].lba, 45000);
    assert_eq!(tracks[2].filename, "track03.bin");
}

#[test]
fn parse_gdi_quoted_filenames_with_spaces() {
    let gdi = "1\n1 0 4 2352 \"my game track01.bin\" 0\n";
    let tracks = parse_gdi(gdi).unwrap();
    assert_eq!(tracks[0].filename, "my game track01.bin");
}

#[test]
fn parse_gdi_count_mismatch_is_rejected() {
    assert!(parse_gdi("2\n1 0 4 2352 track01.bin 0\n").is_err());
}

#[test]
fn gdi_spans_cover_whole_track_files() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "track01.bin", 100 * 2352);
    write_file(dir.path(), "track02.raw", 50 * 2352);
    let tracks = parse_gdi("2\n1 0 4 2352 track01.bin 0\n2 100 0 2352 track02.raw 0\n").unwrap();
    let spans = gdi_track_spans(&tracks, dir.path()).unwrap();
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].byte_len, 100 * 2352);
    assert_eq!(spans[1].byte_len, 50 * 2352);
    assert_spans_tile_files(&spans);
}
