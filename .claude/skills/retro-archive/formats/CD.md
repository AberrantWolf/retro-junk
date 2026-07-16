# Compact Disc — Physical & Logical Format

## Scope & how to use this doc

This is the **foundational reference** for how CDs (and CD-based game discs) are structured: the
physical medium, the sector, the subchannel, addressing, scrambling, and offsets. It is the canonical
home for the shared basics that the per-format docs used to each re-explain. When a topic has a
dedicated doc, this file states the essentials and links out rather than duplicating depth:

- Raw-dump master format → [Redumper.md](Redumper.md)
- Verification standard & DAT → [Redump.md](Redump.md)
- Cue sheets / cue quirks → [CUE.md](CUE.md), [CUECompat.md](CUECompat.md)
- GD-ROM index files → [GDI.md](GDI.md)
- Compressed containers → [CHD.md](CHD.md), [RVZ.md](RVZ.md)
- A specific bad-dump quirk → [ZeroPaddedAudioTracks.md](ZeroPaddedAudioTracks.md)

> **Sourcing:** Facts already established elsewhere in this skill tree are cited as `See X.md`, and those
> docs carry the original external citations (redumper README, ECMA-130, MAME CHD spec, PSX-SPX,
> cdrdao(1), etc.). Sections that fill gaps the repo hadn't previously documented — the colored books,
> EFM/CIRC, capacity/timing math, MSF↔LBA conversion, the 150-sector pregap, and EDC/ECC internals —
> are drawn from the governing standards (ECMA-130 / ISO 10149 and the Philips/Sony "Rainbow Books") and
> are standard, high-confidence CD facts; they are marked **[std]** where added here rather than
> harvested from a repo doc. Anything genuinely uncertain is marked inline.

## The colored "books" (standards) **[std]**

CD standards were published by Philips/Sony as color-named books. Knowing which book a disc obeys tells
you its sector rules:

| Book | Standard | Defines | Sector model |
|------|----------|---------|--------------|
| **Red** | IEC 60908 | CD-DA (audio) | 2352 raw PCM, no sync/header |
| **Yellow** | ECMA-130 / ISO 10149 | CD-ROM | Mode 1 (2048) & Mode 2 (2336) |
| **Yellow (XA ext.)** | CD-ROM XA | Mode 2 Form 1 (2048) / Form 2 (2324) | interleaved data+A/V |
| **Green** | — | CD-i | Mode 2 based |
| **Orange** | — | Recordable (CD-R/RW), **multisession** | adds writable sessions |
| **White** | — | Video CD | Mode 2 Form 2 MPEG |
| **Blue** | — | Enhanced CD / CD-Extra | stamped multisession (audio + data session) |

Game discs are almost always **Yellow Book + CD-ROM XA** (PS1, Saturn) with **Red Book** audio tracks in
mixed mode. **ECMA-130** is the authoritative free spec for the physical channel, scrambling, and
error-correction layers. (See [Redumper.md](Redumper.md) § Scrambling for the repo's ECMA-130 use.)

## Physical layer (brief) **[std]**

Data is a single spiral of **pits and lands** read at constant linear velocity. Two coding layers sit
*below* the 2352-byte sector and are usually invisible to dumps:

- **EFM (Eight-to-Fourteen Modulation):** each byte becomes a 14-bit channel word (+3 merge bits) chosen
  to bound run lengths (3–11) so the reader can recover its clock and stay DC-balanced. This is *why* the
  sector data is scrambled before mastering — long constant runs would violate EFM's balance (see
  Scrambling below).
- **CIRC (Cross-Interleaved Reed–Solomon Coding):** the physical-layer error correction, distinct from
  the per-sector EDC/ECC. Drives apply it in hardware; it's not present in a 2352-byte sector dump.

Redumper deliberately reads *above* these layers (raw, hardware ECC disabled) to capture the true
mastered bytes — see [Redumper.md](Redumper.md).

## Logical hierarchy

```
Disc
 └─ Session(s)                     multisession = Orange/Blue Book (e.g. CD-Extra)
     ├─ Lead-in   (holds the TOC)
     ├─ Program area
     │   └─ Track(s)               data or audio; ≤ 99 per disc
     │       └─ Index(es)          INDEX 00 = pregap, INDEX 01 = track start, 02+ = subdivisions
     └─ Lead-out
```

- **Track** — the top-level content unit; a data track or a Red Book audio track. Redump preserves the
  full track structure including pregaps and mixed-mode boundaries (see [Redump.md](Redump.md),
  [CUE.md](CUE.md)).
- **Index** — `INDEX 01` marks a track's start; `INDEX 00` marks pregap within/before a track. In-file
  `INDEX 00` pregap data belongs to the *following* track as Redump distributes it (see
  [CHD.md](CHD.md) § round-trip, [CUE.md](CUE.md)).
- **Lead-in / lead-out** — bracket the program area; the lead-in carries the TOC. Redumper captures a
  superset extending into both, and a positive combined offset can push real bytes into lead-out (why
  captured lead-out / the `.asus` cache matter — see [Redumper.md](Redumper.md) § Offsets).
- **Session** — appears here mainly via FULL_TOC multisession capability and the raw dump's lead-in
  prepend; most game discs are single-session.

### Addressing: MSF and LBA **[std]**

Two coordinate systems, **75 frames per second**:

- **MSF** (Minutes:Seconds:Frames) — the physical time code stored in each sector header (bytes 12–14)
  and used in cue `INDEX`/pregap times.
- **LBA** (Logical Block Address) — the linear sector index used by GDI track starts, CHD, and tooling.

Conversion:

```
LBA = (M × 60 + S) × 75 + F − 150
```

The **−150** is the standard **2-second (150-frame) pregap** at the start of track 1: LBA 0 corresponds
to **MSF 00:02:00**. This is why cue sheets show `INDEX 01 00:02:00`, and why CHD track metadata shows
`PREFRAMES:150` for a leading pregap. Redumper prepends **45150 sectors** to `.scram` (10 min, the max
negative-MSF range) so first-session lead-in is addressable (see [Redumper.md](Redumper.md)).

## Sector anatomy — the canonical table

Every CD sector is **2352 raw bytes** on the disc. What's inside depends on the track type/mode. This is
the shared table the per-format docs reference (PSX.md/Saturn.md/CHD.md/ZeroPaddedAudioTracks.md keep
only their console-specific offset constant and point here).

**Sync** (data sectors only): the 12-byte pattern `00 FF FF FF FF FF FF FF FF FF FF 00`
(`00 FF×10 00`). Audio sectors have **no sync/header** — they are raw PCM.

**Header** (data sectors, offset 12, 4 bytes): MSF (3 bytes) + **Mode** byte (offset 15).

| Track / mode | User bytes | Full 2352-byte layout (offsets) | User-data offset |
|---|---:|---|---:|
| **Audio (Red Book)** | 2352 | 2352 PCM, 16-bit LE stereo @ 44.1 kHz — no sync/header | 0 |
| **Mode 0** | 0 | 12 sync + 4 header + 2336 **zero** | — |
| **Mode 1** | 2048 | 12 sync + 4 header + **2048 data** + 4 EDC + 8 reserved(0) + 276 ECC | 16 |
| **Mode 2 (bare)** | 2336 | 12 sync + 4 header + **2336 data** (no EDC/ECC split) | 16 |
| **Mode 2 Form 1** (XA) | 2048 | 12 sync + 4 header + 8 subheader + **2048 data** + 4 EDC + 276 ECC | 24 |
| **Mode 2 Form 2** (XA) | 2324 | 12 sync + 4 header + 8 subheader + **2324 data** + 4 EDC(opt) | 24 |

Notes reconciling the prior docs:
- **Mode 1 trailer = 288 bytes** = 4 EDC + 8 reserved(zero) + 276 ECC. **Mode 2 Form 1 trailer = 280
  bytes** = 4 EDC + 276 ECC — the 8-byte subheader occupies what would be Mode 1's reserved bytes, so
  Form 1 has *no* separate reserved field. (This reconciles CHD.md's lumped "280 ECC/EDC" with PSX.md's
  "4 EDC + 276 ECC".)
- **ECC** is two Reed–Solomon parity layers, **P (172 bytes) + Q (104 bytes) = 276**. **EDC** is a
  32-bit CRC over the sector. `edccchk` validates these on *descrambled* sectors (see
  [Redumper.md](Redumper.md)). **[std]**
- **Mode 0** (all-zero body) exists in the spec but is rarely seen in game dumps; listed for
  completeness. **[std]**
- **CD-ROM XA** = Mode 2 with an 8-byte **subheader** (File, Channel, Submode, Coding — stored twice)
  enabling interleaved data (Form 1) and streaming A/V (Form 2). PS1 and Saturn are XA. See
  [PSX.md](PSX.md), [Saturn.md](Saturn.md).

**Container sector sizes derived from the above:**

| Size | Meaning |
|---:|---|
| 2048 | "cooked" user data (Mode 1 / Mode 2 Form 1) — the ISO sector |
| 2324 | Mode 2 Form 2 streaming payload |
| 2336 | bare Mode 2 body |
| 2352 | full raw sector (all modes on disc; audio) |
| **2448** | 2352 raw **+ 96 subchannel** — how [CHD.md](CHD.md) stores CD media |

## Subchannel (P–W)

Interleaved with the main channel are **96 bytes of subchannel per sector**, carrying eight bit-planes
named **P, Q, R, S, T, U, V, W**:

- **P** — a simple flag marking track vs. pause/lead-in regions. **[std]**
- **Q** — timing/addressing and the TOC: track number, index, and running MSF. Redumper detects the disc
  **write offset** from the difference between the data-sector MSF and the subchannel-**Q** MSF, and
  corrects Q in memory at split time (never rewriting it). See [Redumper.md](Redumper.md) § Offsets.
- **R–W** — six planes usable as packs for **CD+G** (graphics), **CD+MIDI**, and **CD-TEXT**. **[std for
  CD-TEXT]** (the repo documents CD+G/CD+MIDI via redumper; CD-TEXT is added here.)

The channels are **bit-interleaved** on disc. Redumper's `.subcode` stores them **raw / not
demultiplexed** (96 B/sector), deliberately *incompatible* with DiscImageCreator's deinterleaved `.sub`,
so that **subchannel-based copy protection** (libcrypt, SecuROM) and R–W packs survive for later
analysis. See [Redumper.md](Redumper.md) § File inventory. CHD records subchannel via `SUBTYPE`
`NONE`/`RW`/`RW_RAW` (see [CHD.md](CHD.md)).

## TOC and disc description

The **TOC** (Table of Contents), stored in the lead-in and readable from subchannel Q, lists track
starts, types, and the lead-out position. Redumper stores it two ways: **`.toc`** (RAW TOC, legacy) and
**`.fulltoc`** (RAW FULL_TOC, **multisession-capable** — the authoritative one used to build the cue).
Related lead-in structures — **ATIP** (recordable media info), **PMA** (program memory area, for
incomplete recordings), and **CD-TEXT** — exist but are rarely relevant to stamped game discs. **[std
for ATIP/PMA]**

A separate, editor-facing description is the **cdrdao/CDRWin "TOC file"** — a cue-like text descriptor
with a disc-type header (`CD_DA` / `CD_ROM` / `CD_ROM_XA`); see [CUE.md](CUE.md) § TOC format and
[CUECompat.md](CUECompat.md). Don't confuse the on-disc TOC with a `.toc` text file.

## Scrambling (why raw sector bytes ≠ ISO bytes)

CD-ROM data sectors are **XOR-scrambled** during mastering (everything *except* the 12-byte sync) with a
15-bit LFSR, polynomial **x¹⁵ + x¹ + 1**, reset per sector — so the EFM stream stays DC-balanced.
Descrambling is deterministic and losslessly reversible. Redumper stores sectors **scrambled** (raw read,
hardware descramble/ECC off) as the most faithful capture; the split step descrambles to produce the
final bin. Full detail, rationale, and the `.scram`/`.scrap` distinction live in
[Redumper.md](Redumper.md) § Scrambling & offsets.

## Offsets (why hashes shift)

Three quantities, covered in depth in [Redumper.md](Redumper.md) § Offsets:

- **Drive read offset** — per-drive sample skew; raw dumps are already corrected for it.
- **Disc write offset** — per-pressing mastering shift.
- **Combined offset = read + write** — the raw dump is *not* combined-offset corrected; the write offset
  is detected at split (from data-MSF vs Q-MSF). Redump `.bin`s are stored at **combined-offset-corrected**
  alignment, so descrambling without applying it shifts every sample and per-track hashes won't match the
  DAT. Audio spillover from a non-zero offset is trimmed at split.

## Track types & special layouts

- **Data vs audio tracks** — a track's cue mode string containing "MODE" is data; `AUDIO` is a Red Book
  track (2352 PCM). See [CUE.md](CUE.md) § Identifying data tracks.
- **Pregaps/gaps** — in-file `INDEX 00` pregap (stored in the track file) vs `PREGAP`/`POSTGAP`
  directive gaps (generated, *not* stored). CHD encodes these as `PREFRAMES`. CDRWin uses
  `SILENCE`/`START`/`ZERO`. See [CUE.md](CUE.md), [CHD.md](CHD.md).
- **Mixed-mode** — a data track plus Red Book audio tracks on one disc (typical of PS1/Saturn). Redump
  preserves the full mixed structure.
- **Multi-track data / single-bin vs split** — Redump uses one bin per track (`Game (Track N).bin`);
  single-bin multi-track images split by INDEX boundaries. See [CUE.md](CUE.md), [CHD.md](CHD.md).
- **GD-ROM high-density area** — Dreamcast's proprietary 1.2 GB CD has a low-density CD-compatible area
  plus a **high-density area** whose first track (track 3) starts conventionally at **LBA 45000**. See
  [GDI.md](GDI.md), [Redump.md](Redump.md) § GDI.
- **Bad-dump quirk** — some PS1 rips write audio tracks as *zero-padded Mode 2 sectors* (which keep the
  sync pattern) instead of raw PCM, breaking naive boundary detection. See
  [ZeroPaddedAudioTracks.md](ZeroPaddedAudioTracks.md).

## Capacity & timing math **[std]**

At 75 sectors/second:

| Disc length | Sectors | @2048 (data) | @2352 (audio/raw) |
|---|---:|---:|---:|
| 74 min | 333,000 | ~650 MiB (682 MB) | ~747 MB |
| 80 min | 360,000 | ~703 MiB (737 MB) | ~806 MB |

So "650 MB" (PS1) and "~700 MB" (PS2 CD) refer to the same medium at different fill/mode — not a
contradiction. DVD/UMD/GD-ROM capacities are governed by their own specs (see per-console docs).

## Image / container representations at a glance

| Container | Sector unit | Stores | Drops | Doc |
|---|---|---|---|---|
| **cue + bin** | 2352 raw | full tracks + cue layout | subchannel, lead-in/out | [CUE.md](CUE.md) |
| **GDI + tracks** | 2048/2352 | GD-ROM track set | — | [GDI.md](GDI.md) |
| **cdrdao/CDRWin .toc** | 2352 | cue-like descriptor | (descriptor only) | [CUE.md](CUE.md) |
| **redumper raw folder** | 2352 scrambled + 96 sub | **superset** (lead-in/out, subchannel, state) | nothing | [Redumper.md](Redumper.md) |
| **CHD** | **2448** (2352+96) | lossless compressed, subchannel | — | [CHD.md](CHD.md) |
| **ISO** | 2048 | cooked user data only | audio, subchannel, raw layers | [Redump.md](Redump.md) |
| others | — | `.cdi`, `.mdf/.mds`, `.cso/.zso`, `.pbp`, `.ecm`, `.ird` | varies | per-console docs |

## Hashing & verification

Redump records **CRC32 + MD5 + SHA1 per track** (plus a cue checksum); there is no whole-disc hash. Data
tracks are hashed over the **full raw 2352-byte sector** (sync + header + ECC/EDC), so **Redump
checksums are not compatible with 2048-byte ISO files**. Compressed containers must **decompress to the
raw representation before hashing** (project rule): CHD verification decompresses hunks, strips the
96-byte subchannel, and hashes the 2352-byte sectors of the data track. See [Redump.md](Redump.md)
§ Checksums and [CHD.md](CHD.md) § Hashing. Redumper's `.log` emits the same per-track hashes as
clrmamepro `<rom .../>` lines (parseable by `retro-junk-dat`) — see [Redumper.md](Redumper.md).

## Per-console CD specifics

| System | Medium | CD notes | Docs |
|---|---|---|---|
| **PS1** | CD-ROM XA (mixed mode) | reserved sectors 0–17 (license/logo/PVD); `SYSTEM.CNF BOOT=` serial; user data @24 | [PSX.md](PSX.md), consoles/PSX_Overview |
| **PS2** | CD-ROM **and** DVD-5/9 | `BOOT2=`; DVD path uses 2048-B units | [PS2.md](PS2.md), consoles/PS2_Overview |
| **Saturn** | CD-ROM XA | Mode 1 data @0x10; IP.BIN boot header (big-endian) at track 1 start | [Saturn.md](Saturn.md), consoles/Saturn_Overview |
| **Dreamcast** | GD-ROM | low+high density; HD area @LBA 45000; GDI or bin/cue | [GDI.md](GDI.md), consoles/Dreamcast_Overview |
| **PSP** | UMD (DVD-class) | 2048-B units; verified via scene DBs, not Redump | consoles/PSP_Overview, [CHD.md](CHD.md) |
| **PS3** | Blu-ray (encrypted) | `.ird` reconstructs/verifies decrypted image | [Redump.md](Redump.md) § IRD |

GameCube/Wii use mini-DVD (not CD) via `nod`/RVZ — see [RVZ.md](RVZ.md); they share the
"decompress-to-standard-before-hashing" rule.

## See also
- [Redumper.md](Redumper.md) — raw dump master (scrambling, offsets, subchannel, split)
- [Redump.md](Redump.md) — verification standard, DAT format, per-track hashing
- [CUE.md](CUE.md) / [CUECompat.md](CUECompat.md) — cue sheets and their quirks
- [GDI.md](GDI.md) — GD-ROM index files
- [CHD.md](CHD.md) / [RVZ.md](RVZ.md) — compressed containers
- [ZeroPaddedAudioTracks.md](ZeroPaddedAudioTracks.md) — a raw-audio bad-dump quirk

## Sources
- **ECMA-130** (2nd ed.) — CD-ROM physical layer, sector modes, EDC/ECC, CIRC, EFM, scrambling LFSR:
  https://ecma-international.org/publications-and-standards/standards/ecma-130/
- Philips/Sony **Rainbow Books** (Red/Yellow/Green/Orange/White/Blue) — the color-book standard set (industry references; not free) **[std]**
- **ISO 9660** filesystem (PVD at sector 16, "CD001") — see [PSX.md](PSX.md) § Sources
- **MAME CHD** format spec — see [CHD.md](CHD.md) § Sources
- **PSX-SPX** CD-ROM format — see [PSX.md](PSX.md) § Sources
- redumper README + **redumper source** — see [Redumper.md](Redumper.md) § Sources
- cdrdao(1) / CUETools — see [CUE.md](CUE.md) § Sources
- All per-fact repo provenance consolidated from the CD-knowledge harvest, 2026-07-16.
