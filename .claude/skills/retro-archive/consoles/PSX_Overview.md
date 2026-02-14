# PlayStation_PSX_Overview.md

# Sony PlayStation (PSX) Data Storage Guide

## Console Overview
- **Release Dates**: Japan (December 3, 1994), North America (September 9, 1995), Europe (September 29, 1995)
- **Active Years**: 1994-2006
- **Regional Variants**:
  - NTSC-J (Japan/Asia): Different BIOS and game compatibility
  - NTSC-U (North America): Region locked to US/Canada titles
  - PAL (Europe/Australia): 50Hz refresh rate, different timing

## Storage Media
- **Disc Capacity**: 650MB CD-ROM
- **Disc Format**: CD-DA with data tracks for mixed audio/data
- **Save Storage**: Memory Cards (128KB, 15 blocks)
- **Multi-disc Games**: Up to 4 discs for larger games

## Archival Storage
### Recommended Formats
- **.bin/.cue**: Complete disc image with audio tracks
- **.iso**: Single-track data-only games
- **.chd**: Compressed Hunks of Data (MAME format)

### Best Practices
- Use BIN/CUE for games with CD audio tracks
- Preserve subchannel data for copy protection
- Include memory card saves (.mcr format)
- Document disc revisions and regional differences
- Archive multi-disc games as complete sets

## Emulation Storage
### Recommended Formats
- **.chd**: Best compression and compatibility
- **.pbp**: PSP-style compressed format
- **.bin/.cue**: Universal compatibility

### Considerations
- Audio tracks essential for many games
- Memory card files stored separately
- BIOS files required for emulation
- Multi-disc games need disc swapping support
- Total library size: ~400GB uncompressed

## ROM Format Reference
See [PSX.md](../formats/PSX.md) for detailed header format, checksum algorithms, and detection method.

## Digital Storage Considerations
- **Space Requirements**: Moderate to high
- **Backup Strategy**: Important for disc preservation
- **Organization**: Group multi-disc games together
- **Metadata**: Use Redump database for verification
- **Compression**: CHD offers excellent compression ratios
- **Audio Quality**: Preserve CD audio at original quality
