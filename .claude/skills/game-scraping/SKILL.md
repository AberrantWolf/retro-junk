---
skill: game-scraping
description: Knowledge on how to access and use various data sources for identifying games and acquiring various media and metadata relating to those games.
---

# ROM Identification, Metadata, & Media

## Overview
This project builds a program that identifies game ROMs by their checksums and retrieves
metadata/media using multiple sources:
1. **ScreenScraper.fr API** — A community-driven retro game database with rich metadata and media.
2. **No-Intro DAT files** — Curated catalogs of verified cartridge/ROM-chip dumps.
3. **Redump DAT files** — Curated catalogs of verified optical disc dumps (CD/DVD/GD-ROM/Blu-ray).

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
- Redump DAT files are downloadable from http://redump.org/downloads/ (no account required).
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
