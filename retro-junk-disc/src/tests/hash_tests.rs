use crate::hash::*;
use crate::sector::{CD_SYNC_PATTERN, RAW_SECTOR_SIZE, SECTOR_USER_DATA_START};
use std::io::Cursor;

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
fn test_hash_raw_bin_track1() {
    let data_sectors = 20;
    let audio_sectors = 8;
    let bin = make_multi_track_bin(data_sectors, audio_sectors);

    let data_track_bytes = data_sectors * RAW_SECTOR_SIZE as usize;
    let (expected_crc, expected_sha1, expected_md5) = reference_hashes(&bin[..data_track_bytes]);

    let mut cursor = Cursor::new(bin);

    // First find the data track size
    let data_size = find_raw_bin_data_track_size(&mut cursor)
        .unwrap()
        .expect("Should detect multi-track boundary");

    let algorithms = retro_junk_core::HashAlgorithms::All;
    let hashes = hash_raw_bin_track1(&mut cursor, algorithms, data_size, None).unwrap();

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
fn test_compute_track1_size_two_tracks() {
    let cue = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
  TRACK 02 AUDIO
    INDEX 00 54:04:50
    INDEX 01 54:04:52
"#;
    let sheet = crate::cue::parse_cue(cue).unwrap();
    let bin_size = 615_612_480u64;
    let (size, warnings) = compute_track1_size_from_cue(&sheet, bin_size);
    assert_eq!(size, Some(243352 * 2352));
    assert_eq!(size, Some(572_363_904));
    assert!(warnings.is_empty());
}

#[test]
fn test_compute_track1_size_is_mode_aware() {
    // MODE2/2336 sectors: Track 1 boundary must use 2336, not the raw 2352.
    let cue = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2336
    INDEX 01 00:00:00
  TRACK 02 MODE2/2336
    INDEX 01 00:00:10
"#;
    let sheet = crate::cue::parse_cue(cue).unwrap();
    let bin_size = 10_000_000u64;
    let (size, warnings) = compute_track1_size_from_cue(&sheet, bin_size);
    assert_eq!(size, Some(10 * 2336));
    assert!(warnings.is_empty());
}

#[test]
fn test_compute_track1_size_single_track_returns_none() {
    let cue = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
"#;
    let sheet = crate::cue::parse_cue(cue).unwrap();
    let (size, warnings) = compute_track1_size_from_cue(&sheet, 615_612_480);
    assert!(size.is_none());
    assert!(warnings.is_empty());
}

#[test]
fn test_compute_track1_size_no_index_warns() {
    let cue = r#"FILE "game.bin" BINARY
  TRACK 01 MODE2/2352
  TRACK 02 AUDIO
"#;
    let sheet = crate::cue::parse_cue(cue).unwrap();
    let (size, warnings) = compute_track1_size_from_cue(&sheet, 615_612_480);
    assert!(size.is_none());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("no INDEX 01"));
}

/// Build a synthetic BIN with real data sectors followed by zero-padded
/// "audio" sectors (sync pattern + Mode 2 header + all-zero user data).
/// This simulates a bad dump where the audio track was written as empty
/// Mode 2 sectors.
fn make_zero_padded_bin(data_sectors: usize, audio_sectors: usize) -> Vec<u8> {
    let mut bin = Vec::with_capacity((data_sectors + audio_sectors) * RAW_SECTOR_SIZE as usize);

    // Data sectors: sync + Mode 2 + submode 0x08 + pseudo-random user data
    for i in 0..data_sectors {
        let mut sector = [0u8; RAW_SECTOR_SIZE as usize];
        sector[0..12].copy_from_slice(&CD_SYNC_PATTERN);
        sector[15] = 0x02; // Mode 2
        sector[18] = 0x08; // Submode: data flag
        for (j, byte) in sector[SECTOR_USER_DATA_START..2072].iter_mut().enumerate() {
            *byte = ((i * 251 + j * 97) & 0xFF) as u8;
        }
        bin.extend_from_slice(&sector);
    }

    // Audio sectors: sync + Mode 2 + submode 0x20 + ALL-ZERO user data
    for _i in 0..audio_sectors {
        let mut sector = [0u8; RAW_SECTOR_SIZE as usize];
        sector[0..12].copy_from_slice(&CD_SYNC_PATTERN);
        sector[15] = 0x02; // Mode 2
        sector[18] = 0x20; // Submode: Form 2, no data flag
        // User data bytes 24..2352 remain zero
        bin.extend_from_slice(&sector);
    }

    bin
}

#[test]
fn test_find_zero_padded_boundary() {
    let data_sectors = 100;
    let audio_sectors = 20;
    let bin = make_zero_padded_bin(data_sectors, audio_sectors);
    let mut cursor = Cursor::new(bin);

    let result = find_zero_padded_track_boundary(&mut cursor).unwrap();
    assert_eq!(result, Some(data_sectors as u64 * RAW_SECTOR_SIZE));
}

#[test]
fn test_find_zero_padded_single_track_none() {
    // All real data, no zero-padded sectors → None
    let bin = make_zero_padded_bin(50, 0);
    let mut cursor = Cursor::new(bin);

    let result = find_zero_padded_track_boundary(&mut cursor).unwrap();
    assert_eq!(result, None);
}

#[test]
fn test_detect_data_track_size_chains_methods() {
    // Normal multi-track BIN (audio lacks sync pattern) → detected by primary method
    let normal_bin = make_multi_track_bin(20, 10);
    let mut cursor = Cursor::new(normal_bin);
    let result = detect_data_track_size(&mut cursor).unwrap();
    assert_eq!(result, Some((20 * RAW_SECTOR_SIZE, false)));

    // Zero-padded BIN (audio has sync + zero data) → detected by secondary method
    let padded_bin = make_zero_padded_bin(20, 10);
    let mut cursor = Cursor::new(padded_bin);
    let result = detect_data_track_size(&mut cursor).unwrap();
    assert_eq!(result, Some((20 * RAW_SECTOR_SIZE, true)));

    // Single track (all data) → None
    let single_bin = make_multi_track_bin(20, 0);
    let mut cursor = Cursor::new(single_bin);
    let result = detect_data_track_size(&mut cursor).unwrap();
    assert_eq!(result, None);
}

#[test]
fn test_hash_with_zero_padded_track_has_correct_hash() {
    let data_sectors = 30;
    let audio_sectors = 10;
    let bin = make_zero_padded_bin(data_sectors, audio_sectors);

    let data_track_bytes = data_sectors * RAW_SECTOR_SIZE as usize;
    let (expected_crc, _, _) = reference_hashes(&bin[..data_track_bytes]);

    let mut cursor = Cursor::new(bin);
    let result = detect_data_track_size(&mut cursor).unwrap();
    let (data_size, was_zero_padded) = result.expect("Should detect zero-padded boundary");
    assert!(was_zero_padded);
    assert_eq!(data_size, data_track_bytes as u64);

    let algorithms = retro_junk_core::HashAlgorithms::Crc32;
    let hashes = hash_raw_bin_track1(&mut cursor, algorithms, data_size, None).unwrap();
    assert_eq!(
        hashes.crc32, expected_crc,
        "CRC32 should match data track only"
    );
}
