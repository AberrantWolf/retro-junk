# Nintendo GameCube Disc Format

## File Extensions
- `.iso` - Standard disc image
- `.gcm` - GameCube disc image (1.4GB exactly)
- `.rvz` - Dolphin's compressed format
- `.tgc` - Demo disc format

## Disc Header Format (0x0000-0x043F) - "boot.bin"

| Offset | Size | Field |
|--------|------|-------|
| 0x0000 | 4 | Game Code (Console ID + Game Code + Country) |
| 0x0004 | 2 | Maker Code |
| 0x0006 | 1 | Disk ID |
| 0x0007 | 1 | Version |
| 0x0008 | 1 | Audio Streaming |
| 0x0009 | 1 | Stream Buffer Size |
| 0x000A | 18 | Unused (zeros) |
| 0x001C | 4 | DVD Magic Word (0xC2339F3D) |
| 0x0020 | 992 | Game Name |
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

## Detection Method

1. Check for DVD Magic Word (0xC2339F3D) at offset 0x001C
2. Verify file size is exactly 1,459,978,240 bytes for GCM format
3. Check for valid Game Code format
4. Verify FST and DOL offsets point to valid data

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
