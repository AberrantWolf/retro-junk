# Changelog

## 0.1.2

- Fixed hashing of compressed GameCube/Wii disc images (RVZ, WIA, WBFS, CISO, GCZ) to decompress before hashing for correct Redump DAT matching
- Added GameCube and Wii disc identification with RVZ/WBFS/CISO/GCZ compressed format support
- Added PS2 disc identification and hashing
- Refactored hashing code to share disc-hashing logic across PS1 and PS2
- Fixed DAT download URLs for GameCube, Wii, and PS2 (was requesting wrong filenames from LibRetro GitHub)
- Fixed serial matching for Redump product codes (e.g., `DL-DOL-GBIE-0-USA` now matchable by 4-char game code)
- Fixed disc-based games reverting to "Ambiguous" status after rescan
- Fixed "Ambiguous" status showing no explanation in GUI detail panel

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
