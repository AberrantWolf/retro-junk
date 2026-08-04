# NEC PC Engine / TurboGrafx-16 Data Storage Guide

## Console Overview
- **Manufacturer**: NEC Home Electronics, co-designed with Hudson Soft
- **Release Dates**: Japan (October 30, 1987 as PC Engine), North America
  (August 29, 1989 as TurboGrafx-16), Europe (1990, TurboGrafx limited release)
- **Active Years**: 1987–1994 (Japan far outlasted the Western releases)
- **Regional Variants** — the same hardware sold under different names, which
  is why the catalog keeps `pce` and `tg16` as separate library identities:
  - **PC Engine** (Japan, 1987): the original white console
  - **TurboGrafx-16** (North America, 1989): larger case, altered HuCard edge
    connector, so Japanese cards do not physically fit
  - **PC Engine CoreGrafx / CoreGrafx II / Shuttle / GT / LT** (Japan):
    cosmetic and portable revisions, same software library
  - **SuperGrafx** (Japan, 1989): enhanced hardware, five exclusive games,
    **separate DAT and separate `.sgx` extension** — not the same library
  - **PC Engine Duo / TurboDuo** (1991/1992): console with the CD add-on built
    in; its disc software is the PC Engine CD library, cataloged as `pcecd`
  - **TurboGrafx-16 PAL** (Europe, 1990): a PAL-adjusted US machine sold in a
    limited release (chiefly France, the UK and Spain, via the Sodipeng
    distributor) running the **US** card library in localized boxes — see the
    regional-filing section below, because this is *not* a third library

## Regional Filing: Europe Belongs Under `pce`

**Rule this project follows: a European release of this console is filed as
PC Engine (`pce` in the archive, `pcengine` in an ES-DE playable projection),
never as TurboGrafx-16. Only North America gets the American names — `tg16`
for cards, `tg-cd` for discs. Europe and Japan share `pce` / `pcecd`.**

This is the opposite of the rule for the Mega Drive, so it is worth stating
why, and the databases settle it. Measured 2026-08-04:

| Set | Japan | USA | Europe |
|-----|-------|-----|--------|
| No-Intro `NEC - PC Engine - TurboGrafx 16` (991 entries) | 654 | 198 | **0** |
| Redump `NEC - PC Engine CD & TurboGrafx CD` (551 entries) | 365 | 41 | **0** |

There is not one Europe-only dump in either set. (No-Intro carries 32 entries
tagged `(USA, Europe)`, and every single one is a Wii U Virtual Console
re-release, not a physical card.) The reason is the hardware history above:
NEC's European machine was a PAL TurboGrafx running the American card
library, so no European-specific cards or discs were ever pressed. Compare
Sega, which really did press a distinct European Mega Drive library — that is
why `genesis` + Europe correctly maps to `megadrive` while `pce` + Europe
maps to nothing at all.

What a European collector actually owns, then, is imported Japanese hardware
and Japanese software; the console was an import phenomenon in Britain and
France and was covered in the press under its Japanese name throughout. So
the European shelf *is* the PC Engine shelf. Sending it to `tg16` on the
strength of a region string would split one collection across two folders
with nothing in the second one that belongs there.

(Confidence note: the DAT counts above are measured and re-checkable. The
characterization of collector practice is a general read of the community,
not a survey.)

### Where this is enforced

- `regional_physical_platform` (`retro-junk-archive/src/collection.rs`) — the
  archive's platform+region → folder table. The `pce` arm lists North America
  only; Europe falls through to `None` and stays under `pce`.
- `esde::system_directory` (`retro-junk-frontend/src/esde.rs`) — the playable
  projection. `pce` + Europe → `pcengine`, `pcecd` + Europe → `pcenginecd`.
- Both are covered by tests that name this reasoning, so a future
  "simplification" that folds the PC Engine and Mega Drive region lists
  together will fail rather than silently re-split European collections.

Note that removing a regional mapping is not reversible by the archive's
regional migration, which only moves releases *into* regional folders. An
archive that filed a European release under `tg16` before 2026-08-04 keeps it
there; with zero such dumps in either database this is theoretical, but it is
the reason the mapping table's doc comment now warns about deletions.

## Storage Media
- **HuCard**: credit-card-shaped ROM card, 1–20 Mbit (128 KB – 2.5 MB)
- **Typical Sizes**: 2–4 Mbit; 20 Mbit only for Street Fighter II' CE
- **Save Storage**: none on the card itself — saves live in the console's
  internal backup RAM or the external Tennokoe Bank / Turbo Booster-Plus
- **CD-ROM²/Super CD-ROM²**: the disc add-on, archived separately (Redump)

## Archival Storage
### Recommended Formats
- **.pce**: headerless raw dump — the No-Intro standard representation
- Strip any 512-byte copier header before archiving; it is dumping-tool
  residue, not part of the cartridge

### Best Practices
- Verify against No-Intro `NEC - PC Engine - TurboGrafx 16` (that is the
  LibRetro mirror's filename spelling — no hyphen before `16`)
- Reject bit-reversed dumps rather than storing them as variants
- Record which market a card came from separately — the ROM bytes do not say
- Archive HuCard and CD-ROM² libraries under distinct platforms; they share a
  console but not a medium, a database, or a matching strategy

## Emulation Storage
### Recommended Formats
- **.pce**: universally accepted by Mednafen, Beetle PCE, Ootake

### Considerations
- Total HuCard library is small — a complete set is well under 200 MB
- Emulators generally accept headered dumps, so a file that plays fine may
  still fail DAT matching; the header is the usual reason
- SuperGrafx titles need an emulator that models the extra hardware

## ROM Format Reference
See [PCEngine.md](../formats/PCEngine.md) for the (absent) header layout, the
copier-header rule, and detection notes.

## Sources

- [No-Intro DAT-o-MATIC](https://datomatic.no-intro.org/)
- [Redump PC Engine CD / TurboGrafx-CD](https://redump.info/discs/system/pce/)
  (datfile: `https://redump.info/datfile/pce/`)
- [Archaic Pixels — PC Engine hardware documentation](http://archaicpixels.com/Main_Page)
