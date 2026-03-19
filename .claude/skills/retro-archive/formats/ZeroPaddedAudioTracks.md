# Zero-Padded Audio Tracks in BIN Dumps

## Problem

Some PS1 (and potentially other CD-based) BIN dumps have audio tracks written with Mode 2 sector headers and zero-filled user data instead of raw PCM audio. This is a **bad dump** — real CD audio tracks are headerless raw 16-bit PCM at 44.1 kHz. The dump tool either failed to capture the audio or padded the BIN to full disc size with empty Mode 2 sectors.

The data track is intact and hashable, but the audio data is lost.

## Sector Layout

### Real data sector (Track 1)
```
Bytes 0-11:   CD sync pattern (00 FF FF FF FF FF FF FF FF FF FF 00)
Bytes 12-14:  MSF (minutes/seconds/frames)
Byte 15:      Mode (0x02 = Mode 2)
Bytes 16-23:  Subheader (file/channel/submode/coding, repeated)
              Submode typically 0x08 (data) or 0x64/0x89
Bytes 24-2351: User data (NON-ZERO)
```

### Zero-padded filler sector (fake "audio" track)
```
Bytes 0-11:   CD sync pattern (present! — this is the problem)
Bytes 12-14:  MSF
Byte 15:      Mode 2
Bytes 16-23:  Subheader with submode 0x20 (Form 2, no data flag)
Bytes 24-2351: ALL ZEROS
```

### Real audio sector (normal dump)
```
Bytes 0-2351: Raw PCM audio data (NO sync pattern, NO headers)
```

## Why Standard Detection Fails

The standard `find_raw_bin_data_track_size()` uses a binary search checking for the CD sync pattern at the start of each sector. Normal audio tracks lack sync patterns, so the boundary is easily found.

Zero-padded filler sectors **do** have sync patterns, so the entire file appears to be one contiguous data track. The hash then covers both tracks, producing a CRC that doesn't match Redump's Track 1 hash.

## Detection Heuristic

The secondary detection (`find_zero_padded_track_boundary()`) checks both:
1. Sync pattern present (sector has CD header structure)
2. User data region (bytes 24+) contains non-zero data

A sector with a sync pattern but all-zero user data is classified as filler, not real data. Binary search finds the boundary between real and filler sectors.

## Example

**Street Fighter Zero 2 (Japan)**:
- BIN = 615,612,480 bytes
- Track 1 (data): 572,363,904 bytes (243,352 sectors)
- Track 2 (zero-padded): 43,248,576 bytes (18,388 sectors)
- Redump Track 1 CRC: `e8f6f832`
- Without fix: hash covers entire BIN → CRC `38485b19` (wrong)
- With fix: hash covers Track 1 only → CRC matches Redump

## Source

Discovered through analysis of ~20 PSX BIN dumps that failed Redump hash matching. The zero-padded pattern was confirmed by examining raw sector bytes at the Track 1/Track 2 boundary.
