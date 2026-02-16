---
description: LibRetro database — enhanced No-Intro DATs with serial, region, and release date fields
---

# LibRetro Database (Enhanced No-Intro DATs)

## Overview

The LibRetro project maintains enhanced versions of No-Intro DAT files at
`github.com/libretro/libretro-database`. These are a **superset** of standard No-Intro data — same
game names, same ROM hashes (name/size/crc/md5/sha1), plus additional fields like `serial`,
`region`, `releaseyear`, `releasemonth`, and `releaseday`.

This makes them a **drop-in replacement** for standard No-Intro DATs — no data is lost, only gained.

**Source credit:** Maintained by Rob Loach et al. under the LibRetro project.

## Repository Location

- **Repo:** `https://github.com/libretro/libretro-database`
- **No-Intro DATs:** `metadat/no-intro/` directory (ClrMamePro format)
- **Redump DATs:** `metadat/redump/` directory
- **Other metadata:** `metadat/developer/`, `metadat/publisher/`, `metadat/genre/`, etc.

## How to Obtain

Clone the repo or download individual DAT files via raw GitHub URLs:

```
https://raw.githubusercontent.com/libretro/libretro-database/master/metadat/no-intro/Nintendo%20-%20Nintendo%2064.dat
```

DAT filenames match the system name (e.g., `Nintendo - Nintendo 64.dat`,
`Sega - Mega Drive - Genesis.dat`).

## Enhanced Fields

LibRetro DATs use ClrMamePro format with these extra fields beyond standard No-Intro:

| Field          | Level      | Description                                            |
| -------------- | ---------- | ------------------------------------------------------ |
| `serial`       | Game + ROM | Product/game code (e.g., "NO7E", "T-48073-00", "BJBE") |
| `region`       | Game       | Region string (e.g., "USA", "Japan", "Europe")         |
| `releaseyear`  | Game       | 4-digit release year (e.g., "1998")                    |
| `releasemonth` | Game       | 2-digit release month (e.g., "12")                     |
| `releaseday`   | Game       | 2-digit release day (e.g., "15")                       |

The `serial` field appears at both game level and inside `rom ()` — they are duplicated. The
game-level serial is authoritative.

## Serial Coverage by System

Serial field availability varies significantly by system:

| System        | DAT Name                                       | Serials?                           | Notes                              |
| ------------- | ---------------------------------------------- | ---------------------------------- | ---------------------------------- |
| N64           | Nintendo - Nintendo 64                         | **Yes** (most games)               | 4-char game code (e.g., "NO7E")    |
| Genesis       | Sega - Mega Drive - Genesis                    | **Yes** (most games)               | Product codes (e.g., "T-48073-00") |
| GBA           | Nintendo - Game Boy Advance                    | **Yes** (most games)               | 4-char game code (e.g., "BJBE")    |
| NDS           | Nintendo - Nintendo DS                         | **Yes** (most games, ~15k entries) | Game codes                         |
| GBC           | Nintendo - Game Boy Color                      | **Partial** (~152 entries)         | Limited coverage                   |
| 32X           | Sega - 32X                                     | **Partial** (~12 entries)          | Very limited                       |
| SNES          | Nintendo - Super Nintendo Entertainment System | **Minimal** (~28 entries)          | Mostly late-era titles             |
| NES           | Nintendo - Nintendo Entertainment System       | **Minimal** (~8 entries)           | Mostly Korean/beta titles          |
| GB            | Nintendo - Game Boy                            | **No**                             | No serial fields                   |
| Master System | Sega - Master System - Mark III                | **No**                             | No serial fields                   |
| Game Gear     | Sega - Game Gear                               | **No**                             | No serial fields                   |

Games without serials in the DAT still work fine — they fall back to hash-based matching only.

## Comparison to Standard No-Intro DATs

| Aspect           | Standard No-Intro                         | LibRetro Enhanced           |
| ---------------- | ----------------------------------------- | --------------------------- |
| Format           | Logiqx XML                                | ClrMamePro text             |
| Game names       | Identical                                 | Identical                   |
| ROM hashes       | Identical (CRC/MD5/SHA1)                  | Identical (CRC/MD5/SHA1)    |
| Serial numbers   | Not included                              | Included (where available)  |
| Region field     | Not included                              | Included                    |
| Release dates    | Not included                              | Included                    |
| Source           | datomatic.no-intro.org (account required) | GitHub (public, no account) |
| Update frequency | Daily updates available                   | Updated periodically        |

## Example Entry

```
game (
	name "GoldenEye 007 (USA)"
	region "USA"
	serial "NGEE"
	releaseyear "1997"
	releasemonth "8"
	releaseday "25"
	rom ( name "GoldenEye 007 (USA).z64" size 12582912 crc DBC23B14 md5 ... sha1 ... serial "NGEE" )
)
```

## Serial Format Differences: ROM Headers vs DAT Entries

ROM analyzers extract full serial strings from headers, but LibRetro DATs typically store shorter
game codes. The matcher (`retro-junk-dat/src/matcher.rs`) bridges this gap by extracting the core
game code from prefixed formats.

| System  | Analyzer Output | DAT Serial   | Core Code                     |
| ------- | --------------- | ------------ | ----------------------------- |
| N64     | `NUS-NSME`      | `NSME`       | `NSME`                        |
| GBA     | `AGB-BJBE`      | `BJBE`       | `BJBE`                        |
| NDS     | `NTR-ADME-USA`  | `ADME`       | `ADME`                        |
| Genesis | `T-48073-00`    | `T-48073-00` | (kept as-is)                  |
| SNES    | `SNS-ZL-USA`    | `SNS-ZL-USA` | (kept as-is, variable length) |

The extraction handles `NUS-`, `AGB-`, `NTR-`, `DMG-`, `CGB-` prefixes by taking the second
hyphen-delimited segment. Other formats (Sega product codes, SNES codes) are matched as-is.

## Preference

**LibRetro enhanced DATs are preferred over standard No-Intro DATs** because:

1. They are a strict superset (no data loss)
2. They include serial numbers useful for cross-referencing with ROM header data
3. They are publicly accessible on GitHub without requiring an account
4. They include region and release date metadata
