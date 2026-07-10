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

*Analysis foundation:*
- `retro-junk-core` — bottom-level types and traits (`RomAnalyzer`, `ReadSeek`, `RomIdentification`, `AnalysisError`, `Region`, `AnalysisOptions`, `DatSource`)
- `retro-junk-disc` — shared CD-ROM/optical disc utilities: ISO 9660 parsing, CUE sheets, CHD reading, track-aware hashing
- `retro-junk-nintendo` — NES, SNES, N64, GameCube, Wii, Wii U, GB, GBA, DS, 3DS
- `retro-junk-sony` — PS1, PS2, PS3, PSP, Vita
- `retro-junk-sega` — SG-1000, Master System, Genesis, Sega CD, 32X, Saturn, Dreamcast, Game Gear
- `retro-junk-microsoft` — Xbox, Xbox 360
- `retro-junk-dat` — DAT file parsing (No-Intro XML, Redump CSV) and GDB caching (no console-specific logic)
- `retro-junk-lib` — glue layer: hasher, rename/matching, `AnalysisContext` registering all analyzers. Re-exports `retro-junk-core` types for convenience.

*Catalog foundation:*
- `retro-junk-catalog` — game catalog data model (`Work`, `Release`, `Media`, `Asset`, `Disagreement`, `Override`); YAML I/O; no DB
- `retro-junk-db` — SQLite persistence for the catalog: schema/migrations, CRUD, library-entry tracking

*Cross-cutting:*
- `retro-junk-frontend` — gaming-frontend metadata generators (ES-DE gamelist.xml, miximage composition); depends only on `core`
- `retro-junk-scraper` — ScreenScraper API client, credentials, asset download, system-code mapping
- `retro-junk-import` — ETL pipeline: DAT → catalog, enrichment, disagreement detection, local-collection reconciliation

*Presentation:*
- `retro-junk-cli` — CLI frontend (clap)
- `retro-junk-gui` — desktop GUI (egui/eframe)
- `retro-junk-gui-cjk` — thin wrapper around `retro-junk-gui` with the full CJK font feature enabled

**Dependency graph:**
```
    retro-junk-core                retro-junk-catalog
         |                                |
    retro-junk-disc                  retro-junk-db
         |                                |
    +----+----+----+                      |
    |    |    |    |                      |
 nintendo sony sega microsoft             |
    |    |    |    |                      |
    +----+----+----+                      |
         |                                |
    retro-junk-dat                        |
         |                                |
    retro-junk-lib     retro-junk-frontend
         |                    |
         +-- retro-junk-scraper
                    |
              retro-junk-import  ---------+
                    |
          CLI / GUI / GUI-CJK   (presentation)
```

Notes:
- `retro-junk-disc` is used by the disc-based CD/DVD platform crates (`sony`, `sega`). Cartridge-only crates (`nintendo`, `microsoft`) depend only on `core`. Nintendo disc analyzers (GameCube, Wii) use the `nod` crate directly rather than `retro-junk-disc`; Xbox/360 use their own ISO handling.
- `retro-junk-frontend` is `core`-only by design — it describes output formats for ES-DE and similar, not analysis.
- `retro-junk-import` is the only crate that bridges the analysis and catalog foundations.

**Key types:**
- `RomAnalyzer` trait (in `retro-junk-core`) — central abstraction; each console implements this, including DAT-related methods
- `RomIdentification` — output struct returned by analyzers (builder pattern)
- `AnalysisContext` (in `retro-junk-lib`) — registry of all analyzers; used by CLI/GUI to dispatch
- `AnalysisError` — error enum using `thiserror`
- `ReadSeek` — trait alias for `Read + Seek` used as the reader parameter
- Catalog model (in `retro-junk-catalog`): `Work` (abstract game), `Release` (region-specific variant), `Media` (ROM/disc file), `Asset` (box art/screenshot/etc.), `Disagreement` (cross-source conflict), `Override` (user correction)

DAT and scraper integration is implemented via trait methods on `RomAnalyzer`. See `.claude/skills/retro-archive/new-analyzer.md` for the full trait method reference and new-analyzer checklist.

Platform crates own ALL console-specific knowledge. No console-specific code exists in `retro-junk-core`, `retro-junk-dat`, or `retro-junk-lib`.

## Shared Code Principles

- **One implementation per algorithm.** Hashing, checksum, and byte-order normalization have exactly one canonical implementation. The hasher in `retro-junk-lib` delegates platform-specific logic via analyzer trait methods.
- **Serial format normalization** lives in `retro-junk-dat/src/matcher.rs` — the single place bridging analyzer serial output to DAT serial lookup.
- **DAT sources:** No-Intro (cartridge, via LibRetro enhanced DATs) and Redump (disc, from redump.info). See `.claude/skills/game-scraping/` for full details.
- **Catalog is the long-lived store.** Raw DAT files seed and update the catalog DB; the DB is intended to become the matching source of truth over time. Ephemeral `DatIndex` lookup from raw DATs is the current runtime path but is being migrated out (see TODO.md) — don't deepen coupling to it.

**IMPORTANT**: Prioritize code change suggestions that avoid repeated code! Actively look for ways to keep the codebase "DRY". With every plan, include a section about how the plan keeps the code base DRY, and how the plan improves the codebase.

**IMPORTANT**: Include in the plan a section about how the plan maintains and improves best practices.

**NOTE**: If DRY and best-practices improvements are out of scope for a plan, include a section to document in TODO.md the potential improvements for later updates.

## Conventions

- **Builder pattern** on `RomIdentification`: chain `.with_serial()`, `.with_internal_name()`, `.with_region()`, `.with_platform()`; set other fields directly
- **Platform-specific data** goes in the `extra: HashMap<String, String>` field (e.g., mapper, mirroring, format)
- **Checksums** use `checksum_status:<name>` keys in `extra` for display
- **`&'static str`** for all analyzer metadata methods (platform name, extensions, folder names)
- **`thiserror`** for errors; use `AnalysisError::invalid_format()`, `corrupted_header()`, `unsupported()` constructors
- **Magic byte detection** in `can_handle()` — peek and rewind via `SeekFrom::Start(0)`
- **Edition 2024**, workspace-level package metadata
- **Separate Tests** from the code files, either by a tests/ folder or a code_tests.rs file included by path in the source.
- **Don't Repeat Yourself** (DRY) means that if we're rewriting basically the same thing in multiple places, that should become a shared function
- **Refactor** is better than rewrite
- **Pointless tests** are the kind that are trivially provable -- creating a struct will obviously work, no need to test it, for instance
- **Compressed format hashing** must decompress to the community-standard representation before hashing. DAT databases (Redump, No-Intro) store hashes of uncompressed data, not compressed containers. Analyzers that support compressed formats (e.g., RVZ, CHD) MUST implement `compute_container_hashes()` to decompress and hash the inner data. Never hash compressed container bytes for DAT matching.
