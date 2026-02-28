# 🕹️ retro-junk

A CLI tool for analyzing, renaming, and scraping metadata for retro game ROMs and disc images. Supports 23 consoles across Nintendo, Sony, Sega, and Microsoft platforms.

## 📦 Install

**From GitHub Releases** (prebuilt binaries for macOS, Linux, Windows):

```bash
# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/AberrantWolf/retro-junk/releases/latest/download/retro-junk-cli-installer.sh | sh

# Windows (PowerShell)
irm https://github.com/AberrantWolf/retro-junk/releases/latest/download/retro-junk-cli-installer.ps1 | iex
```

**From source:**

```bash
cargo install --path retro-junk-cli
```

## 🚀 Quick Start

retro-junk expects ROMs organized in console-named folders (e.g., `snes/`, `n64/`, `ps1/`). Set your library path once with `retro-junk settings library-path /path/to/roms`, or pass `-L /path/to/roms` to any command.

```bash
# See which consoles and folder names are supported
retro-junk list

# Analyze ROM headers and validate integrity
retro-junk analyze

# Rename ROMs to canonical No-Intro / Redump names (preview first!)
retro-junk rename --dry-run

# Scrape metadata and media from ScreenScraper
retro-junk scrape --dry-run
```

Use `--help` on any command for the full list of options.

## 🔧 Commands

| Command | Description |
|---------|-------------|
| `list` | Show supported consoles and their folder names |
| `analyze` | Extract header metadata and validate ROM integrity |
| `rename` | Rename ROMs to canonical names via serial or hash matching |
| `repair` | ⚗️ *Experimental* — Repair trimmed/truncated ROMs by padding to match DAT checksums |
| `scrape` | Download metadata and media from ScreenScraper |
| `cache` | Manage cached DAT and GDB files (`list`, `fetch`, `clear`, `gdb-list`, `gdb-fetch`, `gdb-clear`) |
| `credentials` | Set up and test ScreenScraper API credentials (`setup`, `show`, `test`, `path`) |
| `settings` | Manage app settings like library path (`show`, `library-path`) |
| `catalog` | Manage the game catalog database (`import`, `enrich`, `scan`, `lookup`, `stats`, and more) |

**Global flags:** `-L` to set library path, `-c` to filter consoles, `-n` / `--dry-run` to preview, `-l` / `--limit` to cap per-console.

## 🎮 Supported Consoles

| Platform | Consoles |
|----------|----------|
| **Nintendo** | NES, SNES, N64, GameCube, Wii, Wii U, Game Boy, GBA, DS, 3DS |
| **Sony** | PS1, PS2, PS3, PSP, Vita |
| **Sega** | SG-1000, Master System, Genesis, Sega CD, 32X, Saturn, Dreamcast, Game Gear |
| **Microsoft** | Xbox, Xbox 360 |

## ⚠️ Known Limitations

- **Partial analyzer coverage** — Header analysis and serial-based matching are only implemented for NES, SNES, N64, GB, GBA, DS, 3DS, Genesis, and PS1. Other consoles rely on hash-based matching only.
- **Disc images** — Full ISO/BIN+CUE/CHD parsing is only implemented for PS1. Other disc consoles use hash matching.
- **Frontend output** — Only ES-DE (`gamelist.xml`) is supported. Pegasus, LaunchBox, etc. are not yet implemented.
- **Compressed ROMs** — No support for reading ROMs inside ZIP or 7z archives.
- **GUI** — Not yet implemented.

## 📄 License

MIT
