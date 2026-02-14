# Sega Saturn Disc Format

Used by: [Sega Saturn](../consoles/Saturn_Overview.md)

## File Extensions
- `.bin/.cue` - Binary disc image with cue sheet (preferred for accuracy)
- `.iso` - ISO disc image (loses audio tracks)
- `.chd` - Compressed Hunks of Data (modern format, preserves all tracks)
- `.mdf/.mds` - Media Descriptor File format

## IP.BIN Header (Initial Program)

The Saturn boot header is located at the start of Track 1 data (byte offset 0x0000 in the data track). It occupies the first 256 bytes of the system area.

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0x000 | 16 | Hardware ID | `SEGA SEGASATURN ` (16 chars, space-padded) |
| 0x010 | 16 | Maker ID | Company identifier (e.g., `SEGA ENTERPRISES`) |
| 0x020 | 10 | Product Number | Serial number (e.g., `MK-81009  `) |
| 0x02A | 6 | Version | Version string (e.g., `V1.000`) |
| 0x030 | 8 | Release Date | Date string `YYYYMMDD` |
| 0x038 | 8 | Device Info | Peripheral codes (see below) |
| 0x040 | 10 | Compatible Area | Region codes: `J`=Japan, `T`=Asia, `U`=North America, `B`=Brazil, `K`=Korea, `A`=Asia PAL, `E`=Europe, `L`=Latin America |
| 0x04A | 6 | Reserved | Padding |
| 0x050 | 16 | Compatible Peripheral | Peripheral compatibility flags |
| 0x060 | 112 | Game Name | Title string (space-padded) |
| 0x0D0 | 4 | IP Size | Size of Initial Program in bytes |
| 0x0D4 | 4 | Stack-M | Master SH2 stack pointer |
| 0x0D8 | 4 | Stack-S | Slave SH2 stack pointer |
| 0x0DC | 4 | 1st Read Addr | Load address for first executable |
| 0x0E0 | 4 | 1st Read Size | Size of first executable |
| 0x0E4 | 28 | Reserved | Padding to 0x100 |

All multi-byte values are big-endian.

### Device Info Codes
- `J` - Standard controller
- `M` - Analog controller / mission stick
- `A` - Mouse
- `G` - Light gun
- `K` - Keyboard
- `T` - Multitap
- `S` - Steering wheel
- `E` - RAM cartridge

## Detection Method

1. Read first 16 bytes of the data track
2. Check for the magic string `SEGA SEGASATURN ` at offset 0x000
3. If using BIN/CUE, parse the CUE sheet to find Track 1 data offset (skip 16-byte sync/header for Mode 1 sectors at raw offset 0x010)
4. For ISO images, the header starts at byte 0x000

## CD-ROM XA Sector Format

- **Mode 2/Form 1:** 2048 bytes data + error correction
- **Mode 2/Form 2:** 2324 bytes data (for streaming audio/video)
- **Raw sector:** 2352 bytes (sync + header + data + ECC/EDC)
- **Subheader:** File/channel information for interleaving

## Boot System

1. BIOS reads IP.BIN from the system area
2. Validates hardware ID string
3. Checks region compatibility against console region
4. Loads the Initial Program (boot code) from IP.BIN
5. IP loads the first read file (main executable) to the specified address
6. Execution begins at the entry point

## Sources
- [Sega Saturn Developer Documentation](https://docs.exodusemulator.com/Archives/SSDDV25/segahtml/prgg/sofg/disc/hon/p01_10.htm)
- [Saturn Technical Bulletins](https://segaretro.org/images/c/c7/ST-TECH.pdf)
- [Sega Retro - Saturn Header](https://segaretro.org/Sega_Saturn_header)
