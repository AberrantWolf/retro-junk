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
- `retro-junk-lib` — core types and `RomAnalyzer` trait
- `retro-junk-nintendo` — NES, SNES, N64, GameCube, Wii, Wii U, GB, GBA, DS, 3DS
- `retro-junk-sony` — PS1, PS2, PS3, PSP, Vita
- `retro-junk-sega` — SG-1000, Master System, Genesis, Sega CD, 32X, Saturn, Dreamcast, Game Gear
- `retro-junk-microsoft` — Xbox, Xbox 360
- `retro-junk-cli` — CLI frontend (clap)
- `retro-junk-gui` — GUI frontend (stub)

**Key types in `retro-junk-lib`:**
- `RomAnalyzer` trait — central abstraction; each console implements this
- `RomIdentification` — output struct returned by analyzers (builder pattern)
- `AnalysisContext` — registry of all analyzers; used by CLI to dispatch
- `AnalysisError` — error enum using `thiserror`
- `ReadSeek` — trait alias for `Read + Seek` used as the reader parameter

## Implementing a New Analyzer

Use `retro-junk-nintendo/src/nes.rs` as the reference implementation.

1. Create `src/<console>.rs` in the platform crate
2. Implement `RomAnalyzer` for your struct:
   - `analyze()` — parse header, return `RomIdentification`
   - `can_handle()` — detect via magic bytes, return bool
   - `platform_name()`, `short_name()`, `folder_names()`, `manufacturer()`, `file_extensions()` — return `&'static str` / `&'static [&'static str]`
   - `analyze_with_progress()` — delegate to `analyze()` for small ROMs
3. Re-export from the platform crate's `lib.rs`
4. Register in `retro-junk-cli/src/main.rs` `create_context()`

## Shared Code Principles

- **One implementation per algorithm.** Hashing, checksum, and byte-order normalization must have
  exactly one canonical implementation. Currently these live in `retro-junk-lib` (e.g.,
  `retro-junk-lib::n64` for N64 byte-order detection and normalization). Both analyzers and DAT
  matching import from there.
- **Serial format normalization** lives in `retro-junk-dat/src/matcher.rs` — the single place that
  bridges analyzer serial output (e.g., `NUS-NSME-USA`) to DAT serial lookup (e.g., `NSME`).
- **DAT source:** LibRetro enhanced DATs from `libretro/libretro-database` (not `libretro-mirrors/nointro-db`).
  These are a strict superset of standard No-Intro DATs with serial, region, and release date fields.

## Planned Restructuring

The current architecture puts all DAT-related logic (parsing, caching, matching, renaming) in
`retro-junk-dat`, with CLI commands implemented directly in `retro-junk-cli`. This means a future
GUI would have to reimplement the command orchestration logic.

**Target architecture:**
- `retro-junk-core` (rename current `retro-junk-lib`) — bottom-level crate with core types, traits
  (`RomAnalyzer`, `ReadSeek`, `RomIdentification`, `AnalysisError`, `Region`), and shared
  platform-specific utilities (N64 byte-order, etc.)
- `retro-junk-nintendo/sega/sony/microsoft` — analyzer implementations, depend on `core`
- `retro-junk-dat` — DAT file parsing and caching ONLY, depend on `core`
- `retro-junk-lib` (new glue layer) — matching, renaming, analysis orchestration, `AnalysisContext`,
  progress reporting. Depends on `core`, `dat`, and all platform crates. This is where "doing things"
  lives.
- `retro-junk-cli` / `retro-junk-gui` — thin presentation layers over `lib`

**Why:** Platform-specific logic (like N64 byte-order normalization) must not be duplicated across
crates. The glue layer gives both CLI and GUI access to the same matching/renaming/analysis logic
without reimplementation.

## Conventions

- **Builder pattern** on `RomIdentification`: chain `.with_serial()`, `.with_internal_name()`, `.with_region()`, `.with_platform()`; set other fields directly
- **Platform-specific data** goes in the `extra: HashMap<String, String>` field (e.g., mapper, mirroring, format)
- **Checksums** use `checksum_status:<name>` keys in `extra` for display
- **`&'static str`** for all analyzer metadata methods (platform name, extensions, folder names)
- **`thiserror`** for errors; use `AnalysisError::invalid_format()`, `corrupted_header()`, `unsupported()` constructors
- **Magic byte detection** in `can_handle()` — peek and rewind via `SeekFrom::Start(0)`
- **Edition 2024**, workspace-level package metadata
