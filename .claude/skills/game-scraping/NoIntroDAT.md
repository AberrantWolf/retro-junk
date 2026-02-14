---
description: Information on accessing and parsing No-Intro and Redump DAT files
---

# No-Intro & Redump Databases (DAT Files)

## Overview

**No-Intro** catalogs verified, unmodified dumps of cartridge/ROM-chip media.
**Redump** catalogs verified, bit-perfect dumps of optical disc media (CD, DVD, GD-ROM, Blu-ray).

Neither project provides a public web API. Data is distributed as **DAT files** — XML documents in
Logiqx format.

## How to Obtain DAT Files

### No-Intro
1. Create an account at https://datomatic.no-intro.org/
2. Navigate to the "Download" section.
3. Choose a specific system, or choose "Daily" for a bulk pack of all DATs.
4. Download in **Logiqx XML** format (recommended for programmatic parsing).

### Redump
1. Go to http://redump.org/downloads/ (no account required).
2. Download DATs by system. Direct links follow the pattern:
   `http://redump.org/datfile/<system>/`
3. Full list of system identifiers at http://wiki.redump.org/

DAT files should be stored locally (e.g., a `dat/` directory) and updated periodically.

## DAT File Format (Logiqx XML)

Both No-Intro and Redump use the same Logiqx XML format:

```xml
<?xml version="1.0"?>
<!DOCTYPE datafile SYSTEM "http://www.logiqx.com/Dats/datafile.dtd">
<datafile>
  <header>
    <name>Nintendo - Game Boy Advance</name>
    <description>Nintendo - Game Boy Advance (20240101-123456)</description>
    <version>20240101-123456</version>
    <author>No-Intro</author>
    <homepage>No-Intro</homepage>
    <url>https://www.no-intro.org</url>
  </header>
  <game name="Legend of Zelda, The - The Minish Cap (USA)">
    <description>Legend of Zelda, The - The Minish Cap (USA)</description>
    <rom name="Legend of Zelda, The - The Minish Cap (USA).gba"
         size="16777216"
         crc="1A318DE4"
         md5="b0f3b3a553e39a1e0e4e62c1f843b228"
         sha1="574e362a21a3a53e84a6f3b3db137babe8c15c2e" />
  </game>
</datafile>
```

Redump entries typically have multiple `<rom>` children — one per track file and one for the CUE
sheet:

```xml
<game name="Castlevania - Symphony of the Night (USA)">
  <description>Castlevania - Symphony of the Night (USA)</description>
  <rom name="Castlevania - Symphony of the Night (USA).cue"
       size="93" crc="A1B2C3D4" md5="..." sha1="..." />
  <rom name="Castlevania - Symphony of the Night (USA) (Track 1).bin"
       size="47552800" crc="E5F6A7B8" md5="..." sha1="..." />
  <rom name="Castlevania - Symphony of the Night (USA) (Track 2).bin"
       size="22325280" crc="C9D0E1F2" md5="..." sha1="..." />
</game>
```

## Key XML Elements

| Element/Attribute      | Description                                         |
|------------------------|-----------------------------------------------------|
| `<header>`             | Metadata about the DAT (system name, version, etc.) |
| `<game>`               | One entry per game/ROM. `name` attr = canonical name|
| `<game>/<description>` | Human-readable game title                           |
| `<rom>`                | ROM/track file details                              |
| `<rom>` `name`         | Canonical filename                                  |
| `<rom>` `size`         | File size in bytes                                  |
| `<rom>` `crc`          | CRC32 checksum (uppercase hex, 8 characters)        |
| `<rom>` `md5`          | MD5 checksum (lowercase hex, 32 characters)         |
| `<rom>` `sha1`         | SHA1 checksum (lowercase hex, 40 characters)        |

## Header Stripping

No-Intro DATs catalog **headerless** ROMs. Many ROM files have platform-specific headers that must
be removed before hashing, or the checksums will not match.

| Platform | Header      | Size     | Detection                                    |
|----------|-------------|----------|----------------------------------------------|
| NES      | iNES        | 16 bytes | Starts with `NES\x1a`                        |
| NES      | NES 2.0     | 16 bytes | Starts with `NES\x1a`, bit 3 of byte 7 set  |
| SNES     | SMC/SWC     | 512 bytes| File size mod 1024 == 512                    |
| Lynx     | LNX         | 64 bytes | Starts with `LYNX\x00`                       |
| N64      | V64 (byteswap) | 0     | No header, but bytes may need word-swapping  |
| GBA      | None        | 0        | No header — hash file directly               |
| Genesis  | None        | 0        | No header — hash file directly               |

**Strategy**: Check for known header signatures. If present, skip the header bytes before hashing.
For SNES, check `file_size % 1024 == 512` — if true, strip the first 512 bytes.

## Compressed ROM Handling

ROMs are often distributed in ZIP or 7z archives. Always hash the **contained file**, not the
archive:

- **ZIP**: The CRC32 stored in the ZIP central directory matches the uncompressed file's CRC32.
  This allows a quick No-Intro CRC lookup without decompressing. For MD5/SHA1, decompress first.
- **7z**: Must decompress to compute any hash.
- **Multi-file archives**: Some archives contain multiple ROMs. Handle each file individually.

## Naming Conventions

### No-Intro
```
Title (Region) [(flags)]
```
- **Region codes**: (USA), (Europe), (Japan), (World), (USA, Europe), etc.
- **Revision**: (Rev 1), (Rev 2), etc.
- **Beta/Proto**: (Beta), (Proto), (Sample)
- **Unlicensed**: (Unl)
- **Flags in brackets**: [b] = bad dump, [!] = verified good dump (older convention)

### Redump
```
Title (Region) (Languages) (Version) (Status) (Disc N)
```
- **Region**: Full names — (USA), (Japan), (Europe), (Germany), etc.
- **Languages**: ISO codes — (En), (En,Fr,De), etc.
- **Disc**: (Disc 1), (Disc 2) for multi-disc games
- Serial numbers tracked in the database but not in filenames.

## Redump Disc Image Formats

| Format   | Used For                              | Notes                              |
|----------|---------------------------------------|------------------------------------|
| BIN/CUE  | CD-based (PS1, Saturn, Sega CD, etc.) | One BIN per track, CUE as index   |
| GDI      | Dreamcast GD-ROM                      | Proprietary GD-ROM format          |
| ISO      | DVD-based (PS2, GameCube, Wii, Xbox)  | Single file, 2048-byte sectors     |
| CHD      | Compressed conversion                 | Not native Redump, but widely used |

**Important**: Redump checksums cover raw 2352-byte CD sectors. If you have a 2048-byte-sector ISO
ripped differently, checksums will not match.

## Parsing Strategy (Rust)

```rust
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;
use std::io::BufRead;

#[derive(Debug, Clone)]
struct DatRom {
    game_name: String,
    description: String,
    rom_name: String,
    size: u64,
    crc32: String,
    md5: String,
    sha1: String,
}

#[derive(Debug, Default)]
struct DatIndex {
    by_crc: HashMap<String, DatRom>,
    by_md5: HashMap<String, DatRom>,
    by_sha1: HashMap<String, DatRom>,
}

impl DatIndex {
    /// Look up a ROM by hash. SHA1 is preferred, then MD5, then CRC32.
    fn lookup(&self, crc: Option<&str>, md5: Option<&str>, sha1: Option<&str>) -> Option<&DatRom> {
        if let Some(h) = sha1 {
            if let Some(rom) = self.by_sha1.get(&h.to_uppercase()) {
                return Some(rom);
            }
        }
        if let Some(h) = md5 {
            if let Some(rom) = self.by_md5.get(&h.to_uppercase()) {
                return Some(rom);
            }
        }
        if let Some(h) = crc {
            if let Some(rom) = self.by_crc.get(&h.to_uppercase()) {
                return Some(rom);
            }
        }
        None
    }
}

/// Parse a Logiqx XML DAT file (No-Intro or Redump) into a hash index.
fn parse_dat_file<R: BufRead>(reader: R) -> Result<DatIndex, Box<dyn std::error::Error>> {
    let mut xml = Reader::from_reader(reader);
    let mut buf = Vec::new();
    let mut index = DatIndex::default();

    let mut current_game_name = String::new();
    let mut current_description = String::new();
    let mut in_game = false;
    let mut in_description = false;

    loop {
        match xml.read_event_into(&mut buf)? {
            Event::Start(e) => match e.name().as_ref() {
                b"game" => {
                    in_game = true;
                    current_game_name = e.attributes()
                        .filter_map(|a| a.ok())
                        .find(|a| a.key.as_ref() == b"name")
                        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                        .unwrap_or_default();
                    current_description.clear();
                }
                b"description" if in_game => {
                    in_description = true;
                }
                _ => {}
            },
            Event::Empty(e) if e.name().as_ref() == b"rom" && in_game => {
                let mut rom = DatRom {
                    game_name: current_game_name.clone(),
                    description: current_description.clone(),
                    rom_name: String::new(),
                    size: 0,
                    crc32: String::new(),
                    md5: String::new(),
                    sha1: String::new(),
                };
                for attr in e.attributes().filter_map(|a| a.ok()) {
                    let val = String::from_utf8_lossy(&attr.value).into_owned();
                    match attr.key.as_ref() {
                        b"name" => rom.rom_name = val,
                        b"size" => rom.size = val.parse().unwrap_or(0),
                        b"crc" => rom.crc32 = val.to_uppercase(),
                        b"md5" => rom.md5 = val.to_uppercase(),
                        b"sha1" => rom.sha1 = val.to_uppercase(),
                        _ => {}
                    }
                }
                if !rom.crc32.is_empty() {
                    index.by_crc.insert(rom.crc32.clone(), rom.clone());
                }
                if !rom.md5.is_empty() {
                    index.by_md5.insert(rom.md5.clone(), rom.clone());
                }
                if !rom.sha1.is_empty() {
                    index.by_sha1.insert(rom.sha1.clone(), rom.clone());
                }
            }
            Event::Text(e) if in_description => {
                current_description = e.unescape()?.into_owned();
            }
            Event::End(e) => match e.name().as_ref() {
                b"game" => in_game = false,
                b"description" => in_description = false,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(index)
}
```

## Mapping DAT Systems to ScreenScraper System IDs

Build a mapping between DAT file system names (from `<header><name>`) and ScreenScraper numeric
system IDs. Call ScreenScraper's `systemesListe.php` endpoint for the authoritative list.

Partial mapping:

```rust
/// Map No-Intro/Redump DAT header names to ScreenScraper system IDs.
fn dat_name_to_screenscraper_id(dat_name: &str) -> Option<u32> {
    match dat_name {
        // No-Intro (cartridge systems)
        "Nintendo - Nintendo Entertainment System (Headered)" => Some(3),
        "Nintendo - Super Nintendo Entertainment System" => Some(4),
        "Nintendo - Game Boy" => Some(9),
        "Nintendo - Game Boy Color" => Some(10),
        "Nintendo - Game Boy Advance" => Some(12),
        "Nintendo - Nintendo 64" => Some(14),
        "Nintendo - Nintendo DS" => Some(15),
        "Nintendo - Nintendo 3DS" => Some(17),
        "Sega - Mega Drive - Genesis" => Some(1),
        "Sega - Master System - Mark III" => Some(2),
        "Sega - Game Gear" => Some(21),
        "Sega - 32X" => Some(19),
        "Atari - 2600" => Some(26),
        "NEC - PC Engine - TurboGrafx-16" => Some(31),
        "SNK - Neo Geo Pocket" => Some(25),
        "SNK - Neo Geo Pocket Color" => Some(82),
        // Redump (disc systems)
        "Sony - PlayStation" => Some(57),
        "Sony - PlayStation 2" => Some(58),
        "Sony - PlayStation 3" => Some(59),
        "Sony - PlayStation Portable" => Some(61),
        "Sony - PlayStation Vita" => Some(62),
        "Sega - Dreamcast" => Some(23),
        "Sega - Saturn" => Some(22),
        "Sega - Mega-CD - Sega CD" => Some(20),
        "Nintendo - GameCube" => Some(13),
        "Nintendo - Wii" => Some(16),
        "Nintendo - Wii U" => Some(18),
        "Microsoft - Xbox" => Some(32),
        "Microsoft - Xbox 360" => Some(33),
        "NEC - PC Engine CD - TurboGrafx-CD" => Some(114),
        _ => None,
    }
}
```

## Workflow: Full ROM Identification Pipeline

1. **Hash the ROM file** — compute CRC32, MD5, SHA1, and file size. Strip headers first if needed.
2. **DAT lookup** — search the appropriate local DAT index by hash:
   - No-Intro for cartridge ROMs.
   - Redump for disc images (match against individual track files).
   - If found: dump is a verified good copy. Extract canonical name, region, etc.
   - If not found: may be a hack, bad dump, or not yet cataloged.
3. **ScreenScraper lookup** — send hashes and system ID to `jeuInfos.php`.
   - Returns rich metadata: description, box art, screenshots, videos, release dates, etc.
4. **Merge results** — combine DAT verification status with ScreenScraper metadata.

## Related Tools

ROM managers that consume DAT files include **CLR-MAME-PRO**, **RomVault**, and **Romulus**.
These tools can verify, rename, and organize ROM collections against DAT files. For programmatic
use, parse the XML directly as shown above.

## Important Notes

- No-Intro focuses on **unmodified, verified dumps** only. Hacks, translations, overdumps,
  and bad dumps are excluded.
- Redump focuses on **bit-perfect disc copies** using raw 2352-byte sectors for CDs.
- DAT files are updated frequently. Re-download periodically for accuracy.
- Each DAT covers one system. You will need multiple DATs for multi-system support.
- Both databases use unique checksums — each hash appears only once per DAT.
