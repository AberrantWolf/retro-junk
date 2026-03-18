# TODO

## Bugs


## Features

- [ ] **Database management GUI** screen for all sorts of database tasks, including viewing and merging conflicts, importing and previewing enrichment, and maybe even direct database editing

- [ ] **Move media and data on rename** — If we've already scraped media and rename a game, we need to move the data associated with it (images, gamelist.xml entries, etc. under `roms-media/`).

- [ ] **Figure out multi-file WBFS setups** - I don't know what we're meant to do with them or how to treat them

- [ ] **Custom multi-select view** in the game details panel, rather than showing details for the most-recent selection in the list

- [ ] **Show hash match status in detail panel** — After hashing, the detail panel shows CRC32/SHA1/MD5 values but doesn't visually indicate whether they match known DAT entries. Add a match/mismatch indicator next to hash values.

## Analyzer: Compressed Disc Formats

- [ ] **GameCube NKit support** — NKit is a lossy-compressed format (`.nkit.iso`, `.nkit.gcz`) that removes junk/padding data. Hashes will not match Redump unless converted back to full ISO. May need special handling or a warning that NKit images can't be verified against Redump.

- [ ] **Check nod v2.0 stability** — The `nod` crate v2.0 may bring API changes. Check for stability and migration when it releases.

## DAT Source Coverage

- [ ] **Wii U has no Redump DAT** — Redump.org has no Wii U disc entries or datfile download. The previous LibRetro "Nintendo - Wii U (Digital)" DAT was not real Redump data. DAT support for Wii U is currently disabled. Options: (1) find an alternative DAT source for Wii U, (2) re-enable using LibRetro's DAT with `DatSource::NoIntro` if the data is good enough, or (3) wait for Redump to add Wii U support.

- [ ] **Verify all Redump slugs work** — After switching disc-based DAT downloads from LibRetro to redump.org direct, verify that all slug mappings actually return valid data: `psx`, `ps2`, `ps3`, `psp`, `ss`, `mcd`, `dc`, `gc`, `wii`, `xbox`, `xbox360`. Some systems may have restricted access or different slug conventions on redump.org.

## Data Model & Import Pipeline

- [ ] **Migrate matching source-of-truth from raw DAT files to catalog DB** —
  Currently, hash/serial matching runs against in-memory `DatIndex` built from
  downloaded DAT files each session. Raw DAT files should only seed, update,
  enrich, and fix the catalog DB. The DB should become the authoritative source
  for matching, enabling persistent corrections (e.g., adding missing regional
  entries, resolving cross-region matches) that survive DAT re-downloads.

- [ ] **Re-import after migration v4** — Schema is now at version 4 (`screen_title`, `cover_title` columns added in v3). Run `catalog import all` followed by `catalog enrich` on existing databases to populate `revision`, `variant`, `screen_title`, and `cover_title` fields. This is a one-time user/ops action, not a code gap.

## CLI

- [x] **Flesh out `list` command output** — Resolved: the standalone `list` command was folded into `catalog lookup`. `catalog lookup --type platforms` shows ID, name, manufacturer, year, media type, release/media counts.

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

- [x] **Consolidate byte-reading helpers within Nintendo crate** — Deleted `ds.rs` private helpers and imported from `n3ds::common` (made `pub(crate)`). Also added bounds checking (`Option<T>` return) to the shared helpers.

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

- [ ] **Decide on user-facing "Media" vs "Assets" terminology** — Rust types were renamed from `MediaType`/`MediaStatus`/etc. to `AssetType`/`AssetStatus`/etc. to disambiguate from physical media types. However, UI strings still say "Scrape Media", "Re-scrape Media", "No scraped media", "Media complete", etc. Decide whether to keep user-facing labels as "Media" (more intuitive to users) or align them with the code terminology ("Assets").

- [x] **Remove dead `CliError` variants** — Removed `DatError` and `Analysis` variants and their constructors.

## Code Health: Safety & Robustness

Audit findings from 2026-03-17. Focus: panic-prone parsing, silent errors, inconsistent patterns.

### Phase 1: Panic-Prone Parsing (Critical)

- [x] **SNES checksum divide-by-zero** — False positive: the else branch only runs when `power != rom_size`, guaranteeing `remainder` is non-empty (`rom_size > power` always holds).

- [x] **N3DS unchecked buffer indexing** — Changed all six helpers in `n3ds/common.rs` to return `Option<T>` with bounds-checked `buf.get()`. Updated all callers in ncsd.rs, ncch.rs, cia.rs, mod.rs. Also consolidated DS duplicate helpers (resolves DRY TODO).

- [x] **Nintendo disc `unwrap()` on slice conversions** — Replaced `try_into().unwrap()` with direct array construction (e.g., `[buf[0x18], buf[0x19], ...]`). Safe because buffer is `[0u8; 0x440]` with all accesses within bounds.

- [x] **ISO 9660 directory record buffer overrun** — Added `if data.len() < 33 { return None; }` upfront check in `parse_directory_record()`.

- [x] **NES header length not validated** — False positive: `parse_ines_header` takes `&[u8; 16]`, a fixed-size array reference. All indexing is compile-time safe.

- [x] **iNES exponent overflow** — Added `if exponent >= 32` guard before both PRG and CHR ROM `1u32 << exponent` shifts, returning `AnalysisError::corrupted_header`.

- [x] **ISO 9660 unbounded memory allocation** — Added `MAX_ISO_FILE_SIZE` (256 MB) constant and validation in both `read_file_content()` and `read_file_from_chd()`.

### Phase 2: Correctness & Data Integrity

- [x] **WiiU missing `dat_source()` override** — Added `fn dat_source() -> DatSource::Redump` to WiiU analyzer.

- [x] **Remove debug `println!` in GameBoy** — Deleted the `println!("gb/c serial: {}", serial)` line.

- [x] **ClrMamePro DAT silent size parsing failure** — Replaced `unwrap_or(0)` with explicit `match` that logs a warning and returns `None` to skip entries with invalid sizes.

- [x] **Miximage `unwrap()` panic** — Replaced `unwrap()` with `if let Some(layout)` pattern.

- [ ] **`unchecked_transaction()` prevents auto-rollback** — `transaction()` requires `&mut Connection` but public API uses `&Connection`. Changing signatures would be a larger refactor. The `unchecked_transaction()` usage is correct for these top-level, non-nested contexts; rollback happens on drop.

- [x] **u64-to-i64 overflow in DAT import** — Replaced bare `as` casts with `i64::try_from().ok()` and `i32::try_from().ok()`.

- [x] **Genesis checksum overflow** — False positive: `u32::MAX as u64 + 1` fits in u64. Replaced `as u64` with explicit `u64::from()` for clarity.

### Phase 3: GUI Silent Error Modes

- [x] **Error dialog for failed operations** — Added `UserError` struct, `error_list` field, `push_error()` helper, and `error_dialog` widget. `HashFailed`, `ScrapeEntryFailed`, `ScrapeFatalError`, `ExportComplete(Err)`, `DatLoadFailed`, and tag dialog DB failures now show a modal error dialog. (Note: these messages were always matched in `handle_message()` and logged, but never surfaced to the user.)

- [ ] **Folder scan errors silent** — `retro-junk-gui/src/backend/scan.rs:40-43`: Scan errors are logged but never shown to users. Empty results are indistinguishable from errors. Low priority — background noise, not user-initiated.

- [ ] **Cache save failures silent** — `retro-junk-gui/src/app.rs:200-228`: `save_library_cache()`, `save_console_cache()`, `save_entry_cache()` failures only produce `log::warn!()`. Low priority — background housekeeping, visible in log viewer.

- [ ] **Loading state can persist forever** — `retro-junk-gui/src/app.rs:73-76`: `loading_library` flag has no timeout. If startup thread crashes, UI shows "Loading..." forever. Same issue with `ScanStatus::Scanning` in `state.rs:95-100`. This is a UI state management issue, not an error dialog issue.

- [ ] **Rate-limit batch error dialogs** — During large batch operations (e.g. hashing hundreds of files), many `HashFailed` errors could flood the dialog. Consider capping displayed errors (e.g. show first N, then "and X more...").

- [ ] **Error dialog "copy to clipboard" button** — Add a button to copy error details for bug reports.

### Phase 4: Analyzer Consistency

- [ ] **Missing `expects_serial()` in 4 analyzers** — NES, SNES, GameBoy, Genesis all implement `extract_dat_game_code()` but don't declare `expects_serial()`, creating ambiguity for DAT diagnostics.

- [ ] **Genesis missing "format" key** — `retro-junk-sega/src/genesis.rs`: All other analyzers insert a `"format"` key into the `extra` HashMap; Genesis does not. Breaks UI display consistency.

- [ ] **Inconsistent seek/rewind in `can_handle()`** — SNES `can_handle()` calls `detect_mapping()` which seeks multiple times but doesn't guarantee reader position reset. Other analyzers (GBA, GameBoy) mix `let _ = reader.seek()` with explicit error checking.

### Phase 5: API & Type Polish

- [ ] **Add missing trait derives** — `FileHashes` and `AnalysisProgress` should derive `PartialEq, Eq`. `DiscGroup` should derive `PartialEq, Eq, Hash`. All fields support these traits.

- [ ] **Complete `RomIdentification` builder pattern** — Only 4 of 11 fields have builder methods (`with_serial`, `with_internal_name`, `with_region`, `with_platform`). Add `with_version()`, `with_file_size()`, `with_expected_size()`, `with_maker_code()`, `with_checksum()`, `with_extra()`.

- [ ] **Make `PlatformParseError` field private** — `retro-junk-core/src/platform.rs:222`: Public `String` field is never accessed directly. Make it `(String)` (private) for encapsulation.

- [ ] **No SQL LIMIT/OFFSET bounds** — `retro-junk-db/src/queries.rs` (8+ functions): Pagination parameters are interpolated without validation. A limit of `u32::MAX` could exhaust memory. Add reasonable caps.

## Code Health: UX Consistency

Audit findings from 2026-03-17.

### Naming & Terminology

- [ ] **"ROM" vs "entry" vs "game" inconsistency** — CLI help says "ROMs" but data model is `GameEntry` which includes multi-disc folders. Standardize to "entries" or "games" in user-facing text.

- [ ] **"Catalog" vs "Database" mixed** — GUI code uses `catalog_db` and "Catalog Tools" but also "Library cache: stored in catalog DB". Pick one term for user-facing text.

- [ ] **"Compute" vs "Calculate" hashes** — Hash backend says "Computing hashes" but buttons say "Calculate All Hashes". Pick one verb.

- [x] **`RomFilterArgs` filters consoles, not ROMs** — Renamed to `ConsoleFilterArgs`.

### Missing User Feedback

- [ ] **Keyboard shortcuts undocumented** — Ctrl+1/2/3 (view switching), Cmd+A (select all), arrow keys, Page Up/Down, Enter, Escape are implemented but never documented in UI. Add a help dialog or tooltips.

- [ ] **Settings path validation absent** — `retro-junk-gui/src/views/settings.rs:110-141`: Invalid metadata/media directory paths accepted without feedback. User discovers the problem only on first use.

- [ ] **DAT status not visible in console tree** — Console tree shows scan status and entry count but no indicator for DAT load state. Users can't tell why serial matching isn't working.

- [ ] **Cancellation lacks confirmation** — Clicking Cancel on an operation provides no visual acknowledgment. Add a brief "Cancelled" state.

## Code Health: DRY Violations (2026-03-17)

- [ ] **Repeated path extension checking** — `retro-junk-lib/src/scanner.rs` has 4+ copies of the `.extension().and_then().map().unwrap_or(false)` pattern. Extract `has_extension(path, ext)` utility function.

- [x] **Hardcoded disc sector sizes** — Extracted to `retro-junk-disc::sector` as `RAW_SECTOR_SIZE`, `ISO_SECTOR_SIZE`, `MODE1_DATA_OFFSET`, `MODE2_FORM1_DATA_OFFSET`, etc. Used by both Sony and Sega crates.

- [x] **Near-duplicate `read_file_content` / `read_file_from_chd`** — Unified in `retro-junk-disc` crate. Both functions now live in `iso9660.rs` and `chd.rs` respectively, with shared `DirectoryRecord` type and consistent interfaces.

## Enrichment Pipeline Hardening

Audit findings from 2026-02-25. Goal: make `catalog enrich` reliable enough to run hands-off on a server for months.

All 15 items resolved — see commit history for details.
