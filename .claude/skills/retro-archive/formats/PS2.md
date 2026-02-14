# Sony PlayStation 2 Disc Format

Used by: [Sony PlayStation 2](../consoles/PS2_Overview.md)

## File Extensions
- `.iso` - ISO 9660 DVD image (most common)
- `.bin/.cue` - Binary image with cue sheet (for CD-based PS2 games)
- `.chd` - Compressed Hunks of Data (modern format)
- `.cso` - Compressed ISO (used by OPL)
- `.zso` - LZ4-compressed ISO

## Disc Format

PS2 games shipped on both CD-ROM and DVD-ROM media:
- **CD-ROM:** ~700MB, Mode 2/Form 1, 2048-byte sectors (early titles)
- **DVD-5:** 4.7GB single layer (most games)
- **DVD-9:** 8.5GB dual layer (large games)

## Primary Volume Descriptor

Standard ISO 9660 PVD at sector 16 (byte offset 0x8000):

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0x8000 | 1 | Type Code | `0x01` (Primary Volume Descriptor) |
| 0x8001 | 5 | Standard ID | `CD001` |
| 0x8006 | 1 | Version | `0x01` |
| 0x8028 | 32 | System Identifier | `PLAYSTATION` (space-padded) |
| 0x8048 | 32 | Volume Identifier | Game title |
| 0x8050 | 8 | Volume Space Size | Total sectors (both-endian) |
| 0x8078 | 4 | Volume Set Size | Usually 1 |
| 0x8080 | 4 | Volume Seq Number | Usually 1 |
| 0x8084 | 4 | Logical Block Size | 2048 (both-endian) |

## SYSTEM.CNF

The boot configuration file is located in the root directory. It is a text file containing key-value pairs:

```
BOOT2 = cdrom0:\SLUS_200.62;1
VER = 1.00
VMODE = NTSC
```

Key fields:
- `BOOT2` - Path to the main ELF executable (PS2 games use `BOOT2`, PS1 used `BOOT`)
- `VER` - Game version
- `VMODE` - Video mode (`NTSC` or `PAL`)

## Serial Number Format

PS2 serials follow the format `XXXX-NNNNN` or `XXXX_NNN.NN`:
- **SCUS/SLUS** - North America (Sony/Licensed)
- **SCES/SLES** - Europe
- **SCPS/SLPS/SLPM** - Japan
- **SCKA/SLKA** - Korea

The serial appears in:
1. The `BOOT2` path in SYSTEM.CNF (e.g., `SLUS_200.62`)
2. The volume identifier in the ISO PVD

## Detection Method

1. Read the ISO 9660 PVD at byte offset 0x8000
2. Verify `CD001` magic at offset 0x8001
3. Check System Identifier at offset 0x8028 for `PLAYSTATION`
4. Look for `SYSTEM.CNF` in the root directory
5. Parse SYSTEM.CNF and verify it contains a `BOOT2` key (distinguishes PS2 from PS1 which uses `BOOT`)
6. The boot executable path typically starts with `cdrom0:\` followed by the serial

## Regional Protection

PS2 uses two layers of region protection:
- **Hardware region lock:** BIOS checks disc region code against console region
- **MechaCon authentication:** The disc drive controller validates disc authenticity via "wobble groove" data burned into retail discs

Region codes in the disc:
- `A` - North America (NTSC-U)
- `E` - Europe (PAL)
- `J` - Japan (NTSC-J)

## Sources
- [PlayStation 2 Technical Specifications](https://en.wikipedia.org/wiki/PlayStation_2_technical_specifications)
- [Copetti PS2 Architecture Analysis](https://www.copetti.org/writings/consoles/playstation-2/)
- [PCSX2 Documentation](https://pcsx2.net/)
