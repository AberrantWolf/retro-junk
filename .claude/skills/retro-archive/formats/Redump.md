# Redump Preservation Project

## Overview

Redump (redump.org) is a disc preservation database and internet community dedicated to collecting
precise and accurate information about every video game ever released on optical media of any system.
The goal is to create "blueprints" of the data on console and computer game discs, producing verified
1:1 copies of the original media with all data intact.

Redump was founded as a response to the widespread practice of distributing disc images with data
loss, such as stripping audio tracks, removing pregap data, or using lossy compression. Redump
specifically aims to preserve every bit of data on the original disc, including subchannel data,
pregaps, and mixed-mode audio/data structures.

## How Redump Differs from No-Intro

| Aspect | Redump | No-Intro |
|--------|--------|----------|
| Media type | Optical disc (CD, DVD, GD-ROM, Blu-ray) | Cartridges and ROM chips |
| Focus | Disc-accurate images with full track structure | Clean cartridge dumps, playable ROMs |
| Sector format | Raw 2352-byte sectors (full raw with ECC/EDC) | N/A (ROM dumps are flat binary) |
| Audio tracks | Preserved as separate track files | N/A |
| Parent/clone info | Not provided in DAT files | Provided |
| Image size | Larger (preserves all mastering data) | Smaller (just the ROM data) |
| Systems | Disc-based consoles and computers | Cartridge-based consoles and handhelds |

Both projects use Logiqx XML DAT files and share the same general philosophy of keeping only the
best verified dumps without modifications, but they serve different media types.

## Systems Covered

Redump covers virtually every platform that used optical media. The complete system list is
documented at http://wiki.redump.org/index.php?title=System_list. Key systems include:

### Sony
- Sony PlayStation (PSX) — `datfile/psx`
- Sony PlayStation 2 (PS2) — `datfile/ps2`
- Sony PlayStation 3 (PS3) — `datfile/ps3`
- Sony PlayStation Portable (PSP) — `datfile/psp`
- Sony PlayStation Vita — `datfile/psv`

### Sega
- Sega Mega CD / Sega CD — `datfile/mcd`
- Sega Saturn — `datfile/ss`
- Sega Dreamcast — `datfile/dc`
- Sega Chihiro (arcade) — `datfile/chihiro`
- Sega Naomi / Naomi 2 — `datfile/naomi` / `datfile/naomi2`

### Nintendo
- Nintendo GameCube — `datfile/gc`
- Nintendo Wii — `datfile/wii`
- Nintendo Wii U — `datfile/wiiu`

### NEC
- PC Engine CD / TurboGrafx CD — `datfile/pce`
- PC-FX / PC-FXGA — `datfile/pc-fx`
- PC-88 series — `datfile/pc-88`
- PC-98 series — `datfile/pc-98`

### Microsoft
- Microsoft Xbox — `datfile/xbox`
- Microsoft Xbox 360 — `datfile/xbox360`

### Others
- Panasonic 3DO — `datfile/3do`
- Philips CD-i — `datfile/cdi`
- Neo Geo CD — `datfile/ngcd`
- Commodore Amiga CD32 — `datfile/cd32`
- Commodore Amiga CDTV — `datfile/cdtv`
- Atari Jaguar CD — `datfile/ajcd`
- IBM PC compatible — `datfile/pc`
- FM Towns — `datfile/fmt`
- Apple Macintosh — `datfile/mac`
- Bandai Pippin — `datfile/pippin`
- Bandai Playdia — `datfile/playdia`
- Namco System 246 — `datfile/ns246`
- Photo CD — `datfile/photo-cd`
- DVD-Video — `datfile/dvd-video`
- Blu-ray Video — `datfile/bd-video`
- Audio CD — `datfile/audio-cd`
- Various arcade and amusement systems (Konami System 573, Konami FireBeat, Konami e-Amusement, etc.)

As of 2024, Redump surpassed 50,000 catalogued PC discs alone.

## DAT File Format

Redump uses the **Logiqx XML** format, the same format used by No-Intro and TOSEC. This is an
industry-standard format supported by all major ROM management tools.

### Structure

```xml
<?xml version="1.0"?>
<!DOCTYPE datafile PUBLIC "-//Logiqx//DTD ROM Management Datafile//EN"
                          "http://www.logiqx.com/Dats/datafile.dtd">
<datafile>
  <header>
    <name>Sony - PlayStation</name>
    <description>Sony - PlayStation</description>
    <version>20240101-000000</version>
    <date>2024-01-01</date>
    <author>no-intro</author>
    <homepage>Redump</homepage>
    <url>http://redump.org</url>
  </header>
  <game name="Crash Bandicoot (USA)">
    <category>Games</category>
    <description>Crash Bandicoot (USA)</description>
    <rom name="Crash Bandicoot (USA).cue" size="105" crc="..." md5="..." sha1="..."/>
    <rom name="Crash Bandicoot (USA) (Track 1).bin" size="..." crc="..." md5="..." sha1="..."/>
    <rom name="Crash Bandicoot (USA) (Track 2).bin" size="..." crc="..." md5="..." sha1="..."/>
  </game>
</datafile>
```

### Key Differences from No-Intro DATs
- Each track file gets its own `<rom>` entry with separate checksums
- A `.cue` file entry is included, also with its own checksum
- No parent/clone relationships are expressed in Redump DATs
- Multi-disc games appear as separate `<game>` entries

### Optional URL Parameters

The Redump DAT download URL accepts parameters to include additional fields:

```
http://redump.org/datfile/ps2/serial,version
```

This adds `serial` and `version` elements to game entries in the XML.

## Obtaining DAT Files

DAT files can be downloaded directly from redump.org without requiring registration:

```
http://redump.org/datfile/<system-id>/
```

For example:
- `http://redump.org/datfile/psx/` — PlayStation 1
- `http://redump.org/datfile/ps2/` — PlayStation 2
- `http://redump.org/datfile/dc/` — Dreamcast
- `http://redump.org/datfile/ss/` — Sega Saturn
- `http://redump.org/datfile/gc/` — Nintendo GameCube

The full list of download links is at:
http://wiki.redump.org/index.php?title=List_of_DB_Download_Links

DAT files can be imported into ROM managers such as:
- **CLRMAMEPro** — the classic standard
- **RomVault** — modern alternative
- **oxyROMon** — Rust-based open source tool
- **Retool** — filter/customize Redump and No-Intro DATs by region/language

## Disc Image Formats

Redump catalogues images in several formats depending on the system:

### BIN/CUE (Most Common for CD-based Systems)
Used for: PSX, Sega CD, Saturn, Dreamcast (CD portion), PC Engine CD, Neo Geo CD, 3DO, etc.

- **One BIN file per track** — each audio or data track is a separate `.bin` file
- **CUE sheet** — describes the track layout, modes, pregaps, and index points
- Sector size: **2352 bytes raw** (includes sync, header, ECC/EDC — not just user data)
- Example:
  ```
  Crash Bandicoot (USA).cue
  Crash Bandicoot (USA) (Track 1).bin
  Crash Bandicoot (USA) (Track 2).bin
  ```

### GDI (Dreamcast GD-ROM)
Used for: Sega Dreamcast (GD-ROM format)

- A `.gdi` descriptor file plus multiple track files
- GD-ROM has a high-density area and a standard CD area
- Redump distributes Dreamcast images as BIN/CUE (the CD area portion) or GDI

### ISO (DVD-based Systems)
Used for: PS2, GameCube, Wii, Xbox, Xbox 360, etc.

- Single `.iso` file containing the full disc image
- DVD sector size: 2048 bytes (user data only, not raw)
- Some systems use custom disc formats (GameCube uses Miniature DVD with proprietary structure)

### IRD (PS3)
Used for: Sony PlayStation 3

- `.ird` (ISO Reconstruction Data) files used to reconstruct and verify PS3 disc images
- PS3 uses Blu-ray with encryption; IRD files allow verification of decrypted images

### CHD (Compressed Hunks of Data)
Not a Redump native format, but commonly used as a lossless conversion of Redump BIN/CUE and
GDI images. Supported by MAME and most modern emulators. The `chdman` tool converts BIN/CUE
and GDI to CHD and back.

## Checksums

Redump tracks **three checksums per file** (per track):

| Checksum | Size | Purpose |
|----------|------|---------|
| CRC32    | 32-bit | Fast matching, used by ROM managers |
| MD5      | 128-bit | Legacy, still used for quick database lookups |
| SHA1     | 160-bit | Primary integrity verification |

### Important Notes on Checksums
- **Per-track, not per-disc**: Each track file (`.bin`) and the CUE sheet have their own
  individual checksums. There is no single whole-disc checksum.
- **2352-byte raw sectors**: Data track checksums cover the full raw sector including sync
  bytes, header, ECC, and EDC — not just the 2048-byte user data. This means Redump
  checksums are NOT compatible with ISO files.
- **Verification**: You can verify a dump by entering its MD5 or SHA1 into the quick search
  on redump.org.

## Naming Convention

Redump uses a consistent naming scheme:

```
Title (Region) (Languages) (Version) (Status) (Edition) (Disc N)
```

### Region Tags
Full region names are spelled out (not abbreviated):
- `(USA)` — United States
- `(Japan)` — Japan
- `(Europe)` — Europe (PAL)
- `(Germany)` — specific European country
- `(France)`, `(Spain)`, `(Italy)`, etc.
- `(World)` — used only when matching dumps from all major regions exist; rare

### Language Tags
Comma-separated ISO language codes when multiple languages are present:
- `(En)` — English
- `(En,Fr,De,Es,It)` — multilingual

### Version Tags
- `(v1.0)`, `(v1.1)`, `(Rev 1)` — revision information
- `(v1.0) (Rev 1)` — both version and revision

### Status Tags
- `(Demo)` — demo disc
- `(Promo)` — promotional copy
- `(Beta)` — beta version

### Disc Tags
- `(Disc 1)`, `(Disc 2)`, etc.
- `(Disc A)`, `(Disc B)` — when labelled alphabetically on the physical disc

### Serial Information
Serial numbers from the disc label are tracked in the database and can be included in
DAT exports via URL parameters, but are not part of the filename itself.

### Examples
```
Final Fantasy VII (USA) (Disc 1).cue
Final Fantasy VII (USA) (Disc 2).cue
Final Fantasy VII (USA) (Disc 3).cue
Crash Bandicoot (USA).cue
Tekken 3 (Europe) (En,Fr,De,Es,It).cue
Ridge Racer (Japan) (Demo).cue
```

## Dumping Tools

The official tools for creating Redump-verified dumps are:

- **DiscImageCreator** — the long-standing standard for CD/DVD dumping
- **redumper** — newer open-source tool, cross-platform, increasingly preferred
- **MPF (Media Preservation Frontend)** — GUI frontend for DiscImageCreator

These tools produce the split-track BIN/CUE format that Redump standardizes on, and also
capture subchannel data and write offset information for verification.

## Sources

- Redump main site: http://redump.org/
- Redump Wiki: http://wiki.redump.org/index.php?title=Redump.org
- System List: http://wiki.redump.org/index.php?title=System_list
- Download Links: http://wiki.redump.org/index.php?title=List_of_DB_Download_Links
- Redump Search Parameters: http://wiki.redump.org/index.php?title=Redump_Search_Parameters
- "What is Redump in ROMs?" (ROMsFun): https://romsfun.com/what-is-redump-in-roms/
- "Redump and No-Intro Sets Explained" (PulseGeek): https://pulsegeek.com/articles/redump-and-no-intro-sets-explained-for-accuracy/
- Retool (DAT filter utility): https://github.com/unexpectedpanda/retool
- verifydump (CHD/RVZ Redump verification): https://github.com/j68k/verifydump
- "The Working Archivist's Guide to Enthusiast CD-ROM Archiving Tools" (2024):
  https://www.mistys-internet.website/blog/blog/2024/09/13/the-working-archivists-guide-to-enthusiast-cd-rom-archiving-tools/
- Recalbox Wiki on CHDMAN: https://wiki.recalbox.com/en/tutorials/utilities/rom-conversion/chdman
- PS2-HOME tutorial on MD5 verification: https://www.ps2-home.com/forum/viewtopic.php?t=67
- GBAtemp comparison of ROM sets: https://gbatemp.net/threads/a-comparison-of-romsets-wiki-no-roms.504082/
