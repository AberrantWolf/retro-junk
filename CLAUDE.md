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

DAT and scraper integration is implemented via trait methods on `RomAnalyzer`. See `.claude/skills/retro-archive/new-analyzer.md` for the full trait method reference and new-analyzer checklist.

Platform crates own ALL console-specific knowledge. No console-specific code exists in `retro-junk-core`, `retro-junk-dat`, or `retro-junk-lib`.

## Shared Code Principles

- **One implementation per algorithm.** Hashing, checksum, and byte-order normalization have exactly one canonical implementation. The hasher in `retro-junk-lib` delegates platform-specific logic via analyzer trait methods.
- **Serial format normalization** lives in `retro-junk-dat/src/matcher.rs` — the single place bridging analyzer serial output to DAT serial lookup.
- **DAT sources:** No-Intro (cartridge, via LibRetro enhanced DATs) and Redump (disc, from redump.org). See `.claude/skills/game-scraping/` for full details.

**IMPORTANT**: Prioritize code change suggestions that avoid repeated code! Actively look for ways to keep the codebase "DRY". With every plan, include a section about how the plan keeps the code base from having meaningful chunks of repeated logic in multiple places.

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
