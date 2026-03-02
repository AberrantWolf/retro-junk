use super::*;
use crate::disc_test_helpers::{make_iso, make_iso_with_system_cnf, make_raw_bin};
use std::io::Cursor;

// PS1 tests use "BOOT" key for SYSTEM.CNF
fn make_ps1_iso_with_serial(serial: &str) -> Vec<u8> {
    make_iso_with_system_cnf(serial, "BOOT")
}

// -- can_handle tests --

#[test]
fn test_can_handle_ps1_iso() {
    // A PLAYSTATION ISO with no SYSTEM.CNF is accepted (best guess PS1)
    let data = make_iso("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    assert!(analyzer.can_handle(&mut cursor));
}

#[test]
fn test_can_handle_ps1_iso_with_boot() {
    // A PLAYSTATION ISO with BOOT in SYSTEM.CNF is accepted
    let data = make_ps1_iso_with_serial("SLUS_012.34");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    assert!(analyzer.can_handle(&mut cursor));
}

#[test]
fn test_can_handle_ps2_iso_rejected() {
    // A PLAYSTATION ISO with BOOT2 in SYSTEM.CNF is rejected by PS1
    let data = make_iso_with_system_cnf("SLUS_012.34", "BOOT2");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    assert!(!analyzer.can_handle(&mut cursor));
}

#[test]
fn test_can_handle_non_ps1_iso() {
    let data = make_iso("SOME_OTHER_SYS");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    assert!(!analyzer.can_handle(&mut cursor));
}

#[test]
fn test_can_handle_raw_bin() {
    let data = make_raw_bin("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    assert!(analyzer.can_handle(&mut cursor));
}

#[test]
fn test_can_handle_cue() {
    let cue = b"FILE \"game.bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n";
    let mut cursor = Cursor::new(cue.to_vec());
    let analyzer = Ps1Analyzer::new();
    assert!(analyzer.can_handle(&mut cursor));
}

// -- Analyze ISO tests --

#[test]
fn test_analyze_iso_basic() {
    let data = make_iso("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::new().quick(true);
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.platform, Some(Platform::Ps1));
    assert_eq!(result.internal_name.as_deref(), Some("TEST_VOLUME"));
    assert_eq!(
        result.extra.get("format").map(|s| s.as_str()),
        Some("ISO 9660")
    );
}

#[test]
fn test_analyze_raw_bin_basic() {
    let data = make_raw_bin("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::new().quick(true);
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.platform, Some(Platform::Ps1));
    assert_eq!(
        result.extra.get("format").map(|s| s.as_str()),
        Some("Raw BIN (2352)")
    );
}

#[test]
fn test_analyze_non_ps1_iso_rejected() {
    let data = make_iso("XBOX SYSTEM");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::default();
    assert!(analyzer.analyze(&mut cursor, &options).is_err());
}

// -- Full analysis with SYSTEM.CNF --

#[test]
fn test_analyze_iso_with_serial() {
    let data = make_ps1_iso_with_serial("SLUS_012.34");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.serial_number.as_deref(), Some("SLUS-01234"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Usa]);
    assert!(result.extra.contains_key("boot_path"));
    assert_eq!(result.extra.get("vmode").map(|s| s.as_str()), Some("NTSC"));
}

#[test]
fn test_analyze_iso_quick_mode_still_extracts_serial() {
    let data = make_ps1_iso_with_serial("SLUS_012.34");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::new().quick(true);
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.serial_number.as_deref(), Some("SLUS-01234"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Usa]);
}

#[test]
fn test_analyze_iso_european_serial() {
    let data = make_ps1_iso_with_serial("SLES_123.45");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.serial_number.as_deref(), Some("SLES-12345"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Europe]);
}

#[test]
fn test_analyze_iso_japanese_serial() {
    let data = make_ps1_iso_with_serial("SLPS_000.01");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.serial_number.as_deref(), Some("SLPS-00001"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Japan]);
}

#[test]
fn test_analyze_ps2_disc_rejected() {
    // PS1 analyzer should reject discs with BOOT2 in SYSTEM.CNF
    let data = make_iso_with_system_cnf("SLUS_012.34", "BOOT2");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::default();
    assert!(analyzer.analyze(&mut cursor, &options).is_err());
}

// -- CUE analysis tests --

#[test]
fn test_analyze_cue_basic() {
    let cue = "FILE \"game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n";
    let mut cursor = Cursor::new(cue.as_bytes().to_vec());
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::new().quick(true);
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(
        result.extra.get("format").map(|s| s.as_str()),
        Some("CUE Sheet")
    );
    assert_eq!(
        result.extra.get("total_tracks").map(|s| s.as_str()),
        Some("1")
    );
    assert_eq!(
        result.extra.get("data_tracks").map(|s| s.as_str()),
        Some("1")
    );
    assert_eq!(
        result.extra.get("audio_tracks").map(|s| s.as_str()),
        Some("0")
    );
    assert_eq!(
        result.extra.get("bin_file").map(|s| s.as_str()),
        Some("game.bin")
    );
}

#[test]
fn test_analyze_cue_multi_track() {
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
    let mut cursor = Cursor::new(cue.as_bytes().to_vec());
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::new().quick(true);
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(
        result.extra.get("total_tracks").map(|s| s.as_str()),
        Some("3")
    );
    assert_eq!(
        result.extra.get("data_tracks").map(|s| s.as_str()),
        Some("1")
    );
    assert_eq!(
        result.extra.get("audio_tracks").map(|s| s.as_str()),
        Some("2")
    );
}

// -- DAT methods --

#[test]
fn test_extract_dat_game_code() {
    let analyzer = Ps1Analyzer::new();
    assert_eq!(
        analyzer.extract_dat_game_code("SLUS-01234"),
        Some("SLUS-01234".to_string())
    );
}

#[test]
fn test_extract_dat_game_code_multi_disc_fixups() {
    let analyzer = Ps1Analyzer::new();

    assert_eq!(
        analyzer.extract_dat_game_code("SCUS-94164"),
        Some("SCUS-94163-1".to_string())
    );
    assert_eq!(
        analyzer.extract_dat_game_code("SCUS-94165"),
        Some("SCUS-94163-2".to_string())
    );
    assert_eq!(
        analyzer.extract_dat_game_code("SCUS-94422"),
        Some("SCUS-94421-1".to_string())
    );
    assert_eq!(
        analyzer.extract_dat_game_code("SCUS-94163"),
        Some("SCUS-94163".to_string())
    );
}

// -- File extensions --

#[test]
fn test_file_extensions() {
    let analyzer = Ps1Analyzer::new();
    let exts = analyzer.file_extensions();
    assert!(exts.contains(&"iso"));
    assert!(exts.contains(&"bin"));
    assert!(exts.contains(&"chd"));
    assert!(!exts.contains(&"cue"));
    assert!(!exts.contains(&"img"));
    assert!(!exts.contains(&"pbp"));
    assert!(!exts.contains(&"ecm"));
}
