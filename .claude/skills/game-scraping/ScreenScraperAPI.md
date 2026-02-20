---
description: Information on accessing and using the screenscraper.fr API
---

# ScreenScraper.fr API v2

## Base URL

    https://api.screenscraper.fr/api2/

## Authentication Parameters (required on ALL requests)

| Parameter      | Description                              |
|----------------|------------------------------------------|
| `devid`        | Developer identifier (assigned by SS)    |
| `devpassword`  | Developer password (assigned by SS)      |
| `softname`     | Name of your application                 |
| `ssid`         | (Optional) User's ScreenScraper username |
| `sspassword`   | (Optional) User's ScreenScraper password |

## Response Format
- Set `output=json` or `output=xml` (default is XML).
- Always prefer JSON for easier parsing.

## Available Endpoints

### Infrastructure and User Info

| Endpoint           | Purpose                                       |
|--------------------|-----------------------------------------------|
| `ssinfraInfos.php` | Server load, thread count, API access stats   |
| `ssuserInfos.php`  | User quotas, thread limits, request counts    |

### Reference / List Endpoints

| Endpoint                   | Purpose                            |
|----------------------------|------------------------------------|
| `systemesListe.php`        | List of all systems with IDs       |
| `regionsListe.php`         | List of regions                    |
| `languesListe.php`         | List of languages                  |
| `genresListe.php`          | List of genres                     |
| `famillesListe.php`        | List of game families              |
| `classificationsListe.php` | List of age rating classifications |
| `userlevelsListe.php`      | User level definitions             |
| `nbJoueursListe.php`       | Player count options               |
| `supportTypesListe.php`    | Support/media types                |
| `romTypesListe.php`        | ROM types                          |
| `mediasSystemeListe.php`   | Available media types for systems  |
| `mediasJeuListe.php`       | Available media types for games    |
| `infosJeuListe.php`        | Available info fields for games    |
| `infosRomListe.php`        | Available info fields for ROMs     |

### Core Game Endpoints

| Endpoint            | Purpose                                              |
|---------------------|------------------------------------------------------|
| `jeuInfos.php`      | **PRIMARY** — Get full game info + media by ROM hash |
| `jeuRecherche.php`  | Search games by name (returns up to 30 results)      |

### Media Download Endpoints

| Endpoint                | Purpose                |
|-------------------------|------------------------|
| `mediaJeu.php`          | Download game images   |
| `mediaVideoJeu.php`     | Download game videos   |
| `mediaManuelJeu.php`    | Download game manuals  |
| `mediaSysteme.php`      | Download system images |
| `mediaVideoSysteme.php` | Download system videos |
| `mediaGroup.php`        | Download group images  |
| `mediaCompagnie.php`    | Download company images|

### Contribution Endpoints

| Endpoint             | Purpose                     |
|----------------------|-----------------------------|
| `botNote.php`        | Submit game ratings         |
| `botProposition.php` | Submit info/media proposals |

## Primary Lookup: jeuInfos.php

This is the main endpoint for identifying a ROM and getting game data.

### Request Parameters

| Parameter     | Required | Description                                      |
|---------------|----------|--------------------------------------------------|
| `devid`       | Yes      | Developer ID                                     |
| `devpassword` | Yes      | Developer password                               |
| `softname`    | Yes      | Application name                                 |
| `output`      | No       | "xml" (default) or "json"                        |
| `ssid`        | No       | User's ScreenScraper ID                          |
| `sspassword`  | No       | User's ScreenScraper password                    |
| `crc`         | Yes*     | CRC32 hash of the ROM file (uppercase hex)       |
| `md5`         | Yes*     | MD5 hash of the ROM file (lowercase hex)         |
| `sha1`        | Yes*     | SHA1 hash of the ROM file (lowercase hex)        |
| `systemeid`   | Yes      | Numeric system ID (from `systemesListe.php`)     |
| `romtype`     | Yes      | "rom", "iso", or "dossier" (folder)              |
| `romnom`      | Yes      | ROM filename with extension (URL-encoded)        |
| `romtaille`   | Yes*     | ROM file size in bytes                           |
| `serialnum`   | No       | Force search by serial number                    |
| `gameid`      | No       | Force search by ScreenScraper game ID (skips ROM)|

*At least one hash (CRC, MD5, or SHA1) should be provided. Sending all three plus file size
gives the best matching accuracy.*

### Example Request

    https://api.screenscraper.fr/api2/jeuInfos.php?devid=xxx&devpassword=yyy&softname=zzz&ssid=test&sspassword=test&output=json&crc=50ABC90A&systemeid=1&romtype=rom&romnom=Sonic%20The%20Hedgehog%202%20(World).zip&romtaille=749652

### Response Structure (JSON)

The response is nested under `response`. The three top-level sections are:

```json
{
  "header": {
    "APIversion": "2.0",
    "dateTime": "2024-01-15 12:00:00",
    "success": "true",
    "error": ""
  },
  "response": {
    "serveurs": { ... },
    "ssuser": { ... },
    "jeu": { ... }
  }
}
```

#### serveurs — Server status
- `serveurcpu1`, `serveurcpu2` — CPU utilization percentage
- `threadsmin` — API accesses in last 60 seconds
- `nbscrapeurs` — Current active scrapers

#### ssuser — User info and quotas
- `maxthreads` — Max concurrent threads allowed
- `maxdownloadspeed` — Max download speed (KB/s)
- `requeststoday` — Requests made today
- `maxrequestspermin` — Max requests per minute
- `maxrequestsperday` — Max requests per day
- `maxrequestskoperday` — Max failed (not found) requests per day

#### jeu — Game data

**Important**: Most fields use nested arrays with typed objects, not flat keys.

- `id` — ScreenScraper game ID (string)
- `romid` — ROM ID (string)
- `notgame` — "0" or "1" (non-game: demo, app, BIOS)

- `noms` — Array of `{ "region": "us", "text": "Game Title" }` objects:
  ```json
  "noms": [
    { "region": "us", "text": "Super Mario World" },
    { "region": "jp", "text": "Super Mario World" },
    { "region": "eu", "text": "Super Mario World" }
  ]
  ```

- `systeme` — Object: `{ "id": "4", "text": "Super Nintendo" }`
- `editeur` — Publisher object: `{ "id": "1", "text": "Nintendo" }`
- `developpeur` — Developer object: `{ "id": "1", "text": "Nintendo EAD" }`
- `joueurs` — Player count object: `{ "text": "1-2" }`
- `note` — Rating out of 20 object: `{ "text": "18" }`

- `synopsis` — Array of `{ "langue": "en", "text": "Description..." }` objects:
  ```json
  "synopsis": [
    { "langue": "en", "text": "An epic platformer adventure..." },
    { "langue": "fr", "text": "Une aventure de plateforme épique..." }
  ]
  ```
  Note: the language key is `"langue"` (French), not `"language"`.

- `dates` — Array of `{ "region": "us", "text": "1990-11-21" }` objects

- `genres` — Array of genre objects, each containing nested `noms`:
  ```json
  "genres": [
    { "id": "1", "noms": [
      { "langue": "en", "text": "Platform" },
      { "langue": "fr", "text": "Plateforme" }
    ]}
  ]
  ```

- `classifications` — Age ratings by organization

- `medias` — Array of media objects with `type`, `url`, `region`, `format`, and checksums:
  ```json
  "medias": [
    {
      "type": "ss",
      "parent": "jeu",
      "url": "https://www.screenscraper.fr/media/...",
      "region": "us",
      "crc": "XXXXXXXX",
      "md5": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
      "sha1": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
      "size": "12345",
      "format": "png"
    }
  ]
  ```

  Known media `type` values:
  | Type                | Description                        |
  |---------------------|------------------------------------|
  | `ss`                | Screenshot (in-game)               |
  | `sstitle`           | Title screen screenshot            |
  | `fanart`            | Fan art / background               |
  | `box-2D`            | Box art (2D, flat)                 |
  | `box-3D`            | Box art (3D, angled)               |
  | `wheel`             | Logo/wheel image                   |
  | `wheel-hd`          | Logo/wheel image (high-res)        |
  | `screenmarquee`     | Marquee image                      |
  | `video`             | Video capture                      |
  | `video-normalized`  | Normalized video capture           |
  | `support-2D`        | Cartridge/disc art (2D)            |
  | `support-texture`   | Cartridge/disc art (texture)       |
  | `manuel`            | Manual (PDF)                       |
  | `bezel-16-9`        | Bezel (16:9)                       |
  | `flyer`             | Promotional flyer                  |

- `roms` — List of all known ROMs for this game, each with:
  - `romfilename`, `romsize`, `romcrc`, `rommd5`, `romsha1`
  - Flags: `beta`, `demo`, `trad`, `hack`, `unl`, `alt`, `best`
  - `retroachievement` — RetroAchievements compatible flag

- `rom` — The specific matched ROM object:
  - `romfilename`, `romsha1`, `romregions`

## Common System IDs

This is a partial list. Always call `systemesListe.php` at startup to build an up-to-date mapping.

| System                    | ID  |
|---------------------------|-----|
| Sega Mega Drive/Genesis   | 1   |
| Sega Master System        | 2   |
| Nintendo NES/Famicom      | 3   |
| Super Nintendo (SNES)     | 4   |
| Nintendo Game Boy         | 9   |
| Nintendo Game Boy Color   | 10  |
| Nintendo Game Boy Advance | 12  |
| Nintendo GameCube         | 13  |
| Nintendo 64               | 14  |
| Nintendo DS               | 15  |
| Nintendo Wii              | 16  |
| Nintendo 3DS              | 17  |
| Nintendo Wii U            | 18  |
| Sega 32X                  | 19  |
| Sega CD                   | 20  |
| Sega Game Gear            | 21  |
| Sega Saturn               | 22  |
| Sega Dreamcast            | 23  |
| SNK Neo Geo Pocket        | 25  |
| Atari 2600                | 26  |
| NEC PC Engine/TurboGrafx  | 31  |
| Microsoft Xbox            | 32  |
| Microsoft Xbox 360        | 33  |
| PlayStation               | 57  |
| PlayStation 2             | 58  |
| PlayStation 3             | 59  |
| PSP                       | 61  |
| PlayStation Vita          | 62  |
| SNK Neo Geo Pocket Color  | 82  |
| NEC PC Engine CD          | 114 |
| Neo Geo                   | 142 |

## Serial Number Adaptation

Serial numbers extracted from ROM headers may need platform-specific adaptation
before passing to the `serialnum` parameter. For example, N64 ROMs store serials
as `NUS-NSME-USA` but ScreenScraper may match better on the extracted game code `NSME`.

The `extract_scraper_serial()` method on `RomAnalyzer` handles this adaptation.
By default it delegates to `extract_dat_game_code()`, which works for most platforms.
Override per-console when ScreenScraper needs a different format than DAT matching.

The scraper lookup tries the adapted serial first, then falls back to the raw serial
if they differ.

## Rate Limiting Best Practices

1. Call `ssuserInfos.php` at startup to learn your limits.
2. Track `requeststoday` and `requestskotoday` against daily maximums.
3. Implement a request queue with delays to stay under `maxrequestspermin`.
4. Cache responses aggressively — game metadata rarely changes.
5. Authenticated users get significantly higher limits than anonymous users.
6. Financial contributors (Bronze+) get additional threads and higher quotas.

## Error Handling

- **HTTP 200 with error content**: Check for error messages in response body.
- **HTTP 401/403**: Bad credentials.
- **HTTP 429**: Rate limited — back off.
- **HTTP 500+**: Server error — retry with exponential backoff.
- **Empty or "Erreur" in response**: ROM not found in database.
- Check `closefornomember` and `closeforleecher` in infra status — API may be temporarily
  restricted during high server load.

## Code Example (Rust)

```rust
use crc32fast::Hasher as Crc32Hasher;
use md5::{Digest as _, Md5};
use reqwest::blocking::Client;
use serde::Deserialize;
use sha1::{Digest as _, Sha1};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Credentials for ScreenScraper API authentication.
struct Credentials {
    dev_id: String,
    dev_password: String,
    soft_name: String,
    ss_id: Option<String>,
    ss_password: Option<String>,
}

/// Hashes computed from a ROM file.
struct RomHashes {
    crc: String,
    md5: String,
    sha1: String,
    size: u64,
}

fn hash_rom(path: &Path) -> std::io::Result<RomHashes> {
    let data = fs::read(path)?;

    let mut crc = Crc32Hasher::new();
    crc.update(&data);
    let crc_val = crc.finalize();

    Ok(RomHashes {
        crc: format!("{:08X}", crc_val),
        md5: format!("{:x}", Md5::digest(&data)),
        sha1: format!("{:x}", Sha1::digest(&data)),
        size: data.len() as u64,
    })
}

/// Top-level response wrapper for jeuInfos.php.
#[derive(Deserialize)]
struct JeuInfosResponse {
    response: JeuInfosData,
}

#[derive(Deserialize)]
struct JeuInfosData {
    jeu: GameInfo,
}

/// Game info from ScreenScraper. Fields use nested arrays, not flat keys.
#[derive(Deserialize)]
struct GameInfo {
    id: String,
    #[serde(default)]
    noms: Vec<RegionText>,
    #[serde(default)]
    synopsis: Vec<LangueText>,
    #[serde(default)]
    dates: Vec<RegionText>,
    #[serde(default)]
    medias: Vec<Media>,
    #[serde(default)]
    editeur: Option<IdText>,
    #[serde(default)]
    developpeur: Option<IdText>,
}

#[derive(Deserialize)]
struct RegionText {
    region: String,
    text: String,
}

#[derive(Deserialize)]
struct LangueText {
    langue: String,
    text: String,
}

#[derive(Deserialize)]
struct IdText {
    id: String,
    text: String,
}

#[derive(Deserialize)]
struct Media {
    #[serde(rename = "type")]
    media_type: String,
    url: String,
    #[serde(default)]
    region: String,
    #[serde(default)]
    format: String,
}

fn lookup_game(
    client: &Client,
    rom_path: &Path,
    system_id: u32,
    creds: &Credentials,
) -> Result<GameInfo, Box<dyn std::error::Error>> {
    let hashes = hash_rom(rom_path)?;
    let filename = rom_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();

    let mut params: HashMap<&str, String> = HashMap::new();
    params.insert("devid", creds.dev_id.clone());
    params.insert("devpassword", creds.dev_password.clone());
    params.insert("softname", creds.soft_name.clone());
    params.insert("output", "json".into());
    params.insert("crc", hashes.crc);
    params.insert("md5", hashes.md5);
    params.insert("sha1", hashes.sha1);
    params.insert("systemeid", system_id.to_string());
    params.insert("romtype", "rom".into());
    params.insert("romnom", filename.into_owned());
    params.insert("romtaille", hashes.size.to_string());

    if let Some(ref id) = creds.ss_id {
        params.insert("ssid", id.clone());
    }
    if let Some(ref pw) = creds.ss_password {
        params.insert("sspassword", pw.clone());
    }

    let resp: JeuInfosResponse = client
        .get("https://api.screenscraper.fr/api2/jeuInfos.php")
        .query(&params)
        .send()?
        .json()?;

    Ok(resp.response.jeu)
}
```

### Extracting localized data from nested arrays

```rust
impl GameInfo {
    /// Get the game name for a preferred region, falling back to the first available.
    fn name_for_region(&self, preferred: &str) -> Option<&str> {
        self.noms
            .iter()
            .find(|n| n.region == preferred)
            .or_else(|| self.noms.first())
            .map(|n| n.text.as_str())
    }

    /// Get the synopsis for a preferred language, falling back to English.
    fn synopsis_for_language(&self, preferred: &str) -> Option<&str> {
        self.synopsis
            .iter()
            .find(|s| s.langue == preferred)
            .or_else(|| self.synopsis.iter().find(|s| s.langue == "en"))
            .map(|s| s.text.as_str())
    }

    /// Get all media URLs of a given type (e.g., "ss", "box-2D", "wheel").
    fn media_by_type(&self, media_type: &str) -> Vec<&Media> {
        self.medias.iter().filter(|m| m.media_type == media_type).collect()
    }
}
```

## Filesystem-Illegal Characters in Game Titles

No-Intro canonical names may contain characters that are illegal on some filesystems.
The most common case is `:` (colon), which appears in many game titles but is forbidden
on Windows (NTFS) and macOS (HFS+/APFS Finder layer).

Examples:
- `Castlevania - Circle of the Moon (USA)` — no problem
- `Yu-Gi-Oh! - The Sacred Cards (USA)` — no problem
- `Shin Megami Tensei - Devil Children - Book of Light (Japan)` — no problem
- `Bokujou Monogatari 3 - Harvest Moon: Boy Meets Girl (Japan)` — colon in title

Users who have substituted illegal characters (e.g., `:` → `-`) will fail filename
lookup because the `romnom` parameter won't match ScreenScraper's database.

**Fallback behavior:**
- Hash lookup is the natural fallback for this situation, since the file content is unchanged
- If hash lookup also fails, the specific ROM dump may not be in ScreenScraper's database
- Serial lookup (when available) is unaffected by filename character substitution

**Possible future improvements:**
- Try `jeuRecherche.php` (game name search) as an additional lookup tier for cases where
  both filename and hash fail but we have a reasonable game name to search for

## Multi-System Analyzers and Per-ROM System IDs

Some analyzers handle multiple ScreenScraper system IDs. The `GameBoyAnalyzer` covers
both Game Boy (system 9) and Game Boy Color (system 10). Currently we send system ID 9
for all GB/GBC ROMs and accept system 10 responses via `acceptable_system_ids()` in
`systems.rs`.

The analysis result already knows the CGB flag — the `extra["format"]` field distinguishes
`"Game Boy Color (Exclusive)"` (0xC0), `"Game Boy Color (Compatible)"` (0x80), and
`"Game Boy"`. A possible future improvement would be to use this per-ROM to send the
correct system ID (10 for GBC-exclusive, 9 for GB/hybrid), which could improve match
accuracy for edge cases where ScreenScraper has different entries per system.
