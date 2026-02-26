//! Quick hash verification tool for PS1 disc images.
//!
//! Usage: cargo run -p retro-junk-sony --example hash_check -- <path-to-bin-file>
//!
//! Computes CRC32, SHA1, and MD5 of the file using three methods:
//!   1. Whole file (naive)
//!   2. First N bytes matching the expected Redump Track 1 size
//!   3. Via the analyzer's compute_container_hashes (what the real code uses)

use std::env;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <path-to-bin-or-cue-or-chd>", args[0]);
        std::process::exit(1);
    }
    let path = &args[1];

    // Also accept an optional expected size (for Track 1 hash test)
    let expected_size: Option<u64> = args.get(2).and_then(|s| s.parse().ok());

    let mut file = File::open(path).expect("Failed to open file");
    let file_size = file.seek(SeekFrom::End(0)).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();

    println!("File: {}", path);
    println!("File size: {} bytes", file_size);
    println!();

    // Method 1: Hash the whole file
    println!("=== Method 1: Whole file ===");
    hash_range(&mut file, 0, file_size);

    // Method 2: Hash just Track 1 (if expected_size given)
    if let Some(track1_size) = expected_size {
        println!();
        println!(
            "=== Method 2: First {} bytes (expected Track 1) ===",
            track1_size
        );
        hash_range(&mut file, 0, track1_size);
    }

    // Method 3: Via analyzer's compute_container_hashes
    println!();
    println!("=== Method 3: Analyzer compute_container_hashes ===");
    file.seek(SeekFrom::Start(0)).unwrap();
    let analyzer = retro_junk_sony::Ps1Analyzer;
    let algorithms = retro_junk_core::HashAlgorithms {
        crc32: true,
        sha1: true,
        md5: true,
    };
    use retro_junk_core::RomAnalyzer;
    match analyzer.compute_container_hashes(&mut file, algorithms) {
        Ok(Some(hashes)) => {
            println!("  CRC32:     {}", hashes.crc32);
            println!("  SHA1:      {}", hashes.sha1.as_deref().unwrap_or("n/a"));
            println!("  MD5:       {}", hashes.md5.as_deref().unwrap_or("n/a"));
            println!("  Data size: {}", hashes.data_size);
        }
        Ok(None) => {
            println!("  (analyzer returned None — not a container format)");

            // Fall back to the standard hasher path
            println!();
            println!("=== Method 3b: Standard hasher (with header skip) ===");
            file.seek(SeekFrom::Start(0)).unwrap();
            let skip = analyzer
                .dat_header_size(&mut file, file_size)
                .expect("dat_header_size failed");
            println!("  Header skip: {} bytes", skip);
            let data_size = file_size - skip;
            println!("  Data size: {} bytes", data_size);
            hash_range(&mut file, skip, data_size);
        }
        Err(e) => {
            println!("  Error: {}", e);
        }
    }

    // Expected values
    println!();
    println!("=== Expected (Redump USA Track 1) ===");
    println!("  CRC32:     05be47b2");
    println!("  SHA1:      f967119e006695a59a6442237f9fc7c7811cf7bf");
    println!("  MD5:       acbb3a2e4a8f865f363dc06df147afa2");
    println!("  Data size: 538655040");
}

fn hash_range(file: &mut File, offset: u64, length: u64) {
    use sha1::Digest;

    file.seek(SeekFrom::Start(offset)).unwrap();

    let mut crc = crc32fast::Hasher::new();
    let mut sha = sha1::Sha1::new();
    let mut md5_ctx = md5::Context::new();

    let mut buf = [0u8; 64 * 1024];
    let mut remaining = length;

    while remaining > 0 {
        let to_read = remaining.min(buf.len() as u64) as usize;
        let n = file.read(&mut buf[..to_read]).unwrap();
        if n == 0 {
            break;
        }
        crc.update(&buf[..n]);
        sha.update(&buf[..n]);
        md5_ctx.consume(&buf[..n]);
        remaining -= n as u64;
    }

    println!("  CRC32:     {:08x}", crc.finalize());
    println!("  SHA1:      {:x}", sha.finalize());
    println!("  MD5:       {:x}", md5_ctx.compute());
    println!("  Data size: {}", length);
}
