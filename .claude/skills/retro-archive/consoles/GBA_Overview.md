# Nintendo Game Boy Advance Data Storage Guide

## Console Overview
- **Release Dates**: Japan (March 21, 2001), North America/Europe (June 2001)
- **Active Years**: 2001-2010
- **Regional Variants**:
  - Game Boy Advance (2001): Original horizontal design
  - Game Boy Advance SP (2003): Clamshell design with frontlight (AGS-001) / backlight (AGS-101)
  - Game Boy Micro (2005): Ultra-compact design, no GB/GBC compatibility

## Storage Media
- **Cartridge Capacity**: 4MB to 32MB typical, up to 128MB maximum
- **Typical ROM Sizes**: 4MB-32MB for most games
- **Storage Technology**: ROM with Flash memory for saves
- **Save Methods**: SRAM, EEPROM, or Flash memory
- **Backwards Compatibility**: Plays Game Boy and Game Boy Color games

## ROM Format
See [GBA Format Reference](../formats/GBA.md) for full header layout, checksum algorithm, game code/region mapping, and save type detection.

## Archival Storage
### Recommended Formats
- **.gba**: Standard Game Boy Advance ROM format
- **Raw binary**: Headerless dumps for preservation
- **.sav**: Save data files

### Best Practices
- Preserve complete ROM dumps with correct headers
- Include save data files (.sav format)
- Document save type (SRAM/EEPROM/Flash)
- Archive different regional versions
- Preserve homebrew and flash cart compatibility

## Emulation Storage
### Recommended Formats
- **.gba**: Universal emulator compatibility
- **.zip**: Compressed format supported by most emulators
- **.7z**: Higher compression alternative

### Considerations
- 32-bit ARM processor enables complex games
- Save data stored in separate files
- Real-time clock support in some games
- Link cable features require special emulator setup
- Total library size: ~24GB (complete set)

## Digital Storage Considerations
- **Space Requirements**: Moderate - fits on standard USB drive
- **Backup Strategy**: Regular backups recommended
- **Organization**: Separate by region and save type
- **Metadata**: Use No-Intro or GoodGBA databases
- **Compression**: Good space savings with ZIP compression
- **Emulation**: Well-supported by mGBA and other active emulators
