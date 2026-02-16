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

## Information Sources

- [ES-DE GitLab Repository](https://gitlab.com/es-de/emulationstation-de)
- [ES-DE User Guide](https://gitlab.com/es-de/emulationstation-de/-/blob/master/USERGUIDE.md)
- [Original EmulationStation GAMELISTS.md](https://github.com/Aloshi/EmulationStation/blob/master/GAMELISTS.md)
- [Skyscraper Frontend Support](https://gemba.github.io/skyscraper/FRONTENDS/)
- [EmuDeck ES-DE Documentation](https://emudeck.github.io/tools/steamos/es-de/)
