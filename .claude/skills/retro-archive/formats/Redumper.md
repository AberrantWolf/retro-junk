# Redumper Raw Dump Format

## Overview

[redumper](https://github.com/superg/redumper) (© Hennadiy Brych, 2021–2025) is a low-level
CD/DVD/Blu-ray dumper and, alongside DiscImageCreator, one of the two dumping tools blessed by the
[Redump](Redump.md) preservation project. Unlike a finished `.cue`/`.bin` set, a redumper *raw dump
folder* preserves a **superset** of the disc: raw (still-scrambled) main-channel data with captured
lead-in/lead-out, raw subchannel, an error-state map, and the full TOC. From that superset the
final Redump-compatible `.bin`/`.cue` (see [CUE.md](CUE.md)) can be **regenerated deterministically**.

This makes a redumper folder an ideal *pristine archival master*: keep the raw folder, and derive a
playable `.bin`/`.cue` — or a compressed [CHD](CHD.md)/[RVZ](RVZ.md) — on demand.

> **Sourcing:** File semantics below are from the redumper README
> (`https://github.com/superg/redumper`, `README.md`) and the Redump Wiki
> "[Redumper](https://wiki.redump.info/index.php?title=Redumper)" and
> "[Dumping Guide (redumper CLI)](https://wiki.redump.info/index.php?title=Dumping_Guide_%28redumper_CLI%29)"
> pages. Offset/scrambling detail is cross-referenced against ECMA-130 and the Redump forum threads
> cited inline. Researched 2026-07-16, then **verified against the redumper source tree** (`main`,
> 2026-07): file set (`cd_dump.ixx`, `dvd_dump.ixx`, `skeleton.ixx`), `.state` width (`common.ixx`),
> the `disc` pipeline order (`redumper.ixx:262`), DVD ISO finalization (`dvd_split.ixx`), and the
> `dat:`/`<rom>` log format (`rom_entry.ixx`). Any remaining "not verified" points are marked inline.

## File inventory (CD dump)

redumper writes a set of same-basename files. For a CD dump the notable ones are:

| File | Contents |
|------|----------|
| **`.scram`** | Data channel RAW (**scrambled**) main-channel dump, **drive-read-offset corrected**. Contiguous raw 2352-byte sectors *with* extended lead-in/lead-out. Prepends **45150 sectors** (10 min, the max negative MSF range) so first-session lead-in is addressable. |
| **`.scrap`** | **Legacy — removed from redumper in commit `76d1cfd` (2025-06-06); only present in older dumps.** Was the variant for data tracks dumped via the **BE read command**, stored **unscrambled** at drive read offset, mutually exclusive with `.scram`. Modern redumper unifies on `.scram`. Ingest should still *recognize* it in old folders. |
| **`.subcode`** | Subchannel dump in **RAW, not demultiplexed** form (96 bytes/sector, P–W bit-interleaved). Deliberately **incompatible with DiscImageCreator's `.sub`**. Kept raw so subchannel protections (libcrypt, SecuROM) and R–W packs (CD+G/CD+MIDI) survive. Q is corrected *in memory* at split time, never rewritten. |
| **`.state`** | redumper error-state map. **1 byte per entry** (`enum class State : uint8_t`, `common.ixx:31`). **CD:** one entry per 4-byte stereo sample → **588 bytes/sector** (includes the 45150-sector negative-LBA prefix, read-offset indexed). **DVD:** one entry per **LBA → 1 byte/sector** (`dvd_dump.ixx`) — CD and DVD `.state` differ. States: `SUCCESS`, `SUCCESS_SCSI_OFF`, `SUCCESS_C2_OFF`, `ERROR_C2`, `ERROR_SKIP`. Drives incremental `refine` re-reads. |
| **`.cache`** | Drive cache / lead-in dump written by the `dump::extra` step on cache-exploitable drives (see `.asus`). |
| **`.toc`** | Table of Contents, RAW TOC format (legacy). |
| **`.fulltoc`** | Table of Contents, RAW FULL_TOC format (multisession-capable). Authoritative TOC used to build the CUE. |
| **`.cue`** | redump.info-compatible CUE sheet (generated at split). |
| **`.bin`** | redump.info-compatible split track binaries (descrambled, offset-corrected). Present only *after* the split step — this is the derived Redump representation, not raw master data. |
| **`.log`** | Validation log: drive info, read/combined offsets, error counts, and per-track CRC32/MD5/SHA1 in DAT-like form. The verification oracle for a submission. |
| **`.asus` / `.cache`** | LG/ASUS/LITE-ON full cache dump, used to recover extra lead-out sectors (needed for positive-combined-offset discs). Only on cache-exploitable drives. |
| **`.sdram`** | RAW **DVD** dump, drive-read-offset corrected. **Only written in raw mode (`--dvd-raw`, omnidrive firmware).** Default DVD dumping writes `.iso` directly (see DVD section). |
| **`.sbram`** | RAW **Blu-ray** dump, drive-read-offset corrected. Raw mode only (`--bd-raw`). |

DVD dumps additionally emit `.physical`, `.manufacturer`, and conditionally `.bca` / `.security`
(`dvd_dump.ixx`).

**Confirmed against source (redumper `main`, 2026-07):**
- **`.hash`** is **real but `skeleton`-command-only** — written by `redumper skeleton --skeleton` (`skeleton.ixx`), a text file of per-ISO9660 SHA-1 lines paired with a `.skeleton`. **Not** produced by a default `disc` run.
- **`.absolute`, `.index`** are **spurious** — they appear nowhere in redumper source. Do not detect them.

### Approximate sizes (CD)
- `.scram` ≈ `(45150 + sector_count) × 2352` bytes
- `.subcode` ≈ `sector_count × 96` bytes
- `.state` ≈ `sector_count × 588` entries
- final `.bin` (per track) = `track_sector_count × 2352` bytes (no lead-in prepend)

## Recognizing a redumper folder

Presence of `.scram`/`.scrap` (CD) or `.sdram`/`.sbram` (DVD/BD), plus a redumper `.log` and
`.fulltoc`. The `.subcode` + `.state` pair is a strong CD-redumper signature, distinct from
DiscImageCreator's `.scm`/`.img`/`.sub`. A DVD/BD folder has **no** `.subcode`/`.state`/`.scrap` —
detect media type from which raw file is present.

## Raw → Redump BIN/CUE (the `split` step)

redumper regenerates the Redump representation itself — you do not hand-roll it. Running `redumper`
with no command executes the aggregate **`disc`** pipeline, whose verbatim step list (confirmed in
`redumper.ixx:262`) is:

```
dump  dump::extra  protection  refine  dvdkey  split  hash  info
```

Note `protection` runs **before** `refine`, there is **no separate "verify" step**, and `dump::extra`
(cache / lead-in recovery) is its own step. `split` and `hash` are **drive-free and re-runnable** over
an existing folder — both **require `--image-name`** (`--image-path` defaults to the cwd). CD `split`
needs `.scram` + `.state` + `.toc` present and won't overwrite existing tracks without `--overwrite`.

`split` does:
1. Reads `.fulltoc` + `.subcode` for track boundaries, pregaps, INDEX points (TOC-based and
   subchannel-Q-based splits both supported; Q corrected in memory).
2. **Descrambles** every data-track sector (see scrambling below). `.scrap` is already unscrambled.
3. **Detects disc write offset** and applies **combined-offset** correction (e.g. via the addressing
   difference between data-sector MSF and subchannel-Q MSF), plus intelligent audio-offset detection to
   remove non-zero spillover.
4. Writes one descrambled `.bin` per track + a redump-compatible `.cue`, and computes per-track
   CRC32/MD5/SHA1 (emitted to `.log`).

`split` **fails** if a track's sector range contains unrecovered C2/SCSI errors (per `.state`), or if
the combined offset is positive and the needed lead-out was not captured — so a raw folder is not
*guaranteed* to yield a Redump-matching BIN.

## Scrambling & offsets (why raw hashes ≠ Redump)

**CD scrambling (ECMA-130 / Yellow Book):** every data-sector byte *except* the 12-byte sync
(`00 FF×10 00`) is XOR-scrambled during mastering with a 15-bit LFSR (x¹⁵+x¹+1, reset per sector) to
keep the EFM stream DC-balanced. Drives normally descramble in hardware; redumper issues the raw
("D8") read with descrambling/ECC **disabled** to capture true on-disc bytes. Storing scrambled is the
most faithful capture, lets redumper measure read offset and spot sectors the drive would silently
correct, and is losslessly reversible (descramble is deterministic — ~20 lines or a 2340-byte table).

**Three offsets:**
- **Drive read offset** — per-drive sample skew. Raw dumps are *already* corrected for this.
- **Disc write offset** — per-pressing mastering shift.
- **Combined offset = read + write.** The raw dump is **not** combined-offset corrected; the write
  offset is determined later, at split.

Redump `.bin`s are stored at **combined-offset-corrected** alignment. Descrambling without applying the
write offset shifts every sample, so per-track hashes won't match the DAT. A positive combined offset
can push real bytes into lead-out (hence wanting captured lead-out / the `.asus` cache). Offset-*shift*
discs (a mastering defect) are corrected with `--correct-offset-shift` and submitted as "Fixed" dumps.
The `.log` records read/combined offset and the resulting hashes. (Refs: redumper README;
[Combined offset in EAC](http://forum.redump.org/topic/7649/combined-offset-in-eac);
[offset shifting discs](http://forum.redump.org/topic/48206/offset-shifting-discs/).)

## DVD / Blu-ray

Not CD-scrambled and no subchannel — most CD complexity vanishes. DVD/BD dumping uses redumper's
"high-level mode". **By default, redumper writes the `.iso` directly during dump** (2048-byte
sectors — the community-standard Redump DVD representation); for a default DVD dump `split` only
*validates* and throws on residual SCSI errors. **Only raw mode (`--dvd-raw` / `--bd-raw`, omnidrive
firmware) writes `.sdram`/`.sbram`**, and there `split` does the finalization —
`dvd_extract_iso`/`bd_extract_iso` descramble, EDC-validate, zero-fill invalid sectors, and write the
`.iso` (`dvd_split.ixx`). DVD `.state` is 1 byte per LBA (not per-sample). See [Redump.md](Redump.md)
for DVD = ISO.

## Tooling landscape

- **redumper itself** is the best "parser": `redumper split`/`hash` regenerate Redump-compatible
  BIN/CUE + hashes from a raw folder. Robust ingest = shell out to it. **No JSON/machine-readable
  output exists** — the only structured result is the `dat:` block in stdout/`.log`, which is
  clrmamepro `<rom name= size= crc= md5= sha1= />` lines (`rom_entry.ixx`) — **the exact format
  `retro-junk-dat` already parses**, so route the log's rom lines through the existing DAT parser
  rather than writing a bespoke scraper. (The ecosystem tool MPF parses the `.log` the same way.)
- **No mature Rust crate** reads `.scram`/`.subcode` or descrambles CD sectors. Generic LFSR crates
  (`lfsr`) exist but you'd still implement the ECMA-130 scrambler. `chd-rs` (already used here) and
  `cdtoc` don't touch redumper raw. Native reimplementation = effectively porting redumper's split.
- **[miniscram](https://github.com/hughobrien/miniscram)** (Go, GPL-3.0) — delta-compresses a `.scram`:
  re-scrambles the split `.bin` via ECMA-130 and stores only the delta vs the raw, round-trip-verified
  before deleting the source (claims ~530×–2700×). **CD-only; does not cover `.subcode`/`.state`**;
  refuses variable write offsets. A candidate future "compress archive" action, but it shrinks only
  `.scram`, not the whole folder.
- **edccchk** (C, GPL) validates EDC/ECC on descrambled 2352-byte sectors. **verifydump** (Go, MIT)
  and the repo's existing DAT/matcher path model the verify-against-Redump step. **chdman** already
  wrapped here for BIN/CUE ↔ CHD.

## Archival guidance

**Keep as pristine master** (lossless, regenerable): `.scram` (or a legacy `.scrap`), `.subcode`,
`.fulltoc` (+`.toc`), `.log`, `.state`, and `.asus`/`.cache` if present. **Do not** treat a derived
`.bin` as able to reconstruct raw — raw→BIN is one-way lossy.

**Determinism caveat:** redumper ships rolling builds (no semver, no changelog) and split/offset
behavior has changed repeatedly (e.g. build b728 "DVD ecc → null bytes"; multiple offset reworks;
`.scrap` removal). So re-splitting is **not guaranteed byte-identical across versions.** Frame the
archival promise as *"regenerates **a** DAT-matching bin,"* not *"these exact bytes."* Record the
redumper build (printed atop every `.log`), and treat **Redump-DB hash matching** — not byte-identical
re-splits — as the source of truth. A CHD cached from an older split stays valid because it matched the
DAT when made; identity is the normalized hash, not the split run.

**Recommended ingest flow:** store raw folder as master → `redumper split`+`hash` (or native reader)
→ compare per-track hashes to the Redump DAT via the existing matcher → mark verified → lazily generate
`.bin`/`.cue` or CHD for playback, treating the derived files as a *cache*, not master.

## See also
- [Redump.md](Redump.md) — the preservation project, DAT format, BIN/CUE conventions
- [CUE.md](CUE.md) / [GDI.md](GDI.md) — cue sheet / GD-ROM index formats
- [CHD.md](CHD.md) / [RVZ.md](RVZ.md) — compressed playable containers
- [ZeroPaddedAudioTracks.md](ZeroPaddedAudioTracks.md) — audio-track edge cases in splitting
