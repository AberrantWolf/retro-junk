# NES_Famicom_Overview.md

# Nintendo Entertainment System / Famicom Data Storage Guide

## Console Overview
- **Release Dates**: Japan (July 15, 1983 as Famicom), North America (October 18, 1985), Europe (1986-1987)
- **Active Years**: 1983-1995 (officially discontinued 2003)
- **Regional Variants**: 
  - Famicom (Japan): Red/white design, hardwired controllers, expansion audio
  - NES (International): Gray design, removable controllers, no expansion audio
  - Various regional lockout chips (10NES, CIC)

## Storage Media
- **Cartridge Capacity**: 32KB typical, up to 1MB with memory mappers
- **Typical ROM Sizes**: 16KB-512KB for most games
- **Storage Technology**: ROM chips with optional battery-backed SRAM for saves

## Archival Storage
### Recommended Formats
- **.nes**: Standard iNES format with 16-byte header
- **.unf**: UNIF format for complex mappers and homebrew
- **Raw binary dumps**: For preservation without headers

### Best Practices
- Preserve original ROM dumps with correct headers
- Document mapper information and PCB details
- Include save data (.sav files) when applicable
- Store multiple regional variants when available

## Emulation Storage
### Recommended Formats
- **.nes**: Most compatible with emulators
- **.zip**: Compressed format supported by modern emulators
- **.7z**: Alternative compression with better ratios

### Considerations
- Header information crucial for proper emulation
- Some homebrew requires UNIF format
- Battery saves typically stored as separate .sav files
- Total library size: ~237MB (complete North American set)

## Digital Storage Considerations
- **Space Requirements**: Minimal - entire library fits on small USB drive
- **Backup Strategy**: Multiple copies recommended due to small size
- **Organization**: Sort by region, then alphabetically
- **Metadata**: Include game database information (No-Intro DAT files)
- **Compression**: ZIP compression saves ~30-50% space with no performance impact
