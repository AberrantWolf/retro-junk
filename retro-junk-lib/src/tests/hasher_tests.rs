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
