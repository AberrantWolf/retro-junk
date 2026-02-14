# Sega Master System ROM Format

Used by: [Sega Master System](../consoles/MasterSystem_Overview.md)

## File Extensions
- `.sms` - Sega Master System ROMs
- `.gg` - Game Gear ROMs (same format)

## Header Format (0x7FF0-0x7FFF)

The Master System uses a 16-byte header located at offset 0x7FF0 (can also be at 0x1FF0 or 0x3FF0, but 0x7FF0 is standard).

| Offset | Size | Field |
|--------|------|-------|
| 0x7FF0 | 8 | TMR SEGA signature (ASCII "TMR SEGA") |
| 0x7FF8 | 2 | Reserved space (usually 0x00 0x00, 0xFF 0xFF, or 0x20 0x20) |
| 0x7FFA | 2 | Checksum (little-endian) |
| 0x7FFC | 2.5 | Product code (BCD format) |
| 0x7FFE | 0.5 | Version number (low 4 bits) |
| 0x7FFF | 0.5 | Region code (high 4 bits) |
| 0x7FFF | 0.5 | ROM size (low 4 bits) |

## Detection Method

1. Check for "TMR SEGA" ASCII string at offset 0x7FF0
2. Verify header is present at 0x7FF0, 0x3FF0, or 0x1FF0
3. Export Master System BIOS requires valid header to boot

## Region Codes (0x7FFF high 4 bits)

| Value | System/Region |
|-------|---------------|
| 0x3 | SMS Japan |
| 0x4 | SMS Export |
| 0x5 | GG Japan |
| 0x6 | GG Export |
| 0x7 | GG International |

## ROM Size Codes (0x7FFF low 4 bits)

| Value | ROM Size |
|-------|----------|
| 0xC | 32KB |
| 0xE | 64KB |
| 0xF | 128KB |
| 0x0 | 256KB |
| 0x1 | 512KB |

## Sources

- [SMS Power ROM Header Documentation](https://www.smspower.org/Development/ROMHeader)
