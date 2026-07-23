# Microsoft Xbox (Original) Data Storage Guide

## Console Overview
- **Release Dates**: North America (November 15, 2001), Japan (February 22, 2002), Europe (March 14, 2002)
- **Active Years**: 2001-2009
- **Regional Variants**:
  - Different case colors (black, crystal, limited editions)
  - Regional lockout via dashboard and game regions
  - Xbox Live service integration (2002-2010)

## Storage Media
- **Disc Capacity**: 4.7GB DVD-5 (single layer), 8.5GB DVD-9 (dual layer)
- **Internal Storage**: 8GB/10GB internal hard drive
- **Save Storage**: Internal HDD, memory units (8MB)
- **Disc Format**: Custom Xbox Game Disc (XGD) format

## Archival Storage
### Recommended Formats
- **Redump-style full-disc image**: Preservation master containing both disc partitions
- **.xiso**: Xbox filesystem image suitable for emulation; see [XISO](../formats/XISO.md)
- **Extracted folders**: For modded Xbox file structure

### Best Practices
- Preserve complete disc images with all regions
- Include hard drive contents and save data
- Archive Xbox Live downloadable content
- Document dashboard versions and modifications
- Preserve BIOS and kernel files

## Emulation Storage
### Recommended Formats
- **.xiso.iso**: Required by xemu; this is an XISO despite its `.iso` extension
- **HDD images**: Complete system preservation

### Considerations
- Xemu emulator in active development
- Redump-style full-disc ISOs are not directly supported and must be converted to XISO
- CHD is not supported by xemu
- Requires Xbox BIOS files for emulation
- Save data stored on virtual hard drive
- Xbox Live features not emulated
- Total library size: ~2TB uncompressed

## Digital Storage Considerations
- **Space Requirements**: High - large game files
- **Backup Strategy**: Important for complete preservation
- **Organization**: Separate by region and disc type
- **Metadata**: Use Redump database for verification
- **Legal Issues**: BIOS files required but copyrighted
- **Emulation**: Improving but still developing

## References

- [xemu disc image documentation](https://xemu.app/docs/disc-images/)
- [xemu format FAQ](https://xemu.app/docs/faq/)
