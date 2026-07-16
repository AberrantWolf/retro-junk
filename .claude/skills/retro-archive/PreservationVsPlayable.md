# Preservation Masters vs. Playable Copies

## The core tension

A serious game collection wants two things that pull in opposite directions:

- **Preservation** wants the most faithful possible capture — every bit on the disc, including
  subchannel, lead-in/lead-out, scrambling and errors (a [redumper](formats/Redumper.md) raw folder, a
  full [Redump](formats/Redump.md) bin/cue set). These files are large, fragile to "improve," and often
  not directly loadable by an emulator.
- **Playability** wants something small, convenient, and directly runnable ([CHD](formats/CHD.md),
  [RVZ](formats/RVZ.md), a merged single-file image) — usually a *lossy-for-convenience* derivative
  (compressed, junk-stripped, or re-laid-out).

The archival-community answer is **not to choose**: keep a pristine **master** and derive one or more
**access copies** from it, treating the access copies as disposable and regenerable. This document
records how established tools model that split, so retro-junk (and anyone reading these notes) can reuse
proven patterns instead of reinventing them.

> **Sourcing:** Prior-art claims below cite each tool's own docs/source. Surveyed 2026-07-16. Confidence
> markers are inline; "no tool automates X" claims are the weakest (absence-of-evidence after targeted
> search) and flagged as such.

## The two-tier model

Every mature system converges on the same shape:

1. **A canonical master store** — content-addressed by hash, never played from, never mutated in place.
   Its job is integrity and provenance.
2. **Disposable built/access views** — generated from the master per a policy (a DAT, a target
   emulator, a device profile). Cheap to rebuild; safe to delete.

The **link between the two is a content hash** — identity is "what the data *is*," not "which file it's
in." retro-junk already has this: `compute_container_hashes` normalizes CHD/RVZ down to the
community-standard uncompressed representation before hashing, so a `.chd`, its source `.cue/.bin`, and
(after `redumper split`) a raw dump all resolve to the **same SHA1**. That shared hash is what makes
"one game, several representations" well-founded rather than a guess.

## Prior art

### romba / RomVaultX — the content-addressed depot
**(High confidence.)** Romba and RomVaultX store ROMs in a **depot**: a content-addressed tree where
each file's path is derived from its SHA-1 (e.g. `ab/cd/abcdef…gz`), gzip-compressed, with the hash in
the gzip header for fast verification. You **never play from the depot** — you `build` a target set (a
directory or archive layout for a given DAT) *out of* the depot on demand. This is the purest analog of
an `archive/` (canonical, hashed, cold) vs `roms/` (built, warm, per-use) split: identity is the
content hash; on-disk layout is a per-collection *policy* chosen at build time, not a property of the
file. Take-away: **the master store's layout and the playable layout are independent, joined by hash.**

### igir — link-mode projections
**(High confidence.)** [igir](https://igir.io) builds playable game sets and, crucially, supports
`--link-mode {hardlink,symlink,reflink}` so a playable "copy" is a **reference** to a single source
file with **no byte duplication**. Directly relevant when the archival representation is *itself*
directly playable (e.g. a plain `.iso` or an already-`.chd` archive): the `roms/` entry can be a
reflink/hardlink to the `archive/` file rather than a second copy. igir also ships `dir2dat` (snapshot
an existing tree into a DAT) and `playlist` (generate `.m3u` for multi-disc sets). Take-away:
**projections can be zero-cost; don't duplicate bytes when a link suffices.**

### MAME merged / split / non-merged sets
**(High confidence.)** MAME's canonical example of "one logical game, multiple physical layouts, identity
by DAT hash": the *same* ROM data is packaged as **merged**, **split**, or **non-merged** sets — the
layout is a per-collection policy, not part of the game's identity. CHDs are stored **outside** the ROM
archives, and **delta-CHDs** store only the diff against a parent CHD. Take-away: **layout is policy;
large media lives beside, not inside; deltas exploit shared data** (cf. [miniscram](formats/Redumper.md)
delta-compressing `.scram` against the split `.bin`).

### ES-DE / Batocera / RetroDECK — one path per entry
**(High confidence for ES-DE; Medium for the downstreams, which inherit its model.)** ES-DE and the
frontends built on it model **exactly one file path per game entry**. A game present as both `.bin/.cue`
and `.chd` is handled by **hiding** one or by **converting to CHD**, never by linking two
representations under one metadata entry. ES-DE's "directories interpreted as files" collapses a
multi-file game folder into a single launchable entry — relevant to how a redumper folder should
*present* to a frontend. Take-away: **frontends expect one playable path per game — so expose only the
playable tree to them and keep the archive tree invisible.** Don't try to make ES-DE understand the
archive.

### LaunchBox "Additional Apps"
**(High confidence.)** The strongest *frontend* prior art for "one metadata entry → several launchable
files": regional variants, a marked default "Play Version," etc. It models *versions*, not
master-vs-derivative, but proves the UX of a single game row fanning out to multiple files is viable and
familiar to users. Take-away: **one row, several launch targets, one marked default** is an accepted UX.

### The redumper-raw → derived-playable workflow
**(Medium confidence — absence after targeted search.)** No existing turnkey tool automates "keep the
raw redumper dump as master **and** maintain a derived playable copy under one shared metadata entry."
Archival best practice endorses "always generate access copies from the original low-level dump" as a
**manual** workflow. This is a genuine gap a tool like retro-junk can fill: ingest the raw folder,
`redumper split`/`hash` to a verified bin/cue, match against Redump, and generate/refresh a CHD for
play — all while the master stays untouched.

## Patterns to reuse

- **Identity = normalized content hash**, never a file path. Two representations of one dump are "the
  same game" because they hash the same after normalization.
- **Master store is cold and canonical**: content-addressed or convention-addressed, never played from,
  never edited in place. Access copies are **regenerable and disposable** — a lost `.chd` is a rebuild,
  not a loss.
- **Zero-copy projections** (reflink/hardlink) when the archive is itself playable; only spend bytes on
  a derivative when it actually differs (compression, format conversion).
- **Expose only the playable tree to frontends.** Keep the archive tree out of ES-DE/gamelist scope so
  you never fight one-path-per-entry assumptions.
- **Layout is policy, identity is data.** Where files live (sibling `archive/` vs `roms/`, per-console
  subfolders, selective-sync scopes) is a collection/device policy; it must not change what a dump *is*.

## See also
- [Redumper.md](formats/Redumper.md) — the raw dump format that serves as the preservation master
- [Redump.md](formats/Redump.md) — the community verification standard for the derived representation
- [CHD.md](formats/CHD.md) / [RVZ.md](formats/RVZ.md) — the compressed playable containers

## Sources
- romba: https://github.com/uwedeportivo/romba (depot / content-addressed store, `build`)
- RomVaultX / RomVault depot docs: https://www.romvault.com/
- igir link modes: https://igir.io/output/options/ (`--link-mode`), https://igir.io/commands/ (`dir2dat`, `playlist`)
- MAME ROM/CHD set types: https://docs.mamedev.org/usingmame/aboutromsets.html
- ES-DE — multi-file games & duplicate handling: https://gitlab.com/es-de/emulationstation-de/-/blob/master/FAQ.md
- LaunchBox Additional Apps: https://www.launchbox-app.com/ (manual: Additional Applications)
- Preservation "access copy from master" methodology: general digital-preservation practice (e.g. Library of Congress "master vs. derivative" guidance)
