# ES-DE (EmulationStation Desktop Edition)

**Source:** [ES-DE GitLab](https://gitlab.com/es-de/emulationstation-de) |
[Website](https://es-de.org/) |
[User Guide](https://gitlab.com/es-de/emulationstation-de/-/blob/master/USERGUIDE.md) |
[FAQ](https://gitlab.com/es-de/emulationstation-de/-/blob/master/FAQ.md)

## Overview

ES-DE is the modern successor to the original EmulationStation by Aloshi. It is a desktop-focused
fork that adds built-in scraping, modern theme support, alternative emulator selection per game,
and hundreds of pre-configured system definitions. It is the default frontend for RetroDECK and
is widely used with EmuDeck on Steam Deck.

As of v3.0 (February 2024), the project rebranded from "EmulationStation Desktop Edition" to
"ES-DE" and changed the application data directory from `.emulationstation` to `ES-DE`.

## Data Directory Locations

| Platform | Path |
|----------|------|
| Linux    | `~/.emulationstation/` (pre-v3) or `~/ES-DE/` (v3+) |
| macOS    | `~/ES-DE/` (v3+), previously `~/.emulationstation/` |
| Windows  | `~\.emulationstation\` (pre-v3) or `~\ES-DE\` (v3+) |

## Directory Structure

```
ES-DE/                           (or .emulationstation/ for pre-v3)
  gamelists/
    <system>/                    # One folder per system (e.g., nes/, snes/, n64/)
      gamelist.xml               # Metadata for all games in this system
  downloaded_media/
    <system>/
      screenshots/               # In-game screenshots
      covers/                    # Box art / cover images
      marquees/                  # Logo/marquee images
      3dboxes/                   # 3D rendered box art
      backcovers/                # Back cover images
      fanart/                    # Fan art
      miximages/                 # Composite "mix" images (screenshot + box + logo)
      physicalmedia/             # Physical media images (cart/disc)
      titlescreens/              # Title screen captures
      videos/                    # Video snaps
  custom_systems/
    es_systems.xml               # Custom system definitions
  themes/                        # Installed themes
  collections/                   # Custom/auto collections
  scripts/                       # Event scripts
```

## gamelist.xml Format

Each system has a `gamelist.xml` file. The root element is `<gameList>` containing `<game>` and
optionally `<folder>` elements.

### Complete Example

```xml
<?xml version="1.0"?>
<gameList>
  <game>
    <path>./Super Mario 64 (USA).z64</path>
    <name>Super Mario 64</name>
    <sortname>Super Mario 64</sortname>
    <collectionsortname></collectionsortname>
    <desc>Mario is invited to Peach's castle, but when he arrives Bowser has taken over...</desc>
    <rating>0.9</rating>
    <releasedate>19960623T000000</releasedate>
    <developer>Nintendo EAD</developer>
    <publisher>Nintendo</publisher>
    <genre>Platform</genre>
    <players>1</players>
    <favorite>true</favorite>
    <completed>false</completed>
    <kidgame>true</kidgame>
    <hidden>false</hidden>
    <broken>false</broken>
    <playcount>5</playcount>
    <lastplayed>20240115T143000</lastplayed>
    <controller></controller>
    <altemulator></altemulator>
    <nomultiscrape>false</nomultiscrape>
    <hidemetadata>false</hidemetadata>
    <nogamecount>false</nogamecount>
    <image>./downloaded_media/n64/screenshots/Super Mario 64 (USA).png</image>
    <cover>./downloaded_media/n64/covers/Super Mario 64 (USA).png</cover>
    <marquee>./downloaded_media/n64/marquees/Super Mario 64 (USA).png</marquee>
    <screenshot>./downloaded_media/n64/screenshots/Super Mario 64 (USA).png</screenshot>
    <titlescreen>./downloaded_media/n64/titlescreens/Super Mario 64 (USA).png</titlescreen>
    <video>./downloaded_media/n64/videos/Super Mario 64 (USA).mp4</video>
    <thumbnail>./downloaded_media/n64/covers/Super Mario 64 (USA).png</thumbnail>
    <fanart>./downloaded_media/n64/fanart/Super Mario 64 (USA).png</fanart>
  </game>

  <folder>
    <path>./Hacks</path>
    <name>ROM Hacks</name>
    <desc>Collection of ROM hacks</desc>
  </folder>
</gameList>
```

### Metadata Fields Reference

**Core Metadata:**

| Tag | Type | Description |
|-----|------|-------------|
| `<path>` | string | Path to ROM file (relative to ROM directory or absolute) |
| `<name>` | string | Display name of the game |
| `<sortname>` | string | Alternative name used only for sorting (does not change displayed name) |
| `<collectionsortname>` | string | Sort name specific to collections |
| `<desc>` | string | Game description text |
| `<developer>` | string | Developer name |
| `<publisher>` | string | Publisher name |
| `<genre>` | string | Genre classification |
| `<players>` | string | Number of players (e.g., "1", "1-4") |
| `<releasedate>` | datetime | Release date in `YYYYMMDDTHHMMSS` format (time portion ignored) |
| `<rating>` | float | Rating between 0.0 and 1.0 (displayed as stars; supports half/quarter stars) |

**User Status Flags:**

| Tag | Type | Description |
|-----|------|-------------|
| `<favorite>` | boolean | Marked as favorite |
| `<completed>` | boolean | User has completed the game |
| `<kidgame>` | boolean | Appropriate for children (used by kid-friendly mode) |
| `<hidden>` | boolean | Hidden from the game list |
| `<broken>` | boolean | Marked as non-working |

**Play Tracking (auto-populated):**

| Tag | Type | Description |
|-----|------|-------------|
| `<playcount>` | integer | Number of times launched |
| `<lastplayed>` | datetime | Last played timestamp in `YYYYMMDDTHHMMSS` format |

**ES-DE Specific:**

| Tag | Type | Description |
|-----|------|-------------|
| `<altemulator>` | string | Override the default emulator for this specific game |
| `<controller>` | string | Controller type/configuration |
| `<nomultiscrape>` | boolean | Exclude from multi-game scraping operations |
| `<hidemetadata>` | boolean | Hide metadata display in the UI |
| `<nogamecount>` | boolean | Exclude from game count totals |

**Media Paths:**

| Tag | Type | Description |
|-----|------|-------------|
| `<image>` | path | Primary display image (usually screenshot or mix image) |
| `<screenshot>` | path | In-game screenshot |
| `<cover>` | path | Box art / front cover |
| `<marquee>` | path | Logo / marquee image |
| `<thumbnail>` | path | Thumbnail image (often same as cover) |
| `<video>` | path | Video snap file path |
| `<titlescreen>` | path | Title screen image |
| `<fanart>` | path | Fan art image |

### Date Format

Dates use the format `YYYYMMDDTHHMMSS` (e.g., `19960623T000000` for June 23, 1996). The time
portion is stored but not displayed for release dates. For `<lastplayed>`, the full timestamp is
used.

### Path Resolution

Paths in `<path>` and media tags can be:
- **Relative** (starting with `./`): resolved relative to the system's ROM directory
- **Absolute**: full filesystem path

Using `~` for the home directory is recommended in ES-DE configuration to avoid issues with
localized home directory paths.

## Scraper Support

ES-DE has a built-in multi-threaded scraper supporting:
- **ScreenScraper** (default, requires account for faster access)
- **TheGamesDB**

Scraped data is written directly into the `gamelist.xml` files. Downloaded media goes into
`downloaded_media/<system>/<mediatype>/` directories.

The third-party tool **Skyscraper** also supports ES-DE as an output target and preserves these
ES-DE-specific metadata fields when regenerating gamelists: `altemulator`, `broken`,
`collectionsortname`, `completed`, `controller`, `hidemetadata`, `nogamecount`, `nomultiscrape`.

## Notes for Metadata Generation

- The `gamelist.xml` format is widely supported by scrapers (Skyscraper, Selph's Scraper, ARRM)
  and is the de facto standard for EmulationStation-family frontends.
- When generating gamelist.xml, always include `<path>` and `<name>` at minimum.
- Media paths should be relative (starting with `./`) for portability.
- The `<image>` tag is what ES-DE themes typically display as the primary game image. Many users
  configure "mix images" (composite images combining screenshot + box art + logo) as the `<image>`.
- The `<rating>` field ranges from 0.0 to 1.0. To convert from a 5-star or 10-point scale,
  divide by the maximum value.
- System folder names (used in paths) follow ES-DE's `es_systems.xml` naming convention (e.g.,
  `n64`, `snes`, `megadrive`, `psx`).

## Using retro-junk Output with ES-DE

By default, ES-DE reads gamelists from `~/ES-DE/gamelists/<system>/` and media from
`~/ES-DE/downloaded_media/<system>/`. retro-junk writes gamelists to a `<root>-metadata/`
directory and media to a `<root>-media/` directory by default. To bridge these, ES-DE provides
configurable settings that let it read from wherever retro-junk writes.

### Recommended Portable Setup

This layout works across platforms (including Android) and is SyncThing-friendly with no
symlinks required:

```
~/gaming/                          <- SyncThing root
  ROMs/                            <- ES-DE ROMDirectory
    nes/
      game.nes
      gamelist.xml                 <- retro-junk writes here
    snes/
      game.sfc
      gamelist.xml
  ROMs-media/                      <- ES-DE MediaDirectory
    nes/
      covers/game.png
      screenshots/game.png
    snes/
      covers/game.sfc.png
```

**retro-junk command:**
```bash
retro-junk scrape --root ~/gaming/ROMs --metadata-dir ~/gaming/ROMs
```

Setting `--metadata-dir` to the same path as `--root` places `gamelist.xml` files directly
inside each system's ROM directory, which ES-DE can read with `LegacyGamelistFileLocation`.

**ES-DE settings** (`~/ES-DE/settings/es_settings.xml`, configured per machine):
```xml
<string name="ROMDirectory" value="~/gaming/ROMs" />
<string name="MediaDirectory" value="~/gaming/ROMs-media" />
<bool name="LegacyGamelistFileLocation" value="true" />
```

### Key ES-DE Settings

- **`ROMDirectory`** — where ES-DE looks for ROMs. Point this at your retro-junk `--root`.

- **`MediaDirectory`** — where ES-DE looks for media (covers, screenshots, etc.). Point this
  at your retro-junk media output directory (`<root>-media` by default). ES-DE finds media
  purely by filename matching within this directory — it **ignores media path tags** in
  gamelist.xml entirely. The media tags we write are for compatibility with other tools
  (Skyscraper, other ES forks) that do read them.

- **`LegacyGamelistFileLocation`** — when `true`, ES-DE reads `gamelist.xml` from within the
  ROM directory tree (e.g., `ROMs/nes/gamelist.xml`) instead of `~/ES-DE/gamelists/nes/`.
  This is the recommended setting when using retro-junk with `--metadata-dir` set to `--root`.

### Notes

- Scrape log files will also land in the ROM directory when `--metadata-dir` equals `--root`.
  Use `--no-log` to suppress them if they're unwanted, or leave them — ES-DE ignores
  non-ROM files that don't match known extensions.

- ES-DE matches media files to games by filename (stem must match the ROM filename). Our
  scraper already names media files to match ROMs, so this works automatically.

## Information Sources

- [ES-DE GitLab Repository](https://gitlab.com/es-de/emulationstation-de)
- [ES-DE User Guide](https://gitlab.com/es-de/emulationstation-de/-/blob/master/USERGUIDE.md)
- [Original EmulationStation GAMELISTS.md](https://github.com/Aloshi/EmulationStation/blob/master/GAMELISTS.md)
- [Skyscraper Frontend Support](https://gemba.github.io/skyscraper/FRONTENDS/)
- [EmuDeck ES-DE Documentation](https://emudeck.github.io/tools/steamos/es-de/)
