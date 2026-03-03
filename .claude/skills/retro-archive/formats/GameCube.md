# Nintendo GameCube Disc Format

Used by: [Nintendo GameCube](../consoles/GameCube_Overview.md)

## File Extensions
- `.iso` - Standard disc image (identical to GCM)
- `.gcm` - GameCube disc image (identical to ISO)
- `.rvz` - Dolphin's compressed format
- `.ciso` - Compact ISO compressed format
- `.gcz` - GameCube compressed format
- `.tgc` - Demo disc format

## Full Disc Size

A standard GameCube disc is exactly **1,459,978,240 bytes** (approx 1.4 GB mini-DVD).

## Disc Header Format (0x0000-0x043F) - "boot.bin"

All multi-byte integers are **big-endian**.

| Offset | Size | Field |
|--------|------|-------|
| 0x0000 | 4 | Game Code (Console ID + Game Code + Country) |
| 0x0004 | 2 | Maker Code |
| 0x0006 | 1 | Disk ID |
| 0x0007 | 1 | Version |
| 0x0008 | 1 | Audio Streaming |
| 0x0009 | 1 | Stream Buffer Size |
| 0x000A | 14 | Unused (zeros) |
| 0x0018 | 4 | Wii Magic Word (0x5D1C9EA3 if Wii; zero on GameCube) |
| 0x001C | 4 | DVD Magic Word (0xC2339F3D) |
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

This header layout is shared identically with [Wii discs](Wii.md). The magic words at 0x0018 and 0x001C are what distinguish them.

### Game Code Breakdown (4 bytes at 0x0000)

| Byte | Meaning | Examples |
|------|---------|----------|
| 0 | Console ID | `G` = GameCube game, `D` = GameCube demo, `U` = GameCube utility |
| 1-2 | Short game code | `AL` (Zelda), `M4` (Metroid Prime) |
| 3 | Country | `E` = USA, `J` = Japan, `P` = Europe, `K` = Korea |

Full example: `GALE` = GameCube game (`G`), game code `AL`, USA (`E`).

### Country Byte → Region Mapping

| Code | Region |
|------|--------|
| `E` | USA (NTSC-U) |
| `J` | Japan (NTSC-J) |
| `P` | Europe (PAL) |
| `D` | Germany (PAL) |
| `F` | France (PAL) |
| `S` | Spain (PAL) |
| `I` | Italy (PAL) |
| `U` | Australia (PAL) |
| `L`, `M` | Japanese import to Europe (PAL) |
| `K` | Korea |
| `Q` | Korea (English title) |
| `W` | World |

### Maker Code (2 bytes at 0x0004)

Standard Nintendo 2-character licensee code. Uses the same table as GBA, DS, and 3DS cartridges (e.g., `"01"` = Nintendo R&D1, `"08"` = Capcom, `"41"` = Ubi Soft).

## Detection Method

1. Check for DVD Magic Word `0xC2339F3D` at offset 0x001C (big-endian)
2. Verify Wii Magic Word at offset 0x0018 is **NOT** `0x5D1C9EA3` (to distinguish from Wii discs)
3. Optionally verify file size is exactly 1,459,978,240 bytes for uncompressed GCM/ISO

This two-magic-word check is important because Wii discs may also have the GameCube magic word at 0x001C for backwards compatibility. Checking that 0x0018 is NOT the Wii magic prevents misidentification.

## Hashing Convention (Redump)

GameCube uses **Redump** for DAT matching (disc-based console):
- **DAT name:** `Nintendo - GameCube`
- **Redump system slug:** `gc` (download URL: `http://redump.org/datfile/gc/serial,version`)
- **Hash method:** Hash the full ISO/GCM file directly (CRC32, MD5, SHA1). No header stripping or sector conversion needed — the disc image is already in the correct format (2048-byte DVD sectors).
- **Serial in DAT:** Redump DATs use full product codes (e.g., `DL-DOL-GALE-0-USA`). The 4-byte game code from the disc header (e.g., `GALE`) is matched against DAT serials via sub-segment indexing in the matcher — works with both short codes and full product codes like `DL-DOL-GALE-0-USA`.

Compressed formats (RVZ, CISO, GCZ) require decompression before hashing to match Redump checksums.

## Disc Structure

| Offset | Size | Component |
|--------|------|-----------|
| 0x0000 | 0x0440 | Disk header ("boot.bin") |
| 0x0440 | 0x2000 | Disk header info ("bi2.bin") |
| 0x2440 | Variable | Apploader ("appldr.bin") |
| Variable | Variable | FST ("fst.bin") |
| Variable | Variable | Game files |

## Sources

- [Yet Another GameCube Documentation](https://www.gc-forever.com/yagcd/chap13.html)
- [YAGCD File Formats](https://hitmen.c02.at/files/yagcd/yagcd/chap14.html)
- [Wiibrew Wii Disc format](https://wiibrew.org/wiki/Wii_disc) (shared header layout)
