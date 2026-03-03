# TODO

## Bugs


## Features

- [ ] **Database management GUI** screen for all sorts of database tasks, including viewing and merging conflicts, importing and previewing enrichment, and maybe even direct database editing

- [ ] **Move media and data on rename** — If we've already scraped media and rename a game, we need to move the data associated with it (images, gamelist.xml entries, etc. under `roms-media/`).

- [ ] **Figure out multi-file WBFS setups** - I don't know what we're meant to do with them or how to treat them

- [ ] **Custom multi-select view** in the game details panel, rather than showing details for the most-recent selection in the list

## Analyzer: Compressed Disc Formats

- [ ] **GameCube NKit support** — NKit is a lossy-compressed format (`.nkit.iso`, `.nkit.gcz`) that removes junk/padding data. Hashes will not match Redump unless converted back to full ISO. May need special handling or a warning that NKit images can't be verified against Redump.

- [ ] **Check nod v2.0 stability** — The `nod` crate v2.0 may bring API changes. Check for stability and migration when it releases.

## Data Model & Import Pipeline

- [ ] **Re-import after migration v4** — Schema is now at version 4 (`screen_title`, `cover_title` columns added in v3). Run `catalog import all` followed by `catalog enrich` on existing databases to populate `revision`, `variant`, `screen_title`, and `cover_title` fields. This is a one-time user/ops action, not a code gap.

## CLI

- [ ] **Flesh out `list` command output** — `catalog lookup --type platforms` already shows ID, name, manufacturer, year, media type, release/media counts. Still missing: enrichment coverage percentage and a `--verbose` flag. The standalone `list` command (for analyzer metadata: extensions, folder names, DAT support) could also benefit from additional columns.

## Web Frontend

- [ ] **Create `retro-junk-web` crate** — Web-based frontend for browsing and managing the catalog. Initial scope:
  - Browse platforms, releases, and media with search/filter
  - View release details with associated media assets (box art, screenshots)
  - Collection management (mark owned, add notes)
  - Disagreement review and resolution UI
  - Import/enrichment status and progress
  - Asset coverage dashboard (which releases are missing art)
  - Stack: Axum for HTTP, askama or maud for templates, htmx for interactivity, SQLite read access via shared connection pool. Keep it server-rendered; no SPA framework needed.

## Ideas

- [ ] **Handle modded games and homebrew in library** — Games that are modded or homebrew will never match a DAT and show as red (Unrecognized) permanently, cluttering the console list with false-negative indicators. Think about ways to mark or categorize these (e.g., user-applied "homebrew"/"mod" tag, a separate status like `Excluded`, or a filter to hide them from status rollups) so the console list isn't stuck showing red dots.

- [ ] **Auto-resolve trivial disagreements** — Many disagreements are cosmetic (trailing periods, capitalization, date format differences). Add configurable rules to auto-resolve obvious cases during enrichment, reducing manual review noise.

- [ ] **Export to ES-DE / other frontends** — The `scrape` command and GUI already generate ES-DE gamelist.xml for individual systems. Add a `catalog export` command that generates gamelists from the catalog DB for any/all platforms, pulling metadata and asset paths from the database rather than re-scraping.

- [ ] **Collection verification report** — Extend `catalog verify` to produce a summary report: missing ROMs (in DB but not on disk), unmatched files (on disk but not in DB), hash mismatches, and duplicate ROMs across folders.

- [ ] **DAT freshness checking** — Track when each DAT was last downloaded and warn when DATs are stale. Optionally auto-fetch updated DATs before import.

- [ ] **Multi-disc release grouping improvements** — Currently multi-disc games are grouped by title + region + revision + variant. Consider edge cases: different disc counts across regions, bonus discs, demo discs bundled with retail releases.

- [ ] **ROM health dashboard** — Aggregate view across all platforms: total ROMs scanned, verified vs. unverified, trimmed/padded/repaired, missing from known sets (have DAT entry but no matching file in collection).

- [ ] **Overrides YAML expansion** — The overrides system exists but has limited use. Expand with curated override sets for known problem areas: multi-disc serial mismatches (FF7, etc.), regional title corrections, and publisher name normalization.

- [ ] **Apply game mods** - Most mods come as binary modifications to known-good game hashes, and if your game library applies the mod, then it can also automatically flag it as a mod and adjust the metadata correctly and automatically.

- [ ] **Consider using an ORM** crate to help with data types and database management

## Code Health: DRY Violations

Audit findings from 2026-02-26.

### Shared utility functions

- [ ] **Consolidate byte-reading helpers within Nintendo crate** — `retro-junk-nintendo/src/ds.rs:85-98` still defines private `read_u16_le()` and `read_u32_le()` that duplicate what `n3ds/common.rs:18-68` provides as `pub(crate)`. Either have `ds.rs` import from `n3ds::common`, or extract to a shared `nintendo_util` module in the Nintendo crate.

- [x] **Extract `get_file_size()` helper** — Added `retro_junk_core::util::file_size()` and replaced ~25 instances of the seek-to-end/seek-to-start pattern across all analyzer crates.

- [ ] **Extract header-reading helper with TooSmall error mapping** — The pattern of `read_exact` + `map_err` converting `UnexpectedEof` to `AnalysisError::TooSmall` appears in `nes.rs:569`, `snes.rs:348`, `gameboy.rs:69`, `gba.rs:61`, `n64.rs:129`, `ds.rs:105`, `ncsd.rs:50`, `genesis.rs:176`, `ps1_disc.rs:161`, and others. Add a helper to `retro-junk-core`:
  ```rust
  pub fn read_header(reader: &mut dyn ReadSeek, buf: &mut [u8], expected: u64) -> Result<(), AnalysisError>
  ```

- [x] **Remove trivial `new()` methods from analyzer structs** — Removed 28 trivial `new()` methods from analyzer structs and `EsDeFrontend`. Updated ~250 call sites to use unit struct literals.

- [ ] **Unify `check_broken_references` and `detect_broken_ref_files`** — `rename.rs` has two functions that both iterate a directory, filter by CUE/M3U extensions, read file contents, call `fmt.extract_reference(line)`, and check `.exists()`. They differ only in return type (`BrokenReference` structs vs. file paths). Unify so `detect_broken_ref_files` is implemented in terms of `check_broken_references`.

- [ ] **Extract GUI semantic color palette** — The same logical colors are hardcoded in 4+ GUI files:
  - Warning orange `Color32::from_rgb(230, 160, 30)` — `status_badge.rs`, `detail_panel.rs`
  - Error red `Color32::from_rgb(220, 50, 50)` — `state.rs`, `app.rs`, `detail_panel.rs`
  - Matched green `Color32::from_rgb(50, 180, 50)` — `state.rs`, `app.rs`
  - Ambiguous yellow `Color32::from_rgb(220, 180, 30)` — `state.rs`, `app.rs`, `detail_panel.rs`

  Extract to named constants in a `theme` or `palette` module. `EntryStatus::color()` in `state.rs` partially centralizes this but other callsites bypass it.

### Test helpers

- [ ] **Extract shared test database setup** — Multiple test files implement similar SQLite test database setup:
  - `retro-junk-db/tests/queries.rs` — `setup_db()`, `setup_db_with_assets()`
  - `retro-junk-import/tests/dat_import.rs` — `setup_db()`
  - `retro-junk-import/tests/scan_import.rs` — `setup_db_with_media()`
  - `retro-junk-import/tests/merge.rs` — `setup_db_with_release()`

  Create a shared `test_helpers` module (e.g., in `retro-junk-db` behind a `#[cfg(test)]` or as a dev-dependency feature) that provides reusable setup functions.

## Code Health: GUI Architecture

Audit findings from 2026-02-27.

- [ ] **Decompose `handle_message`** — `state.rs:handle_message` is 787 lines. Each `AppMessage` match arm should be extracted to a named private handler function for readability and testability.

- [ ] **`check_broken_refs_background` lacks cancellation and progress** — The background thread spawned by `scan.rs:check_broken_refs_background` uses `std::thread::spawn` directly (not `spawn_background_op`) and has no cancel token, no progress messages, and only calls `ctx.request_repaint()` once at the end. On a large library this means multi-second blocking with no feedback. Consider batching repaints every N entries or wrapping in `spawn_background_op` with a cancel token.

## Code Health: Cleanup

- [ ] **Remove dead `CliError` variants** — `retro-junk-cli/src/error.rs:28,32` defines `DatError` and `Analysis` variants (and constructors at lines 56, 60) that are never constructed. Remove or use them.

## Enrichment Pipeline Hardening

Audit findings from 2026-02-25. Goal: make `catalog enrich` reliable enough to run hands-off on a server for months.

All 15 items resolved — see commit history for details.
