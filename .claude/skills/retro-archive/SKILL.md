---
name: retro-archive
description: Use when working on retro game archival formats, console-specific dump structures, Redump verification, or preservation workflows in retro-junk.
---
# Retro Archive

Games are stored in files appropriate to having backed them up from their original physical or digital sources.

Games are separated into folders by console, often using a shortcut or shortening of the console name (e.g., Super Nintendo Entertainment System -> snes).

*IMPORTANT:* When you learn something about a specific file format, add that information to a named file in [/formats](/formats) and link to the format information from every console whose archived data uses that format.

## Scraper Databases

There are several tools online for comparing data and matching metadata up with games based on hashes, checksums, and serial numbers.

Notable Sources:

* screenscraper.fr
  * Has a public API
  * Requires a username and password to access multiple threads at once
  * Further requires an API key for beyond the simple API access
* NoIntro
  * DAT files can be found [here](https://github.com/libretro-mirrors/nointro-db/tree/master)
  * Some consoles have multiple DAT files (regular games, and digital games and DLC are separate)
* Redump (redump.info)
  * Covers disc-based systems (CD, DVD, GD-ROM, Blu-ray) — see [formats/Redump.md](formats/Redump.md)
  * Formerly at `redump.org` — that domain is defunct; always use `redump.info` (same URL paths)
  * DAT files downloadable directly from `https://redump.info/datfile/<system>/` (no login required)
  * Uses Logiqx XML format with per-track CRC32, MD5, and SHA1 checksums
  * Standard for verifying BIN/CUE disc images

## More Information

For information on specific consoles, see the following files:
* [Nintendo Entertainment System, NES, Famicom, FC](consoles/NES_Overview.md)
* [SNES, SFC, Super Nintendo, Super Famicom](consoles/SNES_Overview.md)
* [N64, Nintendo 64](consoles/N64_Overview.md)
* [Nintendo GameCube](consoles/GameCube_Overview.md)
* [Nintendo Wii](consoles/Wii_Overview.md)
* [Nintendo Switch](consoles/Switch_Overview.md)
* [GameBoy](consoles/GB_Overview.md)
* [GameBoy Color](consoles/GBC_Overview.md)
* [GameBoy Advance](consoles/GBA_Overview.md)
* [Nintendo DS](consoles/NDS_Overview.md)
* [Nintendo 3DS](consoles/3DS_Overview.md)
* [Sega Master System](consoles/MasterSystem_Overview.md)
* [Sega Genesis, Megadrive](consoles/Genesis_Overview.md)
* [Sega Saturn](consoles/Saturn_Overview.md)
* [Sega Dreamcast](consoles/Dreamcast_Overview.md)
* [NEC PC Engine, TurboGrafx-16, PCE, TG16, CD-ROM²](consoles/PCEngine_Overview.md)
* [Sony PlayStation Portable, PSP](consoles/PSP_Overview.md)
* [Sony PlayStation Vita](consoles/Vita_Overview.md)
* [Sony Playstation](consoles/PSX_Overview.md)
* [Sony Playstation 2](consoles/PS2_Overview.md)
* [Sony Playstation 3](consoles/PS3_Overview.md)
* [Sony Playstation 4](consoles/PS4_Overview.md)
* [Sony Playstation 5](consoles/PS5_Overview.md)
* [Microsoft Xbox](consoles/Xbox_Overview.md)
* [Microsoft Xbox 360](consoles/360_Overview.md)
* [Microsoft Xbox One](consoles/XBO_Overview.md)
* [Microsoft Xbox Series X/S](consoles/XBS_Overview.md)
