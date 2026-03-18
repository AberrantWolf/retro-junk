//! Multi-algorithm hasher for computing CRC32, SHA1, and MD5 in a single pass.
//!
//! Encapsulates the init/feed/finalize pattern used across disc and cartridge
//! hashing functions, eliminating repeated boilerplate.

use sha1::Digest;

use crate::{FileHashes, HashAlgorithms, HashProgressFn};

/// Computes CRC32, SHA1, and/or MD5 hashes in a single streaming pass.
///
/// Wraps the common pattern of conditionally initializing hash algorithms
/// based on `HashAlgorithms`, feeding chunks, tracking progress, and
/// finalizing into a `FileHashes`.
pub struct MultiHasher<'a> {
    crc: Option<crc32fast::Hasher>,
    sha: Option<sha1::Sha1>,
    md5_ctx: Option<md5::Context>,
    bytes_processed: u64,
    data_size: u64,
    on_progress: HashProgressFn<'a>,
}

impl<'a> MultiHasher<'a> {
    /// Create a new hasher for the given algorithms and expected data size.
    ///
    /// `data_size` is used for progress reporting (the "total" in the callback).
    /// Pass 0 if the total size is unknown.
    pub fn new(
        algorithms: HashAlgorithms,
        data_size: u64,
        on_progress: HashProgressFn<'a>,
    ) -> Self {
        Self {
            crc: if algorithms.crc32() {
                Some(crc32fast::Hasher::new())
            } else {
                None
            },
            sha: if algorithms.sha1() {
                Some(sha1::Sha1::new())
            } else {
                None
            },
            md5_ctx: if algorithms.md5() {
                Some(md5::Context::new())
            } else {
                None
            },
            bytes_processed: 0,
            data_size,
            on_progress,
        }
    }

    /// Feed a chunk of data into all active hashers.
    pub fn update(&mut self, chunk: &[u8]) {
        if let Some(ref mut h) = self.crc {
            h.update(chunk);
        }
        if let Some(ref mut h) = self.sha {
            h.update(chunk);
        }
        if let Some(ref mut h) = self.md5_ctx {
            h.consume(chunk);
        }
        self.bytes_processed += chunk.len() as u64;
    }

    /// Feed a chunk and report progress.
    ///
    /// Equivalent to calling `update()` followed by firing the progress
    /// callback with the current cumulative bytes processed.
    pub fn update_with_progress(&mut self, chunk: &[u8]) {
        self.update(chunk);
        if let Some(cb) = self.on_progress {
            cb(self.bytes_processed, self.data_size);
        }
    }

    /// Report progress without feeding data.
    ///
    /// Useful when progress should be reported at a different granularity
    /// than the data chunks (e.g., after processing a full CHD hunk
    /// containing multiple sectors).
    pub fn report_progress(&self) {
        if let Some(cb) = self.on_progress {
            cb(self.bytes_processed, self.data_size);
        }
    }

    /// Finalize all hashers and return the computed hashes.
    pub fn finalize(self) -> FileHashes {
        FileHashes {
            crc32: self
                .crc
                .map(|h| format!("{:08x}", h.finalize()))
                .unwrap_or_default(),
            sha1: self.sha.map(|h| format!("{:x}", h.finalize())),
            md5: self.md5_ctx.map(|h| format!("{:x}", h.compute())),
            data_size: self.data_size,
            warnings: vec![],
        }
    }
}
