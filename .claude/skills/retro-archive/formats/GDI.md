# GDI (GD-ROM track list)

Used by: [Dreamcast](../consoles/Dreamcast_Overview.md)

## Overview

`.gdi` is a plain-text track descriptor for Dreamcast GD-ROM rips — the
GD-ROM equivalent of a CUE sheet. A GD-ROM disc has two areas: a small
single-density (CD-compatible) lead-in area and a much larger high-density
data area starting at a fixed LBA. The GDI format's job is simply to list
every track (data or audio), its starting LBA, sector size, and which file on
disk holds its raw sector data — it does not encode INDEX points or
pregap/postgap directives the way CUE does.

## File Extension
- `.gdi` — GD-ROM track descriptor (text)

## Grammar (as implemented by `retro-junk-disc::track_layout::parse_gdi`)

```
<track_count>
<number> <lba> <type> <sector_size> <filename> <offset>
<number> <lba> <type> <sector_size> <filename> <offset>
...
```

- **First non-empty line**: the total track count. Parsing fails if the
  number of track lines that follow doesn't match this count.
- **One line per track**, exactly 6 whitespace-separated fields:
  | Field | Meaning |
  |---|---|
  | `number` | Track number, 1-based. |
  | `lba` | Starting LBA (logical block address) on the disc. The high-density area conventionally starts at **LBA 45000** — track 3 (the first high-density track) typically has `lba = 45000`. |
  | `type` | `4` = data track, `0` = audio track. |
  | `sector_size` | Bytes per sector in the track file: `2048` for data tracks (cooked, Mode 1 user data only) or `2352` for audio/raw tracks. |
  | `filename` | The track's data file, resolved relative to the `.gdi`'s directory. May be double-quoted if it contains spaces (e.g. `"my game track01.bin"`); the quotes are stripped during parsing. |
  | `offset` | Byte offset into `filename` where this track's data begins. Normally `0` — one file per track is the common convention, matching Redump-style multi-bin cues. |
- **Whitespace**: fields are split on any run of whitespace; blank lines are
  ignored. Unlike `retro-junk-disc::cue`'s CUE parser, the GDI parser does
  not currently special-case tabs vs. spaces beyond ordinary whitespace
  splitting (`str::trim`/`char::is_whitespace`), so both already work
  uniformly.
- **Quoting**: only double quotes are recognized, and only around the
  filename field (`split_gdi_fields` peels a leading `"` and finds the next
  `"` explicitly — there is no `\"`-escaping support; this has not been
  cross-checked against real dumper output beyond the Redump-style rips
  used in `track_layout_tests.rs`, so treat it as *unverified* for exotic
  filenames).

### Per-track byte spans

`gdi_track_spans` treats each track as the *whole* of its file, minus the
declared `offset`: `byte_offset = offset`, `byte_len = file_size - offset`.
Since GDI conventionally uses one file per track (`offset = 0`), this means
each track's span is simply its entire file — there is no in-file
pregap/postgap concept to reconcile, unlike single-bin CUE sheets.

## Relationship to CHD

`chdman createcd` accepts `.gdi` input directly (alongside `.cue`), the same
way it accepts CUE for CD-family discs — GD-ROM is CHD's `Cd` media class,
not `Dvd`. `chdman extractcd -o out.gdi` likewise reconstructs a GDI + track
files from a CHD. `retro-junk-lib::chd_convert` round-trip-verifies GDI
sources the same way it does CUE: by parsing both the source and the
extracted `.gdi` into `TrackSpan`s and comparing track-for-track (see
`formats/CHD.md`'s "Round-trip behavior" notes, which record chdman 0.288
round-trip results empirically; the GDI-specific claims there — LBA/type
layout preserved on extraction — were verified the same way as the CUE
claims, against synthetic discs in `chd_convert_tests.rs`/`track_layout_tests.rs`).

## Sources
- chdman source (`src/tools/chdman.cpp`, MAME repository) — canonical
  producer/consumer of the GDI format; the field layout above matches its
  GDI reader/writer. Referenced here by convention (per this repo's existing
  `formats/CHD.md` sourcing) rather than a specific pinned commit; anyone
  revisiting this should diff against a current MAME checkout.
- `retro-junk-disc/src/track_layout.rs` (`parse_gdi`, `split_gdi_fields`,
  `gdi_track_spans`) — the implementation this document describes.
- LBA 45000 high-density convention: widely documented Dreamcast GD-ROM
  preservation knowledge (e.g. Redump's Dreamcast dumping notes); not
  independently re-derived here.
- Round-trip verification against real chdman: `.claude/skills/retro-archive/formats/CHD.md`
  ("Round-trip behavior" section, chdman 0.288, 2026-07) and
  `retro-junk-lib/src/tests/chd_convert_tests.rs` / `retro-junk-disc/src/tests/track_layout_tests.rs`.
