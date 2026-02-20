use std::io::SeekFrom;

use sha1::Digest;

use retro_junk_core::{ReadSeek, RomAnalyzer};
use retro_junk_dat::error::DatError;
pub use retro_junk_dat::matcher::FileHashes;

const CHUNK_SIZE: usize = 64 * 1024; // 64 KB

/// Compute CRC32 of a file, using the analyzer's DAT trait methods for
/// header stripping and byte-order normalization.
pub fn compute_crc32(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
) -> Result<FileHashes, DatError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    let skip = analyzer
        .dat_header_size(reader, file_size)
        .map_err(|e| DatError::cache(e.to_string()))?;
    let mut normalizer = analyzer
        .dat_chunk_normalizer(reader, skip)
        .map_err(|e| DatError::cache(e.to_string()))?;
    reader.seek(SeekFrom::Start(skip))?;

    let data_size = file_size - skip;
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = vec![0u8; CHUNK_SIZE];

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if let Some(ref mut norm) = normalizer {
            norm(&mut buf[..n]);
        }
        hasher.update(&buf[..n]);
    }

    Ok(FileHashes {
        crc32: format!("{:08x}", hasher.finalize()),
        sha1: None,
        md5: None,
        data_size,
    })
}

/// Compute both CRC32 and SHA1 of a file, using the analyzer's DAT trait methods.
pub fn compute_crc32_sha1(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
) -> Result<FileHashes, DatError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    let skip = analyzer
        .dat_header_size(reader, file_size)
        .map_err(|e| DatError::cache(e.to_string()))?;
    let mut normalizer = analyzer
        .dat_chunk_normalizer(reader, skip)
        .map_err(|e| DatError::cache(e.to_string()))?;
    reader.seek(SeekFrom::Start(skip))?;

    let data_size = file_size - skip;
    let mut crc = crc32fast::Hasher::new();
    let mut sha = sha1::Sha1::new();
    let mut buf = vec![0u8; CHUNK_SIZE];

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if let Some(ref mut norm) = normalizer {
            norm(&mut buf[..n]);
        }
        crc.update(&buf[..n]);
        sha.update(&buf[..n]);
    }

    Ok(FileHashes {
        crc32: format!("{:08x}", crc.finalize()),
        sha1: Some(format!("{:x}", sha.finalize())),
        md5: None,
        data_size,
    })
}

/// Compute CRC32 with a progress callback, using the analyzer's DAT trait methods.
/// The callback receives (bytes_processed, total_bytes).
pub fn compute_crc32_with_progress(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
    progress: &dyn Fn(u64, u64),
) -> Result<FileHashes, DatError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    let skip = analyzer
        .dat_header_size(reader, file_size)
        .map_err(|e| DatError::cache(e.to_string()))?;
    let mut normalizer = analyzer
        .dat_chunk_normalizer(reader, skip)
        .map_err(|e| DatError::cache(e.to_string()))?;
    reader.seek(SeekFrom::Start(skip))?;

    let data_size = file_size - skip;
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut processed: u64 = 0;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if let Some(ref mut norm) = normalizer {
            norm(&mut buf[..n]);
        }
        hasher.update(&buf[..n]);
        processed += n as u64;
        progress(processed, data_size);
    }

    Ok(FileHashes {
        crc32: format!("{:08x}", hasher.finalize()),
        sha1: None,
        md5: None,
        data_size,
    })
}

/// Specification for padding bytes to prepend/append when computing hashes.
#[derive(Debug, Clone)]
pub struct PaddingSpec {
    /// Bytes of fill to prepend before the file data
    pub prepend_size: u64,
    /// Bytes of fill to append after the file data
    pub append_size: u64,
    /// Fill byte value (typically 0x00 or 0xFF)
    pub fill_byte: u8,
}

/// Compute CRC32 and SHA1 of a file with virtual padding prepended/appended.
///
/// Hashes `[prepend padding] + [file data after header skip] + [append padding]`
/// in a single streaming pass. Padding bytes are NOT run through the normalizer
/// (0x00 and 0xFF are byte-order invariant).
///
/// Returns `data_size = prepend + (file_size - skip) + append`.
pub fn compute_crc32_sha1_with_padding(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
    padding: &PaddingSpec,
) -> Result<FileHashes, DatError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    let skip = analyzer
        .dat_header_size(reader, file_size)
        .map_err(|e| DatError::cache(e.to_string()))?;
    let mut normalizer = analyzer
        .dat_chunk_normalizer(reader, skip)
        .map_err(|e| DatError::cache(e.to_string()))?;
    reader.seek(SeekFrom::Start(skip))?;

    let file_data_size = file_size - skip;
    let total_data_size = padding.prepend_size + file_data_size + padding.append_size;

    let mut crc = crc32fast::Hasher::new();
    let mut sha = sha1::Sha1::new();
    let mut buf = vec![0u8; CHUNK_SIZE];

    // Phase 1: prepend padding (not normalized)
    let mut remaining = padding.prepend_size;
    let fill_buf = vec![padding.fill_byte; CHUNK_SIZE];
    while remaining > 0 {
        let n = std::cmp::min(remaining, CHUNK_SIZE as u64) as usize;
        crc.update(&fill_buf[..n]);
        sha.update(&fill_buf[..n]);
        remaining -= n as u64;
    }

    // Phase 2: file data (normalized if applicable)
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if let Some(ref mut norm) = normalizer {
            norm(&mut buf[..n]);
        }
        crc.update(&buf[..n]);
        sha.update(&buf[..n]);
    }

    // Phase 3: append padding (not normalized)
    remaining = padding.append_size;
    while remaining > 0 {
        let n = std::cmp::min(remaining, CHUNK_SIZE as u64) as usize;
        crc.update(&fill_buf[..n]);
        sha.update(&fill_buf[..n]);
        remaining -= n as u64;
    }

    Ok(FileHashes {
        crc32: format!("{:08x}", crc.finalize()),
        sha1: Some(format!("{:x}", sha.finalize())),
        md5: None,
        data_size: total_data_size,
    })
}

/// Compute CRC32, MD5, and SHA1 of a file in a single pass.
/// Used by the scraper for ScreenScraper API lookups.
pub fn compute_all_hashes(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
) -> Result<FileHashes, DatError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    let skip = analyzer
        .dat_header_size(reader, file_size)
        .map_err(|e| DatError::cache(e.to_string()))?;
    let mut normalizer = analyzer
        .dat_chunk_normalizer(reader, skip)
        .map_err(|e| DatError::cache(e.to_string()))?;
    reader.seek(SeekFrom::Start(skip))?;

    let data_size = file_size - skip;
    let mut crc = crc32fast::Hasher::new();
    let mut sha = sha1::Sha1::new();
    let mut md5_ctx = md5::Context::new();
    let mut buf = vec![0u8; CHUNK_SIZE];

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if let Some(ref mut norm) = normalizer {
            norm(&mut buf[..n]);
        }
        crc.update(&buf[..n]);
        sha.update(&buf[..n]);
        md5_ctx.consume(&buf[..n]);
    }

    Ok(FileHashes {
        crc32: format!("{:08x}", crc.finalize()),
        sha1: Some(format!("{:x}", sha.finalize())),
        md5: Some(format!("{:x}", md5_ctx.compute())),
        data_size,
    })
}

#[cfg(test)]
mod tests {
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

        fn analyze_with_progress(
            &self,
            reader: &mut dyn ReadSeek,
            options: &AnalysisOptions,
            _progress_tx: std::sync::mpsc::Sender<retro_junk_core::AnalysisProgress>,
        ) -> Result<RomIdentification, AnalysisError> {
            self.analyze(reader, options)
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
        let expected = compute_crc32_sha1(&mut full_cursor, &NullAnalyzer).unwrap();

        // Hash with padding
        let mut file_cursor = Cursor::new(file_data);
        let padding = PaddingSpec {
            prepend_size: 0,
            append_size: 4,
            fill_byte: 0x00,
        };
        let padded = compute_crc32_sha1_with_padding(&mut file_cursor, &NullAnalyzer, &padding)
            .unwrap();

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
        let expected = compute_crc32_sha1(&mut full_cursor, &NullAnalyzer).unwrap();

        let mut file_cursor = Cursor::new(file_data);
        let padding = PaddingSpec {
            prepend_size: 4,
            append_size: 0,
            fill_byte: 0xFF,
        };
        let padded = compute_crc32_sha1_with_padding(&mut file_cursor, &NullAnalyzer, &padding)
            .unwrap();

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
        let expected = compute_crc32_sha1(&mut full_cursor, &NullAnalyzer).unwrap();

        let mut file_cursor = Cursor::new(file_data);
        let padding = PaddingSpec {
            prepend_size: 8,
            append_size: 8,
            fill_byte: 0x00,
        };
        let padded = compute_crc32_sha1_with_padding(&mut file_cursor, &NullAnalyzer, &padding)
            .unwrap();

        assert_eq!(padded.crc32, expected.crc32);
        assert_eq!(padded.sha1, expected.sha1);
        assert_eq!(padded.data_size, 32);
    }

    #[test]
    fn test_padding_zero_size_is_identity() {
        let file_data = vec![0x99u8; 100];

        let mut cursor1 = Cursor::new(file_data.clone());
        let normal = compute_crc32_sha1(&mut cursor1, &NullAnalyzer).unwrap();

        let mut cursor2 = Cursor::new(file_data);
        let padding = PaddingSpec {
            prepend_size: 0,
            append_size: 0,
            fill_byte: 0x00,
        };
        let padded = compute_crc32_sha1_with_padding(&mut cursor2, &NullAnalyzer, &padding)
            .unwrap();

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
        let expected = compute_crc32_sha1(&mut full_cursor, &NullAnalyzer).unwrap();

        let mut file_cursor = Cursor::new(file_data);
        let padding = PaddingSpec {
            prepend_size: 0,
            append_size,
            fill_byte: 0xFF,
        };
        let padded = compute_crc32_sha1_with_padding(&mut file_cursor, &NullAnalyzer, &padding)
            .unwrap();

        assert_eq!(padded.crc32, expected.crc32);
        assert_eq!(padded.sha1, expected.sha1);
    }
}
