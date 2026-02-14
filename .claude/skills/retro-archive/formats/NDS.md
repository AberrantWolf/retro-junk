# Nintendo DS ROM Format

## File Extensions
- `.nds` - Standard NDS ROMs
- `.dsi` - DSi-enhanced or DSi-only ROMs

## Header Format (0x000–0x1FF)

The NDS cartridge header is 512 bytes (0x200) at the start of the ROM. All multi-byte integers are **little-endian**.

### Core Identification (0x000–0x01F)

| Offset | Size | Field |
|--------|------|-------|
| 0x000 | 12 | Game title (uppercase ASCII, null-padded) |
| 0x00C | 4 | Game code (ASCII: category + ID + region) |
| 0x010 | 2 | Maker code (ASCII, 2-char licensee code) |
| 0x012 | 1 | Unit code (0x00=NDS, 0x02=NDS+DSi, 0x03=DSi only) |
| 0x013 | 1 | Encryption seed select (0x00–0x07) |
| 0x014 | 1 | Device capacity (ROM size = 128 KB << n) |
| 0x015 | 7 | Reserved (zeros) |
| 0x01C | 1 | Reserved |
| 0x01D | 1 | NDS region (0x00=normal, 0x40=Korea, 0x80=China) |
| 0x01E | 1 | ROM version |
| 0x01F | 1 | Autostart flags |

### ARM9 Binary (0x020–0x02F)

| Offset | Size | Field |
|--------|------|-------|
| 0x020 | 4 | ARM9 ROM offset |
| 0x024 | 4 | ARM9 entry address |
| 0x028 | 4 | ARM9 RAM address |
| 0x02C | 4 | ARM9 size |

### ARM7 Binary (0x030–0x03F)

| Offset | Size | Field |
|--------|------|-------|
| 0x030 | 4 | ARM7 ROM offset |
| 0x034 | 4 | ARM7 entry address |
| 0x038 | 4 | ARM7 RAM address |
| 0x03C | 4 | ARM7 size |

### File System Tables (0x040–0x05F)

| Offset | Size | Field |
|--------|------|-------|
| 0x040 | 4 | File Name Table (FNT) offset |
| 0x044 | 4 | FNT size |
| 0x048 | 4 | File Allocation Table (FAT) offset |
| 0x04C | 4 | FAT size |
| 0x050 | 4 | ARM9 overlay offset |
| 0x054 | 4 | ARM9 overlay size |
| 0x058 | 4 | ARM7 overlay offset |
| 0x05C | 4 | ARM7 overlay size |

### Port Settings, Icon, and Security (0x060–0x07F)

| Offset | Size | Field |
|--------|------|-------|
| 0x060 | 4 | Normal command port setting |
| 0x064 | 4 | KEY1 command port setting |
| 0x068 | 4 | Icon/Title (banner) offset |
| 0x06C | 2 | Secure area checksum (CRC-16 of 0x020–0x7FFF) |
| 0x06E | 2 | Secure area delay |
| 0x070 | 4 | ARM9 auto-load hook address |
| 0x074 | 4 | ARM7 auto-load hook address |
| 0x078 | 8 | Secure area disable |

### Size Fields and Nintendo Logo (0x080–0x15F)

| Offset | Size | Field |
|--------|------|-------|
| 0x080 | 4 | Total used ROM size |
| 0x084 | 4 | ROM header size (always 0x4000) |
| 0x088 | 56 | Reserved / DSi fields |
| 0x0C0 | 156 | Nintendo logo (identical to GBA logo) |
| 0x15C | 2 | Logo checksum (CRC-16, always 0xCF56) |
| 0x15E | 2 | Header checksum (CRC-16 of 0x000–0x15D) |

### Debug ROM Info (0x160–0x1FF)

| Offset | Size | Field |
|--------|------|-------|
| 0x160 | 4 | Debug ROM offset |
| 0x164 | 4 | Debug size |
| 0x168 | 4 | Debug RAM address |
| 0x16C | 148 | Reserved (zeros) |

## Detection Method

1. File must be at least 512 bytes (0x200)
2. Read 156 bytes at offset 0xC0
3. Compare against the known Nintendo logo bytes (same as GBA)
4. Optionally verify logo checksum at 0x15C equals 0xCF56

The Nintendo logo at 0xC0 is the **same 156-byte bitmap** used in GBA ROMs (at 0x04). The BIOS verifies both the logo data and its checksum on boot.

## Game Code (0x00C, 4 bytes)

The game code is 4 ASCII characters: `UTTD`

- **U** (byte 1): Category prefix
  - `A`, `B`, `C` — standard NDS retail games
  - `D` — DSi-exclusive
  - `H`, `K` — DSiWare
  - `I`, `V` — DSi-enhanced titles
- **TT** (bytes 2–3): Short title identifier
- **D** (byte 4): Region/destination code

### Region Codes (4th character)
| Code | Region |
|------|--------|
| J | Japan |
| E | USA |
| P | Europe |
| D | Germany (→ Europe) |
| F | France (→ Europe) |
| S | Spain (→ Europe) |
| I | Italy (→ Europe) |
| K | Korea |
| C | China |
| U | Australia (→ Europe/PAL) |
| A | Region-free |
| W | Worldwide |

The serial number format is `NTR-XXXX` for NDS titles and `TWL-XXXX` for DSi-enhanced or DSi-only titles.

## Unit Code (0x012)

| Value | Meaning |
|-------|---------|
| 0x00 | NDS only |
| 0x02 | NDS + DSi enhanced |
| 0x03 | DSi only / DSiWare |

## Device Capacity (0x014)

The ROM chip size is calculated as: **128 KB × 2^n**

| Value | ROM Size |
|-------|----------|
| 6 | 8 MB |
| 7 | 16 MB |
| 8 | 32 MB |
| 9 | 64 MB |
| 10 | 128 MB |
| 11 | 256 MB |
| 12 | 512 MB |

In code: `128 * 1024 * (1 << n)` or equivalently `131072 << n`.

The `Total Used ROM Size` field at 0x080 gives the actual content size within the capacity. Remaining bytes are padded with 0xFF.

## CRC-16 Checksums

NDS uses CRC-16 with polynomial 0x8005, reflected (bit-reversed), initial value 0xFFFF.

### Logo Checksum (0x15C)
- Covers: bytes 0x0C0–0x15B (156 bytes, the Nintendo logo)
- Always equals **0xCF56** for valid ROMs (since the logo data is fixed)

### Header Checksum (0x15E)
- Covers: bytes 0x000–0x15D (350 bytes)
- Primary integrity check for the header

### Secure Area Checksum (0x06C)
- Covers: bytes 0x4000–0x7FFF (the 16 KB secure area)
- **Important:** The stored CRC is over the *encrypted* form of the secure area. Virtually all ROM dumps have the secure area decrypted (identifiable by magic bytes `E7 FF DE FF E7 FF DE FF` at offset 0x4000). Verifying this CRC on decrypted dumps would require re-encrypting with BIOS Blowfish keys, which is impractical. Only verify on the rare encrypted dumps.
- Requires reading 16 KB beyond the header; skip in quick mode

### Secure Area Detection
The first 8 bytes at offset 0x4000 indicate the secure area state:
- `E7 FF DE FF E7 FF DE FF` — Decrypted dump (standard, all common dumpers produce this)
- Any other bytes — Encrypted (original cartridge form, rare in the wild)
- arm9_rom_offset < 0x4000 — No secure area (homebrew)

## NDS Region Byte (0x01D)

| Value | Meaning |
|-------|---------|
| 0x00 | Normal / no restriction |
| 0x40 | Korea |
| 0x80 | China |

## DSi Extended Header (0x180+)

Only relevant when unit_code is 0x02 or 0x03:

| Offset | Size | Field |
|--------|------|-------|
| 0x1B0 | 4 | DSi region flags (bitmask) |
| 0x1C0 | 12 | ARM9i ROM offset, load addr, size |
| 0x1D0 | 12 | ARM7i ROM offset, load addr, size |
| 0x230 | 8 | Title ID |

### DSi Region Flags (0x1B0, bitmask)

| Bit | Region |
|-----|--------|
| 0x01 | Japan |
| 0x02 | USA |
| 0x04 | Europe |
| 0x08 | Australia |
| 0x10 | China |
| 0x20 | Korea |
| 0xFFFFFFFF | Region-free |

## Maker Codes

Same 2-character ASCII licensee code table as GBA. Common codes:

| Code | Publisher |
|------|-----------|
| 01 | Nintendo R&D1 |
| 08 | Capcom |
| 13 | EA |
| 31 | Nintendo |
| 34 | Konami |
| 41 | Ubi Soft |
| 51 | Acclaim |
| 52 | Activision |
| 69 | EA |
| 78 | THQ |

## Source

- [GBATEK DS Cartridge Header](https://problemkaputt.de/gbatek-ds-cartridge-header.htm)
- [RetroReversing DS File Formats](https://www.retroreversing.com/DSFileFormats)
