# LaunchBox / Big Box

**Website:** [launchbox-app.com](https://www.launchbox-app.com/) |
[Plugin API](https://pluginapi.launchbox-app.com/) |
[Games Database](https://gamesdb.launchbox-app.com/) |
[Community Forums](https://forums.launchbox-app.com/) |
[Changelog](https://www.launchbox-app.com/about/changelog)

## Overview

LaunchBox is a game launcher, organizer, and frontend for Windows that supports emulators, DOSBox,
ScummVM, and modern PC games. It was originally a DOSBox frontend but has grown into one of the
most feature-rich game library managers available. **Big Box** is the premium full-screen,
controller-driven "10-foot" interface designed for living rooms and arcade cabinets.

Key capabilities:
- Supports virtually any emulator (RetroArch, MAME, Dolphin, PCSX2, etc.)
- Auto-imports from Steam, GOG, Epic, EA, Xbox, and Amazon
- Built-in scraper using the LaunchBox Games Database (LBGDB, 108,000+ games)
- Rich media support: box art, disc art, clear logos, fan art, videos, manuals
- Big Box theming with animated transitions, video snaps, and secondary screen marquees
- DOSBox and ScummVM native integration
- MAME parent/clone relationship support
- Portable mode support

## Data Storage Location

All data is stored relative to the LaunchBox installation directory (no AppData/registry usage by
default). The installation is fully portable.

```
LaunchBox/                          # Install root
  Data/
    Platforms/
      <PlatformName>.xml            # Per-platform game data
    Platforms.xml                   # Platform definitions
    Emulators.xml                   # Emulator configurations
    Settings.xml                    # Application settings
    Parents.xml                     # Parent/clone relationships (MAME)
  Images/
    <PlatformName>/                 # Per-platform image folders (see Media section)
    Platforms/                      # Platform-level images (console photos, banners)
  Videos/
    <PlatformName>/                 # Per-platform video snaps
  Manuals/
    <PlatformName>/
  Music/
    <PlatformName>/
  Metadata/
    Metadata.xml                    # Downloaded LBGDB database (optional)
  Themes/
  Backups/                          # Auto-backups of Data folder
```

## Per-Platform Game XML Format

Each platform has its own XML file at `Data/Platforms/<PlatformName>.xml` (e.g.,
`Nintendo 64.xml`, `Sega Genesis.xml`). The root element is `<LaunchBox>`.

### Example

```xml
<?xml version="1.0" standalone="yes"?>
<LaunchBox>
  <Game>
    <ID>a1b2c3d4-e5f6-7890-abcd-ef1234567890</ID>
    <Title>Super Mario 64</Title>
    <SortTitle>Super Mario 64</SortTitle>
    <Notes>Mario is invited to Peach's castle...</Notes>
    <ReleaseDate>1996-06-23T00:00:00-05:00</ReleaseDate>
    <ReleaseYear>1996</ReleaseYear>
    <Developer>Nintendo EAD</Developer>
    <Publisher>Nintendo</Publisher>
    <Genre>Platform</Genre>
    <MaxPlayers>1</MaxPlayers>
    <Platform>Nintendo 64</Platform>
    <ApplicationPath>..\Games\N64\Super Mario 64 (USA).z64</ApplicationPath>
    <CommandLine></CommandLine>
    <EmulatorId>a1b2c3d4-e5f6-7890-abcd-ef1234567890</EmulatorId>
    <DatabaseID>1234</DatabaseID>
    <LaunchBoxDbId>1234</LaunchBoxDbId>
    <StarRating>5</StarRating>
    <CommunityStarRating>4.5</CommunityStarRating>
    <CommunityStarRatingTotalVotes>1234</CommunityStarRatingTotalVotes>
    <Region>North America</Region>
    <PlayMode>Single Player</PlayMode>
    <Status>Verified</Status>
    <Source></Source>
    <Series>Super Mario</Series>
    <Version></Version>
    <VideoUrl>https://www.youtube.com/watch?v=...</VideoUrl>
    <WikipediaUrl>https://en.wikipedia.org/wiki/...</WikipediaUrl>
    <Favorite>false</Favorite>
    <Completed>false</Completed>
    <Broken>false</Broken>
    <Hide>false</Hide>
    <Portable>false</Portable>
    <Installed>true</Installed>
    <DateAdded>2023-01-15T10:30:00-05:00</DateAdded>
    <DateModified>2023-06-20T14:22:00-05:00</DateModified>
    <PlayCount>0</PlayCount>
    <PlayTime>0</PlayTime>
    <ManualPath></ManualPath>
    <MusicPath></MusicPath>
    <CloneOf></CloneOf>
    <ReleaseType></ReleaseType>
    <AggressiveWindowHiding>false</AggressiveWindowHiding>
    <DisableShutdownScreen>false</DisableShutdownScreen>
    <StartupLoadDelay>0</StartupLoadDelay>
  </Game>

  <AdditionalApplication>
    <Id>...</Id>
    <GameID>a1b2c3d4-e5f6-7890-abcd-ef1234567890</GameID>
    <ApplicationPath>...</ApplicationPath>
    <Name>Configuration</Name>
    <AutoRunBefore>false</AutoRunBefore>
    <AutoRunAfter>false</AutoRunAfter>
    <WaitForExit>false</WaitForExit>
  </AdditionalApplication>

  <CustomField>
    <GameID>a1b2c3d4-e5f6-7890-abcd-ef1234567890</GameID>
    <Name>Hash-SHA1</Name>
    <Value>9BEFE3F6BCDE1E9E3B0B1B9D3F1F0A2E5C6D7E8F</Value>
  </CustomField>

  <AlternateName>
    <GameID>a1b2c3d4-e5f6-7890-abcd-ef1234567890</GameID>
    <AlternateName>Super Mario 64 (スーパーマリオ64)</AlternateName>
    <Region>Japan</Region>
  </AlternateName>
</LaunchBox>
```

### Game Fields Reference

**Identity:**

| Field | Type | Description |
|-------|------|-------------|
| `<ID>` | GUID | Unique identifier |
| `<Title>` | string | Game title |
| `<SortTitle>` | string | Title used for sorting |
| `<DatabaseID>` | integer | LaunchBox Games Database ID |
| `<LaunchBoxDbId>` | integer | Same as DatabaseID (for linking to LBGDB) |

**Descriptive Metadata:**

| Field | Type | Description |
|-------|------|-------------|
| `<Notes>` | string | Game description/overview |
| `<Developer>` | string | Developer name |
| `<Publisher>` | string | Publisher name |
| `<Genre>` | string | Genre classification |
| `<Series>` | string | Game series/franchise |
| `<Region>` | string | Region (e.g., "North America", "Europe", "Japan") |
| `<PlayMode>` | string | Play mode (e.g., "Single Player", "Cooperative") |
| `<MaxPlayers>` | integer | Maximum number of players |
| `<ReleaseDate>` | datetime | Full release date (ISO 8601 with timezone) |
| `<ReleaseYear>` | integer | Release year |
| `<ReleaseType>` | string | Release type classification |
| `<Version>` | string | Version string |
| `<Platform>` | string | Platform name (must match folder/file name) |

**Ratings:**

| Field | Type | Description |
|-------|------|-------------|
| `<StarRating>` | integer | User's personal rating (0-5 stars) |
| `<CommunityStarRating>` | float | Community average from LBGDB |
| `<CommunityStarRatingTotalVotes>` | integer | Number of community votes |

**External Links:**

| Field | Type | Description |
|-------|------|-------------|
| `<VideoUrl>` | string | YouTube or other video URL |
| `<WikipediaUrl>` | string | Wikipedia page URL |
| `<WikipediaId>` | integer | Wikipedia article ID |

**File Paths:**

| Field | Type | Description |
|-------|------|-------------|
| `<ApplicationPath>` | path | Path to ROM/executable (relative paths use `..` from Data folder) |
| `<CommandLine>` | string | Command-line arguments |
| `<EmulatorId>` | GUID | Assigned emulator's ID |
| `<ManualPath>` | path | Path to game manual |
| `<MusicPath>` | path | Path to music file |
| `<RootFolder>` | path | Root folder path |

**Status Flags:**

| Field | Type | Description |
|-------|------|-------------|
| `<Favorite>` | boolean | User favorite |
| `<Completed>` | boolean | User has completed the game |
| `<Broken>` | boolean | Game is non-working |
| `<Hide>` | boolean | Hidden from library |
| `<Portable>` | boolean | Portable game |
| `<Installed>` | boolean | Game is installed |
| `<Status>` | string | Status text (e.g., "Verified") |
| `<Source>` | string | Import source |

**Play Tracking:**

| Field | Type | Description |
|-------|------|-------------|
| `<DateAdded>` | datetime | When added to library |
| `<DateModified>` | datetime | Last modification date |
| `<PlayCount>` | integer | Number of times played |
| `<PlayTime>` | integer | Total play time |

**Related Data (separate XML elements, linked by `<GameID>`):**

| Element | Description |
|---------|-------------|
| `<AdditionalApplication>` | Extra launchable apps (config tools, alternate versions) |
| `<CustomField>` | Arbitrary key-value metadata (`<Name>` + `<Value>`) |
| `<AlternateName>` | Alternate titles with region association |

## Media Organization on Disk

Images are organized by platform and image type. LaunchBox matches images to games by filename.

```
LaunchBox/Images/
  <PlatformName>/                      # e.g., "Nintendo 64"
    Box - Front/                       # Front box art
      <GameTitle>-01.jpg               # Numbered if multiple images exist
    Box - Back/                        # Back box art
    Box - 3D/                          # 3D rendered box art
    Box - Spine/                       # Box spine
    Box - Full/                        # Full unfolded box scan
    Cart - Front/                      # Cartridge front label
    Cart - Back/                       # Cartridge back
    Cart - 3D/                         # 3D rendered cartridge
    Clear Logo/                        # Transparent logo
    Disc/                              # Disc art (CD/DVD systems)
    Screenshot - Gameplay/             # In-game screenshots
    Screenshot - Game Title/           # Title screen
    Screenshot - Game Select/          # Selection screen
    Screenshot - Game Over/            # Game over screen
    Fanart - Background/               # Fan-made backgrounds
    Fanart - Box - Front/              # Fan-made box art
    Banner/                            # Banner images
    Arcade - Marquee/                  # Arcade marquee
    Arcade - Cabinet/                  # Arcade cabinet photos
    Arcade - Control Panel/            # Control panel images
    Arcade - Controls Information/     # Control layout diagrams
```

**Naming convention:** Image filenames must match the game's `<Title>` field. If multiple images
exist for the same game, they are suffixed with `-01`, `-02`, etc.

**Platform-level images** (console photos, platform banners for Big Box views):
```
LaunchBox/Images/Platforms/
  <PlatformName>/
    Banner/
    Clear Logo/
    Device/                            # Console/hardware photos
    Fanart/
```

## LaunchBox Games Database (LBGDB)

The LBGDB is a community-curated database at https://gamesdb.launchbox-app.com/. There is **no
formal REST API**. Access methods:

- **Built-in scraper:** LaunchBox's import wizard scrapes LBGDB automatically during game import
- **Metadata.zip download:** The entire database is available as a single XML file at
  `https://gamesdb.launchbox-app.com/Metadata.zip`
- **Web scraping:** Third-party tools (e.g., [bigscraper](https://github.com/Fr75s/bigscraper))
  parse the LBGDB website directly

Scraped data is written directly into the per-platform XML files. The `<DatabaseID>` field links
local entries back to their LBGDB records.

## Notes for Metadata Generation

- LaunchBox uses plain XML with no schema validation -- you can write elements in any order.
- The `<ID>` field should be a valid GUID. Generate one per game.
- `<ApplicationPath>` supports relative paths. Use `..` notation relative to the `Data/` folder
  (e.g., `..\..\Games\N64\game.z64`).
- Image filenames must exactly match `<Title>` for LaunchBox to find them automatically.
- `<CustomField>` elements are useful for storing extra data (hashes, source IDs, etc.) that
  LaunchBox doesn't have dedicated fields for.
- Platform names must be consistent across XML filenames, `<Platform>` values, and image folder
  names. LaunchBox uses full platform names (e.g., "Nintendo 64", not "N64").
- Dates use ISO 8601 with timezone offset (e.g., `1996-06-23T00:00:00-05:00`).

## Information Sources

- [LaunchBox Plugin API - IGame Interface](https://pluginapi.launchbox-app.com/html/b33d2055-e2be-3f42-12c6-adbc5668f454.htm)
- [LaunchBox Games Database](https://gamesdb.launchbox-app.com/)
- [eXo Wiki - LaunchBox Platform XML](https://wiki.retro-exo.com/index.php/LaunchBox_Platform_XML)
- [LaunchBox Forums - Where is metadata stored?](https://forums.launchbox-app.com/topic/33370-where-is-the-gamess-metadata-stored/)
- [Extracting LaunchBox Metadata (Medium)](https://thatdatascienceguy.medium.com/extracting-launchboxs-video-game-metadata-getting-data-of-video-games-d900c3470c79)
- [bigscraper (GitHub)](https://github.com/Fr75s/bigscraper)
