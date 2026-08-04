use super::{BANK_SIZE, PceAnalyzer};
use retro_junk_core::RomAnalyzer;
use std::io::Cursor;

/// A dump 512 bytes larger than a whole number of 8 KB banks is a headered
/// dump. Hashing those 512 bytes would produce a digest No-Intro has never
/// seen, so every headered PC Engine ROM would silently fail to match.
#[test]
fn copier_header_is_skipped_before_hashing() {
    let analyzer = PceAnalyzer;
    let mut empty = Cursor::new(Vec::<u8>::new());
    let headered = 256 * BANK_SIZE + 512;
    assert_eq!(
        analyzer.dat_header_size(&mut empty, headered).unwrap(),
        512,
        "a 512-byte copier header must not reach the hasher"
    );
}

/// A clean dump must be hashed whole — skipping 512 bytes here would break
/// the far more common headerless case.
#[test]
fn clean_dump_is_hashed_whole() {
    let analyzer = PceAnalyzer;
    let mut empty = Cursor::new(Vec::<u8>::new());
    for banks in [1_u64, 32, 320] {
        assert_eq!(
            analyzer
                .dat_header_size(&mut empty, banks * BANK_SIZE)
                .unwrap(),
            0,
            "{banks}-bank dump has no copier header"
        );
    }
}

/// A size that is neither a whole number of banks nor banks-plus-512 is not a
/// `HuCard` shape at all; guessing a header off it would corrupt the hash.
#[test]
fn odd_sizes_are_left_alone() {
    let analyzer = PceAnalyzer;
    let mut empty = Cursor::new(Vec::<u8>::new());
    assert_eq!(
        analyzer.dat_header_size(&mut empty, 12345).unwrap(),
        0,
        "unrecognized size must not be trimmed"
    );
}

#[test]
fn recognizes_hucard_shaped_files() {
    let analyzer = PceAnalyzer;
    let mut clean = Cursor::new(vec![0_u8; (4 * BANK_SIZE) as usize]);
    assert!(analyzer.can_handle(&mut clean));

    let mut headered = Cursor::new(vec![0_u8; (4 * BANK_SIZE) as usize + 512]);
    assert!(analyzer.can_handle(&mut headered));

    let mut ragged = Cursor::new(vec![0_u8; 4000]);
    assert!(!analyzer.can_handle(&mut ragged));

    let mut nothing = Cursor::new(Vec::<u8>::new());
    assert!(!analyzer.can_handle(&mut nothing));
}

/// `can_handle` peeks at the file; it must leave the reader where it found it
/// so the next reader of the same handle does not start mid-file.
#[test]
fn can_handle_rewinds_the_reader() {
    use std::io::Seek;
    let analyzer = PceAnalyzer;
    let mut reader = Cursor::new(vec![0_u8; (4 * BANK_SIZE) as usize]);
    analyzer.can_handle(&mut reader);
    assert_eq!(reader.stream_position().unwrap(), 0);
}
