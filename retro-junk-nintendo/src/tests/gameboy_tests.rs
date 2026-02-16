use super::*;
use std::io::Cursor;

/// Build a minimal synthetic GB ROM with a valid Nintendo logo and the given overrides.
/// Returns a 0x8000-byte (32 KB) buffer - the minimum ROM size (code 0x00).
fn make_gb_rom() -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000]; // 32 KB

    // Entry point: NOP + JP 0x0150
    rom[0x0100] = 0x00; // NOP
    rom[0x0101] = 0xC3; // JP
    rom[0x0102] = 0x50; // low byte
    rom[0x0103] = 0x01; // high byte

    // Nintendo logo
    rom[0x0104..0x0134].copy_from_slice(&NINTENDO_LOGO);

    // Title: "TESTGAME" (padded with zeros)
    let title = b"TESTGAME";
    rom[0x0134..0x0134 + title.len()].copy_from_slice(title);

    // CGB flag: 0x00 (DMG only)
    rom[0x0143] = 0x00;
    // SGB flag: 0x00 (no SGB)
    rom[0x0146] = 0x00;
    // Cartridge type: 0x00 (ROM ONLY)
    rom[0x0147] = 0x00;
    // ROM size: 0x00 (32 KB)
    rom[0x0148] = 0x00;
    // RAM size: 0x00 (none)
    rom[0x0149] = 0x00;
    // Destination: 0x01 (International)
    rom[0x014A] = 0x01;
    // Old licensee: 0x01 (Nintendo)
    rom[0x014B] = 0x01;
    // Version: 0x00
    rom[0x014C] = 0x00;

    // Compute and set header checksum
    let mut cksum: u8 = 0;
    for &b in &rom[0x0134..=0x014C] {
        cksum = cksum.wrapping_sub(b).wrapping_sub(1);
    }
    rom[0x014D] = cksum;

    // Compute and set global checksum
    let mut global: u16 = 0;
    for (i, &b) in rom.iter().enumerate() {
        if i != 0x014E && i != 0x014F {
            global = global.wrapping_add(b as u16);
        }
    }
    rom[0x014E] = (global >> 8) as u8;
    rom[0x014F] = (global & 0xFF) as u8;

    rom
}

#[test]
fn test_can_handle_valid() {
    let rom = make_gb_rom();
    let analyzer = GameBoyAnalyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_too_small() {
    let data = vec![0u8; 0x0100]; // Too small
    let analyzer = GameBoyAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

#[test]
fn test_can_handle_bad_logo() {
    let mut rom = make_gb_rom();
    rom[0x0104] = 0xFF; // Corrupt logo
    let analyzer = GameBoyAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_basic_analysis() {
    let rom = make_gb_rom();
    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.internal_name.as_deref(), Some("TESTGAME"));
    assert_eq!(result.platform.as_deref(), Some("Game Boy"));
    assert_eq!(result.version.as_deref(), Some("v0"));
    assert_eq!(result.maker_code.as_deref(), Some("Nintendo"));
    assert_eq!(result.file_size, Some(0x8000));
    assert_eq!(result.expected_size, Some(0x8000));
    assert_eq!(result.regions, vec![Region::World]);
    assert_eq!(result.extra.get("format").unwrap(), "Game Boy");
    assert_eq!(result.extra.get("cartridge_type").unwrap(), "ROM ONLY");
    assert_eq!(result.extra.get("checksum_status:GB Header").unwrap(), "OK");
    assert_eq!(result.extra.get("checksum_status:GB Global").unwrap(), "OK");
}

#[test]
fn test_cgb_compatible() {
    let mut rom = make_gb_rom();
    rom[0x0143] = 0x80; // CGB Compatible
    // Title is now only 11 bytes when CGB flag is set
    // Recompute checksums
    recompute_checksums(&mut rom);

    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.platform.as_deref(), Some("Game Boy Color"));
    assert_eq!(
        result.extra.get("format").unwrap(),
        "Game Boy Color (Compatible)"
    );
}

#[test]
fn test_cgb_exclusive() {
    let mut rom = make_gb_rom();
    rom[0x0143] = 0xC0; // CGB Only
    recompute_checksums(&mut rom);

    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.platform.as_deref(), Some("Game Boy Color"));
    assert_eq!(
        result.extra.get("format").unwrap(),
        "Game Boy Color (Exclusive)"
    );
}

#[test]
fn test_sgb_flag() {
    let mut rom = make_gb_rom();
    rom[0x0146] = 0x03; // SGB features
    recompute_checksums(&mut rom);

    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.extra.get("sgb").unwrap(), "Yes");
}

#[test]
fn test_japan_region() {
    let mut rom = make_gb_rom();
    rom[0x014A] = 0x00; // Japan
    recompute_checksums(&mut rom);

    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.regions, vec![Region::Japan]);
}

#[test]
fn test_mbc_cartridge_types() {
    assert_eq!(cartridge_type_name(0x00), "ROM ONLY");
    assert_eq!(cartridge_type_name(0x01), "MBC1");
    assert_eq!(cartridge_type_name(0x03), "MBC1+RAM+BATTERY");
    assert_eq!(cartridge_type_name(0x13), "MBC3+RAM+BATTERY");
    assert_eq!(cartridge_type_name(0x1B), "MBC5+RAM+BATTERY");
    assert_eq!(cartridge_type_name(0x22), "MBC7+SENSOR+RUMBLE+RAM+BATTERY");
    assert_eq!(cartridge_type_name(0xFE), "HuC3");
    assert_eq!(cartridge_type_name(0x04), "Unknown");
}

#[test]
fn test_rom_size_lookup() {
    assert_eq!(rom_size(0x00), Some(32 * 1024)); // 32 KB
    assert_eq!(rom_size(0x01), Some(64 * 1024)); // 64 KB
    assert_eq!(rom_size(0x02), Some(128 * 1024)); // 128 KB
    assert_eq!(rom_size(0x03), Some(256 * 1024)); // 256 KB
    assert_eq!(rom_size(0x04), Some(512 * 1024)); // 512 KB
    assert_eq!(rom_size(0x05), Some(1024 * 1024)); // 1 MB
    assert_eq!(rom_size(0x06), Some(2 * 1024 * 1024)); // 2 MB
    assert_eq!(rom_size(0x07), Some(4 * 1024 * 1024)); // 4 MB
    assert_eq!(rom_size(0x08), Some(8 * 1024 * 1024)); // 8 MB
    assert_eq!(rom_size(0x09), None); // Invalid
    assert_eq!(rom_size(0xFF), None); // Invalid
}

#[test]
fn test_ram_size_lookup() {
    assert_eq!(ram_size(0x00), Some(0));
    assert_eq!(ram_size(0x01), Some(0)); // Unused
    assert_eq!(ram_size(0x02), Some(8 * 1024)); // 8 KB
    assert_eq!(ram_size(0x03), Some(32 * 1024)); // 32 KB
    assert_eq!(ram_size(0x04), Some(128 * 1024)); // 128 KB
    assert_eq!(ram_size(0x05), Some(64 * 1024)); // 64 KB
    assert_eq!(ram_size(0x06), None); // Invalid
}

#[test]
fn test_header_checksum_correct() {
    let rom = make_gb_rom();
    let mut cursor = Cursor::new(&rom);
    let computed = compute_header_checksum(&mut cursor).unwrap();
    assert_eq!(computed, rom[0x014D]);
}

#[test]
fn test_header_checksum_mismatch() {
    let mut rom = make_gb_rom();
    rom[0x014D] = rom[0x014D].wrapping_add(1); // Corrupt header checksum
    // Don't recompute global (it would fix it)

    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    let status = result.extra.get("checksum_status:GB Header").unwrap();
    assert!(
        status.starts_with("MISMATCH"),
        "Expected MISMATCH, got: {}",
        status
    );
}

#[test]
fn test_global_checksum_correct() {
    let rom = make_gb_rom();
    let mut cursor = Cursor::new(&rom);
    let computed = compute_global_checksum(&mut cursor).unwrap();
    let expected = u16::from_be_bytes([rom[0x014E], rom[0x014F]]);
    assert_eq!(computed, expected);
}

#[test]
fn test_global_checksum_mismatch() {
    let mut rom = make_gb_rom();
    rom[0x014E] = 0xFF; // Corrupt global checksum
    rom[0x014F] = 0xFF;

    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    let status = result.extra.get("checksum_status:GB Global").unwrap();
    assert!(
        status.starts_with("MISMATCH"),
        "Expected MISMATCH, got: {}",
        status
    );
}

#[test]
fn test_new_licensee_code() {
    let mut rom = make_gb_rom();
    rom[0x014B] = 0x33; // Use new licensee code
    rom[0x0144] = b'0';
    rom[0x0145] = b'1'; // "01" = Nintendo R&D1
    recompute_checksums(&mut rom);

    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.maker_code.as_deref(), Some("Nintendo R&D1"));
}

#[test]
fn test_title_with_cgb_flag() {
    let mut rom = make_gb_rom();
    // Set CGB flag, title should be truncated to 11 bytes
    rom[0x0143] = 0x80;
    // Title bytes are at 0x0134..0x013F (11 bytes)
    let title = b"SHORTNAME\0\0";
    rom[0x0134..0x0134 + 11].copy_from_slice(title);
    recompute_checksums(&mut rom);

    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.internal_name.as_deref(), Some("SHORTNAME"));
}

#[test]
fn test_title_full_16_chars() {
    let mut rom = make_gb_rom();
    rom[0x0143] = 0x00; // DMG only - full 16-byte title
    let title = b"ABCDEFGHIJKLMNOP";
    rom[0x0134..0x0134 + 16].copy_from_slice(title);
    recompute_checksums(&mut rom);

    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.internal_name.as_deref(), Some("ABCDEFGHIJKLMNOP"));
}

#[test]
fn test_size_mismatch_truncated() {
    // Make a ROM that claims to be 64 KB but is only 32 KB
    let mut rom = make_gb_rom();
    rom[0x0148] = 0x01; // 64 KB
    recompute_checksums(&mut rom);

    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.file_size, Some(0x8000)); // 32 KB actual
    assert_eq!(result.expected_size, Some(0x10000)); // 64 KB expected
}

#[test]
fn test_too_small_file() {
    let data = vec![0u8; 0x0100]; // Not enough for header
    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(data), &options);
    assert!(result.is_err());
}

#[test]
fn test_cartridge_with_ram() {
    let mut rom = make_gb_rom();
    rom[0x0147] = 0x03; // MBC1+RAM+BATTERY
    rom[0x0149] = 0x03; // 32 KB RAM
    recompute_checksums(&mut rom);

    let analyzer = GameBoyAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(
        result.extra.get("cartridge_type").unwrap(),
        "MBC1+RAM+BATTERY"
    );
    assert_eq!(result.extra.get("ram_size").unwrap(), "32 KB");
}

#[test]
fn test_detect_cgb_mode() {
    assert_eq!(detect_cgb_mode(0x00), None);
    assert_eq!(detect_cgb_mode(0x80), Some("CGB Compatible"));
    assert_eq!(detect_cgb_mode(0xC0), Some("CGB Only"));
    assert_eq!(detect_cgb_mode(0x42), None);
}

/// Helper to recompute both checksums in a ROM buffer.
fn recompute_checksums(rom: &mut Vec<u8>) {
    // Header checksum
    let mut cksum: u8 = 0;
    for &b in &rom[0x0134..=0x014C] {
        cksum = cksum.wrapping_sub(b).wrapping_sub(1);
    }
    rom[0x014D] = cksum;

    // Global checksum
    rom[0x014E] = 0;
    rom[0x014F] = 0;
    let mut global: u16 = 0;
    for &b in rom.iter() {
        global = global.wrapping_add(b as u16);
    }
    rom[0x014E] = (global >> 8) as u8;
    rom[0x014F] = (global & 0xFF) as u8;
}
