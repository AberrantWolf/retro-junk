# retro-junk

A CLI tool for analyzing retro game ROMs and disc images.

## Build

```bash
cargo build --release
```

## Install

```bash
cargo install --path retro-junk-cli
```

## Usage

List supported consoles:
```bash
retro-junk list
```

Analyze ROMs (scans for console-named folders like `snes/`, `n64/`, `ps1/`):
```bash
retro-junk analyze --root /path/to/roms
```

Options:
- `--quick` / `-q` - Minimize disk reads (useful for network shares)
- `--consoles` / `-c` - Filter to specific consoles: `-c snes,n64,ps1`
- `--root` / `-r` - Root path (defaults to current directory)

## Supported Consoles

**Nintendo:** NES, SNES, N64, GameCube, Wii, Wii U, Game Boy, GBA, DS, 3DS

**Sony:** PS1, PS2, PS3, PSP, Vita

**Sega:** SG-1000, Master System, Genesis, Sega CD, 32X, Saturn, Dreamcast, Game Gear

**Microsoft:** Xbox, Xbox 360
