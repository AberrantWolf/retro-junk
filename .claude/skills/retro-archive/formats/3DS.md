# Nintendo 3DS ROM Formats
## File Extensions
- `.3ds` / `.cci` — Game card dumps (NCSD/CCI format)
- `.cia` — Custom Install Archive (eShop / installable format)

## Overview

3DS uses a layered container architecture:

```
CCI (.3ds/.cci):  NCSD header → up to 8 NCCH partitions
CIA (.cia):       CIA header → Cert Chain → Ticket → TMD → NCCH content(s) → Meta
```

Both formats ultimately contain **NCCH** partitions, which hold the actual game content (executable code, filesystem, icons). The NCCH is the common inner format.

---

## NCSD Format (CCI / .3ds / .cci) — Game Card Dumps

NCSD (NCCH Card System Data) is the format for game card images. Contains an NCSD header followed by up to 8 NCCH partitions. Partition 0 (the main game CXI) typically starts at offset `0x4000`.

All multi-byte integers are **little-endian**.

### NCSD Header (0x000–0x1FF)

| Offset | Size | Field |
|--------|------|-------|
| 0x000 | 0x100 | RSA-2048 SHA-256 signature of bytes 0x100–0x1FF |
| 0x100 | 4 | **Magic: `"NCSD"` (0x4E435344)** |
| 0x104 | 4 | Image size in media units (1 MU = 0x200 bytes) |
| 0x108 | 8 | Media ID |
| 0x110 | 8 | Partition FS types (1 byte per partition) |
| 0x118 | 8 | Partition crypt types (1 byte per partition) |
| 0x120 | 0x40 | Partition table: 8 entries × 8 bytes (u32 offset + u32 size, both in MU) |
| 0x160 | 0x20 | ExHeader SHA-256 hash |
| 0x180 | 4 | Additional header size |
| 0x184 | 4 | Sector zero offset |
| 0x188 | 8 | Partition flags (see below) |
| 0x190 | 0x40 | Partition ID table (8 × 8 bytes) |
| 0x1FE | 1 | Support flag (anti-savegame-restore, FW 9.6+) |
| 0x1FF | 1 | Save crypto flag (FW 9.6+) |

### Partition Flags (0x188, 8 bytes)

| Index | Field | Values |
|-------|-------|--------|
| 0 | Backup write wait time | 0–255 seconds |
| 3 | Media card device | 1=NOR, 2=None, 3=BT |
| 4 | Media platform index | 1=CTR (Old 3DS), 2=Snake (New 3DS) |
| **5** | **Media type index** | **0=Inner Device, 1=Card1, 2=Card2, 3=Extended Device** |
| 6 | Media unit size | Exponent: actual = 0x200 << flags[6] |
| 7 | Media card device (old SDK) | SDK 2.X value |

### Card Info Header (0x200–0x3FF)

| Offset | Size | Field |
|--------|------|-------|
| 0x200 | 4 | Writable address (Card2: save offset in MU; Card1: 0xFFFFFFFF) |
| 0x204 | 4 | Card info bitmask |
| 0x300 | 4 | Filled size |
| 0x310 | 2 | Title version |
| 0x312 | 2 | Card revision |
| 0x320 | 8 | CVer title ID |
| 0x328 | 2 | CVer version |

### Initial Data (0x1000–0x11FF)

| Offset | Size | Field |
|--------|------|-------|
| 0x1000 | 0x10 | Card seed KeyY |
| 0x1010 | 0x10 | Encrypted title key |
| 0x1020 | 0x10 | AES-CCM MAC |
| 0x1030 | 0x0C | AES-CCM nonce |
| 0x1100 | 0x100 | Copy of NCCH header bytes 0x100–0x1FF |

### Standard Partition Layout

| Partition | Content |
|-----------|---------|
| 0 | CXI — Main executable (game) |
| 1 | CFA — Electronic manual |
| 2 | CFA — Download Play child (optional) |
| 6 | CFA — New3DS system update (optional) |
| 7 | CFA — Old3DS system update (optional) |

### Detection

1. File size ≥ 0x4200 (NCSD header + minimal NCCH partition 0 header)
2. Read 4 bytes at offset 0x100: must be `"NCSD"` (4E 43 53 44)
3. Validate image_size × 0x200 is reasonable
4. Verify partition 0 offset is typically 0x20 (= 0x4000 / 0x200)

---

## NCCH Format — Inner Container

NCCH is the actual game content container. Two types exist:
- **CXI** (CTR Executable Image) — has ExeFS (code) + RomFS (data)
- **CFA** (CTR File Archive) — RomFS only (manuals, DLC)

### NCCH Header (0x000–0x1FF, relative to partition start)

| Offset | Size | Field |
|--------|------|-------|
| 0x000 | 0x100 | RSA-2048 SHA-256 signature |
| 0x100 | 4 | **Magic: `"NCCH"` (0x4E434348)** |
| 0x104 | 4 | Content size in media units |
| 0x108 | 8 | Partition ID |
| 0x110 | 2 | **Maker code** (2-char ASCII) |
| 0x112 | 2 | NCCH version |
| 0x114 | 4 | Content lock seed hash (FW 9.6+) |
| 0x118 | 8 | **Program ID** (Title ID) |
| 0x130 | 0x20 | Logo region SHA-256 |
| 0x150 | 0x10 | **Product code** (ASCII, e.g. "CTR-P-ABCD", zero-padded) |
| 0x160 | 0x20 | **ExHeader SHA-256** (first 0x400 bytes of ExHeader) |
| 0x180 | 4 | ExHeader size (typically 0x400) |
| 0x188 | 8 | Flags (see below) |
| 0x190 | 4 | Plain region offset (MU) |
| 0x194 | 4 | Plain region size (MU) |
| 0x198 | 4 | Logo region offset (MU) |
| 0x19C | 4 | Logo region size (MU) |
| 0x1A0 | 4 | **ExeFS offset** (MU) |
| 0x1A4 | 4 | **ExeFS size** (MU) |
| 0x1A8 | 4 | ExeFS hash region size (MU) |
| 0x1B0 | 4 | **RomFS offset** (MU) |
| 0x1B4 | 4 | **RomFS size** (MU) |
| 0x1B8 | 4 | RomFS hash region size (MU) |
| 0x1C0 | 0x20 | **ExeFS superblock SHA-256** |
| 0x1E0 | 0x20 | **RomFS superblock SHA-256** |

### NCCH Flags (offset 0x188, 8 bytes)

| Index | Field | Values |
|-------|-------|--------|
| 3 | Crypto method | 0x00=Original, 0x01=7.0+, 0x0A=9.3+ N3DS, 0x0B=9.6+ N3DS |
| 4 | Content platform | 1=CTR (Old 3DS), 2=Snake (New 3DS) |
| 5 | Content type | bits 0-1: form type; bits 2-7: content category |
| 6 | Content unit size | Exponent: unit = 0x200 × 2^flags[6] |
| 7 | Flags | bit 0: fixed crypto key; bit 2: **NoCrypto** (unencrypted); bit 5: seed crypto |

### NCCH ExHeader (at partition + 0x200)

| Offset | Size | Section |
|--------|------|---------|
| 0x000 | 0x200 | System Control Info (SCI) — includes application title at 0x000 (8 bytes ASCII) |
| 0x200 | 0x200 | Access Control Info (ACI) |
| 0x400 | 0x400 | AccessDesc + NCCH HDR public key |

### Product Code Format

The product code at NCCH offset 0x150 follows the pattern: `CTR-X-YYYY`
- `CTR` = 3DS system prefix
- `X` = content type (`P`=product, `N`=eShop demo, `U`=patch, etc.)
- `YYYY` = 4-character game ID (last char often indicates region)

---

## CIA Format (Custom Install Archive) — eShop / Installable

CIA is the installable format for the eShop and CFW tools. All sections aligned to **64-byte boundaries**.

### CIA Header

| Offset | Size | Field |
|--------|------|-------|
| 0x00 | 4 | Header size (usually 0x2020) |
| 0x04 | 2 | Type |
| 0x06 | 2 | Version |
| 0x08 | 4 | Certificate chain size |
| 0x0C | 4 | Ticket size |
| 0x10 | 4 | TMD size |
| 0x14 | 4 | Meta size (0 if no meta) |
| 0x18 | 8 | Content size |
| 0x20 | 0x2000 | Content index bitmask |

### CIA Section Order (each aligned to 64 bytes after header)

1. **Certificate chain**
2. **Ticket** — contains encrypted title key, title ID
3. **TMD** (Title Metadata) — content count, content hashes, title version
4. **Content** — concatenated NCCH partition(s)
5. **Meta** (optional) — dependency list + SMDH icon data at offset 0x400

### Detection

CIA has no magic bytes. Detection uses:
1. Read first 4 bytes as LE u32: usually `0x2020` (bytes: `20 20 00 00`)
2. Validate type/version at 0x04–0x07 are small values (≤ 1)
3. Validate cert/ticket/TMD sizes at 0x08–0x13 are non-zero and reasonable
4. Total of aligned sections should approximate file size

### TMD Content Chunk Records (0x30 bytes each)

| Offset | Size | Field |
|--------|------|-------|
| 0x00 | 4 | Content ID |
| 0x04 | 2 | Content index (0x0000=main, 0x0001=manual, 0x0002=DLP child) |
| 0x06 | 2 | Content type flags |
| 0x08 | 8 | Content size |
| 0x10 | 0x20 | **SHA-256 hash of content** |

---

## Detecting CCI Origin: Game Card vs. Converted from CIA

When a CIA is converted to CCI, several telltale differences remain:

| Check | Game Card Dump | CIA-Converted CCI |
|-------|---------------|-------------------|
| RSA signature (0x000–0x0FF) | Valid (non-zero) | Usually all zeros |
| Card seed KeyY (0x1000, 16 bytes) | Non-zero crypto data | All zeros |
| Media type (0x18D) | 1 (Card1) or 2 (Card2) | Often 0 (Inner Device) |
| Writable address (0x200) | 0xFFFFFFFF (Card1) or valid offset (Card2) | 0x00000000 |
| Partition count | 2–4 (game, manual, updates) | Usually 1–2 (game only, maybe manual) |
| Initial data block (0x1000–0x11FF) | Has encrypted title key, MAC, nonce | Mostly zeros |

### Heuristic Priority

1. Card seed at 0x1000: 16 zero bytes → almost certainly not from a real card
2. Media type at 0x18D: 0 → digital origin
3. RSA signature: all zeros → not an authentic dump
4. Partition count: ≤ 1 non-main partition without update partitions → likely converted

---

## SHA-256 Hashes and Verification

3DS uses **SHA-256** exclusively for integrity.

### Verifiable Without Decryption Keys

The **NCCH header** (bytes 0x100–0x1FF) is always unencrypted, so product code, maker code, program ID, and sizes are always readable.

For NCCHs with the **NoCrypto** flag (flags[7] bit 2 = 1):
- ExHeader hash (NCCH 0x160): verify by hashing 0x400 bytes of ExHeader
- Logo region hash (NCCH 0x130): verify by hashing the logo region
- ExeFS superblock hash (NCCH 0x1C0): verify by hashing ExeFS header region
- RomFS superblock hash (NCCH 0x1E0): verify by hashing RomFS header region

For **encrypted** NCCHs (most retail card dumps): only NCCH header metadata is accessible. Content hashes require decryption first.

### SMDH Icon Data

The ExeFS `icon` file uses **SMDH format** (magic `"SMDH"`, 0x36C0 bytes):
- 16 language title entries at 0x08 (each 0x200 bytes): short title (UTF-16LE, 0x80 bytes) + long title (UTF-16LE, 0x100 bytes) + publisher (UTF-16LE, 0x80 bytes)
- Region lockout bitmask at 0x2018: 0x01=JPN, 0x02=USA, 0x04=EUR, 0x08=AUS, 0x10=CHN, 0x20=KOR, 0x40=TWN
- Language indices: 0=JP, 1=EN, 2=FR, 3=DE, 4=IT, 5=ES, 6=ZH-CN, 7=KO, 8=NL, 9=PT, 10=RU, 11=ZH-TW

---

## Maker Codes

Same 2-character ASCII licensee table as NDS/GBA.

## Sources

- [3dbrew NCCH Format](https://www.3dbrew.org/wiki/NCCH)
- [3dbrew File Formats Category](https://www.3dbrew.org/wiki/Category:File_formats)
- [RetroReversing 3DS File Formats](https://www.retroreversing.com/3DSFileFormats)
- [Alternative 3DS Formats](https://frds.github.io/3DSFileFormats)

