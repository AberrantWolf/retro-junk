# Nintendo Wii Disc Format

Used by: [Nintendo Wii](../consoles/Wii_Overview.md)

## File Extensions
- `.iso` - Standard disc image
- `.wbfs` - Wii Backup File System
- `.rvz` - Dolphin's compressed format
- `.ciso` - Compact ISO compressed format
- `.wia` - Wii ISO Archive

## Disc Sizes

- **Single-layer (DVD-5):** up to 4.7 GB (4,700,000,000 bytes)
- **Dual-layer (DVD-9):** up to 8.5 GB

The layer type can be determined from file size (>4.7 GB = dual-layer).

## Disc Header Format (0x0000-0x043F) - "boot.bin"

The Wii disc header is **identical in layout** to the [GameCube disc header](GameCube.md). All multi-byte integers are **big-endian**.

| Offset | Size | Field |
|--------|------|-------|
| 0x0000 | 4 | Game Code (Console ID + Game Code + Country) |
| 0x0004 | 2 | Maker Code |
| 0x0006 | 1 | Disk ID |
| 0x0007 | 1 | Version |
| 0x0008 | 1 | Audio Streaming |
| 0x0009 | 1 | Stream Buffer Size |
| 0x000A | 14 | Unused (zeros) |
| 0x0018 | 4 | Wii Magic Word (0x5D1C9EA3) |
| 0x001C | 4 | GameCube Magic Word (0xC2339F3D; may or may not be present) |
| 0x0020 | 992 | Game Name (null-terminated ASCII) |
| 0x0400 | 4 | Debug monitor offset |
| 0x0404 | 4 | Debug monitor load address |
| 0x0408 | 24 | Unused (zeros) |
| 0x0420 | 4 | Main executable DOL offset |
| 0x0424 | 4 | FST (File System Table) offset |
| 0x0428 | 4 | FST size |
| 0x042C | 4 | Maximum FST size |
| 0x0430 | 4 | User position |
| 0x0434 | 4 | User length |
| 0x0438 | 8 | Reserved |

### Game Code Breakdown (4 bytes at 0x0000)

| Byte | Meaning | Examples |
|------|---------|----------|
| 0 | Console ID | `R` = Wii game, `S` = Wii (later titles) |
| 1-2 | Short game code | `SB` (Wii Sports), `MG` (Mario Galaxy) |
| 3 | Country | `E` = USA, `J` = Japan, `P` = Europe, `K` = Korea |

Full example: `RSBE` = Wii game (`R`), game code `SB`, USA (`E`) — Wii Sports.

### Country Byte → Region Mapping

Same mapping as GameCube — see [GameCube.md](GameCube.md#country-byte--region-mapping).

### Maker Code (2 bytes at 0x0004)

Same 2-character licensee table as GameCube and other Nintendo platforms.

## Detection Method

1. Check for Wii Magic Word `0x5D1C9EA3` at offset 0x0018 (big-endian)
2. If present → this is a Wii disc (regardless of whether GameCube magic is also at 0x001C)

The detection is simpler than GameCube because the Wii magic at 0x0018 is definitive. A GameCube disc will never have this value at 0x0018.

## Hashing Convention (Redump)

Wii uses **Redump** for DAT matching (disc-based console):
- **DAT name:** `Nintendo - Wii`
- **Redump system slug:** `wii` (download URL: `http://redump.org/datfile/wii/serial,version`)
- **Hash method:** Hash the full ISO file directly (CRC32, MD5, SHA1). No header stripping needed.
- **Serial in DAT:** Redump DATs use full product codes. The 4-byte game code from the disc header (e.g., `RSBE`) can be used for DAT serial matching.

Compressed formats (WBFS, RVZ, CISO, WIA) require decompression before hashing.

## Wii-Specific Disc Structure

Beyond the shared boot.bin header, Wii discs have additional structure not present on GameCube:

- **Partition table** at offset 0x40000 — describes game, update, and channel partitions
- **Encrypted partitions** — game data is AES-encrypted per partition
- **Update partition** — contains system menu updates

For basic identification, only the unencrypted disc header (boot.bin) needs to be read. Partition parsing is needed for accessing game data within the encrypted partitions.

## Sources

- [Wiibrew Wii Disc format](https://wiibrew.org/wiki/Wii_disc)
- [Yet Another GameCube Documentation](https://www.gc-forever.com/yagcd/chap13.html) (shared header layout)
