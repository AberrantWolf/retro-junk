# GameDataBase (GDB) by PigSaint

## Overview

PigSaint's GameDataBase is an open CSV dataset providing rich metadata for retro games:
Japanese titles, developer/publisher names, release dates, and a hierarchical tag system
for genre, player count, languages, and input types. All entries are indexed by
cryptographic hashes (MD5, SHA1, SHA256, SHA512).

- **Repository:** https://github.com/PigSaint/GameDataBase
- **License:** CC BY 4.0 (attribution to PigSaint required)
- **Format:** CSV (one file per system)
- **Encoding:** UTF-8

## CSV Format

### Header Row (12 columns)

```
Screen title @ Exact,Cover title @ Exact,ID,Region,Release date,Developer,Publisher,Tags,MD5,SHA1,SHA256,SHA512
```

### Title Separator

Titles use `@` to separate romanized and native-script versions:

- `"4 Nin Uchi Mahjong@4人打ち麻雀"` → romanized: `4 Nin Uchi Mahjong`, native: `4人打ち麻雀`
- `"Super Mario Bros."` → romanized only (no `@`)

Both `Screen title` and `Cover title` columns use this convention.

### Tag Syntax

Space-separated tags, each prefixed with `#`:

```
#players:2:coop #genre:action>platformer #lang:ja #input:zapper
```

Key tag types:
- `#genre:<path>` — hierarchical genre using `>` separator (e.g., `action>platformer`, `board>mahjong`)
- `#players:<count>[:<mode>]` — player count and optional mode (`vs`, `coop`, `alt`)
- `#lang:<codes>` — comma-separated language codes (e.g., `ja`, `en`, `ja,en`)
- `#input:<type>` — special input devices (e.g., `zapper`, `powerpad`, `paddle`)
- `#port:<source>` — port origin (e.g., `arcade`, `computer`)
- `#rev:<version>` — revision identifier
- `#save:<type>` — save mechanism (e.g., `battery`, `password`, `sram`)
- `#addon:<name>` — required add-on hardware
- `#proto` — prototype/unreleased
- `#unl` — unlicensed
- `#hack` — ROM hack

### Hash Fields

All four hash columns contain hex-encoded checksums. CRC32 and file size are **not** provided.

**Important:** GDB hashes may be full-file hashes (including headers like iNES) or headerless
depending on the system. Verify against known ROMs when integrating a new system.

## CSV Filenames

Each system has one CSV file. Download URL pattern:
```
https://raw.githubusercontent.com/PigSaint/GameDataBase/main/{filename}.csv
```

### Currently Integrated Systems

| System | CSV Filename |
|--------|-------------|
| NES/Famicom | `console_nintendo_famicom_nes` |
| Famicom Disk System | `console_nintendo_famicomdisksystem` |
| SNES/Super Famicom | `console_nintendo_superfamicom_snes` |
| Nintendo 64 | `console_nintendo_nintendo64` |
| Game Boy | `console_nintendo_gameboy` |
| Game Boy Color | `console_nintendo_gameboycolor` |
| SG-1000 / SC-3000 | `console_sega_sg1000_sc3000_othellomultivision` |
| Master System / Mark III | `console_sega_markIII_mastersystem` |
| Mega Drive / Genesis | `console_sega_megadrive_genesis` |
| Mega-CD / Sega CD | `console_sega_megacd_segacd` |
| 32X | `console_sega_super32x` |
| Saturn | `console_sega_saturn` |
| Game Gear | `console_sega_gamegear` |

### Additional Available Systems (not yet integrated)

| System | CSV Filename |
|--------|-------------|
| Game Boy Advance | `console_nintendo_gameboyadvance` |
| Nintendo DS | `console_nintendo_nintendods` |
| GameCube | `console_nintendo_gamecube` |
| Wii | `console_nintendo_wii` |
| Atari 2600 | `console_atari_2600` |
| Atari 7800 | `console_atari_7800` |
| PC Engine / TurboGrafx-16 | `console_nec_pcengine_turbografx16` |
| Neo Geo | `console_snk_neogeo` |
| PlayStation | `console_sony_playstation` |
| Dreamcast | `console_sega_dreamcast` |

## Key Differences from No-Intro/Redump

| Feature | No-Intro/Redump | GameDataBase |
|---------|----------------|--------------|
| Title format | English canonical | English + Japanese (via `@`) |
| Serials | Yes | No |
| Developer/Publisher | No | Yes |
| Genre/Tags | No | Yes (hierarchical) |
| Player count | No | Yes |
| CRC32 | Yes | No |
| SHA1 | Yes | Yes |
| File size | Yes | No |
| ROM header handling | Headerless hashes | TBD per system |

## Integration in retro-junk

GDB is used as a **supplementary enrichment layer** that fills metadata gaps left by
No-Intro/Redump DATs (which provide excellent hash/serial matching but lack rich metadata).

- **Trait method:** `RomAnalyzer::gdb_csv_names()` returns CSV filenames for each platform
- **Cache:** `~/.cache/retro-junk/gdb/` with independent `gdb-meta.json`
- **Matching:** SHA1 hash from media entries (primary), MD5 (fallback)
- **CLI:** `retro-junk catalog enrich-gdb` for batch enrichment,
  `retro-junk cache gdb-fetch/gdb-list/gdb-clear` for cache management

## Attribution

When displaying data sourced from GDB:
- Source comments in code modules reference PigSaint's GameDataBase
- User-facing output should credit: "Source: GameDataBase by PigSaint (CC BY 4.0)"
