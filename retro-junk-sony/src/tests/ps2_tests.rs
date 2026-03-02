use super::*;
use crate::disc_test_helpers::{make_iso, make_iso_with_system_cnf, make_raw_bin};
use std::io::Cursor;

// PS2 tests use "BOOT2" key for SYSTEM.CNF
fn make_ps2_iso_with_serial(serial: &str) -> Vec<u8> {
    make_iso_with_system_cnf(serial, "BOOT2")
}

// -- can_handle tests --

#[test]
fn test_can_handle_ps2_iso() {
    let data = make_ps2_iso_with_serial("SLUS_200.62");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    assert!(analyzer.can_handle(&mut cursor));
}

#[test]
fn test_can_handle_non_ps2_iso() {
    // Non-PLAYSTATION system ID should be rejected
    let data = make_iso("SOME_OTHER_SYS");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    assert!(!analyzer.can_handle(&mut cursor));
}

#[test]
fn test_can_handle_ps1_iso_rejected() {
    // A PLAYSTATION ISO with BOOT (not BOOT2) should be rejected by PS2
    let data = make_iso_with_system_cnf("SLUS_012.34", "BOOT");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    assert!(!analyzer.can_handle(&mut cursor));
}

#[test]
fn test_can_handle_bare_playstation_iso_rejected() {
    // A PLAYSTATION ISO with no SYSTEM.CNF should be rejected by PS2
    // (can't confirm BOOT2 without SYSTEM.CNF)
    let data = make_iso("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    assert!(!analyzer.can_handle(&mut cursor));
}

#[test]
fn test_can_handle_raw_bin() {
    // Raw BIN with no readable SYSTEM.CNF — rejected (can't confirm BOOT2)
    let data = make_raw_bin("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    assert!(!analyzer.can_handle(&mut cursor));
}

#[test]
fn test_can_handle_cue() {
    // CUE sheets are accepted (can't cheaply differentiate)
    let cue = b"FILE \"game.bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n";
    let mut cursor = Cursor::new(cue.to_vec());
    let analyzer = Ps2Analyzer::new();
    assert!(analyzer.can_handle(&mut cursor));
}

// -- Analyze ISO tests --

#[test]
fn test_analyze_iso_basic() {
    let data = make_ps2_iso_with_serial("SLUS_200.62");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    let options = AnalysisOptions::new().quick(true);
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.platform, Some(Platform::Ps2));
    assert_eq!(result.internal_name.as_deref(), Some("TEST_VOLUME"));
    assert_eq!(
        result.extra.get("format").map(|s| s.as_str()),
        Some("ISO 9660")
    );
}

#[test]
fn test_analyze_iso_with_serial() {
    let data = make_ps2_iso_with_serial("SLUS_200.62");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.serial_number.as_deref(), Some("SLUS-20062"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Usa]);
    assert!(result.extra.contains_key("boot_path"));
    assert_eq!(result.extra.get("vmode").map(|s| s.as_str()), Some("NTSC"));
}

#[test]
fn test_analyze_iso_us_serial() {
    let data = make_ps2_iso_with_serial("SLUS_200.62");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.serial_number.as_deref(), Some("SLUS-20062"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Usa]);
}

#[test]
fn test_analyze_iso_eu_serial() {
    let data = make_ps2_iso_with_serial("SLES_501.00");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.serial_number.as_deref(), Some("SLES-50100"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Europe]);
}

#[test]
fn test_analyze_iso_jp_serial() {
    let data = make_ps2_iso_with_serial("SLPS_250.01");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.serial_number.as_deref(), Some("SLPS-25001"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Japan]);
}

#[test]
fn test_analyze_non_ps2_iso_rejected() {
    let data = make_iso("XBOX SYSTEM");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    let options = AnalysisOptions::default();
    assert!(analyzer.analyze(&mut cursor, &options).is_err());
}

#[test]
fn test_analyze_ps1_disc_rejected() {
    // PS2 analyzer should reject discs with BOOT (not BOOT2) in SYSTEM.CNF
    let data = make_iso_with_system_cnf("SLUS_012.34", "BOOT");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    let options = AnalysisOptions::default();
    assert!(analyzer.analyze(&mut cursor, &options).is_err());
}

// -- CUE analysis tests --

#[test]
fn test_analyze_cue_basic() {
    let cue = "FILE \"game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n";
    let mut cursor = Cursor::new(cue.as_bytes().to_vec());
    let analyzer = Ps2Analyzer::new();
    let options = AnalysisOptions::new().quick(true);
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.platform, Some(Platform::Ps2));
    assert_eq!(
        result.extra.get("format").map(|s| s.as_str()),
        Some("CUE Sheet")
    );
    assert_eq!(
        result.extra.get("total_tracks").map(|s| s.as_str()),
        Some("1")
    );
}

// -- DAT methods --

#[test]
fn test_extract_dat_game_code() {
    let analyzer = Ps2Analyzer::new();
    assert_eq!(
        analyzer.extract_dat_game_code("SLUS-20062"),
        Some("SLUS-20062".to_string())
    );
}

// -- File extensions --

#[test]
fn test_file_extensions() {
    let analyzer = Ps2Analyzer::new();
    let exts = analyzer.file_extensions();
    assert!(exts.contains(&"iso"));
    assert!(exts.contains(&"bin"));
    assert!(exts.contains(&"chd"));
    // cue excluded (matches PS1 convention)
    assert!(!exts.contains(&"cue"));
}

// -- DVD layer detection --

#[test]
fn test_dvd_layer_detection_dvd5() {
    // Small ISO → DVD-5
    let data = make_ps2_iso_with_serial("SLUS_200.62");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps2Analyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(
        result.extra.get("dvd_layer").map(|s| s.as_str()),
        Some("DVD-5")
    );
}

// -- Platform and DAT metadata --

#[test]
fn test_platform() {
    let analyzer = Ps2Analyzer::new();
    assert_eq!(analyzer.platform(), Platform::Ps2);
}

#[test]
fn test_dat_names() {
    let analyzer = Ps2Analyzer::new();
    assert_eq!(analyzer.dat_names(), &["Sony - PlayStation 2"]);
}

#[test]
fn test_dat_download_ids() {
    let analyzer = Ps2Analyzer::new();
    assert_eq!(analyzer.dat_download_ids(), &["ps2"]);
}

#[test]
fn test_expects_serial() {
    let analyzer = Ps2Analyzer::new();
    assert!(analyzer.expects_serial());
}
