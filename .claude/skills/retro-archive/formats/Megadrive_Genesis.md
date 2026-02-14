# Sega Megadrive / Genesis ROM Format

Used by: [Sega Genesis / Megadrive](../consoles/Genesis_Overview.md)

## File Extensions
- `.md` - Megadrive ROMs
- `.gen` - Genesis ROMs
- `.bin` - Generic binary format

## Header Format (0x0100-0x01FF)

The Megadrive header spans the 0x100-0x1FF range after the 68000 vectors.

| Offset | Size | Field |
|--------|------|-------|
| 0x0100 | 16 | System type (must start with "SEGA") |
| 0x0110 | 16 | Copyright and release date |
| 0x0120 | 48 | Game title (domestic) |
| 0x0150 | 48 | Game title (overseas) |
| 0x0180 | 14 | Serial number |
| 0x018E | 2 | ROM checksum (big-endian) |
| 0x0190 | 16 | Device support |
| 0x01A0 | 8 | ROM address range |
| 0x01A8 | 8 | RAM address range |
| 0x01B0 | 12 | Extra memory |
| 0x01BC | 12 | Modem support |
| 0x01C8 | 40 | Reserved (fill with spaces) |
| 0x01F0 | 3 | Region support |
| 0x01F3 | 13 | Reserved (fill with spaces) |

## Detection Method

1. **Critical:** Check for "SEGA" at start of system type field (0x0100)
2. Systems with TMSS (Trademark Security System) will refuse to boot without this
3. Typically contains "SEGA MEGA DRIVE" or "SEGA GENESIS"

## ROM Checksum

16-bit checksum calculated by summing all 16-bit words from 0x0200 to end of ROM, keeping only lower bits.

## Sources

- [Plutiedev ROM Header Reference](https://plutiedev.com/rom-header)
