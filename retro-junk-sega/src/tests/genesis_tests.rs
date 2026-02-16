use super::*;
use std::io::Cursor;

/// Build a minimal valid Genesis ROM with the given header fields.
fn make_genesis_rom(
    system_type: &str,
    domestic_title: &str,
    overseas_title: &str,
    serial: &str,
    region_codes: &str,
) -> Vec<u8> {
    // Total ROM: 0x0200 header area + 0x0200 data = 0x0400 bytes
    let mut rom = vec![0u8; 0x0400];

    // 68000 vectors: initial SP at 0x00, initial PC at 0x04
    // (doesn't matter for analysis, but let's set something)
    rom[0x00..0x04].copy_from_slice(&0x00FF_FFFEu32.to_be_bytes()); // SP
    rom[0x04..0x08].copy_from_slice(&0x0000_0200u32.to_be_bytes()); // PC

    // Write header fields (padded to their field sizes with spaces)
    write_field(&mut rom, 0x100, 16, system_type);
    write_field(&mut rom, 0x110, 16, "(C)SEGA 1991.JAN");
    write_field(&mut rom, 0x120, 48, domestic_title);
    write_field(&mut rom, 0x150, 48, overseas_title);
    write_field(&mut rom, 0x180, 14, serial);
    // Device support
    write_field(&mut rom, 0x190, 16, "J");
    // ROM start/end addresses
    rom[0x1A0..0x1A4].copy_from_slice(&0x0000_0000u32.to_be_bytes());
    rom[0x1A4..0x1A8].copy_from_slice(&0x0000_03FFu32.to_be_bytes()); // ROM end = 0x3FF
    // RAM start/end
    rom[0x1A8..0x1AC].copy_from_slice(&0x00FF_0000u32.to_be_bytes());
    rom[0x1AC..0x1B0].copy_from_slice(&0x00FF_FFFFu32.to_be_bytes());
    // Region codes at 0x1F0
    write_field(&mut rom, 0x1F0, 3, region_codes);

    // Fill ROM data area (0x0200..0x0400) with some data
    for i in 0x200..0x400 {
        rom[i] = (i & 0xFF) as u8;
    }

    // Compute and write checksum
    let mut sum: u16 = 0;
    let mut i = 0x200;
    while i + 1 < rom.len() {
        let word = u16::from_be_bytes([rom[i], rom[i + 1]]);
        sum = sum.wrapping_add(word);
        i += 2;
    }
    rom[0x18E..0x190].copy_from_slice(&sum.to_be_bytes());

    rom
}

/// Write a string into a fixed-size field, padding with spaces.
fn write_field(rom: &mut [u8], offset: usize, size: usize, value: &str) {
    let bytes = value.as_bytes();
    let len = bytes.len().min(size);
    rom[offset..offset + len].copy_from_slice(&bytes[..len]);
    for b in &mut rom[offset + len..offset + size] {
        *b = b' ';
    }
}

#[test]
fn test_can_handle_valid() {
    let rom = make_genesis_rom(
        "SEGA MEGA DRIVE",
        "TEST GAME",
        "TEST GAME",
        "GM 00001009-00",
        "JUE",
    );
    let analyzer = GenesisAnalyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_invalid() {
    let data = vec![0xFFu8; 0x200];
    let analyzer = GenesisAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

#[test]
fn test_can_handle_too_small() {
    let data = vec![0u8; 16];
    let analyzer = GenesisAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

#[test]
fn test_header_fields() {
    let rom = make_genesis_rom(
        "SEGA MEGA DRIVE",
        "SONIC THE HEDGEHOG",
        "SONIC THE HEDGEHOG",
        "GM 00001009-00",
        "JUE",
    );
    let analyzer = GenesisAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.internal_name.as_deref(), Some("SONIC THE HEDGEHOG"));
    assert_eq!(result.serial_number.as_deref(), Some("GM 00001009-00"));
    assert_eq!(result.extra.get("system_type").unwrap(), "SEGA MEGA DRIVE");
    assert_eq!(
        result.extra.get("overseas_title").unwrap(),
        "SONIC THE HEDGEHOG"
    );
    assert_eq!(
        result.platform.as_deref(),
        Some("Sega Genesis / Mega Drive")
    );
}

#[test]
fn test_region_decode_multi() {
    let rom = make_genesis_rom("SEGA GENESIS", "TEST", "TEST", "GM 00000000-00", "JUE");
    let analyzer = GenesisAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert!(result.regions.contains(&Region::Japan));
    assert!(result.regions.contains(&Region::Usa));
    assert!(result.regions.contains(&Region::Europe));
    assert_eq!(result.regions.len(), 3);
}

#[test]
fn test_region_decode_single() {
    let rom = make_genesis_rom("SEGA GENESIS", "TEST", "TEST", "GM 00000000-00", "U");
    let analyzer = GenesisAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.regions, vec![Region::Usa]);
}

#[test]
fn test_checksum_valid() {
    let rom = make_genesis_rom("SEGA MEGA DRIVE", "TEST", "TEST", "GM 00000000-00", "J");
    let analyzer = GenesisAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.extra.get("checksum_status:rom").unwrap(), "Valid");
}

#[test]
fn test_checksum_invalid() {
    let mut rom = make_genesis_rom("SEGA MEGA DRIVE", "TEST", "TEST", "GM 00000000-00", "J");
    // Corrupt the stored checksum
    rom[0x18E] = 0xFF;
    rom[0x18F] = 0xFF;

    let analyzer = GenesisAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    let status = result.extra.get("checksum_status:rom").unwrap();
    assert!(
        status.starts_with("Invalid"),
        "expected Invalid, got: {status}"
    );
}

#[test]
fn test_expected_size_exact() {
    let rom = make_genesis_rom("SEGA MEGA DRIVE", "TEST", "TEST", "GM 00000000-00", "J");
    let analyzer = GenesisAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    // ROM end = 0x3FF, file is exactly 0x400 — expected should match file
    assert_eq!(result.expected_size, Some(0x0400));
    assert_eq!(result.file_size, Some(0x0400));
}

#[test]
fn test_padded_rom_not_oversized() {
    let mut rom = make_genesis_rom("SEGA MEGA DRIVE", "TEST", "TEST", "GM 00000000-00", "J");
    // Pad to a larger power-of-2 size (simulates a real dump)
    rom.resize(0x80000, 0x00); // 512 KB
    let analyzer = GenesisAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    // Padded ROM: expected_size should equal file_size (no false oversized report)
    assert_eq!(result.file_size, Some(0x80000));
    assert_eq!(result.expected_size, result.file_size);
    // Checksum should still be valid (only covers 0x0200..=0x03FF)
    assert_eq!(result.extra.get("checksum_status:rom").unwrap(), "Valid");
}

#[test]
fn test_too_small_rom() {
    let data = vec![0u8; 0x100]; // Too small for header
    let analyzer = GenesisAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(data), &options);
    assert!(result.is_err());
}

#[test]
fn test_address_ranges() {
    let rom = make_genesis_rom("SEGA MEGA DRIVE", "TEST", "TEST", "GM 00000000-00", "J");
    let analyzer = GenesisAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(
        result.extra.get("rom_address_range").unwrap(),
        "0x00000000-0x000003FF"
    );
    assert_eq!(
        result.extra.get("ram_address_range").unwrap(),
        "0x00FF0000-0x00FFFFFF"
    );
}
