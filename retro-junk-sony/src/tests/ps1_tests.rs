use super::*;
use std::io::Cursor;

// -- Test helpers --

/// Build a minimal 2048-byte PVD sector with a given system identifier.
fn make_pvd_sector(system_id: &str) -> [u8; 2048] {
    let mut sector = [0u8; 2048];
    sector[0] = 0x01;
    sector[1..6].copy_from_slice(b"CD001");
    sector[6] = 0x01;

    let id_bytes = system_id.as_bytes();
    let len = id_bytes.len().min(32);
    sector[8..8 + len].copy_from_slice(&id_bytes[..len]);
    for i in len..32 {
        sector[8 + i] = b' ';
    }

    let vol = b"TEST_VOLUME";
    sector[40..40 + vol.len()].copy_from_slice(vol);
    for i in vol.len()..32 {
        sector[40 + i] = b' ';
    }

    sector[80..84].copy_from_slice(&200u32.to_le_bytes());
    sector[84..88].copy_from_slice(&200u32.to_be_bytes());

    // Root directory record at offset 156
    sector[156] = 34;
    sector[158..162].copy_from_slice(&18u32.to_le_bytes());
    sector[166..170].copy_from_slice(&2048u32.to_le_bytes());

    sector
}

/// Build a minimal ISO: 16 sectors of padding + PVD at sector 16.
fn make_iso(system_id: &str) -> Vec<u8> {
    let mut data = vec![0u8; 16 * 2048];
    let pvd = make_pvd_sector(system_id);
    data.extend_from_slice(&pvd);
    data
}

/// CD sync pattern.
const CD_SYNC: [u8; 12] = [
    0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
];

/// Wrap 2048 bytes of user data into a raw 2352-byte Mode 2 Form 1 sector.
fn make_raw_sector(user_data: &[u8; 2048]) -> [u8; 2352] {
    let mut sector = [0u8; 2352];
    sector[0..12].copy_from_slice(&CD_SYNC);
    sector[15] = 0x02; // mode 2
    sector[24..24 + 2048].copy_from_slice(user_data);
    sector
}

/// Build a raw BIN: 16 raw empty sectors + raw PVD sector.
fn make_raw_bin(system_id: &str) -> Vec<u8> {
    let empty_user = [0u8; 2048];
    let mut data = Vec::new();
    for _ in 0..16 {
        data.extend_from_slice(&make_raw_sector(&empty_user));
    }
    let pvd = make_pvd_sector(system_id);
    data.extend_from_slice(&make_raw_sector(&pvd));
    data
}

/// Build a directory record for a file.
fn make_dir_record(filename: &str, extent_lba: u32, data_length: u32) -> Vec<u8> {
    let id_bytes = filename.as_bytes();
    let id_len = id_bytes.len();
    let record_len = 33 + id_len + (id_len % 2);
    let mut record = vec![0u8; record_len];
    record[0] = record_len as u8;
    record[2..6].copy_from_slice(&extent_lba.to_le_bytes());
    record[10..14].copy_from_slice(&data_length.to_le_bytes());
    record[25] = 0;
    record[32] = id_len as u8;
    record[33..33 + id_len].copy_from_slice(id_bytes);
    record
}

/// Build a full ISO with a root directory containing SYSTEM.CNF.
fn make_iso_with_system_cnf(serial: &str) -> Vec<u8> {
    let system_cnf_content = format!("BOOT = cdrom:\\{};1\r\nVMODE = NTSC\r\n", serial);
    let cnf_bytes = system_cnf_content.as_bytes();

    let mut data = vec![0u8; 16 * 2048]; // sectors 0-15

    // Sector 16: PVD
    let mut pvd = make_pvd_sector("PLAYSTATION");
    pvd[158..162].copy_from_slice(&18u32.to_le_bytes());
    pvd[166..170].copy_from_slice(&2048u32.to_le_bytes());
    data.extend_from_slice(&pvd);

    // Sector 17: empty
    data.extend_from_slice(&[0u8; 2048]);

    // Sector 18: root directory
    let mut dir_sector = [0u8; 2048];
    let mut pos = 0;

    let dot_record = make_dir_record("\0", 18, 2048);
    dir_sector[pos..pos + dot_record.len()].copy_from_slice(&dot_record);
    pos += dot_record.len();

    let dotdot_record = make_dir_record("\x01", 18, 2048);
    dir_sector[pos..pos + dotdot_record.len()].copy_from_slice(&dotdot_record);
    pos += dotdot_record.len();

    let cnf_record = make_dir_record("SYSTEM.CNF;1", 19, cnf_bytes.len() as u32);
    dir_sector[pos..pos + cnf_record.len()].copy_from_slice(&cnf_record);

    data.extend_from_slice(&dir_sector);

    // Sector 19: SYSTEM.CNF content
    let mut cnf_sector = [0u8; 2048];
    cnf_sector[..cnf_bytes.len()].copy_from_slice(cnf_bytes);
    data.extend_from_slice(&cnf_sector);

    data
}

// -- can_handle tests --

#[test]
fn test_can_handle_ps1_iso() {
    let data = make_iso("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    assert!(analyzer.can_handle(&mut cursor));
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
    let data = make_iso_with_system_cnf("SLUS_012.34");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::default(); // non-quick
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.serial_number.as_deref(), Some("SLUS-01234"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Usa]);
    assert!(result.extra.contains_key("boot_path"));
    assert_eq!(result.extra.get("vmode").map(|s| s.as_str()), Some("NTSC"));
}

#[test]
fn test_analyze_iso_quick_mode_still_extracts_serial() {
    let data = make_iso_with_system_cnf("SLUS_012.34");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::new().quick(true);
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    // Serial extraction is fast (1-2 sector reads) so it runs even in quick mode
    assert_eq!(result.serial_number.as_deref(), Some("SLUS-01234"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Usa]);
}

#[test]
fn test_analyze_iso_european_serial() {
    let data = make_iso_with_system_cnf("SLES_123.45");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.serial_number.as_deref(), Some("SLES-12345"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Europe]);
}

#[test]
fn test_analyze_iso_japanese_serial() {
    let data = make_iso_with_system_cnf("SLPS_000.01");
    let mut cursor = Cursor::new(data);
    let analyzer = Ps1Analyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut cursor, &options).unwrap();
    assert_eq!(result.serial_number.as_deref(), Some("SLPS-00001"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Japan]);
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
    // Normal serial passes through unchanged
    assert_eq!(
        analyzer.extract_dat_game_code("SLUS-01234"),
        Some("SLUS-01234".to_string())
    );
}

#[test]
fn test_extract_dat_game_code_multi_disc_fixups() {
    let analyzer = Ps1Analyzer::new();

    // FF7 disc 2 boot serial → suffixed catalog serial
    assert_eq!(
        analyzer.extract_dat_game_code("SCUS-94164"),
        Some("SCUS-94163-1".to_string())
    );
    // FF7 disc 3 boot serial → suffixed catalog serial
    assert_eq!(
        analyzer.extract_dat_game_code("SCUS-94165"),
        Some("SCUS-94163-2".to_string())
    );
    // Star Ocean disc 2 boot serial → suffixed catalog serial
    assert_eq!(
        analyzer.extract_dat_game_code("SCUS-94422"),
        Some("SCUS-94421-1".to_string())
    );

    // FF7 disc 1 is NOT in the fixup table — it passes through as-is
    // (the matcher's suffix preference handles disc 1 via -0 lookup)
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
    // cue and img excluded: cue files don't contain game data (just
    // references to bin/img tracks), and scanning them caused spurious errors
    assert!(!exts.contains(&"cue"));
    assert!(!exts.contains(&"img"));
    // pbp and ecm removed per plan
    assert!(!exts.contains(&"pbp"));
    assert!(!exts.contains(&"ecm"));
}
