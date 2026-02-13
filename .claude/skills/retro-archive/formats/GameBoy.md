# Game Boy / Game Boy Color ROM Format

## File Extensions
- `.gb` - Game Boy ROMs
- `.gbc` - Game Boy Color ROMs
- `.sgb` - Super Game Boy enhanced ROMs

## Header Format (0x0100-0x014F)

GB and GBC share the same 80-byte header. The CGB flag byte at 0x0143 determines whether a ROM is DMG-only, CGB-compatible, or CGB-exclusive.

| Offset | Size | Field |
|--------|------|-------|
| 0x0100 | 4 | Entry point (typically NOP + JP nn) |
| 0x0104 | 48 | Nintendo logo (verified by boot ROM) |
| 0x0134 | 15 | Title (may be shorter if CGB flag present) |
| 0x013F | 4 | Manufacturer code (CGB only, overlaps title) |
| 0x0143 | 1 | CGB flag (0x80=compatible, 0xC0=CGB-only) |
| 0x0144 | 2 | New licensee code (ASCII, used when old=0x33) |
| 0x0146 | 1 | SGB flag (0x03=SGB features) |
| 0x0147 | 1 | Cartridge type (MBC + features) |
| 0x0148 | 1 | ROM size code (32KB << N) |
| 0x0149 | 1 | RAM size code |
| 0x014A | 1 | Destination code (0=Japan, 1=International) |
| 0x014B | 1 | Old licensee code (0x33=use new code at 0x0144) |
| 0x014C | 1 | Mask ROM version |
| 0x014D | 1 | Header checksum |
| 0x014E | 2 | Global checksum (big-endian) |

## Detection Method

1. File must be at least 336 bytes (0x0150)
2. Read 48 bytes at offset 0x0104
3. Compare against the known Nintendo logo bytes

The Nintendo logo is the primary detection signature. The boot ROM on real hardware also verifies this logo and will refuse to boot if it doesn't match.

## Title Field

- **DMG-only ROMs** (CGB flag != 0x80/0xC0): Title is 16 bytes at 0x0134-0x0143
- **CGB ROMs** (CGB flag == 0x80/0xC0): Title is 11 bytes at 0x0134-0x013E; bytes 0x013F-0x0142 are the manufacturer code, and 0x0143 is the CGB flag

## Checksum Algorithms

### Header Checksum (0x014D)
Verified by the boot ROM on real hardware. A bad header checksum will prevent the game from booting.

```
x = 0
for byte in rom[0x0134..=0x014C]:
    x = x - byte - 1  (wrapping u8 arithmetic)
result = x
```

### Global Checksum (0x014E-0x014F)
Sum of all bytes in the entire ROM file, excluding the two global checksum bytes themselves. Stored as big-endian u16. **Not verified by hardware** - purely informational for dump verification.

```
sum = 0
for (i, byte) in rom.enumerate():
    if i != 0x014E and i != 0x014F:
        sum = sum + byte  (wrapping u16 arithmetic)
result = sum
```

## ROM Size Codes (0x0148)

| Code | Size |
|------|------|
| 0x00 | 32 KB (no banking) |
| 0x01 | 64 KB (4 banks) |
| 0x02 | 128 KB (8 banks) |
| 0x03 | 256 KB (16 banks) |
| 0x04 | 512 KB (32 banks) |
| 0x05 | 1 MB (64 banks) |
| 0x06 | 2 MB (128 banks) |
| 0x07 | 4 MB (256 banks) |
| 0x08 | 8 MB (512 banks) |

Formula: `32768 << code` bytes.

## RAM Size Codes (0x0149)

| Code | Size |
|------|------|
| 0x00 | None |
| 0x01 | Unused |
| 0x02 | 8 KB (1 bank) |
| 0x03 | 32 KB (4 banks) |
| 0x04 | 128 KB (16 banks) |
| 0x05 | 64 KB (8 banks) |

## Common Cartridge Types (0x0147)

| Code | Type |
|------|------|
| 0x00 | ROM ONLY |
| 0x01 | MBC1 |
| 0x03 | MBC1+RAM+BATTERY |
| 0x05 | MBC2 |
| 0x06 | MBC2+BATTERY |
| 0x0F | MBC3+TIMER+BATTERY |
| 0x10 | MBC3+TIMER+RAM+BATTERY |
| 0x13 | MBC3+RAM+BATTERY |
| 0x19 | MBC5 |
| 0x1B | MBC5+RAM+BATTERY |
| 0x1C | MBC5+RUMBLE |
| 0x1E | MBC5+RUMBLE+RAM+BATTERY |
| 0x22 | MBC7+SENSOR+RUMBLE+RAM+BATTERY |
| 0xFC | POCKET CAMERA |
| 0xFE | HuC3 |
| 0xFF | HuC1+RAM+BATTERY |

## Licensee Codes

- If old licensee code (0x014B) is 0x33, use the 2-character ASCII new licensee code at 0x0144-0x0145
- Otherwise, use the old licensee code byte directly
- CGB-era games typically use the new licensee code system
