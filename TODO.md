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

- [ ] **Parallel enrichment improvements** — The enrichment worker pool exists but could benefit from: resume-from-last-position on interrupted runs, per-platform progress persistence, and smarter rate limit backoff when ScreenScraper returns 429s.

- [ ] **Overrides YAML expansion** — The overrides system exists but has limited use. Expand with curated override sets for known problem areas: multi-disc serial mismatches (FF7, etc.), regional title corrections, and publisher name normalization.
