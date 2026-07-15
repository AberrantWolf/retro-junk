# CHD (Compressed Hunks of Data)

Used by: [Sega Saturn](../consoles/Saturn_Overview.md), [PlayStation](../consoles/PSX_Overview.md), [PlayStation 2](../consoles/PS2_Overview.md), [Sega CD](../consoles/Genesis_Overview.md), [Dreamcast](../consoles/Dreamcast_Overview.md), [PSP](../consoles/PSP_Overview.md)

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

### DVD-media CHDs (`createdvd`) need a different hashing path

Verified 2026-07-15 while wiring PSP's `compute_container_hashes` (CHD
remediation spec, item C2), by reading the `chd` crate's header parser
(`chd-0.3.3/src/header.rs`, local vendored copy at
`~/.cargo/registry/src/.../chd-0.3.3/src/header.rs`) alongside the MAME CHD
spec (see Sources below):

- **CD-media CHDs** (`chdman createcd`) store one raw sector per unit:
  `unit_bytes` is 2352 (no subchannel, `SUBTYPE:NONE`) or 2448 (with 96-byte
  subchannel), i.e. `unit_bytes >= 2352`. They always carry `CHTR`/`CHT2`
  (`CD_ROM_TRACK`/`CD_ROM_TRACK2`) track metadata (see Track Metadata above).
- **DVD-media CHDs** (`chdman createdvd`, used for PS2 DVD games and PSP UMD
  dumps) store one **2048-byte** logical ISO 9660 sector per unit
  (`unit_bytes == 2048`, read directly from the v5 header per
  `header.rs:858` in the crate source — for legacy v3/v4 headers without an
  explicit field the crate falls back to `guess_unit_bytes`/`hunk_bytes`).
  DVD CHDs carry **no** `CD_ROM_TRACK`/`CD_ROM_TRACK2` metadata at all — the
  decompressed logical stream (truncated to the header's `logical_bytes`,
  since hunks are padded to `hunk_size`) *is* the ISO image byte-for-byte,
  with no sync/header/ECC/subchannel to strip.

Before this fix, `retro-junk-disc::hash::hash_chd_raw_sectors` unconditionally
treated every CHD as CD media: with no track metadata it fell back to
"hash all sectors from sector 0" but still multiplied by the hardcoded raw
CD sector size (2352) and read a 2352-byte slice per unit regardless of the
header's actual `unit_bytes`. For a DVD CHD (`unit_bytes == 2048`) this read
past each unit's true boundary — silently mixing in the next sector's bytes
mid-hunk, or slicing out of bounds (panic) at the last unit of a hunk. It was
never exercised because no analyzer wired a CHD-producing DVD-media
extension into `compute_container_hashes` until PSP.

The fix: `hash_chd_raw_sectors` now checks `header.unit_bytes()` up front;
when it's below the raw CD sector size (2352) it delegates to
`hash_chd_whole_stream`, which just streams the decoded logical bytes with
no CD-specific extraction. This also silently fixes the same latent bug for
PS2's DVD (`iso`) CHD path, which shares this function via
`sony_disc::hash_disc_container` → `retro_junk_disc::hash::hash_disc_container`.

## Conversion Tool

`chdman` (part of MAME) converts between CHD and raw formats:

```bash
# BIN/CUE or GDI to CHD (CD media: PS1, Saturn, Sega CD, Dreamcast, PS2 CD games)
chdman createcd -i game.cue -o game.chd

# ISO to CHD (DVD media: PS2 DVD games, PSP UMD)
chdman createdvd -i game.iso -o game.chd

# CHD back to BIN/CUE (add -sb / --splitbin for one bin per track, chdman >= 0.264)
chdman extractcd -i game.chd -o game.cue
chdman extractdvd -i game.chd -o game.iso

# Verify CHD internal integrity (SHA-1 of stored data only — not a round-trip check)
chdman verify -i game.chd
```

Packages: `mame-tools` (Arch/Debian/Ubuntu/Fedora), `rom-tools` (Homebrew); ships with MAME on Windows.

### Round-trip behavior (verified empirically against chdman 0.288, 2026-07)

All of the following were confirmed byte-identical after `createcd`/`createdvd` →
`extractcd`/`extractdvd` round-trips with synthetic discs:

- **Redump multi-bin cue** (one file per track, audio pregap stored in-file as
  INDEX 00): `extractcd -sb` reproduces each track file exactly, and the
  regenerated cue preserves the INDEX 00/01 structure.
- **Single-bin multi-track cue**: plain `extractcd` reproduces the single bin
  exactly; `-sb` splits it into per-track files whose boundaries run from each
  track's first INDEX (00 if present) to the next track's first INDEX. Track
  content is identical regardless of source layout.
- **GDI track sets** (GD-ROM, incl. the LBA-45000 high-density gap): extracting
  with a `.gdi` output path writes a GDI + per-track files, byte-identical to
  the source tracks with LBA/type layout preserved.
- **ISO via createdvd/extractdvd**: byte-identical when the ISO is a multiple
  of 2048 bytes (Redump ISOs are).

Track boundary rule (matches `retro-junk-disc::track_layout`): a track's data
runs from its first INDEX to the next track's first INDEX or end of file —
in-file pregap data belongs to the *following* track, as Redump distributes it.

Other chdman behaviors worth knowing:
- Progress goes to **stderr** as `\r`-separated updates:
  `Compressing, 45.6% complete... (ratio=41.2%)` / `Extracting, 12.0% complete...`;
  banner and error lines are `\n`-terminated. Version is in the banner line
  (`... manager 0.288 (mame0288-dirty)`).
- Without `--force`, chdman refuses to overwrite an existing output (exit 1).
- A cue referencing missing files fails with `ERROR: couldn't find bin file [...]`
  (exit 1), but a *malformed* cue (bad INDEX times) can make chdman derive an
  absurd logical size instead of failing fast — validate inputs first.
- CHD logical size counts 2448-byte frames (2352 + 96 subcode) and tracks are
  padded to 4-frame boundaries inside the CHD; FRAMES metadata holds the true
  track lengths, so extraction is unaffected.

`retro-junk-lib::chd_convert` wraps all of this: chdman detection,
`createcd`/`createdvd` selection via `RomAnalyzer::chd_media_for_extension()`,
progress parsing, and full round-trip verification (extract + per-track span
hash comparison) before any source deletion.

## Rust Crate

The `chd` crate (v0.3) provides full read support:
- Supports all v5 compression codecs (LZMA, Zstd, FLAC, Deflate, Huffman)
- On-demand hunk decompression (lazy — only decompresses requested hunks)
- Metadata access (track layout, codec info)
- Pure Rust dependencies (`flate2`, `lzma-rs`, `ruzstd`, `claxon`)

**No Rust crate can write/create CHDs.** chd-rs is read-only and its
maintainers state there are no plans for write support
(https://github.com/SnowflakePowered/chd-rs). CHD creation must shell out to
`chdman`.

## Sources
- MAME CHD documentation: https://docs.mamedev.org/techspecs/chd_spec.html
- chdman usage: https://wiki.recalbox.com/en/tutorials/utilities/rom-conversion/chdman
- Rust `chd` crate: https://crates.io/crates/chd
- chd-rs read-only status: https://github.com/SnowflakePowered/chd-rs
- verifydump (CHD verification tool): https://github.com/j68k/verifydump
- Round-trip behavior: local experiments with chdman 0.288 (mame-tools, Arch), 2026-07;
  reproduced by `retro-junk-lib/src/tests/chd_convert_tests.rs`
- DVD-media (`createdvd`) `unit_bytes`/lack-of-track-metadata behavior: read
  directly from the vendored `chd` crate source (v0.3.3,
  `~/.cargo/registry/src/index.crates.io-.../chd-0.3.3/src/header.rs` and
  `src/metadata.rs`), cross-referenced against the MAME CHD spec above;
  verified 2026-07-15 while implementing CHD remediation spec item C2
  (`docs/chd-remediation-spec.md`).
