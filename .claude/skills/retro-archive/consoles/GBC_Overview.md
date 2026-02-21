# Nintendo Game Boy Color Data Storage Guide

## Console Overview
- **Release Dates**: Japan (October 21, 1998), North America/Europe (November 1998)
- **Active Years**: 1998-2003
- **Regional Variants**:
  - Multiple colors available (purple, red, blue, yellow, green, etc.)
  - Infrared communication port for select games
  - Backwards compatible with original Game Boy games

## Storage Media
- **Cartridge Capacity**: Up to 8MB maximum capacity
- **Typical ROM Sizes**: 1MB-8MB for Color-only games
- **Storage Technology**: ROM with enhanced Memory Bank Controllers
- **Game Types**: 
  - Color-enhanced (black cartridges): Full color on GBC, grayscale on GB
  - Color-only (clear cartridges): GBC exclusive games

## Archival Storage
### Recommended Formats
- **.gbc**: Game Boy Color ROM format
- **.gb**: For backwards-compatible games
- **Raw binary**: Headerless dumps for preservation

### Best Practices
- Distinguish between color-enhanced and color-only games
- Preserve infrared communication data when applicable
- Include save data and real-time clock data
- Document cartridge type (black vs. clear)
- Archive different regional releases

## Emulation Storage
### Recommended Formats
- **.gbc**: Preferred for Color-specific games
- **.gb**: For backwards-compatible titles
- **.zip**: Compressed format for space saving

### Considerations
- Enhanced graphics and sound over original Game Boy
- Real-time clock support in some games (Pokémon Gold/Silver)
- Infrared features rarely emulated
- Total library size: ~1.5GB (complete set)

## ROM Format Reference
See [GameBoy.md](../formats/GameBoy.md) for detailed header format, checksum algorithms, and detection method. GB and GBC share the same header format at 0x0100-0x014F, differing only in the CGB flag byte at 0x0143.

## Digital Storage Considerations
- **Space Requirements**: Low - manageable collection size
- **Backup Strategy**: Easy to preserve completely
- **Organization**: Separate color-only from enhanced games
- **Metadata**: Use No-Intro database for verification
- **Compression**: Excellent space savings with ZIP/7z
- **Emulation**: Well-supported by multiple mature emulators
