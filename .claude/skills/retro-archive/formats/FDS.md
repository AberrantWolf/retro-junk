# FDS (Famicom Disk System) File Format

Used by: [NES / Famicom](../consoles/NES_Overview.md)

## Overview

FDS disk images store Famicom Disk System game data. Two variants exist:

- **Headered (.fds)**: 16-byte fwNES header followed by raw disk side data
- **Headerless (.fds)**: Raw disk side data with no header

FDS was Japan-only. Each disk is double-sided, with 65500 bytes per side in the .fds image format (gaps and CRCs from the physical media are omitted).

## fwNES Header (16 bytes, optional)

| Byte | Description |
|------|-------------|
| 0-3  | Magic: `FDS\x1A` (`46 44 53 1A`) |
| 4    | Number of disk sides |
| 5-15 | Reserved (zero) |

## Disk Info Block (56 bytes per side)

Each disk side begins with a disk info block:

| Offset | Size | Description |
|--------|------|-------------|
| 0      | 1    | Block type (`0x01`) |
| 1-14   | 14   | Verification string: `*NINTENDO-HVC*` |
| 15     | 1    | Manufacturer code |
| 16-19  | 4    | Game name (3-letter code + version) |
| 20     | 1    | Game type |
| 21     | 1    | Revision number |
| 22     | 1    | Side number (0=A, 1=B) |
| 23     | 1    | Disk number |
| 24     | 1    | Disk type |
| 25     | 1    | Unknown |
| 26     | 1    | Boot file ID |
| 27-30  | 4    | Unknown |
| 31-33  | 3    | Manufacturing date (BCD: year, month, day) |
| 34-40  | 7    | Unknown |
| 41     | 1    | Rewrite count |
| 42-45  | 4    | Unknown |
| 46-48  | 3    | Rewrite date (BCD: year, month, day) |
| 49-55  | 7    | Unknown/reserved |

## Notable Manufacturer Codes

| Code | Manufacturer |
|------|-------------|
| 0x01 | Nintendo |
| 0x08 | Capcom |
| 0xA4 | Konami |
| 0xAF | Namco |
| 0xB2 | Bandai |
| 0xB6 | HAL Laboratory |
| 0xBB | Sunsoft |
| 0xC0 | Taito |
| 0xC3 | Square |
| 0xC5 | Data East |

## Detection

- Headered: first 4 bytes are `FDS\x1A`
- Headerless: first byte is `0x01` followed by `*NINTENDO-HVC*`
- Side count from headerless images: `file_size / 65500`

## Sources

- [NESdev Wiki - FDS file format](https://www.nesdev.org/wiki/FDS_file_format)
- [NESdev Wiki - FDS disk format](https://www.nesdev.org/wiki/FDS_disk_format)
