use super::*;
use std::io::Cursor;

/// Build a synthetic 256 KB GBA ROM with a valid header.
fn make_gba_rom() -> Vec<u8> {
    let size = 256 * 1024; // 256 KB
    let mut rom = vec![0u8; size];

    // Entry point (ARM branch instruction placeholder)
    rom[0x00] = 0x00;
    rom[0x01] = 0x00;
    rom[0x02] = 0x00;
    rom[0x03] = 0xEA; // b instruction

    // Nintendo logo at 0x04
    rom[0x04..0x04 + 156].copy_from_slice(&NINTENDO_LOGO);

    // Title at 0xA0: "TESTGAME" (12 bytes, null-padded)
    let title = b"TESTGAME\0\0\0\0";
    rom[0xA0..0xAC].copy_from_slice(title);

    // Game code at 0xAC: "ATEJ" (A=normal game, TE=game id, J=Japan)
    rom[0xAC..0xB0].copy_from_slice(b"ATEJ");

    // Maker code at 0xB0: "01" (Nintendo R&D1)
    rom[0xB0..0xB2].copy_from_slice(b"01");

    // Fixed value at 0xB2
    rom[0xB2] = FIXED_VALUE;

    // Main unit code at 0xB3
    rom[0xB3] = 0x00;

    // Device type at 0xB4
    rom[0xB4] = 0x00;

    // Reserved area 0xB5-0xBB (zeros)

    // Software version at 0xBC
    rom[0xBC] = 0x00;

    // Compute and set header checksum
    recompute_checksum(&mut rom);

    rom
}

/// Recompute the GBA header complement checksum for a ROM buffer.
fn recompute_checksum(rom: &mut Vec<u8>) {
    let mut sum: u8 = 0;
    for &b in &rom[0xA0..=0xBC] {
        sum = sum.wrapping_add(b);
    }
    rom[0xBD] = (-(sum as i8)).wrapping_sub(0x19) as u8;
}

#[test]
fn test_can_handle_valid() {
    let rom = make_gba_rom();
    let analyzer = GbaAnalyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_too_small() {
    let data = vec![0u8; 0x80]; // Too small
    let analyzer = GbaAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

#[test]
fn test_can_handle_bad_logo() {
    let mut rom = make_gba_rom();
    rom[0x04] = 0xFF; // Corrupt logo
    let analyzer = GbaAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_bad_fixed_value() {
    let mut rom = make_gba_rom();
    rom[0xB2] = 0x00; // Wrong fixed value
    let analyzer = GbaAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_basic_analysis() {
    let rom = make_gba_rom();
    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.internal_name.as_deref(), Some("TESTGAME"));
    assert_eq!(result.platform.as_deref(), Some("Game Boy Advance"));
    assert_eq!(result.serial_number.as_deref(), Some("AGB-ATEJ"));
    assert_eq!(result.maker_code.as_deref(), Some("Nintendo R&D1"));
    assert_eq!(result.version.as_deref(), Some("v0"));
    assert_eq!(result.file_size, Some(256 * 1024));
    assert_eq!(result.expected_size, Some(256 * 1024));
    assert_eq!(result.regions, vec![Region::Japan]);
    assert_eq!(
        result.extra.get("checksum_status:GBA Complement").unwrap(),
        "OK"
    );
    assert_eq!(result.extra.get("game_code").unwrap(), "ATEJ");
}

#[test]
fn test_region_usa() {
    let mut rom = make_gba_rom();
    rom[0xAF] = b'E'; // USA
    recompute_checksum(&mut rom);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.regions, vec![Region::Usa]);
}

#[test]
fn test_region_europe() {
    let mut rom = make_gba_rom();
    rom[0xAF] = b'P'; // Europe
    recompute_checksum(&mut rom);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.regions, vec![Region::Europe]);
}

#[test]
fn test_region_europe_german() {
    let mut rom = make_gba_rom();
    rom[0xAF] = b'D'; // German → Europe
    recompute_checksum(&mut rom);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.regions, vec![Region::Europe]);
}

#[test]
fn test_region_europe_french() {
    let mut rom = make_gba_rom();
    rom[0xAF] = b'F'; // French → Europe
    recompute_checksum(&mut rom);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.regions, vec![Region::Europe]);
}

#[test]
fn test_region_korea() {
    let mut rom = make_gba_rom();
    rom[0xAF] = b'K'; // Korea
    recompute_checksum(&mut rom);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.regions, vec![Region::Korea]);
}

#[test]
fn test_checksum_mismatch() {
    let mut rom = make_gba_rom();
    rom[0xBD] = rom[0xBD].wrapping_add(1); // Corrupt checksum

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    let status = result.extra.get("checksum_status:GBA Complement").unwrap();
    assert!(
        status.starts_with("MISMATCH"),
        "Expected MISMATCH, got: {}",
        status
    );
}

#[test]
fn test_invalid_fixed_value() {
    let mut rom = make_gba_rom();
    rom[0xB2] = 0x42; // Wrong fixed value
    recompute_checksum(&mut rom);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    // can_handle would reject this, but analyze() still works
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    let fixed = result.extra.get("fixed_value").unwrap();
    assert!(
        fixed.contains("INVALID"),
        "Expected INVALID, got: {}",
        fixed
    );
}

#[test]
fn test_software_version() {
    let mut rom = make_gba_rom();
    rom[0xBC] = 3;
    recompute_checksum(&mut rom);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.version.as_deref(), Some("v3"));
}

#[test]
fn test_expected_size_power_of_2() {
    // 256 KB is already a power of 2
    assert_eq!(expected_rom_size(256 * 1024), Some(256 * 1024));
    // 300 KB rounds up to 512 KB
    assert_eq!(expected_rom_size(300 * 1024), Some(512 * 1024));
    // 1 byte rounds up to 1
    assert_eq!(expected_rom_size(1), Some(1));
    // 0 returns None
    assert_eq!(expected_rom_size(0), None);
    // Exactly 32 MB
    assert_eq!(expected_rom_size(32 * 1024 * 1024), Some(32 * 1024 * 1024));
    // Over 32 MB caps at 32 MB
    assert_eq!(expected_rom_size(33 * 1024 * 1024), Some(32 * 1024 * 1024));
}

#[test]
fn test_save_type_sram() {
    let mut rom = make_gba_rom();
    // Place SRAM_V magic string somewhere in the ROM
    let magic = b"SRAM_V";
    rom[0x1000..0x1000 + magic.len()].copy_from_slice(magic);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.extra.get("save_type").unwrap(), "SRAM");
}

#[test]
fn test_save_type_flash1m() {
    let mut rom = make_gba_rom();
    let magic = b"FLASH1M_V";
    rom[0x1000..0x1000 + magic.len()].copy_from_slice(magic);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.extra.get("save_type").unwrap(), "Flash 1M");
}

#[test]
fn test_save_type_eeprom() {
    let mut rom = make_gba_rom();
    let magic = b"EEPROM_V";
    rom[0x1000..0x1000 + magic.len()].copy_from_slice(magic);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.extra.get("save_type").unwrap(), "EEPROM");
}

#[test]
fn test_quick_mode_skips_save_type() {
    let mut rom = make_gba_rom();
    let magic = b"SRAM_V";
    rom[0x1000..0x1000 + magic.len()].copy_from_slice(magic);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions {
        quick: true,
        ..Default::default()
    };
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert!(result.extra.get("save_type").is_none());
}

#[test]
fn test_title_trimming() {
    let mut rom = make_gba_rom();
    // Title with trailing nulls should be trimmed
    rom[0xA0..0xAC].copy_from_slice(b"HI\0\0\0\0\0\0\0\0\0\0");
    recompute_checksum(&mut rom);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.internal_name.as_deref(), Some("HI"));
}

#[test]
fn test_serial_number_format() {
    let rom = make_gba_rom();
    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert!(result.serial_number.as_deref().unwrap().starts_with("AGB-"));
}

#[test]
fn test_too_small_file() {
    let data = vec![0u8; 0x80]; // Not enough for header
    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(data), &options);
    assert!(result.is_err());
}

#[test]
fn test_device_type_nonzero() {
    let mut rom = make_gba_rom();
    rom[0xB4] = 0x01; // Non-zero device type
    recompute_checksum(&mut rom);

    let analyzer = GbaAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.extra.get("device_type").unwrap(), "01");
}

#[test]
fn test_region_from_game_code_function() {
    assert_eq!(region_from_game_code("ATEJ"), Some(Region::Japan));
    assert_eq!(region_from_game_code("ATEE"), Some(Region::Usa));
    assert_eq!(region_from_game_code("ATEP"), Some(Region::Europe));
    assert_eq!(region_from_game_code("ATEK"), Some(Region::Korea));
    assert_eq!(region_from_game_code("ATEC"), Some(Region::China));
    assert_eq!(region_from_game_code("ATE"), None); // Too short
}
