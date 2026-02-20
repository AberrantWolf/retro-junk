# retro-junk

A CLI tool for analyzing, renaming, and scraping metadata for retro game ROMs and disc images. Supports 23 consoles across Nintendo, Sony, Sega, and Microsoft platforms.

## Build

```bash
cargo build --release
```

## Install

```bash
cargo install --path retro-junk-cli
```

## Usage

retro-junk expects ROMs organized in console-named folders (e.g., `snes/`, `n64/`, `ps1/`). Use `retro-junk list` to see recognized folder names for each console.

### List supported consoles

```bash
retro-junk list
```

### Analyze ROMs

Scan ROM files to extract header metadata and validate integrity:

```bash
retro-junk analyze --root /path/to/roms
```

Options:

- `--quick` / `-q` — Minimize disk reads (useful for network shares)
- `--consoles` / `-c` — Filter to specific consoles: `-c snes,n64,ps1`
- `--limit` — Maximum number of ROMs to process per console

### Rename ROMs

Rename ROM files to their canonical No-Intro or Redump names using serial number or hash matching:

```bash
retro-junk rename --root /path/to/roms --dry-run
```

Options:

- `--dry-run` / `-n` — Preview renames without executing
- `--hash` — Force CRC32 hash-based matching (reads full files)
- `--consoles` / `-c` — Filter to specific consoles
- `--limit` — Maximum number of ROMs to process per console
- `--dat-dir` — Use DAT files from a custom directory instead of the cache

### Scrape metadata

Download game metadata and media (covers, screenshots, videos, marquees) from ScreenScraper:

```bash
retro-junk scrape --root /path/to/roms --dry-run
```

Options:

- `--media-types` — Media to download (comma-delimited): `covers,screenshots,videos,marquees`
- `--metadata-dir` — Output directory for metadata (default: `<root>-metadata`)
- `--media-dir` — Output directory for media (default: `<root>-media`)
- `--frontend` — Target frontend format (default: `esde`)
- `--region` — Preferred region for names/media (default: `us`)
- `--language` — Preferred language for descriptions (default: `en`)
- `--dry-run` / `-n` — Preview what would be scraped
- `--skip-existing` — Skip games that already have metadata
- `--no-miximage` — Disable miximage generation
- `--force-redownload` — Redownload all media, ignoring existing files
- `--consoles` / `-c` — Filter to specific consoles
- `--limit` — Maximum number of ROMs to process per console

### Manage DAT cache

DAT files are used for game identification. They are downloaded automatically as needed, or can be managed manually:

```bash
retro-junk cache list              # show cached DATs
retro-junk cache fetch snes,n64    # download DATs for specific systems
retro-junk cache fetch all         # download all DATs
retro-junk cache clear             # remove all cached DATs
```

### Configure ScreenScraper credentials

Scraping requires ScreenScraper API credentials:

```bash
retro-junk config setup    # interactive credential setup
retro-junk config show     # show current credentials
retro-junk config test     # test credentials against the API
retro-junk config path     # print config file path
```

## Supported Consoles

**Nintendo:** NES, SNES, N64, GameCube, Wii, Wii U, Game Boy, GBA, DS, 3DS

**Sony:** PS1, PS2, PS3, PSP, Vita

**Sega:** SG-1000, Master System, Genesis, Sega CD, 32X, Saturn, Dreamcast, Game Gear

**Microsoft:** Xbox, Xbox 360

## Missing Features

**Analyzer implementations:**
Hash-based rename and scraping may work for these systems, but serial-based matching and header metadata are unavailable.

They are registered with DAT metadata, but the analyze() function is not yet implemented.

- GameCube
- Wii
- Wii U
- PS2
- PS3
- PSP
- Vita
- SG-1000
- Master System
- Sega CD
- 32X
- Saturn
- Dreamcast
- Game Gear
- Xbox
- Xbox 360)

**GUI:** The `retro-junk-gui` framework has not been implemented (stub only).

**Frontend formats:** Only ES-DE (`gamelist.xml`) is implemented for metadata/media output. Other frontends (Pegasus, Batocera, LaunchBox, RetroArch playlists) are not yet supported.

**Disc image format support:** PS1 is the only disc-based console with full ISO/BIN+CUE parsing. Other disc consoles (PS2, PS3, PSP, GameCube, Wii, Sega CD, Saturn, Dreamcast, Xbox, Xbox 360) lack disc image header parsing entirely.

**CHD support:** PS1 has full CHD decompression and serial extraction via the `chd` crate. Other disc-based consoles (PS2, Dreamcast, Saturn, Sega CD) list CHD as a supported extension for detection and hash-based matching, but lack CHD-specific header parsing.

**Compressed ROM support:** No support for reading ROMs inside ZIP or 7z archives, which is how many collections are stored.

**Multi-disc game handling:** Rename and scraping both group multi-disc games via `.m3u` folders and loose "(Disc N)" filename detection. However, multi-disc sets are not yet auto-organized into `.m3u` folders during scraping.

## License

MIT
