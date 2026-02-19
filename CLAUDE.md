# retro-junk

Rust workspace for analyzing retro game ROM files and disc images. Identifies format, extracts header metadata, and validates file integrity.

**IMPORTANT:** When learning about consoles and file formats, always document where information was learned. It is important to cache knowledge, as well as to give credit where that knowledge came from originally.

- The correct location for documenting file formats is: `.claude/skills/retro-archive/formats/`
- The correct location for documenting game system and archival information is: `.claude/skills/retro-archive/consoles/`

## Build & Test

```bash
cargo build                              # build all crates
cargo test                               # test all crates
cargo test -p retro-junk-nintendo        # test one crate
cargo install --path retro-junk-cli      # install CLI
cargo run -p retro-junk-cli -- list      # run without installing
cargo run -p retro-junk-cli -- analyze --root /path/to/roms
```

## Architecture

**Workspace crates:**
- `retro-junk-core` — bottom-level types and traits (`RomAnalyzer`, `ReadSeek`, `RomIdentification`, `AnalysisError`, `Region`, `AnalysisOptions`, `DatSource`)
- `retro-junk-nintendo` — NES, SNES, N64, GameCube, Wii, Wii U, GB, GBA, DS, 3DS
- `retro-junk-sony` — PS1, PS2, PS3, PSP, Vita
- `retro-junk-sega` — SG-1000, Master System, Genesis, Sega CD, 32X, Saturn, Dreamcast, Game Gear
- `retro-junk-microsoft` — Xbox, Xbox 360
- `retro-junk-dat` — DAT file parsing and caching ONLY (no console-specific logic)
- `retro-junk-lib` — glue layer: hasher, rename/matching, `AnalysisContext`. Re-exports `retro-junk-core` types for convenience.
- `retro-junk-cli` — CLI frontend (clap)
- `retro-junk-gui` — GUI frontend (stub)

**Dependency graph:**
```
retro-junk-core          (types, traits)
       |
  +----+----+----+
  |    |    |    |
 nintendo sega sony microsoft   (analyzers, DAT trait impls)
  |    |    |    |
  +----+----+----+
       |         |
  retro-junk-dat |   (parsing + caching)
       |         |
  retro-junk-lib     (glue: hasher, rename, AnalysisContext)
       |
  CLI / GUI          (thin presentation)
```

**Key types:**
- `RomAnalyzer` trait (in `retro-junk-core`) — central abstraction; each console implements this, including DAT-related methods
- `RomIdentification` — output struct returned by analyzers (builder pattern)
- `AnalysisContext` (in `retro-junk-lib`) — registry of all analyzers; used by CLI/GUI to dispatch
- `AnalysisError` — error enum using `thiserror`
- `ReadSeek` — trait alias for `Read + Seek` used as the reader parameter

**DAT support via trait methods on `RomAnalyzer`:**
- `dat_source()` — returns `DatSource::NoIntro` (default, cartridge) or `DatSource::Redump` (disc-based consoles); determines the download base URL
- `dat_names()` — returns DAT display names as a slice (e.g., `&["Nintendo - Nintendo 64"]`); multi-DAT consoles return multiple entries, all merged into one `DatIndex`
- `dat_download_ids()` — returns download identifiers for URL construction; defaults to `dat_names()` (No-Intro). Redump consoles override to return system slugs (e.g., `&["psx"]`)
- `has_dat_support()` — convenience: true when `dat_names()` is non-empty
- `dat_header_size()` — bytes to skip before hashing (e.g., 16 for iNES header)
- `dat_chunk_normalizer()` — optional closure for byte-order normalization (e.g., N64 format detection)
- `extract_dat_game_code()` — extracts short game code from full serial (e.g., `NUS-NSME-USA` → `NSME`)

**Scraper support via trait methods on `RomAnalyzer`:**
- `extract_scraper_serial()` — adapts serial for ScreenScraper API lookups; defaults to `extract_dat_game_code()`, override per-console when ScreenScraper needs a different format

Platform crates own ALL console-specific knowledge. No console-specific code exists in `retro-junk-core`, `retro-junk-dat`, or `retro-junk-lib`.

## Implementing a New Analyzer

Use `retro-junk-nintendo/src/nes.rs` as the reference implementation.

1. Create `src/<console>.rs` in the platform crate
2. Implement `RomAnalyzer` for your struct:
   - `analyze()` — parse header, return `RomIdentification`
   - `can_handle()` — detect via magic bytes, return bool
   - `platform_name()`, `short_name()`, `folder_names()`, `manufacturer()`, `file_extensions()` — return `&'static str` / `&'static [&'static str]`
   - `analyze_with_progress()` — delegate to `analyze()` for small ROMs
   - Optionally override DAT methods: `dat_source()`, `dat_names()`, `dat_download_ids()`, `dat_header_size()`, `dat_chunk_normalizer()`, `extract_dat_game_code()`
   - Optionally override scraper methods: `extract_scraper_serial()` (defaults to `extract_dat_game_code()`)
3. Re-export from the platform crate's `lib.rs`
4. Register in `retro-junk-cli/src/main.rs` `create_context()`

## Shared Code Principles

- **One implementation per algorithm.** Hashing, checksum, and byte-order normalization must have
  exactly one canonical implementation. N64 byte-order code lives in `retro-junk-nintendo/src/n64_byteorder.rs`.
  The hasher in `retro-junk-lib` uses analyzer trait methods to delegate platform-specific logic.
- **Serial format normalization** lives in `retro-junk-dat/src/matcher.rs` — the single place that
  bridges analyzer serial output (e.g., `NUS-NSME-USA`) to DAT serial lookup (e.g., `NSME`).
  Game code extraction is done by `analyzer.extract_dat_game_code()` and passed to the matcher.
- **DAT sources:** Two different sources depending on console type:
  - **No-Intro** (cartridge consoles): LibRetro enhanced DATs from `libretro/libretro-database`
    (`metadat/no-intro/`). These are a strict superset of standard No-Intro DATs with serial,
    region, and release date fields. `dat_download_ids()` defaults to `dat_names()`.
  - **Redump** (disc consoles): Downloaded directly from redump.org (`http://redump.org/datfile/{id}/serial,version`).
    The `/serial,version` parameter includes serial numbers in the DAT. Downloads are ZIP archives
    containing a .dat file. `dat_download_ids()` returns system slugs (e.g., `"psx"`, `"ps2"`,
    `"dc"`) used in the URL path.
  **Known issue:** Redump's serial data reflects catalog/product serials (printed on the box),
  not per-disc boot serials (from `SYSTEM.CNF`). For most games these are identical, but some
  multi-disc games list the same catalog serial for all discs (e.g., all three FF7 USA discs
  list `SCUS-94163` instead of per-disc `SCUS-94163`/`94164`/`94165`). This affects both
  redump.org's own DATs and the libretro-database mirror. Hash fallback handles these cases.
  See `.claude/skills/retro-archive/formats/PSX.md` for details.

## Conventions

- **Builder pattern** on `RomIdentification`: chain `.with_serial()`, `.with_internal_name()`, `.with_region()`, `.with_platform()`; set other fields directly
- **Platform-specific data** goes in the `extra: HashMap<String, String>` field (e.g., mapper, mirroring, format)
- **Checksums** use `checksum_status:<name>` keys in `extra` for display
- **`&'static str`** for all analyzer metadata methods (platform name, extensions, folder names)
- **`thiserror`** for errors; use `AnalysisError::invalid_format()`, `corrupted_header()`, `unsupported()` constructors
- **Magic byte detection** in `can_handle()` — peek and rewind via `SeekFrom::Start(0)`
- **Edition 2024**, workspace-level package metadata
