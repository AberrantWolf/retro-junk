# Changelog

## Unreleased

## 0.4.0

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
