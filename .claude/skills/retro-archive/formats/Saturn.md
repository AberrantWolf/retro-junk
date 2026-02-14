# Sega Saturn Disc Format

## File Extensions
- `.bin/.cue` - Binary disc image with cue sheet
- `.iso` - ISO disc image
- `.chd` - Compressed Hunks of Data (modern format)
- `.mdf/.mds` - Media Descriptor File format

## Disc Format Structure

Saturn uses CD-ROM XA format compliant with Semi CD-ROM XA standard.

| Area | Description |
|------|-------------|
| System Area | Boot information and copy protection |
| Volume Descriptors | ISO 9660 filesystem information |
| Data Area | Game files and data |

## Detection Method

1. Check for ISO 9660 volume descriptor at sector
2. Look for Saturn-specific boot files in system area
3. Verify CD-ROM XA format compliance
4. Check for Saturn game executable files

## CD-ROM XA Sector Format

- **Mode 2/Form 1:** 2048 bytes data + error correction
- **Mode 2/Form 2:** 2324 bytes data (for streaming audio/video)
- **Subheader:** File/channel information for interleaving

## Boot System

Saturn uses a specific boot sequence defined in the disc format standards, with boot files located in the system area of the disc.

## Sources
- [Sega Saturn Developer Documentation](https://docs.exodusemulator.com/Archives/SSDDV25/segahtml/prgg/sofg/disc/hon/p01_10.htm)
- [Saturn Technical Bulletins](https://segaretro.org/images/c/c7/ST-TECH.pdf)
