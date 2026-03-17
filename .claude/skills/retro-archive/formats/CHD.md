# CHD (Compressed Hunks of Data)

Used by: [Sega Saturn](../consoles/Saturn_Overview.md), [PlayStation](../consoles/PSX_Overview.md), [PlayStation 2](../consoles/PS2_Overview.md), [Sega CD](../consoles/Genesis_Overview.md)

## Overview

CHD (Compressed Hunks of Data) is a lossless compressed disc image format created by the MAME project. It stores complete disc images including all raw sector data and subchannel information, making it suitable for preservation while achieving significant compression.

CHD is not a native dump format — it is always a conversion from BIN/CUE, GDI, or other raw disc images. The `chdman` tool (part of MAME) handles conversion in both directions.

## File Extension
- `.chd` — Compressed Hunks of Data

## Magic Bytes
- `MComprHD` (8 bytes at offset 0)

## Structure

### Header
- **Magic**: 8 bytes (`MComprHD`)
- **Header length**: 4 bytes
- **Version**: 4 bytes (current is v5)
- **Compressors**: codec identifiers for up to 4 codecs
- **Logical bytes**: total uncompressed size
- **Hunk size**: size of each compressed chunk (typically 19,584 bytes for CD = 8 sectors)
- **Unit size**: smallest addressable unit
- **SHA1**: hash of the raw data for verification

### Hunk Map
Maps logical hunks to compressed data offsets. Each hunk can use a different codec.

### Compressed Hunks
The actual compressed data, stored sequentially. Each hunk is independently decompressible.

## CD-ROM Sector Layout

For CD-ROM disc images, CHD stores sectors as:
- **2352 bytes**: raw sector data (sync + header + user data + ECC/EDC)
- **96 bytes**: subchannel data
- **Total**: 2448 bytes per sector

This means the logical size of a CHD CD image = `total_sectors * 2448`.

## Compression Codecs (v5)

| Codec | ID | Description |
|-------|----|-------------|
| None | `none` | Uncompressed |
| LZMA | `lzma` | LZMA compression |
| Deflate | `zlib` | Zlib/deflate compression |
| FLAC | `flac` | FLAC audio compression |
| Huffman | `huff` | Huffman coding |
| Zstandard | `zstd` | Zstandard compression |
| CD LZMA | `cdlz` | CD-optimized LZMA |
| CD Deflate | `cdzl` | CD-optimized deflate |
| CD FLAC | `cdfl` | CD-optimized FLAC |
| CD Zstandard | `cdzs` | CD-optimized Zstandard |
| AV Huffman | `avhu` | Audio/video Huffman |

CD-optimized codecs separate raw sector data from subchannel data before compression for better ratios.

## Track Metadata

CHD stores track layout as text metadata entries with tags:
- `CHTR` (CdRomTrack) — legacy format
- `CHT2` (CdRomTrack2) — current format

Metadata text format:
```
TRACK:1 TYPE:MODE1_RAW SUBTYPE:NONE FRAMES:150000 PREFRAMES:150
TRACK:2 TYPE:AUDIO SUBTYPE:NONE FRAMES:18995 PREFRAMES:150
```

Fields:
- `TRACK`: track number (1-based)
- `TYPE`: `MODE1_RAW`, `MODE2_RAW`, `AUDIO`
- `SUBTYPE`: `NONE`, `RW`, `RW_RAW`
- `FRAMES`: number of sectors in this track
- `PREFRAMES`: pregap sectors

## Sector Mode Notes

Different systems use different CD-ROM sector modes:
- **Mode 1** (Saturn, Sega CD): 12 sync + 4 header + 2048 data + 288 ECC/EDC
  - User data offset within raw sector: **16 bytes**
- **Mode 2 Form 1** (PlayStation): 12 sync + 4 header + 8 subheader + 2048 data + 280 ECC/EDC
  - User data offset within raw sector: **24 bytes**

When reading user data from CHD, the correct offset depends on the system.

## Hashing for DAT Verification

Redump DATs store checksums of **raw 2352-byte sectors** (without subchannel data). To verify a CHD against Redump:
1. Decompress each hunk
2. Extract 2352-byte raw sector data (strip 96-byte subchannel)
3. Hash only Track 1 sectors (data track)
4. Compare CRC32/SHA1/MD5 against Redump DAT entry

The `retro-junk-disc` crate's `hash_chd_raw_sectors()` function implements this.

## Conversion Tool

`chdman` (part of MAME) converts between CHD and raw formats:

```bash
# BIN/CUE to CHD
chdman createcd -i game.cue -o game.chd

# CHD to BIN/CUE
chdman extractcd -i game.chd -o game.cue

# Verify CHD integrity
chdman verify -i game.chd
```

## Rust Crate

The `chd` crate (v0.3) provides full read support:
- Supports all v5 compression codecs (LZMA, Zstd, FLAC, Deflate, Huffman)
- On-demand hunk decompression (lazy — only decompresses requested hunks)
- Metadata access (track layout, codec info)
- Pure Rust dependencies (`flate2`, `lzma-rs`, `ruzstd`, `claxon`)

## Sources
- MAME CHD documentation: https://docs.mamedev.org/techspecs/chd_spec.html
- chdman usage: https://wiki.recalbox.com/en/tutorials/utilities/rom-conversion/chdman
- Rust `chd` crate: https://crates.io/crates/chd
- verifydump (CHD verification tool): https://github.com/j68k/verifydump
