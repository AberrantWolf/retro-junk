---
description: Step-by-step guide for implementing a new ROM analyzer, including DAT and scraper trait methods
---

# Implementing a New Analyzer

Use `retro-junk-nintendo/src/nes.rs` as the reference implementation.

1. Create `src/<console>.rs` in the platform crate
2. Implement `RomAnalyzer` for your struct:
   - `analyze()` — parse header, return `RomIdentification`
   - `can_handle()` — detect via magic bytes, return bool
   - `platform_name()`, `short_name()`, `folder_names()`, `manufacturer()`, `file_extensions()` — return `&'static str` / `&'static [&'static str]`
   - `analyze_with_progress()` — delegate to `analyze()` for small ROMs
   - Optionally override DAT methods (see below)
   - Optionally override scraper methods (see below)
3. Re-export from the platform crate's `lib.rs`
4. Register in `retro-junk-cli/src/main.rs` `create_context()`

## DAT Support via Trait Methods on `RomAnalyzer`

These methods control how an analyzer integrates with DAT file matching:

- `dat_source()` — returns `DatSource::NoIntro` (default, cartridge) or `DatSource::Redump` (disc-based consoles); determines the download base URL
- `dat_names()` — returns DAT display names as a slice (e.g., `&["Nintendo - Nintendo 64"]`); multi-DAT consoles return multiple entries, all merged into one `DatIndex`
- `dat_download_ids()` — returns download identifiers for URL construction; defaults to `dat_names()` (No-Intro). Redump consoles override to return system slugs (e.g., `&["psx"]`)
- `has_dat_support()` — convenience: true when `dat_names()` is non-empty
- `dat_header_size()` — bytes to skip before hashing (e.g., 16 for iNES header)
- `dat_chunk_normalizer()` — optional closure for byte-order normalization (e.g., N64 format detection)
- `extract_dat_game_code()` — extracts short game code from full serial (e.g., `NUS-NSME-USA` → `NSME`)

## Scraper Support via Trait Methods on `RomAnalyzer`

- `extract_scraper_serial()` — adapts serial for ScreenScraper API lookups; defaults to `extract_dat_game_code()`, override per-console when ScreenScraper needs a different format

## DAT Source Selection

- **No-Intro** (cartridge consoles): LibRetro enhanced DATs from `libretro/libretro-database` (`metadat/no-intro/`). `dat_download_ids()` defaults to `dat_names()`.
- **Redump** (disc consoles): Downloaded from redump.org (`http://redump.org/datfile/{id}/serial,version`). `dat_download_ids()` returns system slugs (e.g., `"psx"`, `"ps2"`, `"dc"`).

See the [game-scraping skill](/Users/scott/Programming/rust/retro-junk/.claude/skills/game-scraping/SKILL.md) for full details on DAT sources, formats, and known issues.
