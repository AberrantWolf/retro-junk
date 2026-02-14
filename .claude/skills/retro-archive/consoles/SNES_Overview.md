# Super Nintendo Entertainment System / Super Famicom Data Storage Guide

## Console Overview
- **Release Dates**: Japan (November 21, 1990), North America (August 23, 1991), Europe (April 11, 1992)
- **Active Years**: 1990-2003
- **Regional Variants**:
  - Super Famicom (Japan): Colorful design, different cartridge shape
  - SNES (North America): Purple/gray design, different controller colors
  - Super Nintendo (Europe): Rainbow logo, same as North American hardware
  - Regional lockout via cartridge shape and CIC chips

## Storage Media
- **Cartridge Capacity**: 256KB to 6MB (48 Mbit maximum)
- **Typical ROM Sizes**: 512KB-4MB for most games
- **Special Chips**: DSP, SuperFX, SA-1 enhancement chips in cartridges
- **Storage Technology**: ROM with optional battery-backed SRAM/FRAM

## Archival Storage
### Recommended Formats
- **.sfc**: Super Famicom format (no header)
- **.smc**: Super MagiCom format (512-byte header)
- **Raw binary**: Headerless dumps for preservation

### Best Practices
- Preserve both headered and headerless versions
- Document special chip information
- Include save data and real-time clock data when applicable
- Maintain separate regional collections
- Archive MSU-1 audio tracks for enhanced versions

## Emulation Storage
### Recommended Formats
- **.sfc**: Preferred by most modern emulators
- **.smc**: Legacy format, still widely supported
- **.zip**: Compressed format for space saving

### Considerations
- Header vs. headerless affects some emulators
- Special chip games require accurate emulation
- MSU-1 games need accompanying audio files
- Total library size: ~1.7GB (complete set)

## ROM Format Reference
See [SNES.md](../formats/SNES.md) for detailed header format, checksum algorithms, and detection method.

## Digital Storage Considerations
- **Space Requirements**: Moderate - fits easily on modern storage
- **Backup Strategy**: Regular backups recommended
- **Organization**: Separate folders for regions and special chip games
- **Metadata**: Use No-Intro or GoodSNES databases
- **Compression**: ZIP saves significant space, 7z even more
- **Special Handling**: MSU-1 games require folder structure preservation
