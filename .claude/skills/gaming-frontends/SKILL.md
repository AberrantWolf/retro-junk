---
skill: gaming-frontends
description: Knowledge about popular gaming frontends with accurate links to their docs pages.
---

# Gaming Frontends

Gaming frontends are applications that provide a unified interface for organizing, browsing, and
launching games from multiple platforms and emulators. When building features to generate
frontend-specific metadata, this skill provides the information needed to understand each frontend's
data format, storage structure, and media organization.

## Frontends Covered

### [ES-DE (EmulationStation Desktop Edition)](ES-DE.md)

- **Website:** https://es-de.org/
- **Source:** https://gitlab.com/es-de/emulationstation-de
- **Docs:** https://gitlab.com/es-de/emulationstation-de/-/blob/master/USERGUIDE.md
- **Platform:** Windows, macOS, Linux, SteamOS
- **License:** MIT
- **Metadata format:** XML (`gamelist.xml` per system)
- **Brief:** The modern successor to EmulationStation. Designed for desktop use with built-in
  scraping (ScreenScraper, TheGamesDB), theme support, and controller-friendly navigation. The
  default frontend for RetroDECK and commonly used with EmuDeck on Steam Deck. Renamed from
  "EmulationStation Desktop Edition" to "ES-DE" in v3.0 (2024), which also changed the data
  directory from `.emulationstation` to `ES-DE`.

### [Pegasus Frontend](Pegasus.md)

- **Website:** https://pegasus-frontend.org/
- **Source:** https://github.com/mmatyas/pegasus-frontend
- **Docs:** https://pegasus-frontend.org/docs/
- **Platform:** Windows, macOS, Linux, Android, Raspberry Pi
- **License:** GPL-3.0
- **Metadata format:** Plain text (`metadata.pegasus.txt` per collection)
- **Brief:** A highly customizable, cross-platform frontend built with Qt/QML. Focuses on
  themability with a powerful QML-based theme API. Can import metadata from EmulationStation
  gamelist.xml, Steam, LaunchBox, and Skraper output. No built-in scraper -- relies on external
  tools or imported data.

### [LaunchBox / Big Box](LaunchBox.md)

- **Website:** https://www.launchbox-app.com/
- **Plugin API:** https://pluginapi.launchbox-app.com/
- **Games DB:** https://gamesdb.launchbox-app.com/
- **Platform:** Windows (also Android)
- **License:** Proprietary (free tier + premium)
- **Metadata format:** XML (per-platform XML files in `Data/Platforms/`)
- **Brief:** A comprehensive game launcher and organizer for Windows. Originally a DOSBox frontend,
  now supports all emulators and modern PC games (Steam, GOG, Epic, etc.). Big Box is the premium
  full-screen "10-foot" interface for TVs and arcade cabinets. Features its own community-curated
  games database (LBGDB) with 108,000+ games. Very rich media support (box art, disc art, clear
  logos, fan art, videos, manuals).

### [Playnite](Playnite.md)

- **Website:** https://www.playnite.link/
- **Source:** https://github.com/JosefNemec/Playnite
- **Docs:** https://api.playnite.link/docs/
- **Platform:** Windows
- **License:** MIT
- **Metadata format:** Directory-based JSON (one JSON file per game, per entity)
- **Brief:** A free, open-source game library manager for Windows. Unifies games from Steam, GOG,
  Epic, Origin, Battle.net, Ubisoft Connect, and emulators into a single library. Uses IGDB as its
  default metadata source. Has a powerful plugin system for metadata (IGDB, ScreenScraper,
  SteamGridDB, etc.) and library integrations. Includes a fullscreen controller-friendly mode.

### [RetroArch](RetroArch.md)

- **Website:** https://www.retroarch.com/
- **Source:** https://github.com/libretro/RetroArch
- **Docs:** https://docs.libretro.com/
- **Platform:** Windows, macOS, Linux, Android, iOS, consoles, web browsers, and more
- **License:** GPL-3.0
- **Metadata format:** JSON playlists (`.lpl` files) + binary `.rdb` databases
- **Brief:** The reference frontend for the libretro API. Primarily an emulator framework (loading
  "cores" via the libretro API), but also serves as a full frontend with playlist management,
  thumbnail display, content scanning, shaders, netplay, and RetroAchievements. Its playlist and
  thumbnail systems are relevant for metadata generation.

## Key Differences for Metadata Generation

| Feature | ES-DE | Pegasus | LaunchBox | Playnite | RetroArch |
|---|---|---|---|---|---|
| **Format** | XML | Plain text | XML | JSON | JSON |
| **Per-system files** | Yes | Yes (per-collection) | Yes | No (flat) | Yes (.lpl) |
| **Media referencing** | Paths in XML | `assets.*` fields or directory convention | Filename matching in image folders | DB file IDs | Filename matching in thumbnail dirs |
| **Scraper-friendly** | gamelist.xml is the standard scraper output format | Can import from scrapers and other frontends | Built-in LBGDB scraper | Plugin-based metadata sources | Built-in scanner + .rdb databases |
| **Custom fields** | No | `x-` prefixed keys | `<CustomField>` elements | Plugin extensions | No |

## Generating Metadata for Frontends

When generating metadata for these frontends, the typical workflow is:

1. **Identify the game** using ROM hashes, serial numbers, or filenames (see [game-scraping](../game-scraping/SKILL.md))
2. **Scrape metadata** from sources like ScreenScraper, IGDB, or LBGDB
3. **Download media** (box art, screenshots, logos, videos) from scraping sources
4. **Generate frontend-specific output:**
   - ES-DE: Write `gamelist.xml` files per system with `<game>` elements
   - Pegasus: Write `metadata.pegasus.txt` files per collection
   - LaunchBox: Write per-platform XML files with `<Game>` elements under `<LaunchBox>` root
   - Playnite: Write individual JSON files per game in the library directory structure
   - RetroArch: Write `.lpl` playlist files and place thumbnails in named directories
5. **Place media files** according to each frontend's directory conventions

See each frontend's detailed file for the exact field mappings, media directory structures, and
file format specifications.
