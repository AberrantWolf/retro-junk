# TODO

## Bugs

- [ ] **Cross-host archive lock on SMB could use share-mode locks.** The archive lock (2026-07-30 rework) uses kernel-enforced OS locks where the filesystem honors them and falls back to the existence+PID+age protocol elsewhere. On macOS smbfs, `flock` silently enforces nothing (verified empirically), so SMB shares always use the fallback — same-host crashes recover instantly via the PID probe, but a *different host's* crashed holder still waits out the 24h age rule. macOS `O_EXLOCK`/`O_SHLOCK` open flags map to SMB share-mode (deny) locks enforced server-side, which would give real cross-host exclusion and server-side crash release; needs `OpenOptionsExt::custom_flags` + libc and a Linux-cifs interop check.

- [ ] **Schema open path trusts the version stamp.** `open_database` decides "migrated" purely from `schema_version`, so a database whose tables don't match the stamped version (e.g. one written by the pre-rebase divergent branch, which used the same version numbers for a different layout) opens "successfully" and fails later with `no such column` at query time. Add a cheap structural sanity probe on open (e.g. `SELECT scan_state FROM library_consoles LIMIT 0` for a sentinel column per recent version) that produces a clear "incompatible database, delete or restore" error instead. Also note `ensure_catalog_database_location` re-copies the legacy cache DB (`~/Library/Caches/retro-junk/dats/catalog.db`) whenever the target is missing — deleting a bad `catalog.db` silently resurrects an equally old one; the legacy file should probably be renamed once migrated instead of retained under its live name. (Diagnosed 2026-07-30.)


- [ ] **Legacy cartridge catalog evidence never claims a complete track set.** Catalog verifications written by the older single-file path record `complete_track_set: false` (verified on a real archive 2026-07-30: 193 of 250 catalog verifications, every no-intro one). `dump_catalog_evidence` requires the flag, so those dumps are not catalog-verified and cannot name their library rows from evidence. Re-running `archive verify-catalog` rewrites them correctly once a catalog is imported, but the archive should either upgrade the flag in place for single-file masters whose recorded hashes still match, or the CLI should report how many dumps carry pre-flag evidence so the re-verify is discoverable.

- [ ] **Carrier catalog bindings do not survive a re-import on another machine.** Media ids are deterministic (`release_id:rom-name-slug`), but they change with the DAT release the id was derived from, so an archive built against one import binds carriers to ids a later or differently-versioned import never creates (verified 2026-07-30: 201 of 248 carriers `unresolved` after a full local import). Everything downstream of `carriers.catalog_media_id` — playable bindings, archive completeness, gaps — silently degrades. Consider rebinding by recorded content hashes during projection when the recorded id is missing, rather than requiring `archive verify-catalog` / `audit-redumper` to rewrite manifests.

- [ ] **Hash provenance is stored but never shown.** `library_entries.hash_source` records when a row's digests were adopted from archive manifests rather than read on this machine, but nothing surfaces it: the detail panel shows CRC32/SHA-1/MD5 with no indication of where they came from, and there is no "re-read this file to confirm" affordance (the hash action skips rows that already have digests unless the include-cached path is used). Plumb `hash_source` through `LibraryEntryRow`/`LibraryEntry` and label adopted rows.

- [ ] **Archive evidence records the raw digests, not the payload digests it computed.** `verify_catalog_files` hashes each single-file master with the analyzer's format-aware normalization (header-skipped) to match the catalog, then discards those digests — the verification evidence keeps only the catalog verdict. Persisting them (alongside the raw digests already in `dump.toml`) would let headered dumps adopt hashes without needing the catalog to confirm the raw form, and would make the archive self-describing for DAT matching.

## Features

- [ ] **Database management GUI** screen for all sorts of database tasks, including viewing and merging conflicts, importing and previewing enrichment, and maybe even direct database editing
  - Partially done (2026-07-10): Tools → Data tab (`views/tools_data.rs`, `backend/catalog_ops.rs`) adds catalog import and GDB/ScreenScraper enrichment (plus DAT/GDB cache fetch/clear); Dashboard/Browse tabs already view stats, disagreements, and tables. Still open: enrichment *preview* and direct database editing.

- [x] **Move media and data on rename** — Done (2026-07-10): renames execute as per-game filesystem transactions (`retro-junk-lib/src/fs_txn.rs`) that carry scraped media files and gamelist.xml path/asset rewrites (`retro_junk_frontend::esde::plan_gamelist_rewrite`) along with the game files, with preflight collision checks and rollback on failure.

- [ ] **Figure out multi-file WBFS setups** - I don't know what we're meant to do with them or how to treat them

- [ ] **Custom multi-select view** in the game details panel, rather than showing details for the most-recent selection in the list

- [ ] **Show hash match status in detail panel** — After hashing, the detail panel shows CRC32/SHA1/MD5 values but doesn't visually indicate whether they match known DAT entries. Add a match/mismatch indicator next to hash values.

- [ ] **Try copy-on-write reflinks before physically copying disposable staging data** —
  For same-filesystem staging on reflink-capable filesystems, attempt a native
  COW clone (`FICLONE` on Linux, `clonefile` on macOS), hash the cloned snapshot,
  and transparently fall back to the existing single-pass copy-and-hash path
  for unsupported or cross-device sources. Never substitute hard links: tools
  such as Redumper require an isolated writable workspace. Keep free-space
  checks conservative because later writes can materialize cloned extents, and
  surface whether each staging operation used a reflink or physical copy.

## Disc Sets & Verification (deferred from 2026-07-10 rename work)

- [ ] **Surface per-track verification in analyze output** — Rename now
  verifies every track of a cue/bin set against Redump per-track hashes
  (`retro-junk-lib/src/disc_set.rs`), but `analyze` still hashes only the
  data track (`retro-junk-disc/src/hash.rs` hashes Track 1 / largest data
  track only). Reuse the disc-set verification to report per-track
  match/mismatch during analyze and in the GUI detail panel.

- [ ] **Scraper vs. import hash divergence** — Under `--force-hash`, the
  scraper hashes the *first* data track of a multi-BIN cue while DAT import
  stores the *largest* data track's hashes. Divergent for Saturn (MODE1 boot
  track + larger main track). The default serial path is unaffected. Pick one
  convention and share the implementation.

- [ ] **Saturn `.mds`/`.mdf` advertised but unimplemented** —
  `saturn.rs` lists `mdf`/`mds` in `file_extensions()` but
  `retro-junk-disc/src/format.rs` has no MDS/MDF detection. Either implement
  or stop advertising.

- [ ] **Saturn analyzer test coverage** — `saturn_tests.rs` covers IP.BIN
  parsing and ISO analysis but has no CUE, raw-BIN, CHD, or hashing tests —
  the paths redumper dumps actually exercise.

- [ ] **Make M3U folder rename fully transactional** — Disc sets inside
  `.m3u` folders and companion media/gamelist moves now run as transactions,
  but the folder rename + playlist write in `execute_m3u_rename` are still
  individual operations without rollback.

## Analyzer: Compressed Disc Formats

- [ ] **GameCube NKit support** — NKit is a lossy-compressed format (`.nkit.iso`, `.nkit.gcz`) that removes junk/padding data. Hashes will not match Redump unless converted back to full ISO. May need special handling or a warning that NKit images can't be verified against Redump.

- [ ] **Check nod v2.0 stability** — The `nod` crate v2.0 may bring API changes. Check for stability and migration when it releases.

## Format Conversion (deferred from 2026-07-10 CHD compression work)

CHD *compression* (cue/gdi/iso → chd via chdman, with round-trip verification)
shipped in `retro-junk-lib::chd_convert` + GUI dialog + CLI `compress`.
Deferred follow-ups:

- [ ] **PREGAP/POSTGAP round-trip gap compensation** — `chd_convert::plan_compression`
  now rejects (`ChdConvertError::UnsupportedLayout`) any CUE that declares
  `PREGAP`/`POSTGAP` (gap not stored in the track file), because chdman
  synthesizes those gaps into the CHD and materializes them again on
  extraction, making the extracted track longer than the source span. A
  disc with such a cue currently just can't be compressed. A future
  enhancement could compensate during `verify_round_trip` (e.g. skip the
  synthesized gap region when comparing, using `CueTrack::pregap_frames`/
  `postgap_frames`) instead of rejecting at plan time.

- [ ] **`convert_cue_to_standard` still requires space-separated directives** —
  The CDRWin→standard cue fixer's directive detection (`upper.starts_with("DATAFILE ")`
  etc.) was not updated to the tab-tolerant keyword/rest split that
  `cue::parse_cue` now uses (2026-07 CHD remediation, Phase A3). A
  tab-separated CDRWin cue would fail to auto-fix even though `parse_cue`
  itself now parses tab-separated cues fine. Low priority: CDRWin-format
  cues encountered in practice have been space-separated. Fixing this
  properly means extracting `parse_cue`'s directive-token/rest split into a
  helper both functions share (DRY win alongside the fix).

- [ ] **CHD decompression (chd → cue/bin)** — the reverse operation. Can be
  done natively (retro-junk-disc already decodes hunks + CHT2 track metadata;
  writing bins + generating a cue is a modest extension) or via
  `chdman extractcd`, which `chd_convert::verify_round_trip` already invokes —
  most of the plumbing exists.

- [ ] **RVZ compression/decompression for GameCube/Wii** — `nod` 2.0
  (currently 2.0.0-alpha.10, what nodtool ships on) adds a `DiscWriter` with
  RVZ/WIA/ISO output, compression options, and multithreading. Requires
  upgrading from nod 1.4 (read-only). Native Rust both directions — no
  external tool needed. Verify via the existing Redump hashing path.

- [ ] **CSO/ZSO/DAX support for PSP** — the PSP analyzer lists `cso`/`dax`
  extensions but cannot actually read them (no decompression). CISO is a
  trivial format (block index + per-block deflate; ZSO uses LZ4) — native
  read *and* write is a small job with `flate2`/`lz4`. Fix the read gap
  first, then offer compression.

- [ ] **Batch/whole-library compression job queue** — GUI compression runs
  per-console selection today. A library-wide "compress everything eligible"
  pass with disk-space preflight (verification needs temp space equal to the
  uncompressed size) would suit large collections.

- [ ] **Optional verification skip / quick mode** — round-trip verification
  roughly doubles wall time per disc. Consider a settings toggle
  ("verify: full round-trip / chdman verify only"), keeping full round-trip
  mandatory whenever source deletion is enabled.

- [ ] **chdman codec/hunk tuning knobs** — expose `--compression` /
  `--hunksize` / `--numprocessors` per platform via analyzer hints (e.g.,
  some PPSSPP guidance favors 2048-byte hunks for PSP CHDs). Defaults are
  fine for current emulators; revisit when evidence appears.

- [ ] **CLI compress should honor the GUI's chdman path setting** — the GUI
  stores `general.chdman_path` in `~/.config/retro-junk/settings.toml`; the
  CLI currently only has `--chdman` + PATH. Needs a shared typed settings
  struct in retro-junk-lib instead of the GUI-owned one (see also the DRY
  note below).

### CHD / analyzer-trait follow-ups (deferred from the 2026-07-14 CHD remediation, Phase F)

- [ ] **`DiscSupport` capability object.** `RomAnalyzer` has accumulated ~5
  independent disc-specific optional methods (`dat_source`, `redump_slug`,
  `dat_names`, `compute_container_hashes`, `chd_extensions`) whose defaults
  fail silently — the Sega CD/Dreamcast hashing gap (closed by the C2
  invariant test) was the proof. Proposed shape: `fn disc_support(&self) ->
  Option<&dyn DiscSupport>` returning one bundle so the compiler forces the
  whole set at once. Large cross-crate refactor; the C2 invariant test
  contains the risk until then.

- [ ] **Case-insensitive m3u entry resolution** in `find_correct_m3u_entry`
  (`retro-junk-lib/src/rename.rs`) — `chd_convert::update_m3u_references` now
  delegates to this machinery (2026-07 CHD remediation, Phase B5), but on
  case-**sensitive** filesystems a playlist entry whose case differs from the
  actual file still misses a fix, because the fallback lookup probes the
  directory only with exact-case candidates. Extend it to probe
  case-insensitively before giving up.

- [ ] **GDI-aware `expand_disc_set`.** `chd_convert::plan_compression`'s gdi
  branch inlines resolve-tracks-and-collect-missing that
  `disc_set::expand_disc_set` provides for cues. `DiscSetFiles` is cue-shaped
  (`cue: PathBuf` field); unifying means generalizing that struct — worth
  doing together with any future `.toc`/`.ccd` support, not before.

## Raw Redumper Archival + Dual Representation (design, 2026-07-16)

Goal: ingest **raw redumper dump folders** (`.scram`/`.subcode`/`.state`/`.fulltoc`/`.log`) as
archival sources, and let one logical game hold **multiple physical representations** — a pristine
archival master and a compressed emulator-playable copy — that live in **separate sibling folders**
(`RetroLibrary/archive/` vs `RetroLibrary/roms/`) yet share metadata and scraped assets. Driven by a
Syncthing-synced library with per-device selective sync. Format knowledge is in
`.claude/skills/retro-archive/formats/Redumper.md`; methodology/prior-art in
`PreservationVsPlayable.md`. Design reviewed by a Fable subagent; its ranked risks are folded in below.

**Decide before building (the load-bearing question):**
- [ ] **Pick the persistence world.** The GUI renders from the **`library_entries` cache world**, not
  the catalog `media` world (they're disjoint; joined only via `cover_title`/`screen_title`
  enrichment, `views/library.rs`). A `media_representations` table under `media` would NOT surface in
  the GUI without either migrating the library view onto the catalog or duplicating representation data
  in the cache world.
  - **Direction (per the "authoritative over cache" lean, 2026-07-16):** make the **catalog** the
    authoritative home for representations (a representations table under `media`, plus the YAML
    catalog as it becomes the matching source of truth), and keep **`library_entries` a pure derived
    cache** — rebuildable, and *read* on hot paths only where hitting the catalog live is **observably**
    too slow. Do not treat the cache as a second primary model. This **converges with** the planned
    "Migrate matching source-of-truth from raw DAT to catalog DB" work below, so the dual-representation
    feature becomes a reason to advance that migration rather than route around it.
  - Fable's "cheapest = model it in the cache world" is therefore reframed as a **performance question,
    not the default**: only cache once a real slowdown is measured. The likeliest place that need shows
    up is the selective-sync / multi-device case (many entries, files absent locally) — measure there
    first, don't preemptively cache. Do not straddle both worlds.

**Data model:**
- [ ] **Add a representation/location model** (`kind` = source/archive/playable, `format` =
  redumper/cue-bin/iso/chd/rvz, `location_id`, entry-point `path`, integrity hashes,
  `redumper_build`). `path` = entry point (cue/gdi/iso/folder); keep per-track detail in the existing
  `media_tracks`. Do **not** enumerate each `.bin` as its own representation.
- [ ] **Do NOT relax `collection`'s `UNIQUE(media_id, user_id)`.** `collection` carries ownership
  (owned/condition/notes), which is per-dump, not per-representation; overloading it muddies ownership
  queries and forces a risky 12-step SQLite table rebuild. Add a separate additive table; deprecate
  `collection.rom_path`.
- [ ] **Identity vs integrity hashes.** Logical identity stays the normalized `media.sha1`
  (`compute_container_hashes` already collapses CHD/RVZ to the uncompressed representation, so all
  playable forms resolve to one SHA1). A raw `.scram` matches no DAT — its representation hash is
  **integrity-only** (bit-rot), not identity.

**Storage layout / multi-root:**
- [ ] **Model `archive/` as a derived sibling directory, not a peer library root.** Reuse the existing
  `assets_dir`/`metadata_dir` sibling-resolution pattern (`state.rs:140-182`): for
  `roms/psx/Game (USA).chd`, resolve `archive/psx/Game (USA)/` by convention. Avoids a multi-root
  rewrite, the `find_by_folder` collision (two roots with a `psx/` folder → one silently dropped,
  `state.rs:70`), and cross-root hash correlation. If true multi-root is ever pursued instead, consoles
  MUST be re-keyed by `(root, folder)` and a cross-root identity key (normalized `media.sha1`) chosen.

**Selective-sync correctness (missed in first design pass):**
- [ ] **Per-device presence ≠ catalog existence.** Under selective sync a device routinely has a
  catalog row whose file is absent locally; `verify_collection` currently treats an absent path as an
  error (`scan_import.rs:225`) and would fire constantly. Make "known but not present here" a normal
  state.
- [ ] **Store representation paths root-relative + `location_id`,** not absolute
  (`scan_import.rs:165`) — absolute paths don't survive different mount points across devices.

**Scanner / ingestion:**
- [ ] **Teach the scanner to see raw folders.** `scan_game_entries` only recognizes top-level files by
  extension and `.m3u` dirs (`scanner.rs:110`). Add detection of a directory containing
  `.scram`/`.sdram`/`.sbram` = redumper archive, a `GameEntry::RedumperRaw` variant, and an
  entry-creation path that lists **archive-only games with no playable file**.
- [ ] **Ingest via `redumper split` + `redumper hash` subprocess,** mirroring `Chdman::detect()`
  (`chd_convert.rs:70`). No JSON output exists; the `.log` `dat:` block is clrmamepro `<rom .../>`
  lines — route them through the **existing `retro-junk-dat` parser**, not a bespoke scraper.
- [ ] **Handle split failure as a first-class state.** `redumper split` throws on unrecovered C2/SCSI
  errors or positive combined offset with missing lead-out. "Archive present, playable unrealizable"
  must be a normal state that stores the `.log` error and does not mark the game bad.
- [ ] **Record the redumper build; verify against Redump DB hashes, not byte-identical re-splits.**
  redumper ships rolling builds with no determinism guarantee across versions.

**GUI/UX:**
- [ ] **One row per game; representation badge cluster** (source/archive/playable, filled/hollow) in
  the game table; a **Representations** section in the detail panel with per-row
  Verify/Regenerate/Compress/Reveal actions; context-menu items for ingest/verify/regenerate. Badges
  must roll up across discs for a multi-disc `.m3u` entry.
- [ ] **Reuse `chd_convert::finalize_verified`** (already round-trip-verifies before deleting sources)
  for the Compress action.

**DB / sync hygiene (independent of this feature but surfaced by it):**
- [ ] **Move the catalog DB from XDG cache → XDG data.** It's in `~/.cache/retro-junk/dats/catalog.db`
  (`app.rs:177`) but CLAUDE.md calls it the long-lived store; cache dirs get cleaned.
- [ ] **Never sync a live WAL SQLite DB** through Syncthing (atomic per-file, not across `.db`/`-wal`;
  corruption risk). Keep the DB out of the synced tree (currently true); sync the YAML catalog and
  rebuild the per-device cache. On open, detect sibling `*.sync-conflict-*` DB files and warn.

**Prior art to borrow** (`PreservationVsPlayable.md`): romba/RomVaultX content-addressed depot + built
views; igir `--link-mode` for zero-copy playable projections; MAME merged/split set policy; expose
only the playable tree to frontends.

## DAT Source Coverage

- [ ] **Wii U has no Redump DAT** — Redump.org has no Wii U disc entries or datfile download. The previous LibRetro "Nintendo - Wii U (Digital)" DAT was not real Redump data. DAT support for Wii U is currently disabled. Options: (1) find an alternative DAT source for Wii U, (2) re-enable using LibRetro's DAT with `DatSource::NoIntro` if the data is good enough, or (3) wait for Redump to add Wii U support.

- [ ] **Verify all Redump slugs work** — After switching disc-based DAT downloads from LibRetro to redump.info direct, verify that all slug mappings actually return valid data. Verified 2026-07-10: `psx` and `ss` both return fresh datfile zips. Still to verify: `ps2`, `ps3`, `psp`, `mcd`, `dc`, `gc`, `wii`, `xbox`, `xbox360`. Some systems may have restricted access or different slug conventions on redump.info.

## Data Model & Import Pipeline

- [ ] **Migrate matching source-of-truth from raw DAT files to catalog DB** —
  Currently, hash/serial matching runs against in-memory `DatIndex` built from
  downloaded DAT files each session. Raw DAT files should only seed, update,
  enrich, and fix the catalog DB. The DB should become the authoritative source
  for matching, enabling persistent corrections (e.g., adding missing regional
  entries, resolving cross-region matches) that survive DAT re-downloads.

- [ ] **Re-import after migration v4** — Schema is now at version 4 (`screen_title`, `cover_title` columns added in v3). Run `catalog import all` followed by `catalog enrich` on existing databases to populate `revision`, `variant`, `screen_title`, and `cover_title` fields. This is a one-time user/ops action, not a code gap.

## Deferred module splits (readability, no behavior change)

- [ ] **Finish splitting `retro-junk-lib/src/rename.rs`** — public types now
  live in `rename/types.rs` (2026-07-18). Extract the remaining cohesive
  areas into `serial`, `m3u`, `plan`, `execute`, and `ref_files`, then convert
  the root to `rename/mod.rs` with re-exports so consumer imports stay stable.
- [ ] **Split `retro-junk-db/src/queries.rs` by aggregate** — extract media,
  release, work, collection, and search queries while preserving the existing
  `retro_junk_db` facade.
- [ ] **Split `retro-junk-gui/src/state.rs` by state ownership** — extract
  library/cache, dialogs, operations/jobs, messages, and browse/selection
  state into `state/` submodules; retain one public state facade.
- [ ] **`Override` selector trio → enum** — `entity_id` / `platform_id` /
  `dat_name_pattern` are alternative targeting modes; an `OverrideTarget`
  enum would make illegal combinations unrepresentable (types.rs + YAML
  serde + `apply_overrides`).
- [ ] **CLI disagreement-resolve choice → enum** — `--source-a` /
  `--source-b` / `--custom <value>` are clap-ArgGroup-exclusive bools plus
  an Option; a single value-carrying enum arg would drop the if-chain.
- [ ] **GUI progressive-analysis state → enum** — `LibraryEntry`'s
  `identification` / `hashes` / `dat_match` Options track analysis
  progress alongside `status: EntryStatus`; a full `AnalysisState` enum
  could encode the progression, but each Option carries independently
  consumed data, so this needs a real design pass, not a mechanical swap.
- [ ] **Centralize CLI catalog-db path resolution** — ~25 clap fields
  repeat `db: Option<PathBuf>` + `unwrap_or_else(default_catalog_db_path)`
  (runtime default, not clap-expressible). Resolve once in main and pass
  the resolved path down.
- [ ] **`scan_import::UnmatchedFile.sha1`** stays `Option<String>` because
  upstream `retro_junk_dat::matcher::FileHashes.sha1` is Option (CRC-only
  hashing mode); revisit if FileHashes ever grows an all-or-nothing hash
  group like the scraper's `RomHashes`.
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

## Testing

- [x] **Adopt `egui_kittest` for headless GUI testing** — Done with the egui 0.35 upgrade (2026-07-10): `egui_kittest` dev-dependency with `Harness::new_eframe` smoke tests in `retro-junk-gui/src/app_tests.rs`, built on the hermetic `RetroJunkApp::with_parts` constructor (no settings/DB disk access). Caveat still applies: native `rfd` dialogs live outside the egui scene graph, so kittest can't reach them — flows under test should route confirmations through egui-native modals or an injectable confirm hook.

- [ ] **Expand kittest coverage** — Current tests are startup smoke tests (sidebar present, welcome screen, view switching). Add coverage for root switching, the fragile-mount dialog, game-table selection/filtering, and dialogs; consider snapshot tests (`egui_kittest` `snapshot`+`wgpu` features) for visual regressions.

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

- [ ] **`console_tree` and `game_table` duplicate the selectable-list pattern** — Both `widgets/console_tree.rs` and `widgets/game_table.rs` are the same kind of view: a focusable (`FocusedPanel`), keyboard-navigable (shared `keyboard_nav`), status-badge-annotated selectable list with one-shot scroll-into-view. They were implemented independently, and the console tree reintroduced two bugs the table had already solved: (1) it scrolled `scroll_to_me` every frame instead of using a one-shot scroll target — now fixed by mirroring `game_table`'s `scroll_to_row` as `scroll_to_console: Option<usize>`; (2) its hand-rolled `ui.horizontal` + conditional badge + `selectable_label` rows suffered auto-ID churn ("changed id between passes" + scroll resets) — now fixed with `push_id(i)`, whereas `game_table` sidesteps it structurally via `egui_extras::TableBuilder` rows + `paint_cell_text` (which allocates no `WidgetRect`). Consider extracting the shared row/selection/scroll-target lifecycle into a common helper so the two views can't drift again. Note: the console tree also needs manufacturer `CollapsingHeader` grouping that `TableBuilder` doesn't model, so this is a shared-helper refactor, not a switch to `TableBuilder`.

## Code Health: GUI Data Tab (2026-07-10)

Follow-ups from adding the Tools → Data tab (`views/tools_data.rs`,
`backend/catalog_ops.rs`), which surfaced the CLI's catalog data-gathering
pipeline (cache fetch, import, GDB/ScreenScraper enrich) in the GUI.

- [ ] **Promote capability-based console resolution to `retro-junk-lib`** — Both
  the CLI (`retro-junk-cli/src/commands/systems.rs`: `resolve_systems` /
  `SystemCapability`) and the GUI (`backend/catalog_ops.rs`: `targets` / `Cap`)
  independently filter `AnalysisContext` by DAT/GDB capability plus a system
  selection. Extract one shared resolver so the two presentation crates can't
  drift on which systems an operation targets.

- [ ] **Embed catalog seed YAML for self-contained import** — Catalog import
  seeds platforms/companies/overrides from a cwd-relative `./catalog` dir (both
  CLI and GUI; GUI adds a `catalog_data_dir` setting as an escape hatch). An
  installed GUI run from an arbitrary cwd will silently skip seeding and produce
  a catalog with no platforms. Embed the ~156K `catalog/` YAML into the binary
  (e.g. `include_dir` + a `seed_bundled(conn)` in `retro-junk-catalog`/`-db`) so
  import is fully self-contained, then drop the cwd fallback.

- [ ] **Coarse cancellation for ScreenScraper enrich in GUI** —
  `catalog_ops::run_ss_enrich` is only cancel-aware at the connect stage; once
  `enrich_releases` starts it runs to completion (bounded by the per-system
  limit). Thread the cancel token into the enrich loop for mid-run stop, as the
  media scraper in `backend/assets.rs` already does per item.

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

- [ ] **CLI `credentials show`/`setup` duplicate credential field metadata** — `retro-junk-cli/src/commands/credentials.rs:78-124` builds its own field list ("dev_id", "dev_password", …) by hand. `retro_junk_scraper::CREDENTIAL_FIELDS` (added 2026-07-15 for the GUI ScreenScraper settings section) is now the single source of field keys/env vars/descriptions; the CLI commands should iterate it instead.

- [ ] **Repeated path extension checking** — `retro-junk-lib/src/scanner.rs` has 4+ copies of the `.extension().and_then().map().unwrap_or(false)` pattern. Extract `has_extension(path, ext)` utility function.

- [x] **Hardcoded disc sector sizes** — Extracted to `retro-junk-disc::sector` as `RAW_SECTOR_SIZE`, `ISO_SECTOR_SIZE`, `MODE1_DATA_OFFSET`, `MODE2_FORM1_DATA_OFFSET`, etc. Used by both Sony and Sega crates.

- [x] **Near-duplicate `read_file_content` / `read_file_from_chd`** — Unified in `retro-junk-disc` crate. Both functions now live in `iso9660.rs` and `chd.rs` respectively, with shared `DirectoryRecord` type and consistent interfaces.

## Enrichment Pipeline Hardening

Audit findings from 2026-02-25. Goal: make `catalog enrich` reliable enough to run hands-off on a server for months.

All 15 items resolved — see commit history for details.

## Phase A follow-ups (automation foundation, 2026-07-30)

- [ ] **Phase A.5: scrape orchestration consolidation** — Extract the duplicated
  tokio scrape cores (`retro-junk-gui/src/backend/assets.rs` vs
  `retro-junk-cli/src/commands/scrape.rs`) into one shared implementation in
  `retro-junk-scraper`, rewrite both callers, then add the `Scrape` convergence
  action kind and `auto_scrape` policy fields. Deliberately deferred from
  Phase A: unattended scraping needs quota throttling first.
- [ ] **Instant-apply imports** — `plan_import` re-hashes on suggestion apply.
  The incoming pipeline already computed the full inventory digests at arrival;
  extend `retro-junk-archive-import` to accept precomputed digests so applying
  a suggestion executes with zero re-reads (matters for large disc dumps over
  network mounts).
- [ ] **Per-release incremental reconcile** — `reconcile_archive_snapshot`
  rebuilds the whole projection; the daemon and executor batch it, but the
  biggest remaining network win is reconciling only the releases an action
  touched.
- [ ] **Miximage staleness derivation** — add a `GenerateMiximage` convergence
  kind once component staleness (source artwork vs generated image) is modeled.
- [ ] **`ArchiveLock::acquire_wait` fairness** — daemon+GUI contention is
  fail-fast/wait polling today; add FIFO fairness if contention proves noisy.
- [ ] **CLI Ctrl-C for `sync`** — the executor is cancel-safe; wire a real
  SIGINT handler into `retro-junk sync` (the daemon already has one).
- [ ] **GUI dirty-tick polling (roadmap B7)** — `runtime_state.dirty_tick`
  already bumps on every coordination commit; add the 1 Hz GUI poll feeding
  `LibraryChangeSet` refresh so daemon writes appear without manual refresh.
- [ ] **Profile editor for `incoming_roots` / `watch_backend`** — the fields
  exist on `CollectionProfile` (settings.toml-editable); add GUI controls next
  to the profile root pickers.
