# Super Nintendo Entertainment System (SNES) ROM Format

## File Extensions
- `.sfc` - Super Famicom ROMs (most common)
- `.smc` - Super Magicom ROMs (common)
- `.fig` - Pro Fighter ROMs
- `.swc` - Super Wild Card ROMs
- `.ufo` - UFO ROMs

## Copier Header Detection

SNES ROMs may have an optional 512-byte copier header at the beginning of the file.

**Detection Method:**
- If `file_size % 1024 == 512`, the ROM likely has a copier header
- Header detection should try offsets with and without the 512-byte offset

## Internal ROM Header Format (0xFFB0-0xFFDF)

The internal ROM header is 48 bytes located at SNES address $00:FFB0-$00:FFDF.

**Physical ROM Offsets:**
- **LoROM:** 0x7FC0 (or 0x81C0 with copier header)
- **HiROM:** 0xFFC0 (or 0x101C0 with copier header)

| Offset | Size | Field |
|--------|------|-------|
| 0xFFB0 | 2 | Maker Code (ASCII) |
| 0xFFB2 | 4 | Game Code (ASCII) |
| 0xFFB6 | 7 | Fixed Value (should be 0x00) |
| 0xFFBD | 1 | Expansion RAM Size |
| 0xFFBE | 1 | Special Version |
| 0xFFBF | 1 | Cartridge Type (Sub-number) |
| 0xFFC0 | 21 | Game Title (JIS X 0201 encoding) |
| 0xFFD5 | 1 | Map Mode |
| 0xFFD6 | 1 | Cartridge Type |
| 0xFFD7 | 1 | ROM Size |
| 0xFFD8 | 1 | RAM Size |
| 0xFFD9 | 1 | Destination Code |
| 0xFFDA | 1 | Fixed Value (should be 0x33) |
| 0xFFDB | 1 | Mask ROM Version |
| 0xFFDC | 2 | Complement Check (little-endian) |
| 0xFFDE | 2 | Checksum (little-endian) |

## Detection Method

1. **Try multiple header locations:**
   - 0x7FC0 (LoROM)
   - 0x81C0 (LoROM with copier header)
   - 0xFFC0 (HiROM)
   - 0x101C0 (HiROM with copier header)

2. **Validate header structure:**
   - Fixed value at 0xFFB6-0xFFBC should be 0x00
   - Fixed value at 0xFFDA should be 0x33
   - Map mode should be valid (see table below)
   - Checksum validation

3. **Reset vector validation:**
   - Reset vector at 0xFFFC should be ≥ 0x8000

## Map Mode Codes (0xFFD5)

| Value | Description |
|-------|-------------|
| 0x20 | 2.68MHz LoROM |
| 0x21 | 2.68MHz HiROM |
| 0x23 | SA-1 |
| 0x25 | 2.68MHz ExHiROM |
| 0x30 | 3.58MHz LoROM |
| 0x31 | 3.58MHz HiROM |
| 0x35 | 3.58MHz ExHiROM |

## Cartridge Type Codes (0xFFD6)

| Value | Description |
|-------|-------------|
| 0x00 | ROM only |
| 0x01 | ROM + RAM |
| 0x02 | ROM + RAM + Battery |
| 0x33 | ROM + SA-1 |
| 0x34 | ROM + SA-1 + RAM |
| 0x35 | ROM + SA-1 + RAM + Battery |

## ROM Size Codes (0xFFD7)

Formula: `2^value` KB

| Value | ROM Size |
|-------|----------|
| 0x08 | 256 KB |
| 0x09 | 512 KB |
| 0x0A | 1 MB |
| 0x0B | 2 MB |
| 0x0C | 4 MB |

## RAM Size Codes (0xFFD8)

Formula: `2^value` KB (maximum 0x07 = 128KB)

| Value | RAM Size |
|-------|----------|
| 0x00 | None |
| 0x01 | 2 KB |
| 0x02 | 4 KB |
| 0x03 | 8 KB |
| 0x04 | 16 KB |
| 0x05 | 32 KB |
| 0x06 | 64 KB |
| 0x07 | 128 KB |

**Exception:** For Super FX (GSU-1), move RAM size to Expansion RAM Size field and set this to 0x00.

## Destination Codes (0xFFD9)

| Value | Region |
|-------|--------|
| 0x00 | Japan |
| 0x01 | USA |
| 0x02 | Europe (enables 50fps PAL mode) |

## Checksum Calculation

### Checksum (0xFFDE)
16-bit sum of all bytes in the ROM.

**Algorithm:**
1. Set checksum bytes to 0x00 0x00 0xFF 0xFF before calculation
2. Sum all bytes in ROM as 16-bit little-endian words
3. For non-power-of-2 ROMs, use mirroring algorithm

### Complement Check (0xFFDC)
16-bit bitwise complement (inverse) of the checksum.

**Verification:** `checksum + complement_check` should equal 0xFFFF.

## CPU Exception Vectors (0xFFE0-0xFFFF)

Located after the ROM header.

| Offset | Size | Mode | Vector |
|--------|------|------|--------|
| 0xFFE4 | 2 | Native | COP |
| 0xFFE6 | 2 | Native | BRK |
| 0xFFE8 | 2 | Native | ABORT |
| 0xFFEA | 2 | Native | NMI |
| 0xFFEE | 2 | Native | IRQ |
| 0xFFF4 | 2 | Emulation | COP |
| 0xFFF8 | 2 | Emulation | ABORT |
| 0xFFFA | 2 | Emulation | NMI |
| 0xFFFC | 2 | Emulation | RESET |
| 0xFFFE | 2 | Emulation | IRQ/BRK |

## Game Title Encoding

- **Character Set:** JIS X 0201 (ASCII + Katakana)
- **Length:** 21 bytes
- **Padding:** Unused bytes filled with spaces (0x20)

## Special Cases

- **BS-X Flash Cartridge:** Game code starts with 'Z' and ends with 'J'
- **Enhancement Chips:** SA-1 and Super FX can override CPU vectors
- **Memory Mapping:** LoROM uses 32KB banks, HiROM provides linear access

## Sources
- [SNESdev Wiki ROM File Formats](https://snes.nesdev.org/wiki/ROM_file_formats)
- [SNESLab ROM Header Documentation](https://sneslab.net/wiki/SNES_ROM_Header)
- [SNES Reader GitHub Implementation](https://github.com/Shobon03/snes-reader)
