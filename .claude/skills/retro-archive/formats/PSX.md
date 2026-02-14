# Sony PlayStation 1 CD-ROM Format

## File Extensions
- `.bin/.cue` - Binary disc image with cue sheet
- `.iso` - ISO 9660 disc image
- `.img` - Generic disc image
- `.chd` - Compressed format

## Disc Format Structure

PlayStation 1 uses ISO 9660 filesystem with CD-XA (Mode2) sectors.

| Sector | Content |
|--------|---------|
| 0-3 | Zerofilled (Mode2/Form1) |
| 4 | License String |
| 5-11 | PlayStation Logo (3278h bytes) |
| 12-15 | Zerofilled (Mode2/Form2) |
| 16 | Primary Volume Descriptor |
| 17+ | Volume Descriptor Set Terminator |

## License String (Sector 4)

| Offset | Size | Content |
|--------|------|---------|
| 0x000 | 32 | " Licensed  by " |
| 0x020 | 38 | Region-specific Sony Computer Entertainment text |

## Primary Volume Descriptor (Sector 16)

| Offset | Size | Field |
|--------|------|-------|
| 0x000 | 1 | Volume Descriptor Type (0x01) |
| 0x001 | 5 | Standard Identifier ("CD001") |
| 0x006 | 1 | Volume Descriptor Version (0x01) |
| 0x008 | 32 | System Identifier ("PLAYSTATION") |
| 0x028 | 32 | Volume Identifier |
| 0x400 | 8 | CD-XA Signature ("CD-XA001") |

## Detection Method

1. Check for "CD001" at offset 0x001 in sector 16
2. Verify "PLAYSTATION" system identifier at offset 0x008
3. Look for "CD-XA001" signature at offset 0x400
4. Check for PlayStation logo in sectors 5-11
5. Verify SCEx copy protection string in wobble (hardware level)

## CD-XA Sector Formats

- **Mode2/Form1:** 2048 bytes data + error correction
- **Mode2/Form2:** 2324 bytes data (for streaming)
- **Subheader:** File, Channel, Submode, Coding info

## Sources

- [PSX-SPX CD-ROM Format](https://psx-spx.consoledev.net/cdromformat/)
