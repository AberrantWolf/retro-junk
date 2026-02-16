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
