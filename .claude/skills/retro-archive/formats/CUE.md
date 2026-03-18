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
- `//` comments are skipped
- When the DATAFILE BIN doesn't exist on disk, the hash code falls back to finding an existing BIN from other FILE entries in the same CUE

### Identifying data tracks for hashing

A track is considered a data track if its mode string contains `"MODE"` (case-insensitive). This works for both standard (`MODE2/2352`) and CDRWin (`MODE2_RAW`) modes. Audio tracks use `AUDIO` which doesn't match.
