# CUE Sheet Formats

CUE sheets are text files describing the track layout of a CD disc image. There are two main variants encountered in retro game preservation.

**Sources:**
- [cdrdao(1) man page](https://man.archlinux.org/man/cdrdao.1.en) — authoritative source for TOC/CDRWin format
- [libodraw CUE sheet format](https://github.com/libyal/libodraw/blob/main/documentation/CUE%20sheet%20format.asciidoc)
- [CUETools wiki](http://cue.tools/wiki/Cue_sheet) — history and overview
- [CUE sheet specification (wyDay)](https://wyday.com/cuesharp/specification.php) — standard format reference

## Standard CUE Format

The most common format. Structure: `FILE` first, then `TRACK` and `INDEX` nested within.

```
FILE "Game (Track 1).bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
FILE "Game (Track 2).bin" BINARY
  TRACK 02 AUDIO
    INDEX 00 00:00:00
    INDEX 01 00:02:00
```

### Key directives

| Directive | Syntax | Description |
|-----------|--------|-------------|
| `FILE` | `FILE "filename" BINARY` | Specifies a data file. Type is usually `BINARY`. |
| `TRACK` | `TRACK <number> <mode>` | Starts a track with explicit number (01-99) and mode. |
| `INDEX` | `INDEX <number> MM:SS:FF` | Index point within a track. INDEX 01 = track start. |

### Track modes

- `MODE1/2048` — ISO data sectors (2048 bytes user data)
- `MODE1/2352` — raw Mode 1 sectors (Saturn, Sega CD)
- `MODE2/2352` — raw Mode 2 sectors (PlayStation)
- `AUDIO` — CD audio (2352 bytes PCM)

### Redump convention

Redump CUE sheets use **one BIN file per track**, named `Game Name (Track N).bin`. All sectors are raw 2352 bytes. This is the standard format and what the parser handles most commonly.

## CDRWin / cdrdao TOC Format

An alternative format used by CDRWin and cdrdao. Key differences from standard CUE:

1. **Disc type header** — starts with `CD_DA`, `CD_ROM`, or `CD_ROM_XA`
2. **`TRACK` has no number** — tracks are auto-numbered sequentially
3. **`TRACK` appears before its file directive** (reversed from standard CUE)
4. **`DATAFILE` instead of `FILE`** for data tracks
5. **`//` comments** (standard CUE uses `;` or `REM`)
6. **Additional directives**: `SILENCE`, `START`, `ZERO`, `NO COPY`, `NO PRE_EMPHASIS`, `TWO_CHANNEL_AUDIO`

### Example (real-world PS1 game)

```
CD_ROM_XA

// Track 1
TRACK MODE2_RAW
NO COPY
DATAFILE "game.bin" 01:32:21 // length in bytes: 16278192

// Track 2
TRACK AUDIO
NO COPY
NO PRE_EMPHASIS
TWO_CHANNEL_AUDIO
SILENCE 00:02:00
FILE "game (Track 1).bin" #16278192 0 00:08:08
START 00:02:00
```

### CDRWin-specific directives

| Directive | Syntax | Description |
|-----------|--------|-------------|
| `CD_ROM_XA` | (standalone) | Disc type: Mode 2 with audio (PlayStation). |
| `CD_ROM` | (standalone) | Disc type: Mode 1 or mixed mode. |
| `CD_DA` | (standalone) | Disc type: audio only. |
| `DATAFILE` | `DATAFILE "filename" [length]` | Data file for data tracks. Length is MSF or bytes. |
| `FILE` / `AUDIOFILE` | `FILE "filename" #offset start length` | Audio file with byte offset and MSF start/length. `AUDIOFILE` is a synonym. |
| `SILENCE` | `SILENCE MM:SS:FF` | Insert silence (used for pre-gaps). |
| `START` | `START [MM:SS:FF]` | Pre-gap length (index 0→1 transition). |
| `ZERO` | `ZERO MM:SS:FF` | Zero data for gaps between track modes. |
| `NO COPY` | (standalone) | Copy prohibition flag. |
| `NO PRE_EMPHASIS` | (standalone) | No pre-emphasis on audio. |
| `TWO_CHANNEL_AUDIO` | (standalone) | Stereo audio flag. |

### CDRWin track modes

| Mode | Block Size | Notes |
|------|-----------|-------|
| `AUDIO` | 2352 | CD audio |
| `MODE1` | 2048 | ISO data |
| `MODE1_RAW` | 2352 | Raw Mode 1 (includes sync/header/ECC) |
| `MODE2` | 2336 | Mode 2 data |
| `MODE2_FORM1` | 2048 | Mode 2 Form 1 data |
| `MODE2_FORM2` | 2324 | Mode 2 Form 2 data |
| `MODE2_FORM_MIX` | 2336 | Mixed form (includes sub-header) |
| `MODE2_RAW` | 2352 | Raw Mode 2 (PlayStation) |

### Important: DATAFILE may reference non-existent files

In CDRWin CUEs converted from other tools, the `DATAFILE` filename may be a **virtual name** that doesn't exist on disk. The actual data is in the combined BIN file referenced by the `FILE` directives. The `#offset` parameter in `FILE` lines indicates where audio data starts within the combined BIN, and the data track occupies bytes 0 through that offset.

## Parsing strategy (retro-junk-disc)

The parser in `retro-junk-disc/src/cue.rs` handles both formats:

- `FILE`, `DATAFILE`, and `AUDIOFILE` all create file entries
- `TRACK` lines accept both `TRACK 01 MODE2/2352` (standard) and `TRACK MODE2_RAW` (CDRWin, auto-numbered)
- Tracks appearing before their file directive are buffered and attached when the next `FILE`/`DATAFILE` is encountered
- Directives are detected by their first whitespace-delimited token rather than a literal-space prefix, so tab-separated cue sheets (real exporter output, not just space-separated ones) parse identically
- `//` comments are skipped
- When the DATAFILE BIN doesn't exist on disk, the hash code falls back to finding an existing BIN from other FILE entries in the same CUE
- `PREGAP`/`POSTGAP` directives are parsed and attached to the current track as frame counts (`CueTrack::pregap_frames`/`postgap_frames`). These represent gap time *not* stored in the track file (unlike an in-file `INDEX 00` pregap). `retro-junk-lib::chd_convert` rejects compressing a CUE that declares either, because chdman's `createcd` synthesizes the missing gap bytes into the CHD and materializes them again on `extractcd`, so the extracted track is longer than the source span and a byte-exact round-trip comparison is impossible.

### Sector size lookup: one canonical table

`retro_junk_disc::cue::sector_size_for_mode(mode: &str) -> u64` is the single
implementation for mapping a TRACK mode string to its sector size in bytes,
used by both `cue::convert_cue_to_standard` (CDRWin→standard conversion) and
`track_layout::cue_track_spans` (byte-span computation for CHD round-trip
verification and Track-1-boundary detection). Standard modes carry the size
after the slash (`MODE1/2352`, `MODE2/2336`); CDRWin bare mode names
(`MODE1`, `MODE2`, `MODE2_FORM1`, `MODE2_FORM2`, `MODE2_FORM_MIX`,
`MODE1_RAW`, `MODE2_RAW`) are looked up by name per the "CDRWin track modes"
table above. The bare-`MODE1`/`MODE2` (no slash, no `_FORM*`/`_RAW` suffix)
entries in that table are CDRWin/cdrdao TOC-format knowledge, sourced from
the [cdrdao(1) man page](https://man.archlinux.org/man/cdrdao.1.en) cited
above — `MODE1` alone (2048 bytes, "cooked" ISO9660 data) and `MODE2` alone
(2336 bytes, Mode 2 data without the Form 1/Form 2 split) are real CDRWin
literals, not just slash-suffixed standard modes with the slash omitted.
Before this unification (2026-07), `cue.rs` and `track_layout.rs` each had
their own divergent copy of this table, and `track_layout.rs`'s copy did not
recognize CDRWin bare names at all (falling back to 2352 for every one of
them) — a CDRWin-mode cue reaching `cue_track_spans` therefore got wrong byte
spans.

### Identifying data tracks for hashing

A track is considered a data track if its mode string contains `"MODE"` (case-insensitive). This works for both standard (`MODE2/2352`) and CDRWin (`MODE2_RAW`) modes. Audio tracks use `AUDIO` which doesn't match.
