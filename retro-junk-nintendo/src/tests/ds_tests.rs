use super::*;
use std::io::Cursor;

/// Build a synthetic NDS ROM with a valid header and decrypted secure area.
/// Size is 0x10000 (64 KB) to include the secure area at 0x4000–0x7FFF.
fn make_nds_rom() -> Vec<u8> {
    let size: usize = 0x10000; // 64 KB, large enough for secure area
    let mut rom = vec![0u8; size];

    // Title at 0x000: "TESTGAME" (12 bytes, null-padded)
    rom[0x000..0x00C].copy_from_slice(b"TESTGAME\0\0\0\0");

    // Game code at 0x00C: "ADME" (A=NDS, DM=game id, E=USA)
    rom[0x00C..0x010].copy_from_slice(b"ADME");

    // Maker code at 0x010: "01" (Nintendo R&D1)
    rom[0x010..0x012].copy_from_slice(b"01");

    // Unit code at 0x012: NDS only
    rom[0x012] = 0x00;

    // Device capacity at 0x014: 0 = 128 KB
    rom[0x014] = 0x00;

    // NDS region at 0x01D: normal
    rom[0x01D] = 0x00;

    // ROM version at 0x01E
    rom[0x01E] = 0x00;

    // ARM9 ROM offset at 0x020
    rom[0x020..0x024].copy_from_slice(&0x4000u32.to_le_bytes());
    // ARM9 size at 0x02C
    rom[0x02C..0x030].copy_from_slice(&0x1000u32.to_le_bytes());

    // ARM7 ROM offset at 0x030
    rom[0x030..0x034].copy_from_slice(&0x8000u32.to_le_bytes());
    // ARM7 size at 0x03C
    rom[0x03C..0x040].copy_from_slice(&0x800u32.to_le_bytes());

    // Icon/title offset at 0x068: no banner
    rom[0x068..0x06C].copy_from_slice(&0u32.to_le_bytes());

    // Total used ROM size at 0x080
    rom[0x080..0x084].copy_from_slice(&(size as u32).to_le_bytes());

    // ROM header size at 0x084 (always 0x4000)
    rom[0x084..0x088].copy_from_slice(&0x4000u32.to_le_bytes());

    // Nintendo logo at 0xC0
    rom[0xC0..0xC0 + 156].copy_from_slice(&NINTENDO_LOGO);

    // Logo checksum at 0x15C
    let logo_crc = crc16(&rom[0xC0..0x15C]);
    rom[0x15C..0x15E].copy_from_slice(&logo_crc.to_le_bytes());

    // Decrypted secure area magic at 0x4000 (standard for all dumps)
    rom[0x4000..0x4008].copy_from_slice(&DECRYPTED_SECURE_AREA_MAGIC);

    // Secure area CRC at 0x06C — in a real ROM this is over the encrypted
    // form, but we set it to zero since decrypted dumps can't verify it
    rom[0x06C..0x06E].copy_from_slice(&0u16.to_le_bytes());

    // Header checksum at 0x15E: CRC-16 of 0x000–0x15D
    recompute_header_checksum(&mut rom);

    rom
}

/// Recompute header CRC-16 for a ROM buffer.
fn recompute_header_checksum(rom: &mut [u8]) {
    let crc = crc16(&rom[0x000..0x15E]);
    rom[0x15E..0x160].copy_from_slice(&crc.to_le_bytes());
}

/// Set up an encrypted secure area with a valid CRC at 0x06C.
fn setup_encrypted_secure_area(rom: &mut Vec<u8>) {
    if rom.len() >= 0x8000 {
        // Write non-magic bytes at 0x4000 so it looks encrypted
        rom[0x4000..0x4008].copy_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]);
        // Compute CRC over 0x4000–0x7FFF and store at 0x06C
        let crc = crc16(&rom[0x4000..0x8000]);
        rom[0x06C..0x06E].copy_from_slice(&crc.to_le_bytes());
        recompute_header_checksum(rom);
    }
}

#[test]
fn test_crc16_known_value() {
    // The logo CRC should always be 0xCF56 for the standard Nintendo logo
    let crc = crc16(&NINTENDO_LOGO);
    assert_eq!(crc, EXPECTED_LOGO_CHECKSUM);
}

#[test]
fn test_can_handle_valid() {
    let rom = make_nds_rom();
    let analyzer = DsAnalyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_too_small() {
    let data = vec![0u8; 0x100]; // Too small
    let analyzer = DsAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

#[test]
fn test_can_handle_bad_logo() {
    let mut rom = make_nds_rom();
    rom[0xC0] = 0xFF; // Corrupt logo
    let analyzer = DsAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_bad_logo_checksum() {
    let mut rom = make_nds_rom();
    rom[0x15C] = 0x00; // Wrong logo checksum
    rom[0x15D] = 0x00;
    let analyzer = DsAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_basic_analysis() {
    let rom = make_nds_rom();
    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.internal_name.as_deref(), Some("TESTGAME"));
    assert_eq!(result.platform.as_deref(), Some("Nintendo DS"));
    assert_eq!(result.serial_number.as_deref(), Some("NTR-ADME"));
    assert_eq!(result.maker_code.as_deref(), Some("Nintendo R&D1"));
    assert_eq!(result.version.as_deref(), Some("v0"));
    assert_eq!(result.file_size, Some(0x10000));
    assert_eq!(result.expected_size, Some(0x10000)); // file == used_rom_size → OK
    assert_eq!(result.regions, vec![Region::Usa]);
    assert_eq!(result.extra.get("game_code").unwrap(), "ADME");
    assert_eq!(result.extra.get("unit_code").unwrap(), "NDS");
}

#[test]
fn test_checksums_ok() {
    let rom = make_nds_rom();
    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(
        result.extra.get("checksum_status:Logo CRC-16").unwrap(),
        "OK"
    );
    assert_eq!(
        result.extra.get("checksum_status:Header CRC-16").unwrap(),
        "OK"
    );
    // Default test ROM has decrypted secure area
    let sa_status = result
        .extra
        .get("checksum_status:Secure Area CRC-16")
        .unwrap();
    assert!(
        sa_status.starts_with("OK"),
        "Expected OK for decrypted dump, got: {}",
        sa_status
    );
    assert_eq!(result.extra.get("secure_area").unwrap(), "Decrypted");
}

#[test]
fn test_encrypted_secure_area_crc_ok() {
    let mut rom = make_nds_rom();
    setup_encrypted_secure_area(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(
        result
            .extra
            .get("checksum_status:Secure Area CRC-16")
            .unwrap(),
        "OK"
    );
    assert_eq!(result.extra.get("secure_area").unwrap(), "Encrypted");
}

#[test]
fn test_encrypted_secure_area_crc_mismatch() {
    let mut rom = make_nds_rom();
    setup_encrypted_secure_area(&mut rom);
    // Corrupt a byte in the secure area
    rom[0x5000] = 0xFF;

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    let status = result
        .extra
        .get("checksum_status:Secure Area CRC-16")
        .unwrap();
    assert!(
        status.starts_with("MISMATCH"),
        "Expected MISMATCH, got: {}",
        status
    );
}

#[test]
fn test_header_checksum_mismatch() {
    let mut rom = make_nds_rom();
    // Corrupt a byte in the header region without recomputing checksum
    rom[0x000] = b'X';

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    let status = result.extra.get("checksum_status:Header CRC-16").unwrap();
    assert!(
        status.starts_with("MISMATCH"),
        "Expected MISMATCH, got: {}",
        status
    );
}

#[test]
fn test_quick_mode_skips_secure_area() {
    let rom = make_nds_rom();
    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions { quick: true, ..Default::default() };
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    // Quick mode doesn't detect secure area state
    assert!(result.extra.get("secure_area").is_none());
}

#[test]
fn test_dsi_enhanced() {
    let mut rom = make_nds_rom();
    rom[0x012] = 0x02; // NDS+DSi
    rom[0x00C] = b'I'; // DSi-enhanced category prefix
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(
        result.platform.as_deref(),
        Some("Nintendo DS (DSi Enhanced)")
    );
    assert_eq!(result.extra.get("unit_code").unwrap(), "NDS+DSi");
    // DSi-enhanced gets TWL prefix
    assert!(result.serial_number.as_deref().unwrap().starts_with("TWL-"));
}

#[test]
fn test_dsi_only() {
    let mut rom = make_nds_rom();
    rom[0x012] = 0x03; // DSi only
    rom[0x00C] = b'D'; // DSi-exclusive category prefix
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.platform.as_deref(), Some("Nintendo DSi"));
    assert_eq!(result.extra.get("unit_code").unwrap(), "DSi");
    assert!(result.serial_number.as_deref().unwrap().starts_with("TWL-"));
}

#[test]
fn test_region_japan() {
    let mut rom = make_nds_rom();
    rom[0x00F] = b'J'; // Japan
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.regions, vec![Region::Japan]);
}

#[test]
fn test_region_europe() {
    let mut rom = make_nds_rom();
    rom[0x00F] = b'P'; // Europe/PAL
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.regions, vec![Region::Europe]);
}

#[test]
fn test_region_korea() {
    let mut rom = make_nds_rom();
    rom[0x00F] = b'K'; // Korea
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.regions, vec![Region::Korea]);
}

#[test]
fn test_region_world() {
    let mut rom = make_nds_rom();
    rom[0x00F] = b'W'; // Worldwide
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.regions, vec![Region::World]);
}

#[test]
fn test_region_australia() {
    let mut rom = make_nds_rom();
    rom[0x00F] = b'U'; // Australia → Europe/PAL
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.regions, vec![Region::Europe]);
}

#[test]
fn test_device_capacity() {
    let mut rom = make_nds_rom();
    rom[0x014] = 9; // 64 MB chip
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    // file_size == used_rom_size and both < chip_capacity → trimmed, OK
    assert_eq!(result.expected_size, Some(0x10000));
    assert_eq!(result.extra.get("dump_status").unwrap(), "Trimmed");
    assert_eq!(result.extra.get("cartridge_capacity").unwrap(), "64 MB");
}

#[test]
fn test_untrimmed_rom() {
    // File size == chip capacity → untrimmed
    let capacity: usize = 128 * 1024; // 128 KB = 128 KB << 0
    let mut rom = make_nds_rom();
    rom.resize(capacity, 0xFF); // pad to full capacity
    rom[0x014] = 0; // device capacity = 128 KB
    // used_rom_size stays at 0x10000 (64 KB), which is < capacity
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.file_size, Some(capacity as u64));
    assert_eq!(result.expected_size, Some(capacity as u64)); // OK, not oversized
    assert_eq!(result.extra.get("dump_status").unwrap(), "Untrimmed");
}

#[test]
fn test_trimmed_rom() {
    // File size == used_rom_size < chip capacity → trimmed
    let mut rom = make_nds_rom();
    rom[0x014] = 9; // 64 MB chip, much larger than 64 KB file
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.file_size, Some(0x10000));
    assert_eq!(result.expected_size, Some(0x10000)); // OK, not truncated
    assert_eq!(result.extra.get("dump_status").unwrap(), "Trimmed");
}

#[test]
fn test_partially_trimmed_rom() {
    // used_rom_size < file_size < chip capacity → partially trimmed
    let mut rom = make_nds_rom();
    rom[0x014] = 1; // 256 KB chip
    rom[0x080..0x084].copy_from_slice(&0x8000u32.to_le_bytes()); // used = 32 KB
    rom.resize(0xC000, 0xFF); // file = 48 KB (between 32 KB and 256 KB)
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.file_size, Some(0xC000));
    assert_eq!(result.expected_size, Some(0xC000)); // OK
    assert_eq!(
        result.extra.get("dump_status").unwrap(),
        "Partially trimmed"
    );
}

#[test]
fn test_actually_truncated_rom() {
    // file_size < used_rom_size → truly truncated
    let mut rom = make_nds_rom();
    rom[0x080..0x084].copy_from_slice(&0x20000u32.to_le_bytes()); // used = 128 KB
    recompute_header_checksum(&mut rom);
    // File is 64 KB but claims to need 128 KB

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions { quick: true, ..Default::default() };
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.file_size, Some(0x10000));
    assert_eq!(result.expected_size, Some(0x20000)); // shows TRUNCATED
}

#[test]
fn test_rom_version() {
    let mut rom = make_nds_rom();
    rom[0x01E] = 2;
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.version.as_deref(), Some("v2"));
}

#[test]
fn test_title_trimming() {
    let mut rom = make_nds_rom();
    rom[0x000..0x00C].copy_from_slice(b"HI\0\0\0\0\0\0\0\0\0\0");
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.internal_name.as_deref(), Some("HI"));
}

#[test]
fn test_nds_region_korea() {
    let mut rom = make_nds_rom();
    rom[0x01D] = 0x40; // Korea region lock
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.extra.get("nds_region_lock").unwrap(), "Korea");
}

#[test]
fn test_nds_region_china() {
    let mut rom = make_nds_rom();
    rom[0x01D] = 0x80; // China region lock
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.extra.get("nds_region_lock").unwrap(), "China");
}

#[test]
fn test_too_small_file() {
    let data = vec![0u8; 0x100]; // Not enough for header
    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(data), &options);
    assert!(result.is_err());
}

#[test]
fn test_expected_rom_size_calculation() {
    assert_eq!(expected_rom_size_from_capacity(0), 128 * 1024); // 128 KB
    assert_eq!(expected_rom_size_from_capacity(6), 8 * 1024 * 1024); // 8 MB
    assert_eq!(expected_rom_size_from_capacity(7), 16 * 1024 * 1024); // 16 MB
    assert_eq!(expected_rom_size_from_capacity(8), 32 * 1024 * 1024); // 32 MB
    assert_eq!(expected_rom_size_from_capacity(9), 64 * 1024 * 1024); // 64 MB
    assert_eq!(expected_rom_size_from_capacity(10), 128 * 1024 * 1024); // 128 MB
    assert_eq!(expected_rom_size_from_capacity(11), 256 * 1024 * 1024); // 256 MB
    assert_eq!(expected_rom_size_from_capacity(12), 512 * 1024 * 1024); // 512 MB
}

#[test]
fn test_region_from_game_code_function() {
    assert_eq!(region_from_game_code("ADMJ"), Some(Region::Japan));
    assert_eq!(region_from_game_code("ADME"), Some(Region::Usa));
    assert_eq!(region_from_game_code("ADMP"), Some(Region::Europe));
    assert_eq!(region_from_game_code("ADMK"), Some(Region::Korea));
    assert_eq!(region_from_game_code("ADMC"), Some(Region::China));
    assert_eq!(region_from_game_code("ADMU"), Some(Region::Europe)); // Australia → Europe
    assert_eq!(region_from_game_code("ADMA"), Some(Region::World)); // Region-free
    assert_eq!(region_from_game_code("ADMW"), Some(Region::World)); // Worldwide
    assert_eq!(region_from_game_code("ADM"), None); // Too short
}

#[test]
fn test_maker_code_lookup() {
    assert_eq!(maker_code_name("01"), Some("Nintendo R&D1"));
    assert_eq!(maker_code_name("08"), Some("Capcom"));
    assert_eq!(maker_code_name("34"), Some("Konami"));
    assert_eq!(maker_code_name("ZZ"), None);
}

#[test]
fn test_banner_offset_reported() {
    let mut rom = make_nds_rom();
    rom[0x068..0x06C].copy_from_slice(&0x8000u32.to_le_bytes());
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.extra.get("banner_offset").unwrap(), "0x00008000");
}

#[test]
fn test_serial_number_format_nds() {
    let rom = make_nds_rom();
    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert!(result.serial_number.as_deref().unwrap().starts_with("NTR-"));
}

#[test]
fn test_no_secure_area_for_small_file() {
    // File smaller than 0x8000 shouldn't attempt secure area detection
    let mut rom = make_nds_rom();
    rom[0x080..0x084].copy_from_slice(&0x2000u32.to_le_bytes()); // used = 8 KB
    recompute_header_checksum(&mut rom);
    rom.truncate(0x2000); // 8 KB, too small for secure area

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert!(result.extra.get("secure_area").is_none());
}

#[test]
fn test_homebrew_no_secure_area() {
    let mut rom = make_nds_rom();
    // Set arm9_rom_offset < 0x4000 → homebrew, no secure area
    rom[0x020..0x024].copy_from_slice(&0x0200u32.to_le_bytes());
    recompute_header_checksum(&mut rom);

    let analyzer = DsAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();
    assert_eq!(result.extra.get("secure_area").unwrap(), "None (homebrew)");
}
