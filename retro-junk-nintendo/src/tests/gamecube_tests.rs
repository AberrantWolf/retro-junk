use super::*;
use retro_junk_core::{AnalysisOptions, HashAlgorithms, Region, RomAnalyzer};
use std::io::Cursor;

use crate::nintendo_disc;

// ---------------------------------------------------------------------------
// Compressed format detection tests
// ---------------------------------------------------------------------------

/// WBFS magic: "WBFS" at offset 0
#[test]
fn test_compressed_detection_wbfs() {
    let mut data = vec![0u8; 64];
    data[0..4].copy_from_slice(b"WBFS");
    assert!(nintendo_disc::is_compressed_disc(&mut Cursor::new(data)));
}

/// WIA/RVZ magic: "WIA\x01" at offset 0
#[test]
fn test_compressed_detection_wia_rvz() {
    let mut data = vec![0u8; 64];
    data[0..4].copy_from_slice(b"WIA\x01");
    assert!(nintendo_disc::is_compressed_disc(&mut Cursor::new(data)));
}

/// Raw GC ISO is NOT detected as compressed
#[test]
fn test_compressed_detection_raw_iso_not_compressed() {
    let disc = make_default_gc_disc();
    assert!(!nintendo_disc::is_compressed_disc(&mut Cursor::new(disc)));
}

/// Empty/short data is NOT detected as compressed
#[test]
fn test_compressed_detection_too_small() {
    let data = vec![0u8; 2];
    assert!(!nintendo_disc::is_compressed_disc(&mut Cursor::new(data)));
}

/// Compressed detection seeks back to start
#[test]
fn test_compressed_detection_resets_position() {
    let disc = make_default_gc_disc();
    let mut cursor = Cursor::new(disc);
    nintendo_disc::is_compressed_disc(&mut cursor);
    assert_eq!(cursor.position(), 0);
}

/// Build a synthetic GameCube disc image with a valid header.
///
/// The image is 8 KB (enough for header + some padding). The game code,
/// maker code, version, and game name are configurable.
fn make_gc_disc(game_code: &[u8; 4], maker_code: &[u8; 2], version: u8, name: &str) -> Vec<u8> {
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
    // Audio streaming at 0x0008
    disc[0x0008] = 0;

    // Wii magic at 0x0018: NOT set (zeros) — this is GameCube
    // GC magic at 0x001C
    disc[0x001C..0x0020].copy_from_slice(&nintendo_disc::GC_MAGIC.to_be_bytes());

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
fn make_default_gc_disc() -> Vec<u8> {
    make_gc_disc(b"GALE", b"01", 0, "THE LEGEND OF ZELDA")
}

// ---------------------------------------------------------------------------
// can_handle tests
// ---------------------------------------------------------------------------

#[test]
fn test_can_handle_valid() {
    let disc = make_default_gc_disc();
    let analyzer = GameCubeAnalyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(disc)));
}

#[test]
fn test_can_handle_too_small() {
    let data = vec![0u8; 16];
    let analyzer = GameCubeAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

#[test]
fn test_can_handle_bad_magic() {
    let mut disc = make_default_gc_disc();
    // Corrupt the GC magic word
    disc[0x001C..0x0020].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    let analyzer = GameCubeAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(disc)));
}

#[test]
fn test_can_handle_wii_disc_rejected() {
    let mut disc = make_default_gc_disc();
    // Set Wii magic at 0x0018 — this should NOT be detected as GameCube
    disc[0x0018..0x001C].copy_from_slice(&nintendo_disc::WII_MAGIC.to_be_bytes());
    let analyzer = GameCubeAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(disc)));
}

// ---------------------------------------------------------------------------
// analyze tests
// ---------------------------------------------------------------------------

#[test]
fn test_basic_analysis() {
    let disc = make_default_gc_disc();
    let analyzer = GameCubeAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();

    assert_eq!(id.serial_number.as_deref(), Some("GALE"));
    assert_eq!(id.internal_name.as_deref(), Some("THE LEGEND OF ZELDA"));
    assert_eq!(id.regions, vec![Region::Usa]);
    assert_eq!(id.extra.get("game_code").map(|s| s.as_str()), Some("GALE"));
    assert_eq!(id.extra.get("maker_code").map(|s| s.as_str()), Some("01"));
    assert_eq!(
        id.extra.get("maker_name").map(|s| s.as_str()),
        Some("Nintendo R&D1")
    );
    assert_eq!(id.extra.get("format").map(|s| s.as_str()), Some("ISO"));
    assert_eq!(
        id.extra.get("product_code").map(|s| s.as_str()),
        Some("DOL-GALE-0")
    );
    assert_eq!(id.expected_size, Some(GCM_DISC_SIZE));
}

#[test]
fn test_region_usa() {
    let disc = make_gc_disc(b"GALE", b"01", 0, "GAME");
    let analyzer = GameCubeAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(id.regions, vec![Region::Usa]);
}

#[test]
fn test_region_japan() {
    let disc = make_gc_disc(b"GALJ", b"01", 0, "GAME");
    let analyzer = GameCubeAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(id.regions, vec![Region::Japan]);
}

#[test]
fn test_region_europe() {
    let disc = make_gc_disc(b"GALP", b"01", 0, "GAME");
    let analyzer = GameCubeAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(id.regions, vec![Region::Europe]);
}

#[test]
fn test_region_korea() {
    let disc = make_gc_disc(b"GALK", b"01", 0, "GAME");
    let analyzer = GameCubeAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(id.regions, vec![Region::Korea]);
}

#[test]
fn test_version_nonzero() {
    let disc = make_gc_disc(b"GALE", b"01", 2, "GAME");
    let analyzer = GameCubeAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(id.version.as_deref(), Some("1.02"));
}

#[test]
fn test_version_zero_is_none() {
    let disc = make_gc_disc(b"GALE", b"01", 0, "GAME");
    let analyzer = GameCubeAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(id.version, None);
}

#[test]
fn test_game_name_trimming() {
    // Name followed by nulls should be trimmed
    let disc = make_gc_disc(b"GALE", b"01", 0, "TEST GAME");
    let analyzer = GameCubeAnalyzer::new();
    let id = analyzer
        .analyze(&mut Cursor::new(disc), &AnalysisOptions::default())
        .unwrap();
    assert_eq!(id.internal_name.as_deref(), Some("TEST GAME"));
}

#[test]
fn test_wii_disc_rejected_by_analyze() {
    let mut disc = make_default_gc_disc();
    // Set Wii magic, clear GC magic
    disc[0x0018..0x001C].copy_from_slice(&nintendo_disc::WII_MAGIC.to_be_bytes());
    disc[0x001C..0x0020].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    let analyzer = GameCubeAnalyzer::new();
    let result = analyzer.analyze(&mut Cursor::new(disc), &AnalysisOptions::default());
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// DAT method tests
// ---------------------------------------------------------------------------

#[test]
fn test_extract_dat_game_code() {
    let analyzer = GameCubeAnalyzer::new();
    assert_eq!(
        analyzer.extract_dat_game_code("GALE"),
        Some("GALE".to_string())
    );
}

#[test]
fn test_extract_dat_game_code_invalid() {
    let analyzer = GameCubeAnalyzer::new();
    // Too long
    assert_eq!(analyzer.extract_dat_game_code("GALE01"), None);
    // Too short
    assert_eq!(analyzer.extract_dat_game_code("GA"), None);
}

#[test]
fn test_expects_serial() {
    let analyzer = GameCubeAnalyzer::new();
    assert!(analyzer.expects_serial());
}

#[test]
fn test_dat_source() {
    let analyzer = GameCubeAnalyzer::new();
    assert_eq!(analyzer.dat_source(), retro_junk_core::DatSource::Redump);
}

#[test]
fn test_dat_download_ids_defaults_to_dat_names() {
    let analyzer = GameCubeAnalyzer::new();
    // dat_download_ids() should delegate to dat_names() (the default impl)
    assert_eq!(analyzer.dat_download_ids(), &["Nintendo - GameCube"]);
}

#[test]
fn test_dat_names() {
    let analyzer = GameCubeAnalyzer::new();
    assert_eq!(analyzer.dat_names(), &["Nintendo - GameCube"]);
}

// ---------------------------------------------------------------------------
// Container hash tests
// ---------------------------------------------------------------------------

#[test]
fn test_container_hashes_returns_none_for_raw_iso() {
    let disc = make_default_gc_disc();
    let analyzer = GameCubeAnalyzer::new();
    let result = analyzer
        .compute_container_hashes(&mut Cursor::new(disc), HashAlgorithms::Crc32Sha1, None)
        .unwrap();
    assert!(
        result.is_none(),
        "Raw ISO should return None (use standard hasher)"
    );
}
