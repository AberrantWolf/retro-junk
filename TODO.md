# TODO

## Data Model & Import Pipeline

- [ ] **Work reconciliation by ScreenScraper ID** — When multiple releases share the same `screenscraper_id`, merge their works into one canonical work. This collapses regional title variants (e.g., "Super Mario Bros." USA and "Super Mario Brothers" JP) under a single work entity. Prerequisite: the release model rework (revision/variant split) is complete.

- [ ] **Re-import after migration v3** — Run `catalog import all` followed by `catalog enrich` to populate the new `revision` and `variant` fields. Existing releases have empty defaults and work fine, but revisions (Rev A, v1.1) and variants (Greatest Hits, Proto) won't be properly split until re-imported.

## CLI

- [ ] **Flesh out `list` command output** — Currently shows platform name, short name, DAT support indicator, extensions, and folder names. Should additionally show: platform ID (for use in other commands), manufacturer, generation, media type, release count and media count from the catalog DB (when available), and enrichment coverage percentage. Format as a table with column alignment. Provide `--verbose` flag for the extended view.

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

- [ ] **Export to ES-DE / other frontends** — The `scrape` command already generates ES-DE gamelist.xml for individual systems. Add a `catalog export` command that generates gamelists from the catalog DB for any/all platforms, pulling metadata and asset paths from the database rather than re-scraping.

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

- [ ] **Consolidate `format_size()` / `format_bytes()` — 5 duplicate implementations** — There's a canonical `format_bytes()` in `retro-junk-lib/src/util.rs`, but it's reimplemented in:
  - `retro-junk-lib/src/repair.rs:567` — private copy; should import from `crate::util`
  - `retro-junk-nintendo/src/nes.rs:827` — `format_size(u32)`
  - `retro-junk-nintendo/src/snes.rs:792` — `format_size(u64)`
  - `retro-junk-nintendo/src/gameboy.rs:472` — `format_size(u64)`
  - `retro-junk-gui/src/views/settings.rs:125` — slightly different variant

  Make the canonical version in `retro-junk-lib/src/util.rs` accept `u64` (it already does), then replace all 5 duplicates with imports. Platform crates already depend on `retro-junk-lib` transitively through `retro-junk-core`, but may need a direct dependency or the function could go in `retro-junk-core`.

- [ ] **Consolidate `read_ascii()` — 2 duplicate implementations** — Identical function in `retro-junk-nintendo/src/n3ds/common.rs:70` and `retro-junk-sega/src/genesis.rs:57`. Both filter bytes to printable ASCII range (0x20-0x7F) and trim. Move to `retro-junk-core` as a utility since it's cross-platform. Signature: `pub fn read_ascii(buf: &[u8]) -> String`.

- [ ] **Consolidate byte-reading helpers within Nintendo crate** — `retro-junk-nintendo/src/ds.rs:98-110` defines private `read_u16_le()` and `read_u32_le()` that duplicate what `n3ds/common.rs:17-66` already provides as `pub(crate)`. Either have `ds.rs` import from `n3ds::common`, or extract to a shared `nintendo_util` module in the Nintendo crate.

- [ ] **Extract `get_file_size()` helper** — The seek-to-end/seek-to-start pattern for getting file size appears 26+ times across all analyzer crates:
  ```rust
  let file_size = reader.seek(SeekFrom::End(0))?;
  reader.seek(SeekFrom::Start(0))?;
  ```
  Add a `pub fn file_size(reader: &mut dyn ReadSeek) -> Result<u64, AnalysisError>` to `retro-junk-core` (since all analyzers depend on it and use `ReadSeek`). Replace all 26+ instances.

- [ ] **Extract header-reading helper with TooSmall error mapping** — The pattern of `read_exact` + `map_err` converting `UnexpectedEof` to `AnalysisError::TooSmall` appears in `nes.rs:570`, `snes.rs:348`, `gameboy.rs:69`, and others. Add a helper to `retro-junk-core`:
  ```rust
  pub fn read_header(reader: &mut dyn ReadSeek, buf: &mut [u8], expected: u64) -> Result<(), AnalysisError>
  ```

- [ ] **Remove trivial `new()` methods from analyzer structs** — 30+ analyzer structs have manual `fn new() -> Self { Self }` that duplicates `#[derive(Default)]` which they all already have. Remove the manual `new()` methods and use `Default::default()` or struct literal syntax at call sites. Only keep `new()` if the struct has fields that need initialization.

### Test helpers

- [ ] **Extract shared test database setup** — Multiple test files implement similar SQLite test database setup:
  - `retro-junk-db/tests/queries.rs` — `setup_db()`, `setup_db_with_assets()`
  - `retro-junk-import/tests/dat_import.rs` — `setup_db()`
  - `retro-junk-import/tests/scan_import.rs` — `setup_db_with_media()`
  - `retro-junk-import/tests/merge.rs` — `setup_db_with_release()`

  Create a shared `test_helpers` module (e.g., in `retro-junk-db` behind a `#[cfg(test)]` or as a dev-dependency feature) that provides reusable setup functions.

## Enrichment Pipeline Hardening

Audit findings from 2026-02-25. Goal: make `catalog enrich` reliable enough to run hands-off on a server for months.

All 15 items resolved — see commit history for details.
