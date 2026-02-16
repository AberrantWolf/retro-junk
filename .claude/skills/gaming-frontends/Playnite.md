# Playnite

**Website:** [playnite.link](https://www.playnite.link/) |
[Source](https://github.com/JosefNemec/Playnite) |
[API Docs](https://api.playnite.link/docs/) |
[Game SDK Model](https://api.playnite.link/docs/api/Playnite.SDK.Models.Game.html) |
[Metadata Docs](https://api.playnite.link/docs/manual/library/games/metadata.html)

## Overview

Playnite is a free, open-source (MIT) game library manager for Windows. It unifies games from
Steam, GOG, Epic, Origin, Battle.net, Ubisoft Connect, Xbox, and emulators into a single
interface. It features a fullscreen controller-friendly mode, play time tracking, and a powerful
plugin system for metadata sources and library integrations.

Key capabilities:
- Library integration with all major PC game stores
- First-class emulation support with built-in emulator profiles
- IGDB as default metadata source, with plugins for ScreenScraper, SteamGridDB, MobyGames, etc.
- Fullscreen "TV mode" for controller use
- Automatic play time tracking
- All data stored locally (privacy-focused, no remote servers)

## Data Storage

### Database Format

Playnite uses a **directory-based JSON** storage system. Each entity (game, platform, genre, etc.)
is stored as an individual JSON file named by its GUID.

### Default Paths

| Mode | Library path | Config path |
|------|-------------|-------------|
| Installed | `%APPDATA%\Playnite\library\` | `%APPDATA%\Playnite\config.json` |
| Portable  | `<PlayniteDir>\library\` | `<PlayniteDir>\config.json` |

### Directory Structure

```
library/
  database.json                    # Database settings/version metadata
  games/                           # One JSON file per game (named by GUID)
    a1b2c3d4-e5f6-7890-abcd-....json
    ...
  platforms/                       # Platform definitions
  emulators/                       # Emulator configurations
  genres/
  companies/                       # Developers AND publishers (shared collection)
  tags/
  features/                        # Game features (multiplayer, VR, etc.)
  categories/
  series/
  ageratings/
  regions/
  sources/                         # Library sources (Steam, GOG, Manual, etc.)
  completionstatuses/
  scanners/                        # Auto-scan configurations for ROMs
  filterpresets/
  importexclusions/
  files/                           # Media files
    <game-guid>/                   # Per-game media subfolder
      icon.png
      cover.jpg
      background.jpg
```

## Game JSON Structure

Each game is a single JSON file in the `games/` directory. The relational model uses GUID
references to entries in other collections.

### Example

```json
{
  "Id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "Name": "Super Mario 64",
  "SortingName": "Super Mario 64",
  "GameId": "sm64_usa",
  "PluginId": "00000000-0000-0000-0000-000000000000",
  "Description": "<p>Mario is invited to Peach's castle...</p>",
  "Notes": "",
  "Version": "",
  "Manual": "",
  "ReleaseDate": {
    "Year": 1996,
    "Month": 6,
    "Day": 23
  },
  "Links": [
    { "Name": "Wikipedia", "Url": "https://en.wikipedia.org/wiki/Super_Mario_64" }
  ],
  "Roms": [
    { "Name": "Super Mario 64 (USA)", "Path": "D:\\ROMs\\N64\\Super Mario 64 (USA).z64" }
  ],
  "IsInstalled": true,
  "Hidden": false,
  "Favorite": true,
  "InstallDirectory": "",
  "InstallSize": null,
  "Icon": "a1b2c3d4.png",
  "CoverImage": "a1b2c3d4_cover.jpg",
  "BackgroundImage": "a1b2c3d4_bg.jpg",
  "Playtime": 7200,
  "PlayCount": 3,
  "LastActivity": "2024-01-15T14:30:00",
  "Added": "2023-01-15T10:30:00",
  "Modified": "2024-01-15T14:30:00",
  "UserScore": 90,
  "CriticScore": 94,
  "CommunityScore": 96,
  "GenreIds": ["guid1", "guid2"],
  "DeveloperIds": ["guid3"],
  "PublisherIds": ["guid4"],
  "CategoryIds": [],
  "TagIds": [],
  "FeatureIds": ["guid5"],
  "PlatformIds": ["guid6"],
  "SeriesIds": ["guid7"],
  "AgeRatingIds": [],
  "RegionIds": ["guid8"],
  "SourceId": "guid9",
  "CompletionStatusId": "guid10",
  "GameActions": [
    {
      "Type": 1,
      "Name": "Play with Mupen64Plus",
      "EmulatorId": "guid11",
      "EmulatorProfileId": "guid12",
      "IsPlayAction": true
    }
  ],
  "IncludeLibraryPluginAction": true,
  "OverrideInstallState": false,
  "EnableSystemHdr": false,
  "PreScript": "",
  "PostScript": "",
  "GameStartedScript": "",
  "UseGlobalPreScript": true,
  "UseGlobalPostScript": true,
  "UseGlobalGameStartedScript": true
}
```

### Game Fields Reference

**Core Identity:**

| Field | Type | Description |
|-------|------|-------------|
| `Id` | GUID | Unique identifier |
| `Name` | string | Game title |
| `SortingName` | string | Alternative name for sorting |
| `GameId` | string | Provider-specific ID (e.g., Steam app ID) |
| `PluginId` | GUID | Library plugin that owns this game |

**Descriptive Metadata:**

| Field | Type | Description |
|-------|------|-------------|
| `Description` | string | HTML game description |
| `Notes` | string | User notes |
| `Version` | string | Game version |
| `Manual` | string | Path to game manual |
| `ReleaseDate` | object | `{ Year, Month, Day }` |
| `Links` | array | `[{ Name, Url }]` -- web links |

**Relational References (GUID lists referencing other collections):**

| Field | Referenced collection | Description |
|-------|----------------------|-------------|
| `GenreIds` | `genres/` | Game genres |
| `DeveloperIds` | `companies/` | Developer companies |
| `PublisherIds` | `companies/` | Publisher companies |
| `CategoryIds` | `categories/` | User categories |
| `TagIds` | `tags/` | User-defined tags |
| `FeatureIds` | `features/` | Game features (multiplayer, VR, etc.) |
| `PlatformIds` | `platforms/` | Gaming platforms |
| `SeriesIds` | `series/` | Game series/franchises |
| `AgeRatingIds` | `ageratings/` | Age ratings |
| `RegionIds` | `regions/` | Geographic regions |
| `SourceId` | `sources/` | Library source (Steam, GOG, Manual, etc.) |
| `CompletionStatusId` | `completionstatuses/` | Completion status |

**Media (file IDs, local paths, or HTTP URLs):**

| Field | Type | Description |
|-------|------|-------------|
| `Icon` | string | Game icon |
| `CoverImage` | string | Cover/box art |
| `BackgroundImage` | string | Background image (uniquely supports HTTP URLs directly) |

Media referenced by file ID is stored in `library/files/<game-guid>/`.

**Play Activity:**

| Field | Type | Description |
|-------|------|-------------|
| `Playtime` | ulong | Total play time in seconds |
| `PlayCount` | ulong | Number of launches |
| `LastActivity` | datetime | Last played date |
| `Added` | datetime | Date added to library |
| `Modified` | datetime | Last modification date |

**Scores:**

| Field | Type | Description |
|-------|------|-------------|
| `UserScore` | int? | Personal rating (0-100) |
| `CriticScore` | int? | Critic score (0-100) |
| `CommunityScore` | int? | Community score (0-100) |

**Status:**

| Field | Type | Description |
|-------|------|-------------|
| `IsInstalled` | bool | Whether the game is installed |
| `Hidden` | bool | Hidden from library |
| `Favorite` | bool | Marked as favorite |

**Emulation:**

| Field | Type | Description |
|-------|------|-------------|
| `Roms` | array | `[{ Name, Path }]` -- ROM file list |
| `GameActions` | array | Launch actions (including emulator profiles) |

## Database Collections

The relational model uses 16 typed collections, each in its own directory:

| Collection | Entity type | Description |
|------------|-------------|-------------|
| `games/` | Game | Central game entries |
| `platforms/` | Platform | Gaming platforms (NES, PS1, PC, etc.) |
| `emulators/` | Emulator | Emulator configurations |
| `genres/` | Genre | Game genres |
| `companies/` | Company | Developers and publishers (shared) |
| `tags/` | Tag | User-defined tags |
| `features/` | GameFeature | Game features |
| `categories/` | Category | User categories |
| `series/` | Series | Game series/franchises |
| `ageratings/` | AgeRating | Age ratings |
| `regions/` | Region | Geographic regions |
| `sources/` | GameSource | Library sources |
| `completionstatuses/` | CompletionStatus | Completion tracking |
| `scanners/` | GameScannerConfig | ROM auto-scan configs |
| `filterpresets/` | FilterPreset | Saved filter presets |
| `importexclusions/` | ImportExclusionItem | Excluded import items |

## Metadata Plugin System

Playnite's plugin architecture allows any metadata source. Users configure per-field priority
(e.g., use IGDB for descriptions, SteamGridDB for covers).

**Built-in metadata sources:**
- **IGDB** (Internet Game Database) -- default
- **LaunchBox** -- alternative built-in source

**Community metadata plugins:**
- **ScreenScraper** -- especially useful for retro/emulated games; supports ROM hash lookups
- **SteamGridDB** -- specializes in cover art, grids, heroes, logos, and icons
- **MobyGames**, **GiantBomb**, and others

Plugins return a `GameMetadata` object that can provide all major fields including media files.

## Notes for Metadata Generation

- Playnite uses a relational model. To add a game with a genre, you must first create the genre
  entity in `genres/`, then reference its GUID from the game's `GenreIds` array.
- Developers and publishers share the `companies/` collection.
- Media files go in `library/files/<game-guid>/` and are referenced by filename in the game JSON.
- `BackgroundImage` uniquely supports HTTP URLs directly (not just file IDs or local paths).
- `Description` supports HTML content.
- `ReleaseDate` is an object with `Year`, `Month`, `Day` fields (not a date string).
- Scores (`UserScore`, `CriticScore`, `CommunityScore`) are integers 0-100.
- `Playtime` is stored in seconds.
- For emulated games, populate the `Roms` array with ROM file paths and configure `GameActions`
  with the appropriate emulator profile.
- Backup: Copy the entire `library/` folder, or use Playnite's built-in backup (`--backup` CLI).

## Information Sources

- [Playnite GitHub Repository](https://github.com/JosefNemec/Playnite)
- [Game Class SDK Reference](https://api.playnite.link/docs/api/Playnite.SDK.Models.Game.html)
- [GameMetadata Class](https://api.playnite.link/docs/api/Playnite.SDK.Models.GameMetadata.html)
- [Metadata Plugins Tutorial](https://api.playnite.link/docs/tutorials/extensions/metadataPlugins.html)
- [Emulation Support](https://api.playnite.link/docs/manual/features/emulationSupport/emulationSupportOverview.html)
- [GameDatabase.cs source](https://github.com/JosefNemec/Playnite/blob/master/source/Playnite/Database/GameDatabase.cs)
- [Game.cs source](https://github.com/JosefNemec/Playnite/blob/master/source/PlayniteSDK/Models/Game.cs)
