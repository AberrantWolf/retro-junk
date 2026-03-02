# RVZ (Revolution Compressed)

## Overview

RVZ is a compressed disc image format created by the Dolphin emulator team for GameCube and Wii disc images. It is the successor to the WIA (Wii ISO Archive) format and offers better compression ratios while maintaining the ability to losslessly reconstruct the original disc data.

## Key Properties

- **Platforms**: GameCube, Wii
- **Extension**: `.rvz`
- **Compression**: Lossless (full disc can be reconstructed)
- **Magic bytes**: Shares WIA header magic `WIA\x01` at offset 0 (RVZ is a WIA variant distinguished by internal metadata)
- **Created by**: Dolphin Emulator team

## Related Compressed Formats

All of these store GameCube/Wii disc data in compressed containers:

| Format | Extension | Magic Bytes | Notes |
|--------|-----------|-------------|-------|
| RVZ    | `.rvz`    | `WIA\x01`   | Best compression, recommended by Dolphin |
| WIA    | `.wia`    | `WIA\x01`   | Predecessor to RVZ |
| WBFS   | `.wbfs`   | `WBFS`      | Wii-only, strips unused sectors |
| CISO   | `.ciso`   | `CISO`      | Compact ISO, block-based |
| GCZ    | `.gcz`    | varies      | Dolphin's older compressed format |
| NKit   | `.nkit.*` | varies      | Lossy — removes junk/padding data, cannot match Redump hashes |

## Decompression

The `nod` crate (https://crates.io/crates/nod) by the Dolphin team handles transparent decompression of all supported formats. Key API:

- `nod::Disc::new(path)` — Opens any supported format, auto-detects container type
- `nod::Disc::detect(reader)` — Detects format from magic bytes without full open
- `disc.meta().format` — Returns the specific `nod::Format` variant (Rvz vs Wia vs Wbfs etc.)
- `disc.disc_size()` — Returns uncompressed disc size
- `nod::Disc` implements `Read + Seek` — reads decompressed disc data

## Implementation in retro-junk

Compressed format support lives in `retro-junk-nintendo/src/nintendo_disc.rs`:

- `is_compressed_disc(reader)` — Detects compressed container via `nod::Disc::detect()`
- `open_compressed_disc(path)` — Opens with nod, parses the decompressed disc header

Both `GameCubeAnalyzer` and `WiiAnalyzer` use these shared helpers. The decompressed data is passed to the same `parse_disc_header()` used for raw ISOs, so all header parsing and identification logic is reused.

**Status**: Identification works. Hashing for Redump DAT matching is deferred (requires passing `AnalysisOptions` through `compute_container_hashes()`).

## Sources

- Dolphin Emulator wiki: https://wiki.dolphin-emu.org/index.php?title=Ripping_Games
- nod crate: https://crates.io/crates/nod
- WIA/RVZ format specification (Dolphin source): https://github.com/dolphin-emu/dolphin
