# Changelog

## 0.4.0

- Fixed a copy of an archive being invisible on the machine that copied it.
  Archive identity is recorded in the portable root manifest, but a profile
  created for an existing archive minted a *fresh* id instead of adopting it,
  while the rebuildable SQLite projection is keyed on the manifest's id. So
  pointing a new profile at an rsynced collection reindexed 241 releases and
  then reported "No archived releases are indexed yet", because the UI queried
  an id the reconciler never wrote. Profiles now take their identity from the
  archive they point at, existing profiles whose id has drifted re-adopt it on
  load (CLI, daemon, and GUI alike, so no config surgery is needed), and
  opening the same archive at a second mount re-points the profile it already
  has rather than adding a rival one.
- Fixed one rebuilt playable making the whole archive unprojectable. Build
  evidence is append-only, so rebuilding a derivative in place (a newer
  chdman, a changed recipe) leaves two records naming the same output path —
  but a representation row is the *current* state of one file, and the
  projection admits one row per path, so reindexing aborted with `UNIQUE
  constraint failed: representations.location_role, representations.relative_path`
  and left no archive index at all. The newest build for each output is now
  projected, superseded records are logged rather than silently dropped, and
  every record stays in the archive as history.
- Stopped macOS AppleDouble sidecars from being treated as collection content.
  Copying a library onto exFAT, FAT, or SMB leaves a `._<name>` file beside
  every file carrying extended attributes, and those sidecars keep the
  extension they shadow — so each real game scanned as a phantom twin, and
  inside a preservation dump each one read as a file the manifest never
  recorded, reporting a healthy mirror as an integrity failure. Library
  scanning, archive verification, and ingest now share one rule for host
  filesystem metadata (`._*`, `.DS_Store`, `Thumbs.db`, `desktop.ini`).
- Fixed no playable being buildable from a Redumper raw master held on exFAT
  or SMB. The same `._` sidecars reached two paths the rule had not been
  applied to: staging copied them into the scratch workspace, and the split
  step names the image after the first `.scram`/`.scrap`/`.sdram`/`.sbram` it
  finds — `._disc.scram` sorts ahead of `disc.scram`, so redumper was pointed
  at a 4 KiB resource fork, read garbage track geometry out of `._disc.toc`,
  and failed with `error: unable to establish base LBA`. Every PS1 disc
  archived as a raw master was affected, and neither the dump nor the redumper
  version was ever at fault. Package staging and Redumper file discovery now
  go through the same host-metadata rule, so image-name discovery, log
  collection, and intermediate retention all see only dump content.

- Fixed every archive write failing on an SMB share. macOS smbfs answers each
  directory `fsync` with `ENOTSUP` (verified empirically), and the atomic
  manifest write treated that as a failed write — so an import published its
  package, then rolled the whole release back, leaving no trace but a touched
  directory. Flushing a directory is now a durability hint: refusal by the
  filesystem is accepted, genuine I/O failures (`ENOSPC`, `EIO`) still fail,
  and a completed publish is never rolled back because the hint failed.
- Made whole-archive indexing about 6× faster over a network share (241
  releases: 15.3 s → 2.5 s warm, 23.8 s → 5.1 s cold). Scanning read every
  manifest twice — once to parse, once to digest — and walked releases one at
  a time, so the cost was almost entirely round-trip latency. It now reads
  each manifest once and scans releases concurrently.

- Fixed the Library listing the same playable file twice — once inside its
  archived release and again as an unarchived "playable only" row. A playable
  belongs to the archived carrier whose build evidence produced it, but the
  binding was keyed on that carrier's *catalog medium*, so an archive that is
  unbound, on a platform whose DAT was never imported, or bound to a catalog id
  a later import re-slugged could not own its own playable. Bindings are now
  keyed on the carrier (schema v25, re-derived in place on upgrade), and a
  multi-disc row now owns every archived disc image inside the directory it
  stands for instead of only an exact file-name match.

- Stopped re-reading files for hashes the archive already recorded. Dump
  manifests carry CRC32/MD5/SHA-1 beside SHA-256 for every archived file, but
  the projection kept only SHA-256; it now carries all four, and a library row
  holding a byte-identical mirror of a single-file master is filled from that
  record and named from the catalog medium those digests identify — no second
  read of the file. Adoption requires the recorded digests to match exactly one
  catalog medium (the archive stores raw digests while the library hashes
  format-aware payloads), and such rows record `hash_source='archive_evidence'`
  so a later local hash pass replaces them with digests actually read here.
- Made platform names separator-insensitive, so archive and frontend directory
  spellings (`super-famicom`) resolve to the same platform as the spaced alias
  instead of failing to parse and splitting one platform in two.
- Made library identity survive a machine without a catalog: the archive's own
  catalog verification records which game a dump matched, so a scanned playable
  file whose build evidence points at a current, catalog-verified dump is now
  named and shown as verified (match method `archive evidence`) even when no DAT
  has ever been imported locally. A live catalog hash comparison still wins, and
  user tags are never overwritten.
- Fixed playable relocation losing the file name when build evidence recorded a
  bare output path (written before playable outputs were filed under a platform
  directory): the projection replaced the *file name* with the platform
  directory, so present files were projected as missing and never bound to their
  library rows.

- Made Library archive state release-aware: catalog-analysis bindings now
  connect CHD/M3U entries to preservation carriers, incomplete archives remain
  visibly incomplete, and one action verifies and builds every missing disc
  before projecting a multi-disc M3U. Existing loose playable discs can be
  consolidated by a release playlist without recompression or duplicate rows.
- Reused byte-progress-reporting local Redumper staging between catalog
  verification and CHD creation, avoiding a second archive read over the
  network.
- Added a portable preservation archive with release, physical-copy, carrier, dump, representation, verification, and derivation identities.
- Added catalog-driven `archive import` and a blocking GUI import dialog that discover serial-named dump folders, hash and identify packages, resolve physical copies, retain sources by default, and optionally remove sources only after verification.
- Added `archive import-playable` and matching GUI workflow to promote existing loose-ROM libraries into preservation masters while adopting the original files as byte-identical playable representations. Cartridge matching applies platform-aware header removal and byte-order normalization without changing archived source bytes.
- Added verified atomic ingest that retains source files and rejects symlinks and traversal.
- Added separate archive, playable, and scratch roots through collection profiles.
- Added persistent per-carrier desired playable policies, including retain-intermediate and unverified-build controls.
- Added append-only integrity, Redumper reproduction/catalog, and CHD build evidence.
- Added raw Redumper auditing from disposable copies with complete-track catalog matching.
- Added catalog-gated CHD derivation with mandatory chdman round-trip verification and an explicit unverified opt-in.
- Added a rebuildable SQLite archive projection and release-centric Collection GUI view.
- Added Library availability states for playable-only, archived-and-playable, archived-without-playable, and non-preferred playable formats; per-console playable defaults; and an in-app queue that creates byte-verified cartridge mirrors or round-trip-verified CHDs from preservation masters.
- Made per-console playable-policy changes update only the root manifest and affected SQLite policy rows instead of rescanning and rebuilding the entire archive projection.
- Limited the blocking startup modal to actual catalog location/schema migrations; routine archive reconciliation and saved network-root probing now continue in tracked background work, and ordinary index refresh no longer scans the archive twice.
- Filtered console-default and per-copy preferred playable formats to the conservative set accepted by mainstream emulators, while retaining unsupported legacy selections until explicitly changed.
- Moved the durable catalog database from the cache directory to the platform data directory with validated first-run migration.
- Added policy-driven resumable CHD/RVZ/mirror builds, retained canonical intermediates, multi-disc playlist projection, and explicit integrity/reproduction/catalog/round-trip evidence.
- Added general CRC32/MD5/SHA-1 catalog verification, per-device present/missing/partial/modified/stale state, legacy playable adoption with an Inbox, and recoverable archive locking/staging recovery.
- Added authoritative ScreenScraper supporting-file adoption and frontend projection, plus physical-copy photo/provenance/document manifests and GUI provenance editing.

## 0.3.0

- Made SQLite the authoritative GUI library store, with durable entry IDs,
  command-first edits, catalog-backed matching, and automatic migration and
  repair of existing libraries.
- Moved startup loading, scans, filesystem refreshes, hashing, and media
  discovery off the UI thread while keeping list metadata stable as entries
  are selected.
- Bounded GUI list projections and media memory use: list rows retain only
  lightweight asset-presence state, while image data is loaded and retained
  only for the currently focused detail view.
- Tailored game-list columns to each console's identification capabilities so
  unsupported serial, internal-name, region, and DAT fields are omitted.
- Fixed intermittent debug-build red outlines while scrolling large,
  virtualized game lists without disabling true same-frame ID collision
  warnings.
- The main GUI release now includes full Japanese, Chinese, and Korean font
  support; the separate `retro-junk-gui-cjk` release variant was removed.

## 0.2.0

- CUE sheet compatibility issues are now detected during scan and displayed as warning triangles in the game table and detailed messages in the detail panel, with clear "fixable" vs "re-dump required" messaging
- Added `retro-junk fix-cue` command to detect and convert CDRWin-format CUE sheets to standard CUE format for wider emulator compatibility (e.g., DuckStation rejecting `CD_ROM_XA` headers)
- Added `retro-junk systems` command listing all 25 supported systems with DAT/GDB capability tags, grouped by manufacturer, with optional `--manufacturer` filter
- Multi-system database commands (`catalog import`, `catalog enrich`, `catalog enrich-gdb`) now default to all systems when no arguments are given (was: "No systems specified")
- Unified system name validation across all commands into shared helpers (`resolve_systems`, `resolve_single_system`, `resolve_platform_ids`), replacing ~120 lines of duplicated ad-hoc logic
- All "unknown system" errors now consistently suggest `retro-junk systems` for discoverability
- `catalog gaps` now validates the system name (was: passed raw string to DB with no check)
- Updated help text on all system-accepting commands with examples and `retro-junk systems` hints
- Added Sega Saturn disc identification (ISO, BIN/CUE, CHD) with serial, region, and game name extraction
- Added CHD support for Saturn disc images (all compression codecs supported)
- Added `Region::Asia` and `Region::LatinAmerica` to the region enum
- Added `saturnjp` folder alias for Saturn
- Extracted shared disc utilities into `retro-junk-disc` crate for reuse across Sony, Sega, and future disc-based consoles
- Hardened ROM/disc parsing against malformed input
- Fixed WiiU DAT source, ClrMamePro size parsing, and miximage panic
- Added GUI keyboard navigation (arrow keys, Home/End, Page Up/Down, Ctrl+1/2/3 view switching, Shift+arrow selection)
- Fixed background operations (scan, hash, scrape, rename) targeting the wrong entry when the list changed mid-operation
- Fixed multi-disc `.m3u` folder scanning miscounting discs
- Added in-app log viewer and error dialogs for failed operations
- Added right-click "Copy" context menu to all value labels in the GUI detail panel
- Fixed CHD hashing using hardcoded 2448-byte sector stride instead of the CHD header's actual `unit_bytes`; CHDs without subchannel data (SUBTYPE:NONE) use 2352-byte sectors, causing wrong hashes for Saturn and other disc platforms
- Replaced hardcoded sector size literals across disc, Sony, and Sega crates with shared constants (`ISO_SECTOR_SIZE`, `RAW_SECTOR_SIZE`)
- Restructured `cache` subcommands: `cache list/clear/fetch` and `cache gdb-list/gdb-clear/gdb-fetch` are now `cache dat list/clear/fetch` and `cache gdb list/clear/fetch`
- Added `--force` flag to `cache gdb fetch` (skips re-download when cached, matching `cache dat fetch` behavior)
- Removed `config` alias from `credentials` command (conflicted with `settings`)
- Removed `--root` alias from `--library-path`
- Fixed hash matching for BIN dumps where audio tracks were written as zero-filled Mode 2 sectors instead of raw PCM. A secondary boundary detection now finds the data/filler boundary and warns about the incomplete dump.

## 0.1.2

- Added GUI to cargo-dist releases with per-platform builds (macOS, Linux, Windows)
- Added separate `retro-junk-gui-cjk` download variant with full CJK font support (~16MB larger); base `retro-junk-gui` ships without CJK fonts for a smaller download
- Added GameCube and Wii disc identification with RVZ/WBFS/CISO/GCZ compressed format support
- Added PS2 disc identification and hashing
- Added initial database viewer in GUI Tools view for browsing platforms, works, and releases
- Added `works_for_platform` query to catalog database
- Fixed GUI renames losing file extensions (e.g., PS2 `.iso` becoming `.bin`, GC `.rvz` becoming `.iso`) by centralizing extension handling in a single `target_filename_for_rename()` function used by both CLI and GUI
- Fixed auto-correction of previously damaged file extensions: renames now detect the actual file format at rename time, so misnamed files (e.g., RVZ named `.iso`) get the correct extension
- Fixed compressed disc analysis (RVZ, WIA, etc.) failing silently when `file_path` was missing from `AnalysisOptions` — affected both CLI serial matching and GUI format detection
- Fixed hashing of compressed GameCube/Wii disc images (RVZ, WIA, WBFS, CISO, GCZ) to decompress before hashing for correct Redump DAT matching
- Fixed DAT download URLs for GameCube, Wii, and PS2 (was requesting wrong filenames from LibRetro GitHub)
- Fixed serial matching for Redump product codes (e.g., `DL-DOL-GBIE-0-USA` now matchable by 4-char game code)
- Fixed disc-based games reverting to "Ambiguous" status after rescan
- Fixed "Ambiguous" status showing no explanation in GUI detail panel
- Refactored hashing code to share disc-hashing logic across PS1 and PS2

## 0.1.1

- Set up automated GitHub releases via cargo-dist
- Updated README with install instructions and current command reference
- Embedded ScreenScraper dev credentials in release builds

## 0.1.0

- Initial release
- ROM analysis with header parsing for NES, SNES, N64, GB, GBA, DS, 3DS, Genesis, PS1
- Rename ROMs to canonical No-Intro / Redump names via serial or hash matching
- Scrape metadata and media from ScreenScraper (covers, screenshots, videos, marquees)
- ES-DE frontend output (gamelist.xml)
- DAT file caching from No-Intro and Redump
- Multi-disc game support via .m3u folders
- Catalog database with enrichment from ScreenScraper and GameDataBase
- GUI with library management (early)
- 23 consoles across Nintendo, Sony, Sega, and Microsoft
