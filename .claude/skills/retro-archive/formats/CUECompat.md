# CUE Sheet Compatibility Issues

**Sources:**
- DuckStation GitHub issues and testing
- [cdrdao(1) man page](https://man.archlinux.org/man/cdrdao.1.en) — CDRWin format reference
- [CUE sheet specification (wyDay)](https://wyday.com/cuesharp/specification.php)
- [libodraw CUE format docs](https://github.com/libyal/libodraw/blob/main/documentation/CUE%20sheet%20format.asciidoc)

## Problem

Some CD ripping tools generate CUE sheets in CDRWin/cdrdao TOC format instead of standard CUE format. These are valid but not universally supported by emulators. DuckStation in particular rejects CUE files containing `CD_ROM_XA` disc-type headers.

## CDRWin vs Standard CUE

CDRWin format uses different syntax for the same disc layout:

| Feature | Standard CUE | CDRWin |
|---------|-------------|--------|
| Disc type | (none) | `CD_ROM_XA`, `CD_ROM`, `CD_DA` |
| Data file | `FILE "name" BINARY` | `DATAFILE "name" [MSF]` |
| Audio file | `FILE "name" WAVE` | `AUDIOFILE "name" #offset start length` |
| Track mode | `MODE2/2352` | `MODE2_RAW` |
| Track number | `TRACK 01 MODE2/2352` | `TRACK MODE2_RAW` (auto-numbered) |
| Comments | `REM ...` | `// ...` |
| Extra flags | (none) | `NO COPY`, `NO PRE_EMPHASIS`, `TWO_CHANNEL_AUDIO` |
| Gap handling | `PREGAP`/`POSTGAP` | `SILENCE`, `START`, `ZERO` |

## Track Mode Mapping

| CDRWin | Standard | Sector Size |
|--------|----------|-------------|
| `MODE2_RAW` | `MODE2/2352` | 2352 |
| `MODE1_RAW` | `MODE1/2352` | 2352 |
| `MODE2_FORM1` | `MODE2/2048` | 2048 |
| `MODE2_FORM2` | `MODE2/2324` | 2324 |
| `MODE2_FORM_MIX` | `MODE2/2336` | 2336 |
| `AUDIO` | `AUDIO` | 2352 |

## Conversion Safety

Converting CDRWin CUE to standard CUE is **lossless** — the BIN data is identical, only the CUE metadata changes. The conversion:
- Strips disc-type headers
- Maps CDRWin track modes to standard equivalents
- Converts `DATAFILE` to `FILE ... BINARY`
- Strips CDRWin-only directives
- Adds explicit track numbers

**Cannot auto-convert:**
- `AUDIOFILE` with `#offset` (byte offsets into a larger WAV) — would require splitting the audio file
- `DATAFILE` with MSF length when BIN file is not available — need file size to compute INDEX offsets

## Implementation

Detection and conversion are in `retro-junk-disc/src/cue.rs`:
- `check_cue_compat()` — lightweight scan for CDRWin features
- `convert_cue_to_standard()` — line-by-line rewrite to standard format

CLI command: `retro-junk fix-cue [-n] [--no-backup] [-c consoles]`
