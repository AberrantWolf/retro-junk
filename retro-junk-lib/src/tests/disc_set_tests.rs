use super::*;
use retro_junk_core::{
    AnalysisError, AnalysisOptions, Platform, ReadSeek, RomAnalyzer, RomIdentification,
};
use retro_junk_dat::dat::{DatFile, DatRom};
use std::io::Cursor;
use tempfile::TempDir;

/// Minimal analyzer for tests: returns a fixed serial (or none).
struct FakeAnalyzer {
    serial: Option<&'static str>,
}

impl RomAnalyzer for FakeAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        let mut id = RomIdentification::new();
        if let Some(s) = self.serial {
            id = id.with_serial(s);
        }
        Ok(id)
    }

    fn platform(&self) -> Platform {
        Platform::Ps1
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["cue", "bin"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        true
    }
}

fn plain_hashes(data: &[u8]) -> FileHashes {
    let mut cursor = Cursor::new(data.to_vec());
    hasher::compute_plain_crc32_sha1(&mut cursor, None).unwrap()
}

fn rom_for(name: &str, data: &[u8], serial: Option<&str>) -> DatRom {
    let h = plain_hashes(data);
    DatRom {
        name: name.to_string(),
        size: data.len() as u64,
        crc: h.crc32,
        sha1: h.sha1,
        md5: None,
        serial: serial.map(std::string::ToString::to_string),
    }
}

const TRACK1: &[u8] = b"data track contents: mode2/2352 stand-in";
const TRACK2: &[u8] = b"audio track contents: 2352 stand-in";
const GAME: &str = "Cool Game (USA)";
const SERIAL: &str = "SLUS-01234";

/// Canonical Redump cue for the two-track game.
fn canonical_cue() -> String {
    format!(
        "FILE \"{GAME} (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\nFILE \"{GAME} (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\n"
    )
}

/// DAT index containing the two-track game (with a cue rom entry).
fn two_track_index() -> DatIndex {
    let cue_content = canonical_cue();
    let game = DatGame {
        name: GAME.to_string(),
        region: None,
        roms: vec![
            rom_for(&format!("{GAME}.cue"), cue_content.as_bytes(), Some(SERIAL)),
            rom_for(&format!("{GAME} (Track 1).bin"), TRACK1, Some(SERIAL)),
            rom_for(&format!("{GAME} (Track 2).bin"), TRACK2, Some(SERIAL)),
        ],
        serial: Some(SERIAL.to_string()),
        version: None,
        category: None,
    };
    DatIndex::from_dat(DatFile {
        name: String::new(),
        description: String::new(),
        version: String::new(),
        games: vec![game],
    })
}

/// Write a redumper-style dump: original names, cue referencing the bins.
fn write_dump(dir: &TempDir) -> PathBuf {
    std::fs::write(dir.path().join("dump (Track 1).bin"), TRACK1).unwrap();
    std::fs::write(dir.path().join("dump (Track 2).bin"), TRACK2).unwrap();
    let cue = dir.path().join("dump.cue");
    std::fs::write(
        &cue,
        "FILE \"dump (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\nFILE \"dump (Track 2).bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\n",
    )
    .unwrap();
    cue
}

fn no_progress() -> impl Fn(&Path, u64, u64) {
    |_, _, _| {}
}

#[test]
fn expand_reads_file_lines_in_order() {
    let dir = TempDir::new().unwrap();
    let cue = write_dump(&dir);
    let files = expand_disc_set(&cue).unwrap();
    assert_eq!(files.tracks.len(), 2);
    assert!(files.missing.is_empty());
    assert!(files.tracks[0].ends_with("dump (Track 1).bin"));
    assert!(files.tracks[1].ends_with("dump (Track 2).bin"));
}

#[test]
fn expand_reports_missing_references() {
    let dir = TempDir::new().unwrap();
    let cue = dir.path().join("dump.cue");
    std::fs::write(&cue, "FILE \"ghost.bin\" BINARY\n  TRACK 01 MODE2/2352\n").unwrap();
    let files = expand_disc_set(&cue).unwrap();
    assert!(files.tracks.is_empty());
    assert_eq!(files.missing, vec!["ghost.bin".to_string()]);
}

#[test]
fn plans_full_set_by_serial() {
    let dir = TempDir::new().unwrap();
    let cue = write_dump(&dir);
    let index = two_track_index();
    let analyzer = FakeAnalyzer {
        serial: Some(SERIAL),
    };

    let outcome = plan_disc_set(&cue, &analyzer, &index, &no_progress());
    let DiscSetOutcome::Planned(plan) = outcome else {
        panic!("expected Planned, got {outcome:?}");
    };
    assert_eq!(plan.game_name, GAME);
    assert_eq!(plan.matched_by, MatchMethod::Crc32);
    assert_eq!(plan.cue_target_filename, format!("{GAME}.cue"));
    assert_eq!(plan.tracks.len(), 2);
    assert_eq!(
        plan.tracks[0].target_filename,
        format!("{GAME} (Track 1).bin")
    );
    assert_eq!(
        plan.tracks[1].target_filename,
        format!("{GAME} (Track 2).bin")
    );
    // Rewritten cue matches Redump's canonical cue exactly, so it verifies.
    assert_eq!(
        plan.new_cue_content.as_deref(),
        Some(canonical_cue().as_str())
    );
    assert_eq!(plan.cue_verified, CueVerification::Verified);
    // 2 tracks + cue all rename
    assert_eq!(plan.file_renames().len(), 3);
}

#[test]
fn plans_full_set_by_hash_intersection_without_serial() {
    let dir = TempDir::new().unwrap();
    let cue = write_dump(&dir);
    let index = two_track_index();
    let analyzer = FakeAnalyzer { serial: None };

    let outcome = plan_disc_set(&cue, &analyzer, &index, &no_progress());
    let DiscSetOutcome::Planned(plan) = outcome else {
        panic!("expected Planned, got {outcome:?}");
    };
    assert_eq!(plan.matched_by, MatchMethod::Crc32);
    assert_eq!(plan.game_name, GAME);
}

#[test]
fn already_correct_set_is_not_planned() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(format!("{GAME} (Track 1).bin")), TRACK1).unwrap();
    std::fs::write(dir.path().join(format!("{GAME} (Track 2).bin")), TRACK2).unwrap();
    let cue = dir.path().join(format!("{GAME}.cue"));
    std::fs::write(&cue, canonical_cue()).unwrap();

    let index = two_track_index();
    let analyzer = FakeAnalyzer {
        serial: Some(SERIAL),
    };
    let outcome = plan_disc_set(&cue, &analyzer, &index, &no_progress());
    let DiscSetOutcome::AlreadyCorrect {
        game_name,
        cue_verified,
    } = outcome
    else {
        panic!("expected AlreadyCorrect, got {outcome:?}");
    };
    assert_eq!(game_name, GAME);
    assert_eq!(cue_verified, CueVerification::Verified);
}

#[test]
fn corrupt_track_fails_verification_and_is_not_planned() {
    let dir = TempDir::new().unwrap();
    let cue = write_dump(&dir);
    // Corrupt the audio track after writing the dump
    std::fs::write(dir.path().join("dump (Track 2).bin"), b"CORRUPTED AUDIO").unwrap();

    let index = two_track_index();

    // With a serial: identified but NOT verified
    let analyzer = FakeAnalyzer {
        serial: Some(SERIAL),
    };
    let outcome = plan_disc_set(&cue, &analyzer, &index, &no_progress());
    let DiscSetOutcome::NotVerified { game_name, issues } = outcome else {
        panic!("expected NotVerified, got {outcome:?}");
    };
    assert_eq!(game_name, GAME);
    assert!(!issues.is_empty());

    // Without a serial: no full hash match either
    let analyzer = FakeAnalyzer { serial: None };
    let outcome = plan_disc_set(&cue, &analyzer, &index, &no_progress());
    assert!(matches!(outcome, DiscSetOutcome::Unmatched { .. }));
}

#[test]
fn missing_track_file_is_broken() {
    let dir = TempDir::new().unwrap();
    let cue = write_dump(&dir);
    std::fs::remove_file(dir.path().join("dump (Track 2).bin")).unwrap();

    let index = two_track_index();
    let analyzer = FakeAnalyzer {
        serial: Some(SERIAL),
    };
    let outcome = plan_disc_set(&cue, &analyzer, &index, &no_progress());
    let DiscSetOutcome::Broken { missing } = outcome else {
        panic!("expected Broken, got {outcome:?}");
    };
    assert_eq!(missing, vec!["dump (Track 2).bin".to_string()]);
}

#[test]
fn dump_missing_a_dat_track_is_not_verified() {
    // Cue only references track 1, but the DAT game has two tracks.
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("dump (Track 1).bin"), TRACK1).unwrap();
    let cue = dir.path().join("dump.cue");
    std::fs::write(
        &cue,
        "FILE \"dump (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
    )
    .unwrap();

    let index = two_track_index();
    let analyzer = FakeAnalyzer {
        serial: Some(SERIAL),
    };
    let outcome = plan_disc_set(&cue, &analyzer, &index, &no_progress());
    let DiscSetOutcome::NotVerified { issues, .. } = outcome else {
        panic!("expected NotVerified, got {outcome:?}");
    };
    assert!(
        issues.iter().any(|i| i.contains("Track 2")),
        "issues should name the missing DAT track: {issues:?}"
    );
}

#[test]
fn duplicate_content_tracks_assign_distinct_roms() {
    // Two audio tracks with identical content must map to two distinct
    // DAT rom entries, not both to the first.
    let audio: &[u8] = b"identical audio";
    let game = DatGame {
        name: GAME.to_string(),
        region: None,
        roms: vec![
            rom_for(&format!("{GAME} (Track 1).bin"), TRACK1, Some(SERIAL)),
            rom_for(&format!("{GAME} (Track 2).bin"), audio, Some(SERIAL)),
            rom_for(&format!("{GAME} (Track 3).bin"), audio, Some(SERIAL)),
        ],
        serial: Some(SERIAL.to_string()),
        version: None,
        category: None,
    };
    let tracks = vec![
        PathBuf::from("dump (Track 1).bin"),
        PathBuf::from("dump (Track 2).bin"),
        PathBuf::from("dump (Track 3).bin"),
    ];
    let hashes = vec![
        plain_hashes(TRACK1),
        plain_hashes(audio),
        plain_hashes(audio),
    ];

    let assignment = assign_tracks(&game, &tracks, &hashes).unwrap();
    assert_eq!(assignment, vec![0, 1, 2]);
}

#[test]
fn single_track_set_renames_cue_and_bin() {
    // The Game.cue + Game.bin case the old stem-dedup path got wrong.
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("dump.bin"), TRACK1).unwrap();
    let cue = dir.path().join("dump.cue");
    let cue_content = "FILE \"dump.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n";
    std::fs::write(&cue, cue_content).unwrap();

    let canonical =
        format!("FILE \"{GAME}.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n");
    let game = DatGame {
        name: GAME.to_string(),
        region: None,
        roms: vec![
            rom_for(&format!("{GAME}.cue"), canonical.as_bytes(), Some(SERIAL)),
            rom_for(&format!("{GAME}.bin"), TRACK1, Some(SERIAL)),
        ],
        serial: Some(SERIAL.to_string()),
        version: None,
        category: None,
    };
    let index = DatIndex::from_dat(DatFile {
        name: String::new(),
        description: String::new(),
        version: String::new(),
        games: vec![game],
    });

    let analyzer = FakeAnalyzer {
        serial: Some(SERIAL),
    };
    let outcome = plan_disc_set(&cue, &analyzer, &index, &no_progress());
    let DiscSetOutcome::Planned(plan) = outcome else {
        panic!("expected Planned, got {outcome:?}");
    };
    let renames = plan.file_renames();
    assert_eq!(renames.len(), 2, "both bin and cue must rename");
    assert_eq!(plan.cue_verified, CueVerification::Verified);
}

#[test]
fn rewrite_preserves_indentation_and_file_type() {
    let content = "  FILE \"old.bin\" BINARY\n    TRACK 01 AUDIO\n";
    let mut map = HashMap::new();
    map.insert("old.bin".to_string(), "new.bin".to_string());
    let out = rewrite_cue_references(content, &map).unwrap();
    assert_eq!(out, "  FILE \"new.bin\" BINARY\n    TRACK 01 AUDIO\n");
}

#[test]
fn rewrite_returns_none_when_nothing_matches() {
    let content = "FILE \"other.bin\" BINARY\n";
    let mut map = HashMap::new();
    map.insert("old.bin".to_string(), "new.bin".to_string());
    assert!(rewrite_cue_references(content, &map).is_none());
}
