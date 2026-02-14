# Game Boy Advance ROM Format

Used by: [Game Boy Advance](../consoles/GBA_Overview.md)

## File Extensions
- `.gba` - Standard GBA ROMs
- `.mb` - Multiboot ROMs

## Header Format (0x00-0xBF)

The GBA cartridge header is 192 bytes at the start of the ROM.

| Offset | Size | Field |
|--------|------|-------|
| 0x00 | 4 | Entry point (ARM branch instruction) |
| 0x04 | 156 | Nintendo compressed logo bitmap |
| 0xA0 | 12 | Game title (ASCII, null-padded) |
| 0xAC | 4 | Game code (ASCII: type + ID + region) |
| 0xB0 | 2 | Maker code (ASCII, 2-char licensee code) |
| 0xB2 | 1 | Fixed value (must be 0x96) |
| 0xB3 | 1 | Main unit code |
| 0xB4 | 1 | Device type |
| 0xB5 | 7 | Reserved (zeros) |
| 0xBC | 1 | Software version |
| 0xBD | 1 | Header complement checksum |
| 0xBE | 2 | Reserved (zeros) |

## Detection Method

1. File must be at least 192 bytes (0xC0)
2. Read 156 bytes at offset 0x04
3. Compare against the known Nintendo logo bytes
4. Verify fixed value 0x96 at offset 0xB2

The Nintendo logo is the primary detection signature. The fixed value provides additional confirmation. Checksum is **not** checked during detection — hacked ROMs may have valid logos but modified checksums.

## Game Code (0xAC, 4 bytes)

The game code is 4 ASCII characters:
- **Byte 1**: Game type (A=normal, B=some later games, etc.)
- **Bytes 2-3**: Short game identifier
- **Byte 4**: Region code

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

The serial number is formatted as `AGB-XXXX` where XXXX is the game code.

## Complement Checksum (0xBD)

The GBA BIOS verifies this checksum on boot. A bad checksum will show the "Nintendo" logo but freeze.

```
sum = 0
for byte in rom[0xA0..=0xBC]:
    sum = sum + byte  (wrapping u8 arithmetic)
checksum = (-sum - 0x19) & 0xFF
```

Equivalently: negate the sum, then subtract 0x19, all in wrapping u8 arithmetic.

## ROM Sizes

GBA ROMs have no size field in the header. ROM size is inferred from file size and expected to be a power of 2, from 256 KB to 32 MB. Common sizes:

| Size | Notes |
|------|-------|
| 256 KB | Small games |
| 512 KB | Common for simpler titles |
| 1 MB | |
| 2 MB | |
| 4 MB | Common |
| 8 MB | Common for larger titles |
| 16 MB | Large games |
| 32 MB | Maximum cartridge size |

## Save Types

GBA games use different save mechanisms. The save type is **not** stored in the header but can be detected by scanning the ROM data for magic strings:

| Magic String | Save Type | Size |
|---|---|---|
| `EEPROM_V` | EEPROM | 512B or 8KB |
| `SRAM_V` | SRAM | 32KB |
| `FLASH_V` | Flash | 64KB |
| `FLASH512_V` | Flash 512K | 64KB |
| `FLASH1M_V` | Flash 1M | 128KB |

These strings are embedded in the game's save library code. Scanning requires reading the full ROM (up to 32 MB), so this is skipped in quick analysis mode.

## Maker Codes

The 2-character ASCII maker code at 0xB0 uses the same licensee code table as Game Boy Color. Common codes:

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

## Sources

- [GBATEK Cartridge Header](http://problemkaputt.de/gbatek-gba-cartridge-header.htm)
- [GBATEK GitHub Mirror](https://mgba-emu.github.io/gbatek/)
