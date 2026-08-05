# Identification and status

How retro-junk decides what a file is, what it will let you do with it, and
what colour that shows as. One ladder, one status scale, one code path.

This document is the reference; the code is expected to match it. If they
disagree, that is a bug in one of them.

## Why this exists

A disc's identity was its *largest data track*. That is not an identity: 1029
catalog media rows share their primary hash with another row on the same
platform — `Monster Lair (USA)` against `Wonder Boy III - Monster Lair (Japan)
(Rev 3)`, a demo against its full game, eight Dreamcast cheat discs sharing one
5,729,472-byte track. CD releases routinely share a data track and differ only
in audio tracks.

Keying on the **complete ordered track set** separates every one of those, and
is unique across all 94,481 media rows in a full catalog. So that is the
identity.

## The ladder

Identification returns exactly one outcome. A rung is only tried when the rung
above returns nothing.

| # | Outcome | What it means |
|---|---------|---------------|
| 1 | `Complete` | Every track's sha1 and size agree, in track order, on the same platform. Cartridges, DVDs and single-track discs are the one-track case and reach this for free. |
| 2 | `Unique` | The evidence we hold leaves exactly one possible catalog entry, but not every hash is verified — some tracks unhashed, or only the primary hash known. |
| 3 | `Manual` | The user picked one entry from the candidate list. Carries who/when, and never reports as verified. |
| 4 | `Ambiguous` | Several entries remain possible. Carries the candidates, so the UI can offer them. |
| 5 | `Unidentified` | Nothing narrows it down. |

Two rules hold at every rung:

- **Ambiguity is never resolved by guessing.** More than one candidate at a
  rung yields `Ambiguous`, not the first row.
- **Partial evidence never promotes.** One matching track of five is evidence
  for a candidate list, never for an identity.

### What each outcome permits

| Outcome | Rename | Build playable | Scrape |
|---------|--------|----------------|--------|
| `Complete` | yes | yes | yes |
| `Unique` | yes | yes | yes |
| `Manual` | yes | yes | yes |
| `Ambiguous` | no | no | no |
| `Unidentified` | no | no | no |

`Ambiguous` and `Unidentified` are error markers. They exist to be shown and
fixed, not acted on.

`Unique` and `Manual` are actionable but **never** display as complete. The UI
must say what is missing, in words, naming the fix — the same contract
`UnknownReason::explain` already holds.

## Incompleteness reasons

Whatever makes an identification less than `Complete` is carried with it, so
every surface explains the same gap the same way. A reason names the fix.

- `TracksUnhashed { hashed, total }` — the medium has tracks we have not hashed.
- `PrimaryHashOnly` — we hold the primary track's hash and no per-track hashes.
- `NoCatalogForPlatform` — no DAT imported for this platform, so nothing can be
  verified against.
- `HashesDisagree` — we can name a candidate, but a hash we hold contradicts
  it. **This is worse than not knowing**: the file will probably not play
  correctly.
- `ManuallyChosen` — a person selected this entry; it was never verified.
- `NotCatalogued` — homebrew, a ROM hack or a mod: no DAT will ever list it, so
  `Complete` is unreachable and its absence is not a defect. Reads as
  `Asserted`, not as a gap to be closed.

## Status scale

One scale. Both the general status column and the evidence badges map through
it, and the overall status is **derived from** the evidence rather than
computed beside it, so the two cannot disagree:

```
overall_severity = max(identity_severity, worst_evidence_severity)
```

Five states, because each one asks the user for a *different next action*.
That is the test for whether a state earns its own colour — not how different
it feels, but whether it changes what you would do.

| Severity | Colour | Icon | Meaning | What you do about it |
|----------|--------|------|---------|----------------------|
| `Verified` | green | filled check circle | Complete and checked against a catalog | nothing |
| `Asserted` | blue | pencil / hand mark | A **person** decided this: manual disambiguation, homebrew, a ROM hack, a mod. Correct, but not machine-verifiable and never will be | nothing — this is intentional |
| `Incomplete` | amber | half-filled circle | Something is missing, unverified, or waiting on your decision | finish it: hash the rest, scrape the art, pick between candidates |
| `Broken` | red | warning triangle | Unusable: nothing identifies it, **or** we can name it and a hash contradicts it | investigate or replace the file |
| `Unmeasured` | gray | hollow circle | No measurement is possible yet, and the reason says why | usually: import DATs for this platform |

Hard rules:

- A medium with unhashed or missing tracks is **never** green or blue. It is
  `Incomplete` at best.
- Red means unusable *and* nothing more to go on: either nothing identifies it
  at all, or `HashesDisagree`. `Ambiguous` is amber, not red — it has a
  candidate list, so there is a decision to make rather than a dead end.
- Gray is only for "cannot be measured", never for a zero denominator.
- Blue is never *derived*. If the system worked the answer out, it is green,
  amber, or red. Blue means a human's assertion is load-bearing.

### Why five, and not more or fewer

Two states deliberately **not** split out:

- `Broken` covers both "no idea what this is" and "we know what it claims to
  be and the hashes are wrong". Very different diagnoses, identical response:
  do not use this file. Which one it is belongs in the tooltip and the details
  view, not in a sixth colour.
- `Incomplete` covers a missing track and missing box art, which differ wildly
  in urgency. The overall dot is not the right place to encode *which* aspect
  is incomplete — that is exactly what the per-aspect evidence badges are for,
  and they use this same scale.

Nothing is being merged that would be more useful apart, and no colour is
defined that has no state behind it. `Asserted` is the state that was missing:
homebrew, hacks and mods currently have nowhere honest to sit and read as
unidentified.

### Icons are not decoration here

Colour alone is not sufficient, for three reasons that all apply to this app:

- The two most important states are green and red, which is the most common
  form of colour blindness.
- At table density the indicator is a small dot; hue is the least reliable
  channel at that size.
- Screenshots, logs and the CLI lose colour entirely.

So severity carries a **glyph as well as a hue**, and the glyph alone is
sufficient to tell the states apart. The project already ships Phosphor icons,
so this costs nothing new. `Incomplete` and `Unmeasured` especially need it:
both read as "not done" by colour, and mean quite different things.

## Manual disambiguation

A user may resolve an `Ambiguous` entry by choosing from **its own candidate
list** — never free-form, never an entry that was not a possible match.

- Offered in the details view of both archive releases and library entries.
- Stored **outside the database**, as a content-keyed mark, like tags and
  region corrections. A choice keyed on a path would not survive a rename, and
  a choice that only existed in the database would break the property that the
  database is disposable.
- Re-selectable: choosing again replaces the previous choice.
- Never reports as `Complete`; it reports as `Manual` with
  `ManuallyChosen`.

## Scraping

Scraping queries with the **catalog entry's own** sha1, size and title —
whether identity was reached automatically or manually. What the user's file
hashes to is irrelevant once we know which catalog entry it is.

Without an imported DAT there is no catalog entry to speak for, so scraping
falls back to the computed hashes and **warns on every surface** (CLI and GUI)
that results are less accurate until DATs are imported.

## One code path

These all answer "what is this?" and must funnel into the single identification
entry point rather than each asking their own question:

- `retro-junk-dat`: `match_by_hash`, `match_by_serial`,
  `match_by_serial_with_identification`
- `retro-junk-db/queries`: `match_media_by_hash`, `match_media_by_hashes`,
  `match_media_ids_by_track_hash`, `match_media_by_serial`,
  `match_media_by_serials`, `find_media_by_crc32/sha1/md5/serial`
- `retro-junk-db/archive`: `match_catalog_file`,
  `match_complete_catalog_media` (+ `_any_platform` variants),
  `match_catalog_serial_any_platform`
- `retro-junk-import/dat_import`: `find_media_by_release_and_rom_name`,
  `find_media_by_dat_name`

`match_complete_catalog_media` already implements rung 1 correctly, including
the rule that one matching data track cannot verify a multi-track disc. It is
the seed of the single path, not a thing to reimplement beside.

## The database is disposable

No rebuild turned out to be necessary. The first plan was to re-key media on
content, which would have orphaned every id the archive and collection point
at and forced a rebuild. Keeping the slug id and adding a *content lookup* as
the importer's last resort reaches the same place without breaking a single
reference: a renamed game is found by its track set and keeps the id it had.

The property still matters, though, and every new user decision is held to it —
a disambiguation is a mark beside the collection, not a row. The database
remains rebuildable because everything durable lives outside it — verified
2026-08-05:

- archive releases, carriers, representations, derivations → rebuilt by
  `reconcile_archive_snapshot` from on-disk manifests
- catalog works/releases/media → DATs plus `catalog/` YAML
- region corrections, tags → content-keyed marks beside the collection
- ignore rules → written into the collection root
- playable policies → projected from archive manifests
- profiles → `settings.toml`

The one thing that did **not** live outside the database was 465 dismissed
`adopt_playable` suggestions, and by decision those are not preserved. Any new
user decision — manual disambiguation especially — must be durable outside the
database or this property is lost.

## Where the code is against this document

Landed:

- The ladder itself — `retro-junk-db/src/identify.rs`. One `identify()`, the
  five rungs, the candidate list behind `Ambiguous`, and the rule that a
  manual choice may only name an entry that was already a candidate.
- The severity scale — `retro-junk-backend/src/completion.rs`.
  `Completion::severity` folds identity and every evidence fraction with
  `worst`, so a summary can never read better than the worst badge beside it;
  a property test walks every combination of fraction shapes to hold that.
- One appearance mapping — `retro-junk-gui/src/theme.rs`. The row status dot,
  the per-aspect evidence badges and the library entry badge all resolve
  colour through `severity_color`, and the dot draws `severity_icon` so the
  state survives without colour. The three separate colour tables are gone.
- `LikelyMatched` is amber, not blue, and `Tagged` (homebrew, mods) is blue —
  the two mappings the new rules made wrong.
- The importer finds an existing medium by its complete track set when the
  name has changed (`retro-junk-import/src/dat_import.rs`), so a corrected DAT
  name keeps its entry instead of minting a twin.
- Scraping warns once, through the shared progress channel so both frontends
  get it, when a platform has no catalog to search with.

- Manual disambiguation, end to end: a `Disambiguation` mark kind, the
  `retro_junk_backend::disambiguation` store that reads and writes it, and a
  chooser in the details view that offers the entry's own candidates and
  nothing else. `identify` consults the choice through
  `Evidence::manual_media_id`.
- Marks are filed one per *decision slot* rather than one per file, fixing a
  bug this work uncovered: a region correction and a homebrew tag on the same
  file overwrote each other, because the mark's path was platform and digest
  with no room for two answers.
- `Ambiguous` is amber rather than red — it has candidates, so it is a decision
  to make rather than a dead end.

- One ladder, two ways of gathering. `identify::decide` is the pure rule —
  candidates in, verdict out — and `identify` is the per-file gatherer that
  feeds it. The library scan matches in bulk (a thousand files in two queries)
  and cannot afford a query per file, so it gathers its own way and then calls
  the same `decide`. The status ladder that used to live beside it in
  `apply_catalog_resolution` is gone.
- Narrowing and deciding are separated. Choosing *which* candidate survives —
  by byte order, by an NDS header revision — is gathering; `decide` only
  judges how strong the surviving answer is, and is handed the survivor rather
  than the field it came from.
- Carrier resolution consults disambiguations. When recorded track digests fit
  more than one catalog entry the projection still refuses to guess, but it
  now honours a choice a person already made. The collection root comes from
  the snapshot's own root, so none of the thirty-six callers of
  `reconcile_archive_snapshot` had to change.
- `EntryStatus::Disambiguated` exists and reads as `Asserted` — blue, never
  green, and re-selectable.

Remaining, and deliberately so:

- The raw-DAT matchers in `retro-junk-dat` (`match_by_hash`, `match_by_serial`)
  still answer for themselves. They match against an ephemeral `DatIndex` built
  from DAT files with no database in the picture, so they cannot call
  `identify`, which needs a connection. `CLAUDE.md` already records that this
  path is being migrated out in favour of the catalog database; folding it into
  the ladder is that migration, not a detail of this one.
