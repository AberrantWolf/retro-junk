use std::io::{Read, Seek, SeekFrom};

use sha1::Digest;

use retro_junk_lib::n64::{N64Format, detect_n64_format, normalize_to_big_endian};

use crate::error::DatError;
use crate::systems::{ByteOrderRule, HeaderDetect};

const CHUNK_SIZE: usize = 64 * 1024; // 64 KB

/// Hash results for a file.
#[derive(Debug, Clone)]
pub struct FileHashes {
    pub crc32: String,
    pub sha1: Option<String>,
    /// Size of the data that was hashed (after header stripping)
    pub data_size: u64,
}

/// Detect the N64 format if `byte_order` is `N64`, seeking to read the magic bytes
/// then resetting to `start_pos`.
fn detect_byte_order<R: Read + Seek>(
    reader: &mut R,
    byte_order: &ByteOrderRule,
    start_pos: u64,
) -> Result<Option<N64Format>, DatError> {
    match byte_order {
        ByteOrderRule::None => Ok(None),
        ByteOrderRule::N64 => {
            reader.seek(SeekFrom::Start(start_pos))?;
            let mut magic = [0u8; 4];
            reader.read_exact(&mut magic)?;
            reader.seek(SeekFrom::Start(start_pos))?;
            Ok(detect_n64_format(&magic[..]))
        }
    }
}

/// Apply byte-order normalization to a buffer if needed.
fn apply_byte_order(buf: &mut [u8], n64_format: Option<N64Format>) {
    if let Some(fmt) = n64_format {
        normalize_to_big_endian(&mut buf[..], fmt);
    }
}

/// Compute CRC32 of a file, streaming in 64KB chunks.
/// Optionally skips a header and normalizes byte order before hashing.
pub fn compute_crc32<R: Read + Seek>(
    reader: &mut R,
    header: &HeaderDetect,
    byte_order: &ByteOrderRule,
) -> Result<FileHashes, DatError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    let skip = header.header_size(file_size);
    let n64_fmt = detect_byte_order(reader, byte_order, skip)?;
    reader.seek(SeekFrom::Start(skip))?;

    let data_size = file_size - skip;
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = vec![0u8; CHUNK_SIZE];

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        apply_byte_order(&mut buf[..n], n64_fmt);
        hasher.update(&buf[..n]);
    }

    Ok(FileHashes {
        crc32: format!("{:08x}", hasher.finalize()),
        sha1: None,
        data_size,
    })
}

/// Compute both CRC32 and SHA1 of a file, streaming in 64KB chunks.
/// Optionally skips a header and normalizes byte order before hashing.
pub fn compute_crc32_sha1<R: Read + Seek>(
    reader: &mut R,
    header: &HeaderDetect,
    byte_order: &ByteOrderRule,
) -> Result<FileHashes, DatError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    let skip = header.header_size(file_size);
    let n64_fmt = detect_byte_order(reader, byte_order, skip)?;
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
        apply_byte_order(&mut buf[..n], n64_fmt);
        crc.update(&buf[..n]);
        sha.update(&buf[..n]);
    }

    Ok(FileHashes {
        crc32: format!("{:08x}", crc.finalize()),
        sha1: Some(format!("{:x}", sha.finalize())),
        data_size,
    })
}

/// Compute CRC32 with a progress callback.
/// The callback receives (bytes_processed, total_bytes).
pub fn compute_crc32_with_progress<R: Read + Seek>(
    reader: &mut R,
    header: &HeaderDetect,
    byte_order: &ByteOrderRule,
    progress: &dyn Fn(u64, u64),
) -> Result<FileHashes, DatError> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    let skip = header.header_size(file_size);
    let n64_fmt = detect_byte_order(reader, byte_order, skip)?;
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
        apply_byte_order(&mut buf[..n], n64_fmt);
        hasher.update(&buf[..n]);
        processed += n as u64;
        progress(processed, data_size);
    }

    Ok(FileHashes {
        crc32: format!("{:08x}", hasher.finalize()),
        sha1: None,
        data_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_crc32_simple() {
        let data = b"Hello, World!";
        let mut cursor = Cursor::new(data.to_vec());
        let hashes = compute_crc32(&mut cursor, &HeaderDetect::None, &ByteOrderRule::None).unwrap();
        assert_eq!(hashes.data_size, 13);
        assert_eq!(hashes.crc32.len(), 8); // 8 hex chars
        assert!(hashes.sha1.is_none());
    }

    #[test]
    fn test_crc32_sha1() {
        let data = b"Hello, World!";
        let mut cursor = Cursor::new(data.to_vec());
        let hashes = compute_crc32_sha1(&mut cursor, &HeaderDetect::None, &ByteOrderRule::None).unwrap();
        assert!(hashes.sha1.is_some());
        assert_eq!(hashes.sha1.as_deref().unwrap().len(), 40); // 40 hex chars
    }

    #[test]
    fn test_header_skip() {
        // 512 bytes header + 1024 bytes data = 1536 total
        // 1536 % 1024 = 512
        let mut data = vec![0xFFu8; 512]; // header
        data.extend(vec![0x42u8; 1024]); // ROM data
        let mut cursor = Cursor::new(data);

        let snes_header = HeaderDetect::SizeModulo {
            modulo: 1024,
            remainder: 512,
            skip: 512,
        };

        let hashes = compute_crc32(&mut cursor, &snes_header, &ByteOrderRule::None).unwrap();
        assert_eq!(hashes.data_size, 1024);

        // Verify it hashed only the ROM data (not the header)
        let mut cursor2 = Cursor::new(vec![0x42u8; 1024]);
        let hashes2 = compute_crc32(&mut cursor2, &HeaderDetect::None, &ByteOrderRule::None).unwrap();
        assert_eq!(hashes.crc32, hashes2.crc32);
    }

    #[test]
    fn test_no_header_skip_when_aligned() {
        // 1024 bytes data, no header
        // 1024 % 1024 = 0, so no skip
        let data = vec![0x42u8; 1024];
        let mut cursor = Cursor::new(data);

        let snes_header = HeaderDetect::SizeModulo {
            modulo: 1024,
            remainder: 512,
            skip: 512,
        };

        let hashes = compute_crc32(&mut cursor, &snes_header, &ByteOrderRule::None).unwrap();
        assert_eq!(hashes.data_size, 1024);
    }

    // -- N64 byte-order normalization tests --

    /// Build a minimal N64-like ROM: 4-byte magic + payload, in the given format.
    fn make_n64_rom(format: N64Format, payload: &[u8]) -> Vec<u8> {
        let magic: [u8; 4] = match format {
            N64Format::Z64 => [0x80, 0x37, 0x12, 0x40],
            N64Format::V64 => [0x37, 0x80, 0x40, 0x12],
            N64Format::N64 => [0x40, 0x12, 0x37, 0x80],
        };
        let mut rom = magic.to_vec();

        // Transform payload to the target format
        let mut transformed = payload.to_vec();
        match format {
            N64Format::Z64 => {} // payload is already big-endian
            N64Format::V64 => {
                for i in (0..transformed.len() - 1).step_by(2) {
                    transformed.swap(i, i + 1);
                }
            }
            N64Format::N64 => {
                for chunk in transformed.chunks_exact_mut(4) {
                    chunk.reverse();
                }
            }
        }
        rom.extend_from_slice(&transformed);
        rom
    }

    #[test]
    fn test_n64_z64_no_swap() {
        // .z64 is already big-endian, should hash the same as raw payload
        let payload = vec![0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];
        let rom = make_n64_rom(N64Format::Z64, &payload);
        let mut cursor = Cursor::new(rom);
        let hashes = compute_crc32(&mut cursor, &HeaderDetect::None, &ByteOrderRule::N64).unwrap();

        // Hash of magic + payload (big-endian) — compute reference
        let mut ref_data = vec![0x80, 0x37, 0x12, 0x40];
        ref_data.extend_from_slice(&payload);
        let mut ref_cursor = Cursor::new(ref_data);
        let ref_hashes = compute_crc32(&mut ref_cursor, &HeaderDetect::None, &ByteOrderRule::None).unwrap();

        assert_eq!(hashes.crc32, ref_hashes.crc32);
    }

    #[test]
    fn test_n64_v64_normalized_matches_z64() {
        // .v64 byte-swapped ROM should hash identically to .z64 version
        let payload = vec![0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];

        let z64_rom = make_n64_rom(N64Format::Z64, &payload);
        let v64_rom = make_n64_rom(N64Format::V64, &payload);

        let mut z64_cursor = Cursor::new(z64_rom);
        let mut v64_cursor = Cursor::new(v64_rom);

        let z64_hashes = compute_crc32(&mut z64_cursor, &HeaderDetect::None, &ByteOrderRule::N64).unwrap();
        let v64_hashes = compute_crc32(&mut v64_cursor, &HeaderDetect::None, &ByteOrderRule::N64).unwrap();

        assert_eq!(z64_hashes.crc32, v64_hashes.crc32);
        assert_eq!(z64_hashes.data_size, v64_hashes.data_size);
    }

    #[test]
    fn test_n64_n64_normalized_matches_z64() {
        // .n64 little-endian ROM should hash identically to .z64 version
        let payload = vec![0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];

        let z64_rom = make_n64_rom(N64Format::Z64, &payload);
        let n64_rom = make_n64_rom(N64Format::N64, &payload);

        let mut z64_cursor = Cursor::new(z64_rom);
        let mut n64_cursor = Cursor::new(n64_rom);

        let z64_hashes = compute_crc32(&mut z64_cursor, &HeaderDetect::None, &ByteOrderRule::N64).unwrap();
        let n64_hashes = compute_crc32(&mut n64_cursor, &HeaderDetect::None, &ByteOrderRule::N64).unwrap();

        assert_eq!(z64_hashes.crc32, n64_hashes.crc32);
        assert_eq!(z64_hashes.data_size, n64_hashes.data_size);
    }

    #[test]
    fn test_n64_sha1_also_normalized() {
        let payload = vec![0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];

        let z64_rom = make_n64_rom(N64Format::Z64, &payload);
        let v64_rom = make_n64_rom(N64Format::V64, &payload);

        let mut z64_cursor = Cursor::new(z64_rom);
        let mut v64_cursor = Cursor::new(v64_rom);

        let z64_hashes = compute_crc32_sha1(&mut z64_cursor, &HeaderDetect::None, &ByteOrderRule::N64).unwrap();
        let v64_hashes = compute_crc32_sha1(&mut v64_cursor, &HeaderDetect::None, &ByteOrderRule::N64).unwrap();

        assert_eq!(z64_hashes.crc32, v64_hashes.crc32);
        assert_eq!(z64_hashes.sha1, v64_hashes.sha1);
    }
}
