//! N64 byte-order detection and normalization.
//!
//! N64 ROMs exist in three byte orderings. This module provides the canonical
//! implementation for detecting and normalizing byte order.

/// N64 ROM byte-order format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum N64Format {
    /// .z64 — big-endian (canonical No-Intro format), no swap needed
    Z64,
    /// .v64 — byte-swapped pairs
    V64,
    /// .n64 — little-endian, reverse 4-byte groups
    N64,
}

/// Magic bytes for each N64 format.
pub const MAGIC_Z64: [u8; 4] = [0x80, 0x37, 0x12, 0x40];
pub const MAGIC_V64: [u8; 4] = [0x37, 0x80, 0x40, 0x12];
pub const MAGIC_N64: [u8; 4] = [0x40, 0x12, 0x37, 0x80];

/// Detect the N64 byte-order format from the first 4 bytes of a ROM.
///
/// Returns `None` if the magic bytes don't match any known N64 format.
pub fn detect_n64_format(magic: &[u8]) -> Option<N64Format> {
    if magic.len() < 4 {
        return None;
    }
    match [magic[0], magic[1], magic[2], magic[3]] {
        MAGIC_Z64 => Some(N64Format::Z64),
        MAGIC_V64 => Some(N64Format::V64),
        MAGIC_N64 => Some(N64Format::N64),
        _ => None,
    }
}

/// Normalize a buffer of N64 ROM data to big-endian (.z64) byte order.
///
/// For V64 format, swaps byte pairs: `[A,B,C,D]` → `[B,A,D,C]`
/// For N64 format, reverses 4-byte groups: `[A,B,C,D]` → `[D,C,B,A]`
/// For Z64 format, no transformation is needed.
pub fn normalize_to_big_endian(data: &mut [u8], format: N64Format) {
    match format {
        N64Format::Z64 => {} // already big-endian
        N64Format::V64 => {
            for i in (0..data.len().saturating_sub(1)).step_by(2) {
                data.swap(i, i + 1);
            }
        }
        N64Format::N64 => {
            for chunk in data.chunks_exact_mut(4) {
                chunk.reverse();
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/n64_byteorder_tests.rs"]
mod tests;
