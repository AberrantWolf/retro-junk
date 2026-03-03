use super::*;
use crate::disc_test_helpers::{make_iso, make_iso_with_system_cnf, make_raw_bin};
use std::io::Cursor;

// sony_disc tests use "BOOT" key by default for SYSTEM.CNF
fn make_boot_iso_with_system_cnf(serial: &str) -> Vec<u8> {
    make_iso_with_system_cnf(serial, "BOOT")
}

// -- Format detection tests --

#[test]
fn test_detect_iso_format() {
    let data = make_iso("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    assert_eq!(
        detect_disc_format(&mut cursor).unwrap(),
        DiscFormat::Iso2048
    );
}

#[test]
fn test_detect_raw_bin_format() {
    let data = make_raw_bin("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    assert_eq!(
        detect_disc_format(&mut cursor).unwrap(),
        DiscFormat::RawSector2352
    );
}

#[test]
fn test_detect_chd_magic() {
    let mut data = vec![0u8; 64];
    data[..8].copy_from_slice(CHD_MAGIC);
    let mut cursor = Cursor::new(data);
    assert_eq!(detect_disc_format(&mut cursor).unwrap(), DiscFormat::Chd);
}

#[test]
fn test_detect_cue_text() {
    let cue = b"FILE \"game.bin\" BINARY\r\n  TRACK 01 MODE2/2352\r\n    INDEX 01 00:00:00\r\n";
    let mut cursor = Cursor::new(cue.to_vec());
    assert_eq!(detect_disc_format(&mut cursor).unwrap(), DiscFormat::Cue);
}

#[test]
fn test_detect_invalid_data() {
    let data = vec![
        0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let mut cursor = Cursor::new(data);
    assert!(detect_disc_format(&mut cursor).is_err());
}

// -- PVD parsing tests --

#[test]
fn test_read_pvd_iso() {
    let data = make_iso("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    assert_eq!(pvd.system_identifier, "PLAYSTATION");
    assert_eq!(pvd.volume_identifier, "TEST_VOLUME");
    assert_eq!(pvd.volume_space_size, 200);
}

#[test]
fn test_read_pvd_raw_bin() {
    let data = make_raw_bin("PLAYSTATION");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::RawSector2352).unwrap();
    assert_eq!(pvd.system_identifier, "PLAYSTATION");
    assert_eq!(pvd.volume_identifier, "TEST_VOLUME");
}

#[test]
fn test_pvd_non_playstation() {
    let data = make_iso("SOME_OTHER_SYS");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    assert_eq!(pvd.system_identifier, "SOME_OTHER_SYS");
}

// -- SYSTEM.CNF parsing tests --

#[test]
fn test_parse_system_cnf_standard() {
    let cnf = "BOOT = cdrom:\\SLUS_012.34;1\r\nVMODE = NTSC\r\n";
    let result = parse_system_cnf(cnf).unwrap();
    assert_eq!(result.boot_path, "cdrom:\\SLUS_012.34;1");
    assert_eq!(result.boot_key, BootKey::Boot);
    assert_eq!(result.vmode.as_deref(), Some("NTSC"));
}

#[test]
fn test_parse_system_cnf_boot2() {
    let cnf = "BOOT2 = cdrom0:\\SLPS_123.45;1\r\n";
    let result = parse_system_cnf(cnf).unwrap();
    assert_eq!(result.boot_path, "cdrom0:\\SLPS_123.45;1");
    assert_eq!(result.boot_key, BootKey::Boot2);
    assert_eq!(result.vmode, None);
}

#[test]
fn test_parse_system_cnf_boot2_preferred_over_boot() {
    // When both BOOT and BOOT2 are present, BOOT2 wins (PS2 is more specific)
    let cnf = "BOOT = cdrom:\\OLD.EXE;1\r\nBOOT2 = cdrom0:\\SLUS_999.99;1\r\n";
    let result = parse_system_cnf(cnf).unwrap();
    assert_eq!(result.boot_path, "cdrom0:\\SLUS_999.99;1");
    assert_eq!(result.boot_key, BootKey::Boot2);
}

#[test]
fn test_parse_system_cnf_missing_boot() {
    let cnf = "VMODE = PAL\r\n";
    assert!(parse_system_cnf(cnf).is_err());
}

// -- Serial extraction tests --

#[test]
fn test_extract_serial_slus() {
    assert_eq!(
        extract_serial("cdrom:\\SLUS_012.34;1"),
        Some("SLUS-01234".to_string())
    );
}

#[test]
fn test_extract_serial_sles() {
    assert_eq!(
        extract_serial("cdrom:\\SLES_567.89;1"),
        Some("SLES-56789".to_string())
    );
}

#[test]
fn test_extract_serial_scps() {
    assert_eq!(
        extract_serial("cdrom:\\SCPS_100.01;1"),
        Some("SCPS-10001".to_string())
    );
}

#[test]
fn test_extract_serial_double_backslash() {
    assert_eq!(
        extract_serial("cdrom:\\\\SLUS_012.34;1"),
        Some("SLUS-01234".to_string())
    );
}

#[test]
fn test_extract_serial_no_version() {
    assert_eq!(
        extract_serial("cdrom:\\SLPS_000.01"),
        Some("SLPS-00001".to_string())
    );
}

#[test]
fn test_extract_serial_no_backslash() {
    // Some games use "cdrom:FILENAME" with no path separator
    assert_eq!(
        extract_serial("cdrom:SLUS_006.91;1"),
        Some("SLUS-00691".to_string())
    );
}

#[test]
fn test_extract_serial_ps2_cdrom0() {
    // PS2 uses cdrom0: prefix
    assert_eq!(
        extract_serial("cdrom0:\\SLUS_200.62;1"),
        Some("SLUS-20062".to_string())
    );
}

#[test]
fn test_extract_serial_invalid() {
    assert_eq!(extract_serial("cdrom:\\BOOT.EXE;1"), None);
}

// -- Region mapping tests --

#[test]
fn test_serial_to_region() {
    assert_eq!(serial_to_region("SLUS-01234"), Some(Region::Usa));
    assert_eq!(serial_to_region("SCUS-94900"), Some(Region::Usa));
    assert_eq!(serial_to_region("SLES-01234"), Some(Region::Europe));
    assert_eq!(serial_to_region("SCES-01234"), Some(Region::Europe));
    assert_eq!(serial_to_region("SLPS-01234"), Some(Region::Japan));
    assert_eq!(serial_to_region("SCPS-01234"), Some(Region::Japan));
    assert_eq!(serial_to_region("SLPM-01234"), Some(Region::Japan));
    assert_eq!(serial_to_region("SLKA-01234"), Some(Region::Korea));
    assert_eq!(serial_to_region("XXXX-01234"), None);
}

// -- CUE parsing tests --

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

// -- Full ISO analysis tests --

#[test]
fn test_find_system_cnf_in_iso() {
    let data = make_boot_iso_with_system_cnf("SLUS_012.34");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    let content = find_file_in_root(&mut cursor, DiscFormat::Iso2048, &pvd, "SYSTEM.CNF").unwrap();
    let text = String::from_utf8_lossy(&content);
    assert!(text.contains("SLUS_012.34"));
}

#[test]
fn test_full_iso_serial_extraction() {
    let data = make_boot_iso_with_system_cnf("SLUS_012.34");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    let content = find_file_in_root(&mut cursor, DiscFormat::Iso2048, &pvd, "SYSTEM.CNF").unwrap();
    let text = String::from_utf8_lossy(&content);
    let cnf = parse_system_cnf(&text).unwrap();
    let serial = extract_serial(&cnf.boot_path).unwrap();
    assert_eq!(serial, "SLUS-01234");
}

#[test]
fn test_file_not_found_in_root() {
    let data = make_boot_iso_with_system_cnf("SLUS_012.34");
    let mut cursor = Cursor::new(data);
    let pvd = read_pvd(&mut cursor, DiscFormat::Iso2048).unwrap();
    assert!(find_file_in_root(&mut cursor, DiscFormat::Iso2048, &pvd, "NONEXIST.TXT").is_err());
}

// ---------------------------------------------------------------------------
// Multi-track BIN hashing tests
// ---------------------------------------------------------------------------

/// Build a synthetic multi-track raw BIN: `data_sectors` data sectors (with
/// CD sync pattern) followed by `audio_sectors` audio sectors (random-ish
/// bytes, no sync pattern).
fn make_multi_track_bin(data_sectors: usize, audio_sectors: usize) -> Vec<u8> {
    let mut bin = Vec::with_capacity((data_sectors + audio_sectors) * RAW_SECTOR_SIZE as usize);
    for i in 0..data_sectors {
        let mut sector = [0u8; RAW_SECTOR_SIZE as usize];
        sector[0..12].copy_from_slice(&CD_SYNC_PATTERN);
        sector[15] = 0x02; // Mode 2
        for (j, byte) in sector[24..2072].iter_mut().enumerate() {
            *byte = ((i * 251 + j * 97) & 0xFF) as u8;
        }
        bin.extend_from_slice(&sector);
    }
    for i in 0..audio_sectors {
        let mut sector = [0u8; RAW_SECTOR_SIZE as usize];
        for (j, byte) in sector.iter_mut().enumerate() {
            *byte = ((i * 173 + j * 59 + 0xAA) & 0xFF) as u8;
        }
        sector[0] = 0xAA;
        bin.extend_from_slice(&sector);
    }
    bin
}

/// Compute CRC32/SHA1/MD5 of a byte slice directly (reference implementation).
fn reference_hashes(data: &[u8]) -> (String, String, String) {
    use sha1::Digest;

    let crc = {
        let mut h = crc32fast::Hasher::new();
        h.update(data);
        format!("{:08x}", h.finalize())
    };
    let sha1 = {
        let mut h = sha1::Sha1::new();
        h.update(data);
        format!("{:x}", h.finalize())
    };
    let md5 = {
        let mut ctx = md5::Context::new();
        ctx.consume(data);
        format!("{:x}", ctx.compute())
    };
    (crc, sha1, md5)
}

#[test]
fn test_find_raw_bin_data_track_boundary() {
    let bin = make_multi_track_bin(10, 5);
    let mut cursor = Cursor::new(bin);
    let result = find_raw_bin_data_track_size(&mut cursor).unwrap();
    assert_eq!(result, Some(10 * RAW_SECTOR_SIZE));
}

#[test]
fn test_find_raw_bin_data_track_single_track() {
    let bin = make_multi_track_bin(10, 0);
    let mut cursor = Cursor::new(bin);
    let result = find_raw_bin_data_track_size(&mut cursor).unwrap();
    assert_eq!(result, None);
}

#[test]
fn test_multi_track_bin_hashes_data_only() {
    let data_sectors = 20;
    let audio_sectors = 8;
    let bin = make_multi_track_bin(data_sectors, audio_sectors);

    let data_track_bytes = data_sectors * RAW_SECTOR_SIZE as usize;
    let (expected_crc, expected_sha1, expected_md5) = reference_hashes(&bin[..data_track_bytes]);

    let mut cursor = Cursor::new(bin);
    let analyzer = crate::ps1::Ps1Analyzer;
    let algorithms = retro_junk_core::HashAlgorithms::All;
    use retro_junk_core::RomAnalyzer;
    let result = analyzer
        .compute_container_hashes(&mut cursor, algorithms, None)
        .expect("compute_container_hashes failed");

    let hashes = result.expect("Expected Some(hashes) for multi-track BIN");
    assert_eq!(hashes.crc32, expected_crc, "CRC32 mismatch");
    assert_eq!(
        hashes.sha1.as_deref(),
        Some(expected_sha1.as_str()),
        "SHA1 mismatch"
    );
    assert_eq!(
        hashes.md5.as_deref(),
        Some(expected_md5.as_str()),
        "MD5 mismatch"
    );
    assert_eq!(
        hashes.data_size, data_track_bytes as u64,
        "data_size mismatch"
    );
}

#[test]
fn test_single_track_bin_returns_none() {
    let bin = make_multi_track_bin(20, 0);
    let mut cursor = Cursor::new(bin);
    let analyzer = crate::ps1::Ps1Analyzer;
    let algorithms = retro_junk_core::HashAlgorithms::All;
    use retro_junk_core::RomAnalyzer;
    let result = analyzer
        .compute_container_hashes(&mut cursor, algorithms, None)
        .expect("compute_container_hashes failed");

    assert!(result.is_none(), "Single-track BIN should return None");
}

// ---------------------------------------------------------------------------
// CHD metadata parsing tests
// ---------------------------------------------------------------------------

#[test]
fn test_parse_meta_field_basic() {
    let text = "TRACK:1 TYPE:MODE2_RAW SUBTYPE:NONE FRAMES:229020 PREFRAMES:150";
    assert_eq!(parse_meta_field(text, "TRACK"), Some("1"));
    assert_eq!(parse_meta_field(text, "TYPE"), Some("MODE2_RAW"));
    assert_eq!(parse_meta_field(text, "FRAMES"), Some("229020"));
    assert_eq!(parse_meta_field(text, "PREFRAMES"), Some("150"));
    assert_eq!(parse_meta_field(text, "SUBTYPE"), Some("NONE"));
}

#[test]
fn test_parse_meta_field_missing() {
    let text = "TRACK:1 TYPE:AUDIO SUBTYPE:NONE FRAMES:18995";
    assert_eq!(parse_meta_field(text, "POSTGAP"), None);
    assert_eq!(parse_meta_field(text, "PREGAP"), None);
}

#[test]
fn test_parse_meta_field_audio_track() {
    let text = "TRACK:2 TYPE:AUDIO SUBTYPE:NONE FRAMES:18995 PREFRAMES:150";
    assert_eq!(parse_meta_field(text, "TRACK"), Some("2"));
    assert_eq!(parse_meta_field(text, "TYPE"), Some("AUDIO"));
    assert_eq!(parse_meta_field(text, "FRAMES"), Some("18995"));
}
