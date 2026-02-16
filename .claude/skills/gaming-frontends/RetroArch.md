# RetroArch

**Website:** [retroarch.com](https://www.retroarch.com/) |
[Source](https://github.com/libretro/RetroArch) |
[Docs](https://docs.libretro.com/) |
[Playlists Guide](https://docs.libretro.com/guides/roms-playlists-thumbnails/) |
[Override Guide](https://docs.libretro.com/guides/overrides/)

## Overview

RetroArch is the reference frontend for the libretro API. While primarily an emulator framework
(loading emulator "cores" as shared libraries), it includes a full-featured frontend with playlist
management, thumbnail display, a content scanner, shader support, netplay, and
RetroAchievements integration.

Key capabilities:
- Runs on nearly every platform (Windows, macOS, Linux, Android, iOS, consoles, web browsers)
- Loads emulator cores via the libretro API (100+ cores available)
- Shader pipeline (CRT simulation, upscaling, etc.)
- Netplay for online multiplayer across supported cores
- RetroAchievements integration
- Content scanner that matches ROMs against `.rdb` databases to build playlists
- Per-content, per-directory, and per-core configuration overrides

## Data Directory Locations

| Platform | Default path |
|----------|-------------|
| Linux    | `~/.config/retroarch/` |
| macOS    | `~/Library/Application Support/RetroArch/` |
| Windows  | `%APPDATA%\RetroArch\` |
| Android  | `/storage/emulated/0/RetroArch/` |

Key subdirectories:
```
retroarch/
  retroarch.cfg                    # Main configuration
  playlists/                       # .lpl playlist files
  thumbnails/                      # Thumbnail images
    <PlaylistName>/
      Named_Boxarts/
      Named_Snaps/
      Named_Titles/
  config/                          # Per-core and per-game overrides
    <CoreName>/
      <CoreName>.cfg               # Core-level overrides
      <ContentDir>.cfg             # Content directory overrides
      <GameName>.cfg               # Per-game overrides
      <GameName>.opt               # Per-game core options
  database/
    rdb/                           # .rdb database files (compiled from DATs)
    cursors/                       # .dbc database cursor files
  saves/                           # Save files
  states/                          # Save states
  system/                          # BIOS files
  cores/                           # Libretro core shared libraries
  assets/                          # UI assets
```

## Playlist Format (.lpl)

Playlists are JSON files with the `.lpl` extension stored in the `playlists/` directory. Each
playlist typically represents one system/platform.

### Example: `Nintendo - Nintendo 64.lpl`

```json
{
  "version": "1.5",
  "default_core_path": "/path/to/cores/mupen64plus_next_libretro.so",
  "default_core_name": "Mupen64Plus-Next",
  "label_display_mode": 0,
  "right_thumbnail_mode": 0,
  "left_thumbnail_mode": 0,
  "sort_mode": 0,
  "items": [
    {
      "path": "/roms/n64/Super Mario 64 (USA).z64",
      "label": "Super Mario 64 (USA)",
      "core_path": "DETECT",
      "core_name": "DETECT",
      "crc32": "A03CF6C1|crc",
      "db_name": "Nintendo - Nintendo 64.lpl"
    },
    {
      "path": "/roms/n64/GoldenEye 007 (USA).z64",
      "label": "GoldenEye 007 (USA)",
      "core_path": "/path/to/cores/mupen64plus_next_libretro.so",
      "core_name": "Mupen64Plus-Next",
      "crc32": "DBC23B14|crc",
      "db_name": "Nintendo - Nintendo 64.lpl"
    }
  ]
}
```

### Playlist Header Fields

| Field | Type | Description |
|-------|------|-------------|
| `version` | string | Playlist format version (currently `"1.5"`) |
| `default_core_path` | string | Default core path for entries with `"DETECT"` |
| `default_core_name` | string | Default core display name |
| `label_display_mode` | integer | How labels are displayed (0 = default) |
| `right_thumbnail_mode` | integer | Right thumbnail display mode |
| `left_thumbnail_mode` | integer | Left thumbnail display mode |
| `sort_mode` | integer | Sort mode (0 = default, 1 = alphabetical, etc.) |

### Playlist Entry Fields

| Field | Type | Description |
|-------|------|-------------|
| `path` | string | Full path to the ROM/content file |
| `label` | string | Display name (typically the ROM filename without extension) |
| `core_path` | string | Path to the libretro core, or `"DETECT"` for auto-detection |
| `core_name` | string | Core display name, or `"DETECT"` |
| `crc32` | string | CRC32 hash with `\|crc` suffix (e.g., `"A03CF6C1\|crc"`) |
| `db_name` | string | Database/playlist name this entry belongs to |

### Legacy Playlist Format

Older RetroArch versions used a 6-line-per-entry plain text format (no JSON). The current JSON
format was introduced to support additional fields and be more maintainable.

## Thumbnail System

RetroArch matches thumbnails to playlist entries by filename. Thumbnails are organized in three
categories per playlist.

### Directory Structure

```
thumbnails/
  <PlaylistName>/                  # Must match the .lpl filename (without extension)
    Named_Boxarts/                 # Box art / front covers
      <GameLabel>.png
    Named_Snaps/                   # In-game screenshots
      <GameLabel>.png
    Named_Titles/                  # Title screen captures
      <GameLabel>.png
```

### Naming Convention

Thumbnail filenames must match the `label` field from the playlist entry, with these character
replacements for filesystem safety:

| Character | Replacement |
|-----------|-------------|
| `&` | `_` |
| `*` | `_` |
| `/` | `_` |
| `:` | `_` |
| `` ` `` | `_` |
| `<` | `_` |
| `>` | `_` |
| `?` | `_` |
| `\|` | `_` |
| `"` | `_` |
| `\\` | `_` |

For example, a game labeled `"Sonic the Hedgehog 2 (World)"` would have thumbnails at:
```
thumbnails/Sega - Mega Drive - Genesis/Named_Boxarts/Sonic the Hedgehog 2 (World).png
thumbnails/Sega - Mega Drive - Genesis/Named_Snaps/Sonic the Hedgehog 2 (World).png
thumbnails/Sega - Mega Drive - Genesis/Named_Titles/Sonic the Hedgehog 2 (World).png
```

### Thumbnail Sources

RetroArch can download thumbnails from the [libretro-thumbnails](https://github.com/libretro-thumbnails)
repository. Thumbnails use No-Intro naming conventions. The online updater can bulk-download all
thumbnails for a given playlist.

## .rdb Database Files

RetroArch's `.rdb` files are compiled binary databases in the `database/rdb/` directory. They are
generated from DAT files (No-Intro, Redump, MAME) and contain:

- Game names, descriptions, publishers, developers
- ROM hashes (CRC32, MD5, SHA1)
- Region, release dates, genre
- Serial numbers
- ESRB ratings

The **content scanner** uses these `.rdb` files to match ROM files by hash and automatically
populate playlists with correct game names. Database names match system names (e.g.,
`Nintendo - Nintendo 64.rdb`).

## Runtime Logs

RetroArch tracks per-game play time and last-played timestamps in runtime log files:

```
playlists/logs/
  <CoreName>/
    <ContentName>.lrtl            # Runtime log file
```

Each `.lrtl` file stores runtime hours/minutes/seconds and last-played date/time fields.

## Override Hierarchy

RetroArch applies settings in this order (later overrides earlier):

1. `retroarch.cfg` (global defaults)
2. `config/<CoreName>/<CoreName>.cfg` (core-level overrides)
3. `config/<CoreName>/<ContentDir>.cfg` (content directory overrides)
4. `config/<CoreName>/<GameName>.cfg` (per-game overrides)

Overrides are lightweight -- they only store settings that differ from the parent level.

Similarly, core options have a hierarchy:
1. Global core options
2. Per-content directory options
3. Per-game options (`.opt` files)

## Notes for Metadata Generation

- Playlist filenames should match the system's `.rdb` database name (e.g.,
  `Nintendo - Nintendo 64.lpl` matches `Nintendo - Nintendo 64.rdb`).
- The `db_name` field in each playlist entry should match the playlist filename.
- The `label` field is critical -- it determines thumbnail matching. Use No-Intro naming
  conventions for best compatibility with the libretro-thumbnails repository.
- The `crc32` field uses the format `<HASH>|crc`. This is the CRC32 of the ROM content
  (after any header stripping, matching the DAT/rdb hash).
- Use `"DETECT"` for `core_path` and `core_name` when the playlist's `default_core_path` should
  be used, or specify explicit core paths per entry.
- Thumbnails must be PNG format. The `Named_Boxarts`, `Named_Snaps`, and `Named_Titles`
  directories correspond to the three thumbnail slots RetroArch displays.
- RetroArch's playlist and thumbnail system is relatively simple compared to other frontends --
  it does not store rich metadata (descriptions, genres, etc.) in playlists. That data lives in
  the `.rdb` files only and is displayed when browsing the database view.
- For our metadata generation use case, generating `.lpl` playlists and placing correctly-named
  thumbnails is the primary task. Generating `.rdb` files requires the `libretrodb_tool` and is
  typically not needed (the official `.rdb` files from libretro are sufficient).

## Information Sources

- [RetroArch Documentation](https://docs.libretro.com/)
- [Playlists and Thumbnails Guide](https://docs.libretro.com/guides/roms-playlists-thumbnails/)
- [Overrides Guide](https://docs.libretro.com/guides/overrides/)
- [RetroArch GitHub Repository](https://github.com/libretro/RetroArch)
- [libretro-thumbnails Repository](https://github.com/libretro-thumbnails)
- [libretro-database Repository](https://github.com/libretro/libretro-database)
- [playlist.h source](https://github.com/libretro/RetroArch/blob/master/playlist.h)
- [Lakka Playlists Documentation](https://www.lakka.tv/doc/Playlists/)
- [DeepWiki - RetroArch Playlists and Database](https://deepwiki.com/libretro/RetroArch/7.5-playlists-and-database)
