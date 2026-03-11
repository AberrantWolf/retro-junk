# Changelog

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
