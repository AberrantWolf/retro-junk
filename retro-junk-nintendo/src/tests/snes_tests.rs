use super::*;
use std::io::Cursor;

/// Build a synthetic 256 KB LoROM with a valid header and checksums.
fn make_snes_rom() -> Vec<u8> {
    let size: usize = 256 * 1024; // 256 KB
    let mut rom = vec![0u8; size];

    let base = LOROM_HEADER_BASE as usize;

    // Title: "TEST ROM" padded with spaces
    let title = b"TEST ROM             ";
    rom[base + OFF_TITLE..base + OFF_TITLE + 21].copy_from_slice(title);

    // Map mode: LoROM, SlowROM
    rom[base + OFF_MAP_MODE] = 0x20;

    // ROM type: ROM only
    rom[base + OFF_ROM_TYPE] = 0x00;

    // ROM size: 2^18 = 256 KB, code = 0x09 (2^9 * 1024 = 512 KB... wait)
    // Actually: 1 << code KB. For 256 KB: 256 = 1 << 8, but header stores it as
    // the power: so for 256 KB we want code such that (1 << code) * 1024 = 256*1024
    // That means 1 << code = 256, code = 8.
    rom[base + OFF_ROM_SIZE] = 0x08;

    // RAM size: 0 (no SRAM)
    rom[base + OFF_RAM_SIZE] = 0x00;

    // Country: USA
    rom[base + OFF_COUNTRY] = 0x01;

    // Developer ID: Nintendo (0x01)
    rom[base + OFF_DEVELOPER_ID] = 0x01;

    // Version: 0
    rom[base + OFF_VERSION] = 0x00;

    // Compute and set checksums
    recompute_snes_checksums(&mut rom, base);

    rom
}

/// Build a synthetic 1 MB HiROM with a valid header and checksums.
fn make_snes_hirom() -> Vec<u8> {
    let size: usize = 1024 * 1024; // 1 MB
    let mut rom = vec![0u8; size];

    let base = HIROM_HEADER_BASE as usize;

    // Title
    let title = b"HIROM TEST           ";
    rom[base + OFF_TITLE..base + OFF_TITLE + 21].copy_from_slice(title);

    // Map mode: HiROM, SlowROM
    rom[base + OFF_MAP_MODE] = 0x21;

    // ROM type: ROM + RAM + Battery
    rom[base + OFF_ROM_TYPE] = 0x02;

    // ROM size: 1 MB = 1 << 10 * 1024, code = 0x0A
    rom[base + OFF_ROM_SIZE] = 0x0A;

    // RAM size: 8 KB = 1 << 3 * 1024, code = 0x03
    rom[base + OFF_RAM_SIZE] = 0x03;

    // Country: Japan
    rom[base + OFF_COUNTRY] = 0x00;

    // Developer ID: Square (0xC3)
    rom[base + OFF_DEVELOPER_ID] = 0xC3;

    // Version: 1
    rom[base + OFF_VERSION] = 0x01;

    recompute_snes_checksums(&mut rom, base);

    rom
}

/// Prepend a 512-byte copier header (all zeros) to a ROM.
fn add_copier_header(rom: &[u8]) -> Vec<u8> {
    let mut result = vec![0u8; COPIER_HEADER_SIZE as usize];
    result.extend_from_slice(rom);
    result
}

/// Recompute the SNES checksum and complement for a ROM in memory.
fn recompute_snes_checksums(rom: &mut [u8], header_base: usize) {
    // Initialize: complement = 0xFFFF, checksum = 0x0000
    // These 4 bytes contribute 0x01FE to the sum, which is invariant --
    // no matter what checksum/complement pair we write later (as long as
    // they sum to 0xFFFF), the byte contribution stays 0x01FE.
    rom[header_base + OFF_COMPLEMENT] = 0xFF;
    rom[header_base + OFF_COMPLEMENT + 1] = 0xFF;
    rom[header_base + OFF_CHECKSUM] = 0;
    rom[header_base + OFF_CHECKSUM + 1] = 0;

    // Compute wrapping 16-bit sum
    let rom_size = rom.len() as u64;
    let mut power = 1u64;
    while power * 2 <= rom_size {
        power *= 2;
    }

    let mut sum: u16 = 0;
    if power == rom_size {
        for &byte in rom.iter() {
            sum = sum.wrapping_add(byte as u16);
        }
    } else {
        let base_data = &rom[..power as usize];
        let remainder = &rom[power as usize..];
        let remainder_len = remainder.len();

        for &byte in base_data {
            sum = sum.wrapping_add(byte as u16);
        }
        let mirror_total = power as usize;
        for i in 0..mirror_total {
            sum = sum.wrapping_add(remainder[i % remainder_len] as u16);
        }
    }

    let complement = sum ^ 0xFFFF;

    rom[header_base + OFF_COMPLEMENT] = complement as u8;
    rom[header_base + OFF_COMPLEMENT + 1] = (complement >> 8) as u8;
    rom[header_base + OFF_CHECKSUM] = sum as u8;
    rom[header_base + OFF_CHECKSUM + 1] = (sum >> 8) as u8;
}

// -- can_handle tests --

#[test]
fn test_can_handle_lorom() {
    let rom = make_snes_rom();
    let analyzer = SnesAnalyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_hirom() {
    let rom = make_snes_hirom();
    let analyzer = SnesAnalyzer::new();
    assert!(analyzer.can_handle(&mut Cursor::new(rom)));
}

#[test]
fn test_can_handle_too_small() {
    let data = vec![0u8; 100];
    let analyzer = SnesAnalyzer::new();
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

#[test]
fn test_can_handle_garbage() {
    // Random-ish garbage that's large enough but has no valid header
    let mut data = vec![0xFFu8; 256 * 1024];
    // Ensure checksum fields are garbage (don't accidentally sum to 0xFFFF)
    data[LOROM_HEADER_BASE as usize + OFF_COMPLEMENT] = 0xDE;
    data[LOROM_HEADER_BASE as usize + OFF_CHECKSUM] = 0xAD;
    data[HIROM_HEADER_BASE as usize + OFF_COMPLEMENT] = 0xBE;
    data[HIROM_HEADER_BASE as usize + OFF_CHECKSUM] = 0xEF;
    let analyzer = SnesAnalyzer::new();
    // With all 0xFF bytes, the title check fails (0xFF is not valid ASCII)
    // and mapping checks fail -- should not be detected
    assert!(!analyzer.can_handle(&mut Cursor::new(data)));
}

// -- Basic analysis tests --

#[test]
fn test_analyze_lorom() {
    let rom = make_snes_rom();
    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.platform, Some(Platform::Snes));
    assert_eq!(result.internal_name.as_deref(), Some("TEST ROM"));
    assert_eq!(result.extra.get("mapping").unwrap(), "LoROM");
    assert_eq!(result.extra.get("format").unwrap(), "SFC (headerless)");
    assert_eq!(result.extra.get("country").unwrap(), "USA");
    assert_eq!(result.regions, vec![Region::Usa]);
    assert_eq!(result.version.as_deref(), Some("1.0"));
    assert_eq!(result.file_size, Some(256 * 1024));
}

#[test]
fn test_analyze_hirom() {
    let rom = make_snes_hirom();
    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.internal_name.as_deref(), Some("HIROM TEST"));
    assert_eq!(result.extra.get("mapping").unwrap(), "HiROM");
    assert_eq!(result.extra.get("chipset").unwrap(), "ROM + RAM + Battery");
    assert_eq!(result.extra.get("country").unwrap(), "Japan");
    assert_eq!(result.regions, vec![Region::Japan]);
    assert_eq!(result.version.as_deref(), Some("1.1"));
    assert!(result.extra.get("sram_size").is_some());
}

#[test]
fn test_analyze_with_copier_header() {
    let rom = make_snes_rom();
    let rom_with_copier = add_copier_header(&rom);
    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer
        .analyze(&mut Cursor::new(rom_with_copier), &options)
        .unwrap();

    assert_eq!(result.internal_name.as_deref(), Some("TEST ROM"));
    assert_eq!(result.extra.get("format").unwrap(), "SMC (copier header)");
    assert_eq!(result.extra.get("copier_header").unwrap(), "Yes");
}

#[test]
fn test_analyze_fastrom() {
    let mut rom = make_snes_rom();
    let base = LOROM_HEADER_BASE as usize;
    // Set FastROM bit
    rom[base + OFF_MAP_MODE] = 0x30; // LoROM + FastROM
    recompute_snes_checksums(&mut rom, base);

    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.extra.get("speed").unwrap(), "FastROM (3.58 MHz)");
}

#[test]
fn test_analyze_extended_header() {
    let mut rom = make_snes_rom();
    let base = LOROM_HEADER_BASE as usize;

    // Set developer_id to 0x33 to enable extended header
    rom[base + OFF_DEVELOPER_ID] = 0x33;

    // Set maker code "01" (Nintendo)
    rom[base + OFF_EXT_MAKER_CODE] = b'0';
    rom[base + OFF_EXT_MAKER_CODE + 1] = b'1';

    // Set game code "ABCD"
    rom[base + OFF_EXT_GAME_CODE] = b'A';
    rom[base + OFF_EXT_GAME_CODE + 1] = b'B';
    rom[base + OFF_EXT_GAME_CODE + 2] = b'C';
    rom[base + OFF_EXT_GAME_CODE + 3] = b'D';

    recompute_snes_checksums(&mut rom, base);

    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.serial_number.as_deref(), Some("ABCD"));
    assert_eq!(result.extra.get("game_code").unwrap(), "ABCD");
    assert_eq!(result.extra.get("maker_code_raw").unwrap(), "01");
    assert_eq!(result.maker_code.as_deref(), Some("01 (Nintendo R&D1)"));
}

// -- Checksum tests --

#[test]
fn test_checksum_valid() {
    let rom = make_snes_rom();
    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(
        result.extra.get("checksum_complement_valid").unwrap(),
        "Yes"
    );
    assert_eq!(
        result.extra.get("checksum_status:SNES Internal").unwrap(),
        "OK"
    );
}

#[test]
fn test_checksum_mismatch() {
    let mut rom = make_snes_rom();
    // Corrupt a byte outside the header to change the actual checksum
    rom[0] = 0xFF;

    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    let status = result.extra.get("checksum_status:SNES Internal").unwrap();
    assert!(status.starts_with("MISMATCH"));
}

#[test]
fn test_checksum_complement() {
    let rom = make_snes_rom();
    let base = LOROM_HEADER_BASE as usize;

    let complement =
        u16::from_le_bytes([rom[base + OFF_COMPLEMENT], rom[base + OFF_COMPLEMENT + 1]]);
    let checksum = u16::from_le_bytes([rom[base + OFF_CHECKSUM], rom[base + OFF_CHECKSUM + 1]]);
    assert_eq!(checksum.wrapping_add(complement), 0xFFFF);
}

#[test]
fn test_quick_mode_skips_checksum() {
    let rom = make_snes_rom();
    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::new().quick(true);
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    // In quick mode, no checksum_status field should be present
    assert!(!result.extra.contains_key("checksum_status:SNES Internal"));
}

#[test]
fn test_country_to_region_mapping() {
    assert_eq!(country_to_region(0x00), Region::Japan);
    assert_eq!(country_to_region(0x01), Region::Usa);
    assert_eq!(country_to_region(0x02), Region::Europe);
    assert_eq!(country_to_region(0x0B), Region::China);
    assert_eq!(country_to_region(0x0D), Region::Korea);
    assert_eq!(country_to_region(0x0E), Region::World);
    assert_eq!(country_to_region(0x10), Region::Brazil);
    assert_eq!(country_to_region(0x11), Region::Australia);
}

// -- Metadata tests --

#[test]
fn test_sram_detection() {
    let rom = make_snes_hirom();
    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.extra.get("sram_size").unwrap(), "8 KB");
}

#[test]
fn test_version_number() {
    let mut rom = make_snes_rom();
    let base = LOROM_HEADER_BASE as usize;
    rom[base + OFF_VERSION] = 3;
    recompute_snes_checksums(&mut rom, base);

    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.version.as_deref(), Some("1.3"));
}

#[test]
fn test_game_code_extraction() {
    let mut rom = make_snes_rom();
    let base = LOROM_HEADER_BASE as usize;

    rom[base + OFF_DEVELOPER_ID] = 0x33;
    rom[base + OFF_EXT_GAME_CODE] = b'S';
    rom[base + OFF_EXT_GAME_CODE + 1] = b'M';
    rom[base + OFF_EXT_GAME_CODE + 2] = b'W';
    rom[base + OFF_EXT_GAME_CODE + 3] = b'J';
    recompute_snes_checksums(&mut rom, base);

    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.serial_number.as_deref(), Some("SMWJ"));
    assert_eq!(result.extra.get("game_code").unwrap(), "SMWJ");
}

// -- Edge case tests --

#[test]
fn test_scoring_prefers_correct_mapping() {
    // For a LoROM, the LoROM offset should score higher than HiROM offset
    let rom = make_snes_rom();
    let lo_score = score_header_at(&mut Cursor::new(&rom), LOROM_HEADER_BASE);
    let hi_score = score_header_at(&mut Cursor::new(&rom), HIROM_HEADER_BASE);
    assert!(
        lo_score > hi_score,
        "LoROM score ({}) should be higher than HiROM score ({})",
        lo_score,
        hi_score
    );
}

#[test]
fn test_copier_header_detection() {
    assert!(detect_copier_header(256 * 1024 + 512));
    assert!(!detect_copier_header(256 * 1024));
    assert!(detect_copier_header(512 * 1024 + 512));
    assert!(!detect_copier_header(512 * 1024));
}

#[test]
fn test_detect_mapping_lorom() {
    let rom = make_snes_rom();
    let (offset, has_copier) = detect_mapping(&mut Cursor::new(&rom), rom.len() as u64).unwrap();
    assert_eq!(offset, LOROM_HEADER_BASE);
    assert!(!has_copier);
}

#[test]
fn test_detect_mapping_hirom() {
    let rom = make_snes_hirom();
    let (offset, has_copier) = detect_mapping(&mut Cursor::new(&rom), rom.len() as u64).unwrap();
    assert_eq!(offset, HIROM_HEADER_BASE);
    assert!(!has_copier);
}

#[test]
fn test_detect_mapping_with_copier() {
    let rom = make_snes_rom();
    let rom_with_copier = add_copier_header(&rom);
    let (offset, has_copier) = detect_mapping(
        &mut Cursor::new(&rom_with_copier),
        rom_with_copier.len() as u64,
    )
    .unwrap();
    assert_eq!(offset, COPIER_HEADER_SIZE + LOROM_HEADER_BASE);
    assert!(has_copier);
}

#[test]
fn test_expected_checksums_present() {
    let rom = make_snes_rom();
    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(result.expected_checksums.len(), 1);
    assert_eq!(
        result.expected_checksums[0].algorithm,
        ChecksumAlgorithm::PlatformSpecific("SNES Internal".to_string())
    );
}

#[test]
fn test_hirom_checksum_valid() {
    let rom = make_snes_hirom();
    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer.analyze(&mut Cursor::new(rom), &options).unwrap();

    assert_eq!(
        result.extra.get("checksum_status:SNES Internal").unwrap(),
        "OK"
    );
}

#[test]
fn test_copier_header_checksum_valid() {
    let rom = make_snes_rom();
    let rom_with_copier = add_copier_header(&rom);
    let analyzer = SnesAnalyzer::new();
    let options = AnalysisOptions::default();
    let result = analyzer
        .analyze(&mut Cursor::new(rom_with_copier), &options)
        .unwrap();

    assert_eq!(
        result.extra.get("checksum_status:SNES Internal").unwrap(),
        "OK"
    );
}
