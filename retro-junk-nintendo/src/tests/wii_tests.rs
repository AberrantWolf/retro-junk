use super::*;
use retro_junk_core::{AnalysisOptions, Region, RomAnalyzer};
use std::io::Cursor;

use crate::nintendo_disc;

// ---------------------------------------------------------------------------
// Compressed format detection tests
// ---------------------------------------------------------------------------

/// Raw Wii ISO is NOT detected as compressed
#[test]
fn test_compressed_detection_raw_iso_not_compressed() {
    let disc = make_default_wii_disc();
    assert!(!nintendo_disc::is_compressed_disc(&mut Cursor::new(disc)));
}

/// Compressed detection seeks back to start
#[test]
fn test_compressed_detection_resets_position() {
    let disc = make_default_wii_disc();
    let mut cursor = Cursor::new(disc);
    nintendo_disc::is_compressed_disc(&mut cursor);
    assert_eq!(cursor.position(), 0);
}

/// Build a synthetic Wii disc image with a valid header.
///
/// The image is 8 KB (enough for header + some padding). The game code,
/// maker code, version, and game name are configurable.
fn make_wii_disc(game_code: &[u8; 4], maker_code: &[u8; 2], version: u8, name: &str) -> Vec<u8> {
    let size = 8 * 1024;
    let mut disc = vec![0u8; size];

    // Game code at 0x0000
    disc[0x0000..0x0004].copy_from_slice(game_code);
    // Maker code at 0x0004
    disc[0x0004..0x0006].copy_from_slice(maker_code);
    // Disc ID at 0x0006
    disc[0x0006] = 0;
    // Version at 0x0007
    disc[0x0007] = version;

    // Wii magic at 0x0018
    disc[0x0018..0x001C].copy_from_slice(&nintendo_disc::WII_MAGIC.to_be_bytes());
    // GC magic at 0x001C: NOT set (zeros) — this is Wii

    // Game name at 0x0020 (null-terminated)
    let name_bytes = name.as_bytes();
    let copy_len = name_bytes.len().min(992);
    disc[0x0020..0x0020 + copy_len].copy_from_slice(&name_bytes[..copy_len]);

    // DOL offset at 0x0420 (plausible value)
    disc[0x0420..0x0424].copy_from_slice(&0x00040000u32.to_be_bytes());
    // FST offset at 0x0424
    disc[0x0424..0x0428].copy_from_slice(&0x00080000u32.to_be_bytes());
    // FST size at 0x0428
    disc[0x0428..0x042C].copy_from_slice(&0x00001000u32.to_be_bytes());

    disc
}

/// Build a default test disc (USA, Nintendo, version 0).
fn make_default_wii_disc() -> Vec<u8> {
    make_wii_disc(b"RSBE", b"01", 0, "Wii Sports")
}

// ---------------------------------------------------------------------------
// can_handle tests
// ---------------------------------------------------------------------------

#[test]
fn test_can_handle_valid() {
    let disc = make_default_wii_disc();
    let analyzer = WiiAnalyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(disc)));
}

#[test]
fn test_can_handle_too_small() {
    let data = vec![0u8; 16];
    let analyzer = WiiAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

#[test]
fn test_can_handle_bad_magic() {
    let mut disc = make_default_wii_disc();
    // Corrupt the Wii magic word
    disc[0x0018..0x001C].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    let analyzer = WiiAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(disc)));
}

#[test]
fn test_can_handle_gc_disc_rejected() {
    // A pure GameCube disc (GC magic only, no Wii magic) should be rejected
    let mut disc = vec![0u8; 8 * 1024];
    disc[0x001C..0x0020].copy_from_slice(&nintendo_disc::GC_MAGIC.to_be_bytes());
    let analyzer = WiiAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(disc)));
}

// ---------------------------------------------------------------------------
// analyze tests
// ---------------------------------------------------------------------------

#[test]
fn test_basic_analysis() {
    let disc = make_default_wii_disc();
    let analyzer = WiiAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();

    assert_eq!(id.serial_number.as_deref(), Some("RSBE"));
    assert_eq!(id.internal_name.as_deref(), Some("Wii Sports"));
    assert_eq!(id.regions, vec![Region::Usa]);
    assert_eq!(id.extra.get("game_code").map(|s| s.as_str()), Some("RSBE"));
    assert_eq!(id.extra.get("maker_code").map(|s| s.as_str()), Some("01"));
    assert_eq!(id.extra.get("format").map(|s| s.as_str()), Some("ISO"));
}

#[test]
fn test_region_japan() {
    let disc = make_wii_disc(b"RSBJ", b"01", 0, "Wii Sports");
    let analyzer = WiiAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(id.regions, vec![Region::Japan]);
}

#[test]
fn test_region_europe() {
    let disc = make_wii_disc(b"RSBP", b"01", 0, "Wii Sports");
    let analyzer = WiiAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(id.regions, vec![Region::Europe]);
}

#[test]
fn test_version_nonzero() {
    let disc = make_wii_disc(b"RSBE", b"01", 1, "Wii Sports");
    let analyzer = WiiAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(id.version.as_deref(), Some("1.01"));
}

#[test]
fn test_dvd_layer_single() {
    // 8 KB file is well under 4.7 GB threshold
    let disc = make_default_wii_disc();
    let analyzer = WiiAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(id.extra.get("dvd_layer").map(|s| s.as_str()), Some("DVD-5"));
}

#[test]
fn test_gc_disc_rejected_by_analyze() {
    // A disc with GC magic but no Wii magic should fail
    let mut disc = make_default_wii_disc();
    disc[0x0018..0x001C].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    disc[0x001C..0x0020].copy_from_slice(&nintendo_disc::GC_MAGIC.to_be_bytes());
    let analyzer = WiiAnalyzer::new();
    let result = analyzer.analyze(&mut Cursor::new(disc), &AnalysisOptions::default());
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// DAT method tests
// ---------------------------------------------------------------------------

#[test]
fn test_extract_dat_game_code() {
    let analyzer = WiiAnalyzer::new();
    assert_eq!(
        analyzer.extract_dat_game_code("RSBE"),
        Some("RSBE".to_string())
    );
}

#[test]
fn test_expects_serial() {
    let analyzer = WiiAnalyzer::new();
    assert!(analyzer.expects_serial());
}

#[test]
fn test_dat_source() {
    let analyzer = WiiAnalyzer::new();
    assert_eq!(analyzer.dat_source(), retro_junk_core::DatSource::Redump);
}

#[test]
fn test_dat_download_ids() {
    let analyzer = WiiAnalyzer::new();
    assert_eq!(analyzer.dat_download_ids(), &["wii"]);
}

#[test]
fn test_dat_names() {
    let analyzer = WiiAnalyzer::new();
    assert_eq!(analyzer.dat_names(), &["Nintendo - Wii"]);
}
