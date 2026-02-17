# Sony PlayStation 1 CD-ROM Format

Used by: [Sony PlayStation](../consoles/PSX_Overview.md)

## File Extensions

| Extension | Format | Description |
|-----------|--------|-------------|
| `.iso` | ISO 9660 | 2048 bytes/sector, user data only |
| `.bin` | Raw BIN | 2352 bytes/sector, full raw CD sectors |
| `.img` | Raw BIN | Same as `.bin` |
| `.cue` | CUE Sheet | Text file describing track layout, references `.bin` files |
| `.chd` | CHD | MAME Compressed Hunks of Data |

Not supported (need separate decompression): `.pbp` (PSP eboot), `.ecm` (ECM compressed).

## Disc Format Structure

PlayStation 1 uses ISO 9660 filesystem with CD-XA (Mode2) sectors.

| Sector | Content |
|--------|---------|
| 0-3 | Zerofilled (Mode2/Form1) |
| 4 | License String |
| 5-11 | PlayStation Logo (3278h bytes) |
| 12-15 | Zerofilled (Mode2/Form2) |
| 16 | Primary Volume Descriptor |
| 17+ | Volume Descriptor Set Terminator |

## Raw Sector Layout (Mode 2 Form 1)

A raw 2352-byte sector is laid out as:

| Offset | Size | Content |
|--------|------|---------|
| 0 | 12 | Sync pattern (`00 FF FF FF FF FF FF FF FF FF FF 00`) |
| 12 | 4 | Header (MSF + Mode byte) |
| 16 | 8 | Subheader (File, Channel, Submode, Coding info x2) |
| 24 | 2048 | **User data** |
| 2072 | 4 | EDC (Error Detection Code) |
| 2076 | 276 | ECC (Error Correction Code) |

Total: 12 + 4 + 8 + 2048 + 4 + 276 = **2352 bytes**

Key constant: user data starts at offset **24** within a raw sector.

## License String (Sector 4)

| Offset | Size | Content |
|--------|------|---------|
| 0x000 | 32 | " Licensed  by " |
| 0x020 | 38 | Region-specific Sony Computer Entertainment text |

## Primary Volume Descriptor (Sector 16)

All offsets are within the 2048-byte user data of sector 16.

| Offset | Size | Field |
|--------|------|-------|
| 0x000 | 1 | Volume Descriptor Type (0x01) |
| 0x001 | 5 | Standard Identifier ("CD001") |
| 0x006 | 1 | Volume Descriptor Version (0x01) |
| 0x008 | 32 | System Identifier ("PLAYSTATION") |
| 0x028 | 32 | Volume Identifier |
| 0x050 | 8 | Volume Space Size (both-endian u32, LE at +0, BE at +4) |
| 0x09C | 34 | Root Directory Record (extent LBA at +2 LE, data length at +10 LE) |
| 0x400 | 8 | CD-XA Signature ("CD-XA001") |

## Format Detection

The analyzer detects format in this order (implemented in `detect_disc_format()`):

1. Read first 16 bytes
2. **CHD**: bytes 0-7 match `MComprHD` magic
3. **Raw BIN**: bytes 0-11 match CD sync pattern (`00 FF...FF 00`)
4. **CUE Sheet**: first 512 bytes are printable text containing both `FILE` and `TRACK` keywords
5. **ISO 9660**: byte at offset `16*2048 + 1` starts a "CD001" signature
6. Otherwise: error (not recognized)

After format detection, PS1 identity is confirmed by checking the PVD system identifier starts with "PLAYSTATION".

## SYSTEM.CNF

The file `SYSTEM.CNF` in the root directory identifies the boot executable. Located by walking the ISO 9660 root directory (variable-length records, case-insensitive match, stripping `;1` version suffix).

```
BOOT = cdrom:\SLUS_012.34;1
VMODE = NTSC
```

- `BOOT` or `BOOT2` key provides the boot executable path
- The executable filename encodes the serial: `SLUS_012.34` → `SLUS-01234`
- Serial extraction: take the 4-letter prefix, then collect all digits after it, format as `PREFIX-DIGITS`
- `VMODE` indicates NTSC or PAL (optional)

### Serial Prefix to Region Mapping

| Prefix | Region | Description |
|--------|--------|-------------|
| SLUS, SCUS | USA | Licensed/first-party US releases |
| SLPS, SCPS, SLPM, SIPS | Japan | Licensed/first-party/promo Japanese releases |
| SLES, SCES, SCED | Europe | Licensed/first-party European releases |
| SLKA, SCKA | Korea | Korean releases |
| PAPX, PCPX | Japan | Dev/promo discs |

## CUE Sheet Analysis

CUE sheets are parsed as text, extracting:
- `FILE` entries (filename + type like BINARY)
- `TRACK` entries (number + mode like MODE2/2352 or AUDIO)

Track counts (total, data, audio) are reported. Data tracks have modes containing "MODE"; audio tracks have mode "AUDIO".

When `AnalysisOptions.file_path` is set, the analyzer resolves the first data track's BIN file relative to the CUE file's parent directory, opens it, and extracts the serial/region from its SYSTEM.CNF.

## CHD CD Sector Layout

CHD files storing CD data use **2448 bytes per sector**:
- 2352 bytes raw sector data
- 96 bytes subchannel data

Sectors are packed sequentially into compressed hunks. To read sector N:
- Byte offset = N x 2448
- Hunk number = byte_offset / hunk_size
- Offset within hunk = byte_offset % hunk_size
- User data starts at offset 24 within the raw sector portion

The analyzer reads CHD header metadata (version, hunk size, logical size) without decompression, then optionally decompresses sector 16 (PVD) and walks the filesystem to find SYSTEM.CNF.

## CD-XA Sector Formats

- **Mode2/Form1:** 2048 bytes data + error correction (used for filesystem data)
- **Mode2/Form2:** 2324 bytes data (used for streaming audio/video)
- **Subheader:** File, Channel, Submode, Coding info

## Analyzer Output

The PS1 analyzer populates `RomIdentification` as follows:

| Field | Source |
|-------|--------|
| `platform` | "PlayStation" |
| `serial_number` | Extracted from SYSTEM.CNF boot path (non-quick mode) |
| `internal_name` | Volume Identifier from PVD |
| `regions` | Derived from serial prefix (non-quick mode) |
| `file_size` | Actual file size on disk |
| `expected_size` | PVD volume_space_size x sector_size |

Extra fields vary by format:

| Key | When | Value |
|-----|------|-------|
| `format` | Always | "ISO 9660", "Raw BIN (2352)", "CUE Sheet", or "CHD" |
| `boot_path` | Non-quick, SYSTEM.CNF found | Full boot path from SYSTEM.CNF |
| `vmode` | Non-quick, VMODE present | "NTSC" or "PAL" |
| `total_tracks` | CUE format | Total track count |
| `data_tracks` | CUE format | Number of data tracks |
| `audio_tracks` | CUE format | Number of audio tracks |
| `bin_file` | CUE, single file | Referenced BIN filename |
| `bin_files` | CUE, multiple files | Comma-separated BIN filenames |
| `chd_version` | CHD format | e.g. "v5" |
| `chd_hunk_size` | CHD format | Hunk size in bytes |
| `chd_logical_size` | CHD format | Uncompressed logical size |

## DAT Support

- DAT name: `"Sony - PlayStation"` (Redump)
- Game code extraction: returns the full serial (e.g., `SLUS-01234`) since Redump DATs use full serials

## Sources

- [PSX-SPX CD-ROM Format](https://psx-spx.consoledev.net/cdromformat/)
- [ISO 9660 specification](https://wiki.osdev.org/ISO_9660)
- [CHD format - MAME documentation](https://docs.mamedev.org/techspecs/chdformat.html)
- [chd-rs Rust crate](https://github.com/SnowflakePowered/chd-rs) (used for CHD decompression)
