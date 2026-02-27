# TODO

## Bugs

- [x] **Rescan game drops hashes** — Rescanning resets `entry.status` to `Ambiguous`/`Unrecognized` via the `EntryAnalyzed` handler (`state.rs:715`), but the hash-based re-promotion to `Matched` only runs on `DatLoaded`, which doesn't re-fire during a rescan. Green dots turn yellow just from rescanning. Fix: re-run DAT matching for rescanned entries after `EntryAnalyzed`.

- [x] **Stale `broken_references` after rename** — After `RenameComplete` is handled (`state.rs:~1249`), file paths change but `entry.broken_references` is not reset to `None`. A rename that fixes broken CUE/M3U references still shows the warning triangle until the next full scan. Fix: reset `broken_references = None` for renamed entries so the next repaint or background pass rechecks.

- [x] **Game list load delay** we should show SOMETHING when the app launches if there is a library loading, whether it's a "loading library" overlay or the first quick display of the library, right now it looks like no library is loaded for a few seconds

- [x] **Deselect row doesn't appear deselected** when multiple selected rows, deselecting one doesn't visibly change until you add/remove another row


## Features

- [ ] **Move media and data on rename** — If we've already scraped media and rename a game, we need to move the data associated with it (images, gamelist.xml entries, etc. under `roms-media/`).

- [ ] **Custom multi-select view** in the game details panel, rather than showing details for the most-recent selection in the list

- [ ] **Add space below table** both to fill the remainder of the view, as well as allow space to select the bottom row (mostly blocked by the scroll bar)

## Data Model & Import Pipeline

- [x] **Work reconciliation by ScreenScraper ID** — Fully implemented: `reconcile_works()` in `retro-junk-import/src/reconcile.rs`, `find_reconcilable_works()` query in `retro-junk-db`, `catalog reconcile` CLI command, and auto-runs after `catalog enrich` (skippable via `--no-reconcile`).

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

- [ ] **Auto-resolve trivial disagreements** — Many disagreements are cosmetic (trailing periods, capitalization, date format differences). Add configurable rules to auto-resolve obvious cases during enrichment, reducing manual review noise.

- [ ] **Export to ES-DE / other frontends** — The `scrape` command and GUI already generate ES-DE gamelist.xml for individual systems. Add a `catalog export` command that generates gamelists from the catalog DB for any/all platforms, pulling metadata and asset paths from the database rather than re-scraping.

- [ ] **Collection verification report** — Extend `catalog verify` to produce a summary report: missing ROMs (in DB but not on disk), unmatched files (on disk but not in DB), hash mismatches, and duplicate ROMs across folders.

- [ ] **DAT freshness checking** — Track when each DAT was last downloaded and warn when DATs are stale. Optionally auto-fetch updated DATs before import.

- [ ] **Multi-disc release grouping improvements** — Currently multi-disc games are grouped by title + region + revision + variant. Consider edge cases: different disc counts across regions, bonus discs, demo discs bundled with retail releases.

- [ ] **ROM health dashboard** — Aggregate view across all platforms: total ROMs scanned, verified vs. unverified, trimmed/padded/repaired, missing from known sets (have DAT entry but no matching file in collection).

- [ ] **Overrides YAML expansion** — The overrides system exists but has limited use. Expand with curated override sets for known problem areas: multi-disc serial mismatches (FF7, etc.), regional title corrections, and publisher name normalization.

## Code Health: Error Handling

Audit findings from 2026-02-26. All items resolved — see commit history for details.

## Code Health: DRY Violations

Audit findings from 2026-02-26.

### Shared utility functions

- [x] **Consolidate `format_size()` / `format_bytes()`** — Canonical `format_bytes()` and `format_bytes_approx()` now live in `retro-junk-core/src/util.rs`. All 5 former duplicates now import from core. `retro-junk-lib/src/util.rs` re-exports via `pub use retro_junk_core::util::*`.

- [x] **Consolidate `read_ascii()`** — Canonical `read_ascii()` and `read_ascii_fixed()` now live in `retro-junk-core/src/util.rs`. `n3ds/common.rs` re-exports `pub(crate) use retro_junk_core::util::read_ascii`; `genesis.rs` imports `read_ascii_fixed` aliased as `read_ascii`.

- [ ] **Consolidate byte-reading helpers within Nintendo crate** — `retro-junk-nintendo/src/ds.rs:85-98` still defines private `read_u16_le()` and `read_u32_le()` that duplicate what `n3ds/common.rs:18-68` provides as `pub(crate)`. Either have `ds.rs` import from `n3ds::common`, or extract to a shared `nintendo_util` module in the Nintendo crate.

- [ ] **Extract `get_file_size()` helper** — The seek-to-end/seek-to-start pattern for getting file size appears 26+ times across all analyzer crates:
  ```rust
  let file_size = reader.seek(SeekFrom::End(0))?;
  reader.seek(SeekFrom::Start(0))?;
  ```
  Add a `pub fn file_size(reader: &mut dyn ReadSeek) -> Result<u64, AnalysisError>` to `retro-junk-core` (since all analyzers depend on it and use `ReadSeek`). Replace all 26+ instances.

- [ ] **Extract header-reading helper with TooSmall error mapping** — The pattern of `read_exact` + `map_err` converting `UnexpectedEof` to `AnalysisError::TooSmall` appears in `nes.rs:569`, `snes.rs:348`, `gameboy.rs:69`, `gba.rs:61`, `n64.rs:129`, `ds.rs:105`, `ncsd.rs:50`, `genesis.rs:176`, `ps1_disc.rs:161`, and others. Add a helper to `retro-junk-core`:
  ```rust
  pub fn read_header(reader: &mut dyn ReadSeek, buf: &mut [u8], expected: u64) -> Result<(), AnalysisError>
  ```

- [ ] **Remove trivial `new()` methods from analyzer structs** — 25+ analyzer structs have manual `fn new() -> Self { Self }` that duplicates `#[derive(Default)]` which they all already have. Remove the manual `new()` methods and use `Default::default()` or struct literal syntax at call sites. Only keep `new()` if the struct has fields that need initialization.

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

- [ ] **Fix `unwrap()` panic risk in multi-disc DAT matching** — `state.rs:920` does `entry.disc_identifications.as_ref().unwrap()` outside a `Some` guard. If the prior mutable borrow changes flow, this will panic. Convert to `if let Some(discs)` or restructure to avoid the reborrow.

## Enrichment Pipeline Hardening

Audit findings from 2026-02-25. Goal: make `catalog enrich` reliable enough to run hands-off on a server for months.

All 15 items resolved — see commit history for details.
