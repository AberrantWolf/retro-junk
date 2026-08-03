---
name: game-scraping
description: Use when working on ROM identification, DAT files, ScreenScraper, game metadata, media downloads, or checksum-based game matching in retro-junk.
---

# ROM Identification, Metadata, & Media

## Overview
This project builds a program that identifies game ROMs by their checksums and retrieves
metadata/media using multiple sources:
1. **ScreenScraper.fr API** — A community-driven retro game database with rich metadata and media.
2. **No-Intro DAT files** — Curated catalogs of verified cartridge/ROM-chip dumps.
3. **Redump DAT files** — Curated catalogs of verified optical disc dumps (CD/DVD/GD-ROM/Blu-ray).
4. **LibRetro enhanced DATs** — Enhanced No-Intro DATs with serial, region, and release date fields.
   **This is the correct and only DAT source for retro-junk** (downloaded from
   `libretro/libretro-database`, NOT `libretro-mirrors/nointro-db`). Preferred over raw No-Intro
   DATs because they are a strict superset with serial fields needed for serial-based matching. See
   [LibRetroDB](LibRetroDB.md) for details.

## Architecture Notes
- ScreenScraper is a **live web API** (REST-like, GET requests, returns XML or JSON).
- No-Intro and Redump do **not** have public APIs. Data is accessed via downloadable **DAT files**
  (XML format, Logiqx compatible) that must be parsed locally.
- The typical workflow is:
  1. Hash a local ROM/disc image file (CRC32, MD5, SHA1).
  2. Look up the hash in local DAT files (No-Intro for cartridge ROMs, Redump for disc images)
     to verify/identify the dump.
  3. Query ScreenScraper API with the hash and system ID to get rich metadata and media URLs.

## Key Concepts
- **DAT file**: An XML file following the Logiqx format containing ROM/track names, sizes, and
  checksums (CRC32, MD5, SHA1) for a specific system.
- **System ID**: ScreenScraper uses numeric IDs for each system (e.g., Mega Drive = 1). Use the
  `systemesListe.php` endpoint to get the full mapping.
- **ROM hashing**: Always compute CRC32, MD5, and SHA1 for best matching accuracy. Send all three
  to ScreenScraper when possible.
- **Header stripping**: No-Intro DATs catalog headerless ROMs. Many ROM files in the wild have
  platform-specific headers prepended (e.g., iNES for NES, SMC for SNES, LNX for Lynx). These
  headers must be stripped before hashing or the checksums will not match. See
  [NoIntro DAT](NoIntroDAT.md) for details.
- **Compressed ROMs**: ROM files are often distributed in ZIP or 7z archives. Hash the
  **contained file**, not the archive itself. For ZIP files, the CRC32 from the ZIP directory entry
  can be used as a quick check without full decompression.
- **Rate limiting**: ScreenScraper enforces per-minute and per-day request limits. Always check
  user quota via `ssuserInfos.php` and respect limits.
- **Derivation**: a mod and a homebrew title are the two things no DAT and no scraper describes.
  A mod is identified as the *work it was derived from* — never by its own bytes, which are in
  nobody's database — while keeping its own name and media stem; homebrew is identified by name,
  never by a serial nobody assigned it. The decision lives with the collection
  (`<collection>/.retro-junk/marks/`), not in the rebuildable catalog database. See
  [ScreenScraper API](ScreenScraperAPI.md#files-screenscraper-cannot-know-mods-and-homebrew).

## Credentials and Authentication
- ScreenScraper requires **two layers** of authentication on every request:
  - **Developer credentials**: `devid`, `devpassword`, `softname` (obtained by registering your
    app with ScreenScraper via their forum).
  - **User credentials** (optional but recommended): `ssid`, `sspassword` (the end-user's
    ScreenScraper account).
- Store credentials in environment variables or a config file. **Never hardcode them.**

## Important Constraints
- ScreenScraper API is **free for free/open-source software only**. Commercial use requires
  explicit permission from the ScreenScraper team.
- No-Intro DAT files are downloadable from https://datomatic.no-intro.org/ (account required).
- Redump DAT files are downloadable from https://redump.info/downloads/ (no account required).
  Redump's former domain `redump.org` is defunct — always use `redump.info` (same URL paths).
- ScreenScraper API v2 is in **beta** — endpoints may change without notice.
- Anonymous (unauthenticated) users have severely limited thread/request quotas.
- Implement exponential backoff and respect `maxrequestspermin` and `maxrequestsperday`.

## Caching and Storage Strategy
- **DAT files**: Parse once at startup and build in-memory hash indexes (HashMap keyed by CRC32,
  MD5, SHA1). For large collections, consider a persistent key-value store or SQLite database.
- **ScreenScraper responses**: Cache aggressively — game metadata rarely changes. Store responses
  on disk keyed by game ID or ROM hash. Include a timestamp so stale entries can be refreshed
  periodically (e.g., monthly).
- **Media files**: Download once and store locally. Use the ScreenScraper-provided checksums to
  verify integrity and detect updates.

## Other Cataloging Standards
- **TOSEC** (The Old School Emulation Center) is another cataloging standard that covers a broader
  range of platforms and software types (demos, magazine coverdiscs, applications) beyond what
  No-Intro and Redump cover. TOSEC also uses Logiqx DAT files but has its own naming convention.

## More Detail

- For details on using the screenscraper.fr API, see [ScreenScraper API](ScreenScraperAPI.md)
- For details on reading No-Intro DAT files, see [NoIntro DAT](NoIntroDAT.md)
- For details on LibRetro enhanced DATs, see [LibRetroDB](LibRetroDB.md)
