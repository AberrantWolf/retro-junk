# NEC PC Engine / TurboGrafx-16 HuCard ROM Format

Used by: [PC Engine / TurboGrafx-16](../consoles/PCEngine_Overview.md)

## File Extensions
- `.pce` — HuCard ROM dump (the near-universal convention)
- `.sgx` — SuperGrafx ROM dump; same raw layout, different (larger) hardware
  and a separate No-Intro DAT. **Not** handled by the PC Engine analyzer.

## Header Format

**There isn't one.** A HuCard dump is raw cartridge bytes from offset 0. The
card carries no magic signature, no internal title, no serial, no region byte,
and no checksum field. Nothing in the file identifies the game.

The practical consequence: identification is hash-only. Compute CRC32/SHA-1
over the cartridge bytes and look the digest up in No-Intro. There is no
header-parsing fallback the way there is for the Mega Drive or the SNES.

The 6502-derived HuC6280 CPU's interrupt vectors live at the top of the
address space ($FFF6–$FFFF), which is mapped from the *last* 8 KB bank of the
ROM rather than a fixed file offset, so the vectors' position depends on the
dump's size and mapper. That makes them unreliable as a detection signature.

## Copier Header (512 bytes)

Some older dumps — made by backup units in the same era as SNES `.smc`
copiers — carry an extra 512 bytes in front of the cartridge data.

Detection is by size arithmetic, because HuCards are banked in 8 KB units and
a clean dump is therefore always a whole number of 8 KB banks:

```
if file_size % 8192 == 512  →  first 512 bytes are copier padding, skip them
```

This is the same test Mednafen applies. No-Intro records digests of the
cartridge bytes alone, so those 512 bytes **must** be skipped before hashing
or the headered dump will never match its DAT entry.

Note the divisor differs from the SNES rule (`% 1024 == 512`): a HuCard bank
is 8 KB, not 1 KB, so the coarser test is the correct one here.

## Detection Method

With no signature to check, the only structural test available is shape:

1. Take the file size; subtract 512 if `size % 8192 == 512`.
2. The remainder must be a nonzero multiple of 8192.
3. The remainder must fall within plausible HuCard sizes (see below).

This is a weak test — many unrelated binaries are multiples of 8 KB — so in
practice the `.pce` extension is what routes a file to this analyzer, and the
hash is what identifies it.

## ROM Sizes

| Size | Notes |
|------|-------|
| 128 KB (1 Mbit) | Common early titles |
| 256 KB (2 Mbit) | Most common size |
| 384 KB (3 Mbit) | Requires bank-switching |
| 512 KB (4 Mbit) | |
| 1 MB (8 Mbit) | |
| 2.5 MB (20 Mbit) | Street Fighter II' Champion Edition — the largest commercial HuCard |

## Bit-Order Caveat

A minority of early dumps were produced with the data bus bit order reversed
(bit 0 ↔ bit 7 within each byte), an artifact of how some dumping hardware was
wired. These are recognizably wrong dumps rather than a legitimate variant;
No-Intro does not catalog them, and correcting one means re-reversing every
byte. Nothing in this codebase does that today — such a file simply fails to
match and is reported as unidentified.

## Region

PC Engine (Japan) and TurboGrafx-16 (North America) HuCards are physically
incompatible — the card edge pinout was deliberately altered — but the ROM
*data* format is identical, and nothing in the bytes says which market a card
was sold into. Region comes from the DAT entry's name, never from the file.

## Sources

- [No-Intro DAT name: "NEC - PC Engine - TurboGrafx 16" (LibRetro mirror spelling)](https://datomatic.no-intro.org/)
- [Mednafen PC Engine source — 512-byte header check](https://mednafen.github.io/)
- [PC Engine Software Bible / archaicpixels.com hardware notes](http://archaicpixels.com/Main_Page)
- [GameDataBase CSV: `console_nec_pcengine_turbografx_supergrafx`](../../game-scraping/GameDataBase.md)
