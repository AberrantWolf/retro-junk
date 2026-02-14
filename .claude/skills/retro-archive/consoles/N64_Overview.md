# Nintendo64_Overview.md

# Nintendo 64 Data Storage Guide

## Console Overview
- **Release Dates**: Japan (June 23, 1996), North America (September 29, 1996), Europe (March 1, 1997)
- **Active Years**: 1996-2002
- **Regional Variants**:
  - Different cartridge shapes prevent cross-region play
  - PAL regions had slower refresh rates affecting some games
  - Japanese market had unique colors and exclusive titles

## Storage Media
- **Cartridge Capacity**: 4MB to 64MB (512 Mbit maximum)
- **Typical ROM Sizes**: 8MB-32MB for most games
- **Storage Technology**: ROM cartridges with optional Controller Pak saves
- **Save Methods**: Internal EEPROM, SRAM, or Flash memory

## Archival Storage
### Recommended Formats
- **.z64**: Big-endian format (recommended for preservation)
- **.v64**: Little-endian format (alternative)
- **.n64**: Byte-swapped format (less common)

### Best Practices
- Use .z64 format for consistency and compatibility
- Preserve save data separately (.eep, .sra, .fla files)
- Document cartridge board information
- Include Controller Pak saves when applicable
- Maintain regional separation due to timing differences

## Emulation Storage
### Recommended Formats
- **.z64**: Best compatibility across emulators
- **.zip**: Compressed format supported by most emulators
- **.7z**: Higher compression alternative

### Considerations
- Endianness matters for some emulators
- Save types must be correctly identified
- Some games require specific emulator settings
- Total library size: ~5.5GB (complete international set)

## ROM Format Reference
See [N64.md](../formats/N64.md) for detailed header format, checksum algorithms, and detection method.

## Digital Storage Considerations
- **Space Requirements**: Moderate - manageable on modern storage
- **Backup Strategy**: Multiple backups recommended
- **Organization**: Sort by region, then by save type
- **Metadata**: Use GoodN64 or No-Intro databases
- **Compression**: Significant space savings with ZIP/7z
- **Compatibility**: Verify endianness for target emulator
