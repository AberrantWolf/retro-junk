# iNES / NES 2.0 File Format

Used by: [NES / Famicom](../consoles/NES_Overview.md)

## Overview

The iNES format is the standard file format for NES/Famicom ROM dumps. NES 2.0 is a backward-compatible extension that encodes additional cartridge hardware details. Both use the `.nes` file extension.

## Header Layout (16 bytes)

| Byte | iNES 1.0 | NES 2.0 |
|------|----------|---------|
| 0-3  | `NES\x1A` magic (`4E 45 53 1A`) | Same |
| 4    | PRG ROM size in 16 KB units | PRG ROM size LSB |
| 5    | CHR ROM size in 8 KB units (0 = CHR RAM) | CHR ROM size LSB |
| 6    | Flags 6 | Same |
| 7    | Flags 7 | Same (with NES 2.0 ID in bits 2-3) |
| 8    | PRG RAM size (rarely used) | Mapper MSB [3:0] / Submapper [7:4] |
| 9    | TV system (bit 0: 0=NTSC, 1=PAL) | PRG-ROM MSB [3:0] / CHR-ROM MSB [7:4] |
| 10   | Unused (0) | PRG-RAM shift [3:0] / PRG-NVRAM shift [7:4] |
| 11   | Unused (0) | CHR-RAM shift [3:0] / CHR-NVRAM shift [7:4] |
| 12   | Unused (0) | CPU/PPU Timing (0=NTSC, 1=PAL, 2=Multi, 3=Dendy) |
| 13   | Unused (0) | VS System type / Extended console type |
| 14   | Unused (0) | Miscellaneous ROMs (lower 2 bits) |
| 15   | Unused (0) | Default Expansion Device (lower 6 bits) |

## Flags 6 (Byte 6)

```
76543210
||||||||
||||+--- Mirroring: 0=horizontal, 1=vertical
|||+---- Battery-backed PRG RAM present
||+----- 512-byte trainer at $7000-$71FF
|+------ Four-screen VRAM
+------- Lower nibble of mapper number (bits 0-3)
```

(Bits 4-7 form the lower nibble of the mapper number)

## Flags 7 (Byte 7)

```
76543210
||||||||
||||||+- VS Unisystem
|||||+-- PlayChoice-10
||||+--- NES 2.0 identifier (bits 2-3: if == 0b10, this is NES 2.0)
+------- Upper nibble of mapper number (bits 4-7)
```

## NES 2.0 Identification

Check bits 2-3 of byte 7: if the value is `0b10` (decimal 2), the header uses NES 2.0 format. All other values indicate iNES 1.0.

## Mapper Number Construction

- **iNES 1.0**: `(byte7 >> 4) << 4 | (byte6 >> 4)` = 0-255
- **NES 2.0**: Adds `(byte8 & 0x0F) << 8` for mapper range 0-4095

## NES 2.0 RAM Size Encoding

Bytes 10-11 use a logarithmic shift encoding for RAM sizes:
- If shift value is 0: size is 0 (no RAM)
- Otherwise: size = `64 << shift` bytes
- Shift 7 = 8 KB, Shift 8 = 16 KB, etc.

## NES 2.0 ROM Size (Exponent-Multiplier)

When MSB nibble is 0x0F, the corresponding LSB byte uses exponent-multiplier notation:
- `size = (1 << (byte >> 2)) * ((byte & 0x03) * 2 + 1)`

## File Structure (after header)

1. **Trainer** (512 bytes) - only if bit 2 of byte 6 is set
2. **PRG ROM** data
3. **CHR ROM** data (absent if CHR ROM size is 0)

## Sources

- [NESdev Wiki - iNES](https://www.nesdev.org/wiki/INES)
- [NESdev Wiki - NES 2.0](https://www.nesdev.org/wiki/NES_2.0)
