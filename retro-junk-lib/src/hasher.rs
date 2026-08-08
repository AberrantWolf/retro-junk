use std::io::SeekFrom;
use std::path::Path;

use retro_junk_core::{HashAlgorithms, HashProgressFn, MultiHasher, ReadSeek, RomAnalyzer};
use retro_junk_dat::error::DatError;
pub use retro_junk_dat::matcher::FileHashes;

const CHUNK_SIZE: usize = 64 * 1024; // 64 KB

/// In-place chunk normalizer returned by [`RomAnalyzer::dat_chunk_normalizer`]
/// (e.g. byte-swapping N64 ROMs to the DAT's canonical byte order).
type ChunkNormalizer = Box<dyn FnMut(&mut [u8])>;

/// Try container hashes first; if the analyzer handles the format internally,
/// return the precomputed hashes. Otherwise return None and caller proceeds
/// with streaming.
fn try_container_hashes(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
    algorithms: HashAlgorithms,
    file_path: Option<&Path>,
    on_progress: HashProgressFn<'_>,
) -> Result<Option<FileHashes>, DatError> {
    analyzer
        .compute_container_hashes(reader, algorithms, file_path, on_progress)
        .map_err(|e| DatError::cache(e.to_string()))
}

/// Set up the reader for streaming: determine skip bytes, create normalizer,
/// seek past header. Returns (`data_size`, normalizer).
fn setup_stream(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
) -> Result<(u64, Option<ChunkNormalizer>), DatError> {
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
    normalizer: &mut Option<ChunkNormalizer>,
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
    if let Some(hashes) =
        try_container_hashes(reader, analyzer, algorithms, file_path, on_progress)?
    {
        return Ok(hashes);
    }

    let (data_size, mut normalizer) = setup_stream(reader, analyzer)?;
    let mut hasher = MultiHasher::new(algorithms, data_size, on_progress);

    stream_chunks(reader, &mut normalizer, |chunk| {
        hasher.update_with_progress(chunk);
    })?;

    Ok(hasher.finalize())
}

/// Compute CRC32 and SHA1 of raw file bytes, with no analyzer involvement.
///
/// Used for per-track verification of disc sets: Redump stores each track
/// (`.bin`) as a plain file whose hashes cover the entire file, so no header
/// stripping, normalization, or container extraction applies.
pub fn compute_plain_crc32_sha1(
    reader: &mut dyn ReadSeek,
    on_progress: Option<&dyn Fn(u64, u64)>,
) -> Result<FileHashes, DatError> {
    let data_size = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;
    let mut hasher = MultiHasher::new(HashAlgorithms::Crc32Sha1, data_size, on_progress);
    stream_chunks(reader, &mut None, |chunk| {
        hasher.update_with_progress(chunk);
    })?;
    Ok(hasher.finalize())
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
/// The callback receives (`bytes_processed`, `total_bytes`).
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
/// Used by the scraper for `ScreenScraper` API lookups.
pub fn compute_all_hashes(
    reader: &mut dyn ReadSeek,
    analyzer: &dyn RomAnalyzer,
    file_path: Option<&Path>,
) -> Result<FileHashes, DatError> {
    compute_hashes_internal(reader, analyzer, HashAlgorithms::All, None, file_path)
}

#[cfg(test)]
#[path = "tests/hasher_tests.rs"]
mod tests;
