# MasterSystem_Overview.md

# Sega Master System Data Storage Guide

## Console Overview
- **Release Dates**: Japan (October 20, 1985 as Mark III), North America (June 1986), Europe (1987)
- **Active Years**: 1985-1996 (longer in some regions)
- **Regional Variants**:
  - Mark III (Japan): Original version with different styling
  - Master System (International): Redesigned with improved controllers
  - Master System II: Compact redesign without card slot

## Storage Media
- **Cartridge Capacity**: Up to 4 Mbit (512KB)
- **Sega Card Capacity**: Up to 256 Kbit (32KB)
- **Typical Sizes**: 1-4 Mbit cartridges, 32-256 Kbit cards
- **Save Storage**: Battery-backed SRAM in some cartridges

## Archival Storage
### Recommended Formats
- **.sms**: Standard Master System ROM format
- **.sc**: Sega Card format
- **Raw binary**: Headerless ROM dumps

### Best Practices
- Preserve both cartridge and card games
- Include save data when applicable
- Document regional differences
- Archive different ROM revisions
- Preserve FM sound unit compatibility info

## Emulation Storage
### Recommended Formats
- **.sms**: Universal emulator compatibility
- **.zip**: Compressed format for space saving
- **.sc**: For Sega Card games

### Considerations
- Simple 8-bit architecture, excellent emulation
- FM sound enhancement in some regions
- Save data stored separately when present
- Total library size: ~50MB (complete set)

## ROM Format Reference
See [MasterSystem.md](../formats/MasterSystem.md) for detailed header format, checksum algorithms, and detection method.

## Digital Storage Considerations
- **Space Requirements**: Minimal - entire library very small
- **Backup Strategy**: Easy to backup completely
- **Organization**: Separate cartridges from cards
- **Metadata**: Use No-Intro database for verification
- **Compression**: Significant space savings possible
- **Emulation**: Perfect preservation through emulation
