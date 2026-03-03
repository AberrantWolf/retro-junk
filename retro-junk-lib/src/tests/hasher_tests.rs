use super::*;
use std::io::Cursor;

use retro_junk_core::{AnalysisError, AnalysisOptions, Platform, RomIdentification};

/// Minimal analyzer with no header skip and no normalizer.
struct NullAnalyzer;

impl RomAnalyzer for NullAnalyzer {
    fn analyze(
        &self,
        _reader: &mut dyn ReadSeek,
        _options: &AnalysisOptions,
    ) -> Result<RomIdentification, AnalysisError> {
        Ok(RomIdentification::new())
    }

    fn platform(&self) -> Platform {
        Platform::Nes
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["bin"]
    }

    fn can_handle(&self, _reader: &mut dyn ReadSeek) -> bool {
        true
    }
}

#[test]
fn test_padding_append_matches_concatenated() {
    // Create file data: 4 bytes of 0xAB
    let file_data = vec![0xABu8; 4];
    // Create expected full data: 4 bytes of 0xAB + 4 bytes of 0x00
    let mut full_data = file_data.clone();
    full_data.extend_from_slice(&[0x00u8; 4]);

    // Hash the full data directly
    let mut full_cursor = Cursor::new(full_data);
    let expected = compute_crc32_sha1(&mut full_cursor, &NullAnalyzer, None).unwrap();

    // Hash with padding
    let mut file_cursor = Cursor::new(file_data);
    let padding = PaddingSpec {
        prepend_size: 0,
        append_size: 4,
        fill_byte: 0x00,
    };
    let padded =
        compute_crc32_sha1_with_padding(&mut file_cursor, &NullAnalyzer, &padding).unwrap();

    assert_eq!(padded.crc32, expected.crc32);
    assert_eq!(padded.sha1, expected.sha1);
    assert_eq!(padded.data_size, 8);
}

#[test]
fn test_padding_prepend_matches_concatenated() {
    // Create file data
    let file_data = vec![0xCDu8; 8];
    // Expected: 4 bytes of 0xFF prepended
    let mut full_data = vec![0xFFu8; 4];
    full_data.extend_from_slice(&file_data);

    let mut full_cursor = Cursor::new(full_data);
    let expected = compute_crc32_sha1(&mut full_cursor, &NullAnalyzer, None).unwrap();

    let mut file_cursor = Cursor::new(file_data);
    let padding = PaddingSpec {
        prepend_size: 4,
        append_size: 0,
        fill_byte: 0xFF,
    };
    let padded =
        compute_crc32_sha1_with_padding(&mut file_cursor, &NullAnalyzer, &padding).unwrap();

    assert_eq!(padded.crc32, expected.crc32);
    assert_eq!(padded.sha1, expected.sha1);
    assert_eq!(padded.data_size, 12);
}

#[test]
fn test_padding_both_prepend_and_append() {
    let file_data = vec![0x42u8; 16];
    let mut full_data = vec![0x00u8; 8]; // prepend
    full_data.extend_from_slice(&file_data);
    full_data.extend_from_slice(&[0x00u8; 8]); // append

    let mut full_cursor = Cursor::new(full_data);
    let expected = compute_crc32_sha1(&mut full_cursor, &NullAnalyzer, None).unwrap();

    let mut file_cursor = Cursor::new(file_data);
    let padding = PaddingSpec {
        prepend_size: 8,
        append_size: 8,
        fill_byte: 0x00,
    };
    let padded =
        compute_crc32_sha1_with_padding(&mut file_cursor, &NullAnalyzer, &padding).unwrap();

    assert_eq!(padded.crc32, expected.crc32);
    assert_eq!(padded.sha1, expected.sha1);
    assert_eq!(padded.data_size, 32);
}

#[test]
fn test_padding_zero_size_is_identity() {
    let file_data = vec![0x99u8; 100];

    let mut cursor1 = Cursor::new(file_data.clone());
    let normal = compute_crc32_sha1(&mut cursor1, &NullAnalyzer, None).unwrap();

    let mut cursor2 = Cursor::new(file_data);
    let padding = PaddingSpec {
        prepend_size: 0,
        append_size: 0,
        fill_byte: 0x00,
    };
    let padded = compute_crc32_sha1_with_padding(&mut cursor2, &NullAnalyzer, &padding).unwrap();

    assert_eq!(padded.crc32, normal.crc32);
    assert_eq!(padded.sha1, normal.sha1);
    assert_eq!(padded.data_size, normal.data_size);
}

#[test]
fn test_padding_large_append() {
    // Test with append larger than CHUNK_SIZE (64KB) to ensure loop works
    let file_data = vec![0x01u8; 4];
    let append_size: u64 = 128 * 1024; // 128 KB

    let mut full_data = file_data.clone();
    full_data.extend(std::iter::repeat(0xFFu8).take(append_size as usize));

    let mut full_cursor = Cursor::new(full_data);
    let expected = compute_crc32_sha1(&mut full_cursor, &NullAnalyzer, None).unwrap();

    let mut file_cursor = Cursor::new(file_data);
    let padding = PaddingSpec {
        prepend_size: 0,
        append_size,
        fill_byte: 0xFF,
    };
    let padded =
        compute_crc32_sha1_with_padding(&mut file_cursor, &NullAnalyzer, &padding).unwrap();

    assert_eq!(padded.crc32, expected.crc32);
    assert_eq!(padded.sha1, expected.sha1);
}

#[test]
fn test_progress_callback_reports_total_bytes() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    let file_data = vec![0xAAu8; 256 * 1024]; // 256 KB — spans multiple chunks
    let expected_size = file_data.len() as u64;

    let last_done = Arc::new(AtomicU64::new(0));
    let last_total = Arc::new(AtomicU64::new(0));
    let call_count = Arc::new(AtomicU64::new(0));

    let mut cursor = Cursor::new(file_data.clone());
    let hashes = compute_crc32_sha1_with_progress(
        &mut cursor,
        &NullAnalyzer,
        &|done, total| {
            last_done.store(done, Ordering::Relaxed);
            last_total.store(total, Ordering::Relaxed);
            call_count.fetch_add(1, Ordering::Relaxed);
        },
        None,
    )
    .unwrap();

    // Callback was called at least once per chunk
    assert!(call_count.load(Ordering::Relaxed) > 1);
    // Final callback reported all bytes processed
    assert_eq!(last_done.load(Ordering::Relaxed), expected_size);
    assert_eq!(last_total.load(Ordering::Relaxed), expected_size);

    // Hashes match the non-progress version
    let mut cursor2 = Cursor::new(file_data);
    let expected = compute_crc32_sha1(&mut cursor2, &NullAnalyzer, None).unwrap();
    assert_eq!(hashes.crc32, expected.crc32);
    assert_eq!(hashes.sha1, expected.sha1);
}
