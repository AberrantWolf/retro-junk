use super::*;
use sha2::{Digest, Sha256};
use std::io::Cursor;

use super::super::{NCCH_MAGIC, NCSD_MAGIC};

/// Build a minimal synthetic CCI (NCSD + NCCH partition 0).
/// The NCCH at partition 0 is marked NoCrypto with a valid ExHeader region
/// whose SHA-256 hash can be verified.
fn make_cci() -> Vec<u8> {
    let partition0_offset: u64 = 0x4000;
    let ncch_content_size_mu: u32 = 0x100; // 128 KB of NCCH content
    let total_size = partition0_offset + ncch_content_size_mu as u64 * MEDIA_UNIT;
    let mut rom = vec![0u8; total_size as usize];

    // -- NCSD header --

    // RSA signature: non-zero (simulating authentic card)
    rom[0x00] = 0xAB;
    rom[0x01] = 0xCD;

    // Magic "NCSD" at 0x100
    rom[0x100..0x104].copy_from_slice(&NCSD_MAGIC);

    // Image size in media units
    let image_size_mu = (total_size / MEDIA_UNIT) as u32;
    rom[0x104..0x108].copy_from_slice(&image_size_mu.to_le_bytes());

    // Media ID
    rom[0x108..0x110].copy_from_slice(&0x0004000000ABCDEF_u64.to_le_bytes());

    // Partition 0: offset=0x20 MU (0x4000 bytes), size=0x100 MU
    let p0_offset_mu = (partition0_offset / MEDIA_UNIT) as u32;
    rom[0x120..0x124].copy_from_slice(&p0_offset_mu.to_le_bytes());
    rom[0x124..0x128].copy_from_slice(&ncch_content_size_mu.to_le_bytes());

    // Partition flags: media_platform=1 (CTR), media_type=1 (Card1)
    rom[0x188 + 4] = 1; // platform = CTR
    rom[0x188 + 5] = 1; // media type = Card1

    // Card info: writable address = 0xFFFFFFFF (Card1)
    rom[0x200..0x204].copy_from_slice(&0xFFFFFFFF_u32.to_le_bytes());

    // Filled size at 0x300: less than total (simulates content that doesn't
    // fill the entire card image — typical for real game cards).
    let filled = (partition0_offset + ncch_content_size_mu as u64 * MEDIA_UNIT / 2) as u32;
    rom[0x300..0x304].copy_from_slice(&filled.to_le_bytes());

    // Title version at 0x310
    rom[0x310..0x312].copy_from_slice(&0x0000u16.to_le_bytes());

    // Card seed at 0x1000: non-zero (authentic card)
    rom[0x1000] = 0x42;
    rom[0x1001] = 0x37;

    // -- NCCH header at partition0_offset --
    let p0 = partition0_offset as usize;

    // Magic "NCCH" at partition + 0x100
    rom[p0 + 0x100..p0 + 0x104].copy_from_slice(&NCCH_MAGIC);

    // Content size
    rom[p0 + 0x104..p0 + 0x108].copy_from_slice(&ncch_content_size_mu.to_le_bytes());

    // Partition ID
    rom[p0 + 0x108..p0 + 0x110].copy_from_slice(&0x0004000000ABCDEF_u64.to_le_bytes());

    // Maker code "31" (Nintendo)
    rom[p0 + 0x110..p0 + 0x112].copy_from_slice(b"31");

    // NCCH version
    rom[p0 + 0x112..p0 + 0x114].copy_from_slice(&2u16.to_le_bytes());

    // Program ID
    rom[p0 + 0x118..p0 + 0x120].copy_from_slice(&0x0004000000ABCDEF_u64.to_le_bytes());

    // Product code: "CTR-P-ABCE" (USA)
    let product = b"CTR-P-ABCE\0\0\0\0\0\0";
    rom[p0 + 0x150..p0 + 0x160].copy_from_slice(product);

    // ExHeader size = 0x400
    rom[p0 + 0x180..p0 + 0x184].copy_from_slice(&0x400u32.to_le_bytes());

    // Flags: NoCrypto (flags[7] bit 2), platform=CTR (flags[4]=1), form=executable (flags[5]=3)
    rom[p0 + 0x188 + 3] = 0x00; // crypto method
    rom[p0 + 0x188 + 4] = 0x01; // platform CTR
    rom[p0 + 0x188 + 5] = 0x03; // executable with RomFS
    rom[p0 + 0x188 + 7] = 0x04; // NoCrypto

    // ExeFS: offset in MU from NCCH start, after ExHeader (0x200 header + 0x800 exheader = 0xA00)
    // ExeFS offset = 0xA00 / 0x200 = 5 MU
    // Size = 0x2000 / 0x200 = 16 MU
    rom[p0 + 0x1A0..p0 + 0x1A4].copy_from_slice(&5u32.to_le_bytes());
    rom[p0 + 0x1A4..p0 + 0x1A8].copy_from_slice(&16u32.to_le_bytes());
    rom[p0 + 0x1A8..p0 + 0x1AC].copy_from_slice(&1u32.to_le_bytes()); // hash region = 1 MU

    // Write some ExHeader data at partition + 0x200
    for i in 0..0x400 {
        rom[p0 + 0x200 + i] = (i & 0xFF) as u8;
    }

    // Compute ExHeader SHA-256 hash and store at partition + 0x160
    let exheader_hash = {
        let mut hasher = Sha256::new();
        hasher.update(&rom[p0 + 0x200..p0 + 0x200 + 0x400]);
        hasher.finalize()
    };
    rom[p0 + 0x160..p0 + 0x180].copy_from_slice(&exheader_hash);

    // Write some ExeFS data and compute superblock hash
    let exefs_start = p0 + 5 * MEDIA_UNIT as usize;
    for i in 0..MEDIA_UNIT as usize {
        if exefs_start + i < rom.len() {
            rom[exefs_start + i] = ((i * 3) & 0xFF) as u8;
        }
    }
    let exefs_hash = {
        let mut hasher = Sha256::new();
        let end = (exefs_start + MEDIA_UNIT as usize).min(rom.len());
        hasher.update(&rom[exefs_start..end]);
        hasher.finalize()
    };
    rom[p0 + 0x1C0..p0 + 0x1E0].copy_from_slice(&exefs_hash);

    rom
}

/// Modify a CCI to look like it was converted from a CIA.
fn make_cci_digital_origin() -> Vec<u8> {
    let mut rom = make_cci();

    // Zero the RSA signature
    for b in &mut rom[0x00..0x100] {
        *b = 0;
    }

    // Zero the card seed
    for b in &mut rom[0x1000..0x1010] {
        *b = 0;
    }

    // Set media type to Inner Device (0)
    rom[0x188 + 5] = 0;

    // Set writable address to 0
    rom[0x200..0x204].copy_from_slice(&0u32.to_le_bytes());

    rom
}

// -----------------------------------------------------------------------
// CCI (NCSD) tests
// -----------------------------------------------------------------------

#[test]
fn test_cci_basic_analysis() {
    let rom = make_cci();
    let file_size = rom.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(result.platform.as_deref(), Some("Nintendo 3DS"));
    assert_eq!(result.serial_number.as_deref(), Some("CTR-P-ABCE"));
    assert_eq!(result.maker_code.as_deref(), Some("Nintendo"));
    assert_eq!(result.regions, vec![retro_junk_core::Region::Usa]);
    assert_eq!(result.extra.get("format").unwrap(), "CCI (NCSD)");
    assert_eq!(result.extra.get("product_code").unwrap(), "CTR-P-ABCE");
    assert_eq!(result.extra.get("media_type").unwrap(), "Card1");
}

#[test]
fn test_cci_game_card_origin() {
    let rom = make_cci();
    let file_size = rom.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(
        result.extra.get("origin").unwrap(),
        "Game card dump (likely)"
    );
}

#[test]
fn test_cci_digital_origin() {
    let rom = make_cci_digital_origin();
    let file_size = rom.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(
        result.extra.get("origin").unwrap(),
        "Converted from digital/CIA (likely)"
    );
}

#[test]
fn test_cci_exheader_hash_ok() {
    let rom = make_cci();
    let file_size = rom.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(
        result
            .extra
            .get("checksum_status:ExHeader SHA-256")
            .unwrap(),
        "OK"
    );
}

#[test]
fn test_cci_exheader_hash_mismatch() {
    let mut rom = make_cci();
    // Corrupt ExHeader data
    let p0 = 0x4000;
    rom[p0 + 0x200] = 0xFF;

    let file_size = rom.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    let status = result
        .extra
        .get("checksum_status:ExHeader SHA-256")
        .unwrap();
    assert!(
        status.starts_with("MISMATCH"),
        "Expected MISMATCH, got: {}",
        status
    );
}

#[test]
fn test_cci_exefs_superblock_hash_ok() {
    let rom = make_cci();
    let file_size = rom.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(
        result
            .extra
            .get("checksum_status:ExeFS Superblock SHA-256")
            .unwrap(),
        "OK"
    );
}

#[test]
fn test_cci_quick_mode_skips_hashes() {
    let rom = make_cci();
    let file_size = rom.len() as u64;
    let options = AnalysisOptions {
        quick: true,
        ..Default::default()
    };
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert!(
        result
            .extra
            .get("checksum_status:ExHeader SHA-256")
            .is_none()
    );
    assert!(
        result
            .extra
            .get("checksum_status:ExeFS Superblock SHA-256")
            .is_none()
    );
}

#[test]
fn test_cci_file_size() {
    let rom = make_cci();
    let expected_size = rom.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cci(&mut Cursor::new(rom), expected_size, &options).unwrap();

    assert_eq!(result.file_size, Some(expected_size));
    assert_eq!(result.expected_size, Some(expected_size));
}

#[test]
fn test_cci_title_id() {
    let rom = make_cci();
    let file_size = rom.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(result.extra.get("title_id").unwrap(), "0004000000ABCDEF");
}

#[test]
fn test_cci_partition_count() {
    let rom = make_cci();
    let file_size = rom.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(result.extra.get("partition_count").unwrap(), "1");
}

#[test]
fn test_cci_encryption_nocrypto() {
    let rom = make_cci();
    let file_size = rom.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(result.extra.get("encryption").unwrap(), "None (NoCrypto)");
}

// -----------------------------------------------------------------------
// Origin detection unit tests
// -----------------------------------------------------------------------

#[test]
fn test_origin_detection_card() {
    let ncsd = NcsdHeader {
        image_size_mu: 0x100,
        media_id: 0x0004000000ABCDEF,
        partitions: [
            (0x20, 0x100),
            (0x120, 0x10),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
            (0x200, 0x20),
        ],
        media_type: 1, // Card1
        media_platform: 1,
        writable_address: 0xFFFFFFFF,
        title_version: 0,
        card_revision: 0,
        signature_is_zero: false,
        card_seed_is_zero: false,
        filled_size: 0,
    };
    assert_eq!(detect_cci_origin(&ncsd), CciOrigin::GameCard);
}

#[test]
fn test_origin_detection_digital() {
    let ncsd = NcsdHeader {
        image_size_mu: 0x100,
        media_id: 0x0004000000ABCDEF,
        partitions: [
            (0x20, 0x100),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
            (0, 0),
        ],
        media_type: 0, // Inner Device
        media_platform: 1,
        writable_address: 0,
        title_version: 0,
        card_revision: 0,
        signature_is_zero: true,
        card_seed_is_zero: true,
        filled_size: 0,
    };
    assert_eq!(detect_cci_origin(&ncsd), CciOrigin::Digital);
}

// -----------------------------------------------------------------------
// Trimming tests
// -----------------------------------------------------------------------

#[test]
fn test_cci_untrimmed() {
    // file_size == image_size, filled_size < image_size -> Untrimmed
    let rom = make_cci();
    let file_size = rom.len() as u64;
    let options = AnalysisOptions::default();
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(result.file_size, Some(file_size));
    assert_eq!(result.expected_size, Some(file_size));
    assert_eq!(result.extra.get("dump_status").unwrap(), "Untrimmed");
}

#[test]
fn test_cci_trimmed() {
    // Set filled_size to a value smaller than total, then truncate file to filled_size
    let mut rom = make_cci();
    let filled: u32 = 0x6000; // smaller than total
    rom[0x300..0x304].copy_from_slice(&filled.to_le_bytes());

    // Truncate ROM to filled_size
    rom.truncate(filled as usize);

    let file_size = rom.len() as u64;
    let options = AnalysisOptions {
        quick: true,
        ..Default::default()
    };
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(result.file_size, Some(file_size));
    assert_eq!(result.expected_size, Some(file_size)); // OK, not red
    assert_eq!(result.extra.get("dump_status").unwrap(), "Trimmed");
}

#[test]
fn test_cci_partially_trimmed() {
    // filled_size < file_size < image_size -> Partially trimmed
    let mut rom = make_cci();
    let filled: u32 = 0x6000;
    rom[0x300..0x304].copy_from_slice(&filled.to_le_bytes());

    // Truncate to something between filled and total
    let partial_size = 0x8000;
    rom.truncate(partial_size);

    let file_size = rom.len() as u64;
    let options = AnalysisOptions {
        quick: true,
        ..Default::default()
    };
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(result.file_size, Some(file_size));
    assert_eq!(result.expected_size, Some(file_size)); // OK
    assert_eq!(
        result.extra.get("dump_status").unwrap(),
        "Partially trimmed"
    );
}

#[test]
fn test_cci_genuinely_truncated() {
    // file_size < filled_size -> genuinely truncated, expected = filled_size
    let mut rom = make_cci();
    let total_size = rom.len() as u64;
    let filled: u32 = total_size as u32; // filled = full size
    rom[0x300..0x304].copy_from_slice(&filled.to_le_bytes());

    // Truncate below filled_size (but keep enough for headers)
    let truncated_size = 0x5000;
    rom.truncate(truncated_size);

    let file_size = rom.len() as u64;
    let options = AnalysisOptions {
        quick: true,
        ..Default::default()
    };
    let result = analyze_cci(&mut Cursor::new(rom), file_size, &options).unwrap();

    assert_eq!(result.file_size, Some(file_size));
    assert_eq!(result.expected_size, Some(filled as u64)); // genuinely truncated
    assert!(result.extra.get("dump_status").is_none()); // no status for truncated
}
