use std::io::SeekFrom;
use std::path::Path;

use sha1::Digest;

use retro_junk_core::{HashAlgorithms, ReadSeek, RomAnalyzer};
use retro_junk_dat::error::DatError;
pub use retro_junk_dat::matcher::FileHashes;

const CHUNK_SIZE: usize = 64 * 1024; // 64 KB

/// Try container hashes first; if the analyzer handles the format internally,
/// return the precomputed hashes. Otherwise return None and caller proceeds
/// with streaming.
fn try_container_hashes(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
    algorithms: HashAlgorithms,
    file_path: Option<&Path>,
) -> Result<Option<FileHashes>, DatError> {
    analyzer
        .compute_container_hashes(reader, algorithms, file_path)
        .map_err(|e| DatError::cache(e.to_string()))
}

/// Set up the reader for streaming: determine skip bytes, create normalizer,
/// seek past header. Returns (data_size, normalizer).
fn setup_stream(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
) -> Result<(u64, Option<Box<dyn FnMut(&mut [u8])>>), DatError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    let skip = analyzer
        .dat_header_size(reader, file_size)
        .map_err(|e| DatError::cache(e.to_string()))?;
    let normalizer = analyzer
        .dat_chunk_normalizer(reader, skip)
        .map_err(|e| DatError::cache(e.to_string()))?;
    reader.seek(SeekFrom::Start(skip))?;
    Ok((file_size - skip, normalizer))
}

/// Read chunks from the reader, normalizing each, and pass to the callback.
fn stream_chunks(
    reader: &mut dyn ReadSeek,
    normalizer: &mut Option<Box<dyn FnMut(&mut [u8])>>,
    mut on_chunk: impl FnMut(&[u8]),
) -> Result<(), DatError> {
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if let Some(norm) = normalizer.as_deref_mut() {
            norm(&mut buf[..n]);
        }
        on_chunk(&buf[..n]);
    }
    Ok(())
}

/// Unified internal hash engine. Computes whichever combination of CRC32/SHA1/MD5
/// is requested by `algorithms`, optionally reporting progress via `on_progress`.
fn compute_hashes_internal(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
    algorithms: HashAlgorithms,
    on_progress: Option<&dyn Fn(u64, u64)>,
    file_path: Option<&Path>,
) -> Result<FileHashes, DatError> {
    if let Some(hashes) = try_container_hashes(reader, analyzer, algorithms, file_path)? {
        return Ok(hashes);
    }

    let (data_size, mut normalizer) = setup_stream(reader, analyzer)?;
    let mut crc = crc32fast::Hasher::new();
    let mut sha: Option<sha1::Sha1> = if algorithms.sha1() {
        Some(sha1::Sha1::new())
    } else {
        None
    };
    let mut md5_ctx: Option<md5::Context> = if algorithms.md5() {
        Some(md5::Context::new())
    } else {
        None
    };

    let mut processed: u64 = 0;
    stream_chunks(reader, &mut normalizer, |chunk| {
        crc.update(chunk);
        if let Some(ref mut s) = sha {
            s.update(chunk);
        }
        if let Some(ref mut m) = md5_ctx {
            m.consume(chunk);
        }
        processed += chunk.len() as u64;
        if let Some(cb) = on_progress {
            cb(processed, data_size);
        }
    })?;

    Ok(FileHashes {
        crc32: format!("{:08x}", crc.finalize()),
        sha1: sha.map(|s| format!("{:x}", s.finalize())),
        md5: md5_ctx.map(|m| format!("{:x}", m.compute())),
        data_size,
    })
}

/// Compute both CRC32 and SHA1 of a file, using the analyzer's DAT trait methods.
pub fn compute_crc32_sha1(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
    file_path: Option<&Path>,
) -> Result<FileHashes, DatError> {
    compute_hashes_internal(reader, analyzer, HashAlgorithms::Crc32Sha1, None, file_path)
}

/// Compute CRC32 and SHA1 with a progress callback.
/// The callback receives (bytes_processed, total_bytes).
pub fn compute_crc32_sha1_with_progress(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
    progress: &dyn Fn(u64, u64),
    file_path: Option<&Path>,
) -> Result<FileHashes, DatError> {
    compute_hashes_internal(
        reader,
        analyzer,
        HashAlgorithms::Crc32Sha1,
        Some(progress),
        file_path,
    )
}

/// Compute CRC32, MD5, and SHA1 of a file in a single pass.
/// Used by the scraper for ScreenScraper API lookups.
pub fn compute_all_hashes(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
    file_path: Option<&Path>,
) -> Result<FileHashes, DatError> {
    compute_hashes_internal(reader, analyzer, HashAlgorithms::All, None, file_path)
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
    let (file_data_size, mut normalizer) = setup_stream(reader, analyzer)?;
    let total_data_size = padding.prepend_size + file_data_size + padding.append_size;

    let mut crc = crc32fast::Hasher::new();
    let mut sha = sha1::Sha1::new();

    // Phase 1: prepend padding (not normalized)
    stream_padding(padding.prepend_size, padding.fill_byte, |chunk| {
        crc.update(chunk);
        sha.update(chunk);
    });

    // Phase 2: file data (normalized if applicable)
    stream_chunks(reader, &mut normalizer, |chunk| {
        crc.update(chunk);
        sha.update(chunk);
    })?;

    // Phase 3: append padding (not normalized)
    stream_padding(padding.append_size, padding.fill_byte, |chunk| {
        crc.update(chunk);
        sha.update(chunk);
    });

    Ok(FileHashes {
        crc32: format!("{:08x}", crc.finalize()),
        sha1: Some(format!("{:x}", sha.finalize())),
        md5: None,
        data_size: total_data_size,
    })
}

/// Stream `size` bytes of `fill_byte` in CHUNK_SIZE blocks to the callback.
fn stream_padding(size: u64, fill_byte: u8, mut on_chunk: impl FnMut(&[u8])) {
    if size == 0 {
        return;
    }
    let fill_buf = vec![fill_byte; CHUNK_SIZE];
    let mut remaining = size;
    while remaining > 0 {
        let n = std::cmp::min(remaining, CHUNK_SIZE as u64) as usize;
        on_chunk(&fill_buf[..n]);
        remaining -= n as u64;
    }
}

#[cfg(test)]
#[path = "tests/hasher_tests.rs"]
mod tests;
