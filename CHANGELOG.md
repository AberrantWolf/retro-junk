# Changelog

## 0.4.0

- PC Engine HuCard games are part of the library. NEC's card system had no
  analyzer at all — only its CD add-on did — so a folder of `.pce` files was
  invisible to scanning: the folder never appeared as a console, and no file
  inside it was ever hashed or matched. There is now a `retro-junk-nec` crate
  holding both NEC analyzers (the CD one moved there from the glue crate,
  where it had been the lone piece of console-specific knowledge in a crate
  that is not supposed to hold any). Cards match against No-Intro `NEC - PC
  Engine - TurboGrafx 16`, with the 512-byte copier header some old dumps
  carry skipped before hashing, or every headered dump would silently fail to
  match. The console keeps its regional identities: a `pce`/`pcengine` folder
  and a `tg16`/`turbografx-16` folder stay two separate libraries of one
  console, the way Famicom and NES already do.

- A European PC Engine release is filed as a PC Engine, not a TurboGrafx-16.
  The archive's region table sent Europe to `tg16` while the playable
  projection sent the same release to `pcengine`, so one collection could
  land in two folders depending on which path touched it. The databases
  settle which is right: No-Intro's PC Engine set has zero `(Europe)` dumps
  out of 991, and Redump's PC Engine CD set zero out of 551 — NEC's European
  machine was a PAL TurboGrafx running the American card library, so no
  European-specific cards or discs were ever pressed, and a European shelf of
  this console is imported Japanese software. Europe now stays under
  `pce`/`pcengine` on both paths, and tests on each side state the reasoning
  so the PC Engine and Mega Drive region lists (where Europe *is* a real
  library) are not "simplified" into one. Written up in
  `.claude/skills/retro-archive/consoles/PCEngine_Overview.md`.

- A week-of-changes review (2026-08-03) fixed a batch of defects before they
  shipped:
  - Multi-track discs identified by their complete reproduced track set were
    recording `complete_track_set: false` — the shared evidence helper applied
    the single-file path's rule to both callers — so the one currency
    predicate rejected the evidence and every later build re-ran the
    multi-minute redumper verification. The caller now states what its match
    proved.
  - Catalog lookups from playable folders and archive releases fold their
    platform spelling through `Platform` before querying (`psx` vs `ps1`,
    `gc` vs `gamecube`): the adoption sweep was filing every such file as
    unmatched at confidence 0.1, and folder scrapes of disc platforms
    published nothing into the archive.
  - The archive lock's fallback protocol records the holder's host and only
    PID-probes records this host can vouch for; a network share is shared,
    and machine B could previously "prove" machine A's live holder dead,
    delete its lock, and open the archive to two concurrent writers. Stale
    reclaim also switched to rename-aside, closing the race where two
    contenders both reclaim.
  - The regional-platform migration marker moved to v3: `snesna`,
    `megadrivejp`, and the PC Engine CD split joined the mapping this week,
    and archives that had already run v2 would never have moved those
    releases — new imports and old directories would have split one console
    across two folders forever. The mapping itself now lives in one place,
    shared by the migration and the importer (the importer's copy had
    already drifted).
  - Multi-disc imports passed each disc's *position* as the release's *disc
    count*, so disc 1 landed flat while disc 2 went into the playlist folder,
    and an out-of-order import wrote a "complete" M3U missing a disc. The
    plan now carries the catalog's own disc total.
  - The v24→v25 binding migration collapses duplicate (entry, carrier) rows
    itself; previously the unique index it creates afterwards would fail on
    them and leave the database unable to open.
  - "Track only" auto-import now actually tracks only — the daemon was
    hashing, identifying, and filing suggestions for every package with the
    setting off.
  - Re-filing an identical suggestion keeps the open row instead of
    superseding it: a weak scrape match plus auto-scrape was inserting a new
    row (and bumping every process's refresh signal) every 30-second daemon
    pass, forever. Claim releases are owner-scoped, so a stalled process
    finishing late can no longer delete the claim of whoever took over.
  - Applying an adoption review, and the CLI adoption sweep, now take the
    whole-archive lock before writing evidence, like every other archive
    mutation.
  - GUI: bulk tag/region actions hand their requests to a feeder thread —
    selecting ~300 rows could previously block the render thread on the
    store's bounded queue and freeze the window. Backlog replies carry the
    scope they were computed for, so switching consoles mid-query can no
    longer file one console's backlog under another's name (previously it
    also stuck that way). An inbox refresh signal arriving during an
    in-flight load is no longer swallowed.
  - Scraper: dry runs no longer copy archived artwork into the media tree;
    cancelling during the publish lock wait downgrades the affected targets
    instead of reporting media as scraped that was discarded with the scratch
    directory; PC Engine CD scrapes work (system 114); media downloads cap
    the buffer they pre-allocate from the server's Content-Length claim.

- Frontend media and gamelist entries follow the name a release *currently*
  publishes under. `evidence/` is append-only, so a release rebuilt or
  re-adopted under a corrected name keeps both names in its history — and the
  three functions that drove the frontend projection walked every record rather
  than the current one. So artwork was re-copied under the abandoned name on
  every run, one set per name the release had ever used, and the gamelist kept
  an entry pointing at a file that no longer existed: one game appearing twice
  in ES-DE, once playable and once dead. The rule deciding which build
  describes a file that should exist now already existed and already governed
  the database projection; it is now lifted to release level
  (`current_release_builds`) and used by all of them. Its counterpart names
  what to clean up, so stale media is deleted as projection runs and the
  retired gamelist entries are removed — a name another lineage still publishes
  under is never touched.
- Playables are named one way, whether or not their carrier resolved to a
  catalog medium. A bound carrier took the catalog's name (`Castlevania -
  Symphony of the Night (USA).chd`) and an unbound one was slugified
  (`castlevania-symphony-of-the-night-usa.chd`), so a library holding both read
  as two collections — and binding a carrier later silently changed what its
  playable was called. The archive's own title is the same title the catalog
  would have supplied, so both now produce the readable form, with the region
  written the way a DAT writes it (`(USA)`, not the lowercase `usa` records
  store for comparison). Existing playables keep their names: a present
  playable of the desired format is not a gap, so nothing is rebuilt or renamed
  behind you.
- Catalog names are now made safe to use as filenames before they reach the
  disk. Naming a playable after its DAT entry means writing a name a person
  wrote, and a name is not a filename: `Harvest Moon: Boy Meets Girl (Japan)`
  carries a colon, which is legal on macOS at the POSIX layer and **illegal on
  exFAT and NTFS** — so on the removable and Windows-shared drives collections
  actually live on, that build simply failed. A `/` would have been worse,
  silently meaning a directory. `retro_junk_archive::safe_file_stem` replaces
  only what has to be replaced — a colon becomes ` -`, matching the convention
  No-Intro and Redump already use for subtitles — rather than slugifying a name
  no frontend, scraper, or person asked to have mangled.

- Fixed archived multi-track discs being scraped under a member track's
  filename. The catalog stores a medium's largest *track* as its `rom_name` —
  by design, since that is where a multi-track disc's hashes live — and the
  name offered to the scraper's filename tier was taken from it unconditionally.
  So an archived disc was asked about as `Some Game (USA) (Track 2).bin`: a file
  that exists in no collection, naming a track rather than the disc. The same
  disc scraped from the library was asked about as its actual container, so the
  two surfaces asked one disc two different questions — and for a mod, whose
  identity is its parent's, the wrong name was the only name it had. A
  multi-track medium is now named by its DAT game name, which is the rule
  `whole_medium_stem` already applied to playable builds and renames. The size
  is dropped along with it: `romtaille` narrows a name match to files of that
  size, and the catalog holds no size for a whole medium — only for its tracks.
  Nothing else about the identity changed, because nothing else was wrong: a
  track's digests are real digests of a real file, and the serial names the
  disc.

- The review Inbox can be worked through when it has a thousand rows in it. It
  was built as a card per row, which is right for five and unusable at the size
  a real collection produces: the reference archive files over a thousand
  unaccounted playable files, dominated by two piles — stray text files, and
  GameCube dumps the archive cannot accept yet — that nobody is going to review
  one at a time. It now describes groups instead of listing rows.
  - **A filter on the path**, in the familiar glob shape: `*.txt`, `*/rvz/*`,
    `disc[0-9]`, or a bare word to match anywhere in a path. `*` deliberately
    crosses directory separators, so an extension-shaped and a folder-shaped
    description both work against one whole relative path. The matcher lives in
    one place (`retro_junk_io::glob`) and is used to decide what the list shows,
    what a bulk button will act on, and what the adoption sweep files in the
    first place — three answers that would be dangerous to derive separately.
  - **Bulk dismiss and bulk apply over the filtered set**, with the count in the
    button and in the confirmation. Dismissal closes review rows and touches no
    file; the exact rows it closed come back as an Undo, and re-running adoption
    would surface anything still unaccounted for anyway. The confirmation says
    both of those rather than implying them.
  - **Durable ignore rules**, because dismissal alone is a treadmill: the next
    sweep re-files everything you just closed. A rule describes a group by
    pattern, is consulted *before* the sweep hashes anything, and is stored
    beside the collection one file per rule — so it survives the database being
    rebuilt, travels with the files to another machine, and never produces a
    sync conflict. Revoking one makes the next sweep file those files again.
  - **Adoption reviews are resolvable.** They were the largest population in the
    Inbox and the only button that worked on them was Dismiss. A review now
    carries the candidates the sweep found and would not choose between — the
    archived masters with these exact bytes, or the catalogued media they match
    — and accepting one re-proves the claim from scratch before recording it, so
    a reviewed adoption is never weaker evidence than an automatic one.
  - **Groups, sorting, and one line per row.** Rows fold into piles by kind and
    platform; the default order is newest first, so a fresh arrival is not
    buried under hundreds of old rows already decided against. Arrow keys move,
    Enter applies, `D` dismisses.
  - Everything above is on `retro-junk suggestions` too — `list --kind/--match`,
    `dismiss --match --dry-run`, `reopen`, `ignore`, `ignores`, `unignore` — by
    the same calls the GUI makes, so a decision taken at the terminal and one
    taken in the Inbox are the same decision.
  - Two things that made the old view expensive at this size are gone: it asked
    the filesystem whether each row's file existed *while drawing*, once per row
    per frame, against a library that is often on a network share; and it laid
    out every card every frame. The list is now virtualized — only the rows
    inside the viewport are laid out, so a thousand-row backlog costs what a
    ten-row one does, and a test holds that line.

- Converging an archive stopped re-reproducing the same discs on every run.
  Identifying a Redumper raw master means copying the whole raw dump to scratch
  storage, running `redumper split` and `redumper hash` over the copy, and
  matching the regenerated tracks against the catalog — minutes and gigabytes
  per disc, because redumper writes its output beside its input and the archive
  is never handed to a tool that would write into it. Three things made that
  cost repeat. Failing to find a match left a dump looking exactly like one that
  had never been tried, so every run proposed a fresh reproduction for every
  disc the catalog cannot name; a concluded attempt is now recorded as its own
  state and left alone until the dump's bytes change. The split output was
  thrown away and re-derived by the build that followed, so a disc that was
  identified and then built paid the whole cost twice; both now go through one
  cache keyed on the dump's manifest hash, and the disc is split once. And a run
  over forty discs reported no position in its own queue, while each disc
  reported "0 of 1" for itself — so a long run was indistinguishable from one
  disc being reworked endlessly. Discs that were tried and matched nothing now
  show as `unresolved` in the backlog rather than dropping out of sight;
  re-running one after a catalog update is `archive redumper-audit`, which
  audits regardless of state.
- Progress reports say what they are counting. Every long-running operation now
  reports bytes or work items explicitly, so the CLI and the activity bar render
  "412 MB / 1.1 GB" or "3 / 10" from what the operation actually meant instead
  of inferring it. The old inference was wrong in the visible case: identifying
  one disc reports "0 of 1 dumps", which read as bytes rendered "0 B / 1 B"
  beside a bar that then sat still for the whole reproduction. The reproduction's
  own copy phase reports its byte progress through the same channel now, so the
  bar moves during the part that takes the time.

- Fixed `archive adopt-playable` dying partway through with `FOREIGN KEY
  constraint failed`. A carrier's manifest records which catalog medium it was
  archived against, but that id is minted by whichever DAT import was loaded at
  the time — so an archive built on another machine, or before a DAT update,
  routinely names media this catalog never created. Adoption handed that id
  straight to the database, which refused the row and took the whole run down
  with it, after real files had already been adopted. Half this collection's
  carriers named such an id, so the failure was near-certain. Every reference a
  binding is written with is now resolved against the table it points at before
  the row is stored: the carrier's own recorded medium wins (reindexing already
  re-derived that from digests for exactly this reason), and an id nothing holds
  is simply not written rather than fatal. A carrier the projection has not
  ingested yet binds nothing and says so, which reindexing then fixes.
- Scraping now knows what a mod is. A ROM hack's bytes exist only on the machine
  that made them, so no hash matches them and its filename names a game nobody
  catalogued — every scrape spent three requests proving that and left the game
  with no artwork at all, on every pass, forever. A file you have marked as a
  mod of something is now looked up **as that something**: it wears its parent's
  metadata and artwork while keeping its own name and its own media files, and
  its own digests are never offered to a catalog that has never held them. Its
  header serial *is* still offered, because a hack usually leaves the original
  header alone, and that serial identifies the parent exactly. Homebrew is
  looked up as itself, by name, with the serial suppressed: nobody assigned it
  one, so whatever sits in that field is a placeholder that can only match some
  commercial game by accident — and that accident would publish another game's
  box art into your archive under a homebrew title. "A mod of something" with no
  parent named is not an identity, so those files are skipped without spending a
  request rather than guessed at; the convergence derivation reports them as
  having no scrape identity instead of proposing work that cannot succeed. One
  rule, applied inside the shared scrape core, so `retro-junk scrape`, the GUI's
  artwork actions, and the unattended daemon cannot answer this differently —
  and a mod is only ever as trusted as the parent it resolved to, so a mod known
  only by its parent's name is filed for review rather than published
  unattended. A scraped mod says whose metadata it is wearing.
- Fixed marking a file as a mod recording no mod. The plain tag menu wrote a
  mark that named no parent — a decision nothing could act on and which the
  catalog rebuild would always defer — while the dialog that *does* ask which
  game you modded wrote no mark at all. Marks are now written where the tag is
  written, once, with the catalog at hand to name the parent by something that
  travels: its DAT game name, falling back to its canonical name. Tagging also
  binds the row to what it just created, so a mod is scrapeable as its parent
  the moment you say so rather than after the next full reconcile. A row with no
  digests yet is still tagged, but the dialog now says why the decision cannot
  travel yet — content is the only identity these files have.
- Fixed the archive and the reconciler disagreeing about where marks live. Two
  copies of "which directory holds this collection" had already drifted: one
  answered the archive root, the other its parent. On the usual sibling layout
  they agree, so the bug was invisible — anywhere else, marks were written to a
  directory nothing read them back from. There is one rule now.
- Added portable collection marks, so "this is homebrew" and "this is a mod of
  X" survive moving between machines. Those decisions used to live only in the
  device-local catalog database, which is rebuilt from DATs — so every device
  you tested on had to be told again. They now live beside the collection, in
  `<collection>/.retro-junk/marks/`, as one file per mark named by content
  digest. That shape is deliberate: two machines marking different games never
  touch the same file, so Syncthing has no conflict to raise and rsync has
  nothing to clobber, and the same decision made twice is byte-identical, so
  copies converge instead of fighting (which is why no timestamp is recorded).
  A mark carries the *inputs* catalog ids are minted from, never the ids —
  those are per-DAT-release and do not survive a re-import — so a mod names its
  parent by DAT game name and simply waits, keeping the decision, on a machine
  whose catalog does not have that DAT yet. Marks are applied wherever the
  projection is rebuilt.

- Fixed `complete_track_set` meaning two different things, in both directions.
  Verifications written by the older single-file path left the flag `false`
  even for cartridges — 244 records on the reference archive — and because the
  flag gates catalog-verification entirely, those dumps read as unverified,
  which blocked carrier re-resolution and hash adoption behind them. A `rom`
  single-file master is one file and one "track", so a verified match against
  it was always the complete set; that is now derived from the dump's shape,
  which rewrites no evidence at all. It stays narrow on purpose: a single-file
  `iso` is excluded, because a data-track-only image of a multi-track disc is
  exactly the case the flag exists to catch. The *writer* had the opposite bug,
  recording `true` unconditionally on that path — so one file matched against a
  multi-track medium on its primary digests claimed a completeness nobody
  verified. Match results now carry whether the medium has separate tracks, and
  the flag reflects it. Carrier re-resolution also learned the cartridge case,
  which records no per-track digests and must match on the dump manifest's
  recorded file digests instead. Unresolved carriers on the reference archive:
  149 → 35.
- Stopped asking to re-read disc images the archive had already measured. A
  library row could adopt its digests from the archive only when the playable
  was a *byte-identical mirror* of a single-file master, which a CHD of a
  multi-track disc can never be — so disc rows kept advertising that they
  needed hashing even though the archive held the answer. A derivative whose
  build was round-trip verified (decompressed and compared back against the
  master) and whose master's complete track set matched a catalog medium holds
  the catalog's bytes by construction, and now adopts them. Both flags are
  required; either alone would be a guess. Rows filled this way still record
  `hash_source='archive_evidence'`, so they read as a cache of what the archive
  proved rather than a claim that these bytes were read here.
- Stopped `archive adopt-playable --dry-run` taking the archive write lock. A
  dry run writes nothing, but it held the lock for the whole sweep — long
  enough, over a network mount, to look like a wedged archive to anything else
  that wanted it.
- Fixed a busy-lock message that read as a wedged archive. The lock record
  stores RFC 3339 UTC and the message printed it raw, so a lock taken 34
  minutes earlier reported "started_at=…T02:06" to a reader nine hours ahead of
  UTC. It now reports elapsed time and the holding PID — `held 34m by pid
  53769` — which needs no timezone to interpret, on both the OS-lock and
  existence-fallback paths, which had drifted to different formats.
- Fixed adoption wanting to rewrite evidence for outputs that had not moved.
  "Where does this build's output live" had two implementations: the projection
  followed a recorded path into the frontend's system directory, the archive's
  orphan scan checked the recorded path alone. Evidence written before outputs
  were filed under a platform directory records a bare file name, so on the
  reference archive 248 outputs were `present` to one and `missing` to the
  other — and an adoption run would have appended a redundant evidence record
  for every one of them. Both now resolve through
  `retro_junk_archive::resolve_playable`, with the frontend system directory
  passed in by the only layer that knows it. The same sweep now proposes 1
  genuine move instead of 249.
- Adopted playable files the pipeline never built. A collection assembled
  before the archive existed is full of them: a CHD sitting beside a
  preservation master of the same disc, with nothing connecting the two, so the
  game showed as archived-but-not-playable next to a playable-but-not-archived
  row. Neither existing adoption path could reach it — one searches for a
  recorded output digest that does not exist, the other can only match an
  uncompressed mirror of the master. The proof was already on both sides: a
  catalog verification records the dump's complete ordered track set, and the
  library records each disc image's data-track digest. When those agree, the
  file is that carrier's derivative and build evidence now says so.
- Fixed carrier catalog bindings not surviving a re-import on another machine.
  A media id encodes the DAT release it was minted against, so an archive
  written from one host binds carriers to ids a differently versioned import
  never creates — 201 of 248 carriers on the reference archive read as
  `unresolved`, which hid every catalog-derived binding behind them. The
  projection now re-resolves such a carrier from the track digests the archive
  itself recorded, and reports the binding as `rederived` rather than
  pretending the recorded id resolved.
- Fixed catalog matching refusing a digest it simply did not have. The single-
  file matcher required every digest the *catalog* held to be matched by the
  caller, so a caller carrying only SHA-1 — which is all the archive's track
  evidence records — could never match a medium that also had a CRC-32. An
  absent digest on either side now means "not available to compare"; both sides
  must still bring at least one.
- Fixed a rename discarding the identity it had just established. Library
  entries are keyed by path, so renaming a file and rescanning looked exactly
  like one file vanishing and another appearing: the row came back with no
  digests, no DAT match, and no identification, asking for a re-read of bytes a
  rename cannot change. Identity now follows the file, for single files and for
  multi-disc `.m3u` folders alike.

- Stopped naming a whole disc after one of its own tracks. A Redump entry for
  a multi-track disc lists one ROM per *member file*, so DAT import stores the
  largest track's filename in `Media.rom_name` — and both playable builds and
  library renames took that stem for a container holding the entire disc,
  producing artifacts like `Tenchi Muyou! Ryououki Gokuraku CD-ROM for Sega
  Saturn (Japan) (1M) (Track 1).chd`. The wrong stem then propagated to
  scraped media and the frontend entries derived from it. A whole-medium
  container now takes the disc-level DAT game name; single-file media keep the
  ROM name, which is the only thing distinguishing an N64 `.z64` from its
  `.v64`. The rule lives once, in `retro_junk_dat::tracks`. Along the way this
  fixed game names containing a period (`Dr. Mario (USA)`) being truncated at
  it, because a DAT game name was being run through `Path::file_stem`.
- Fixed a renamed playable orphaning its build evidence, which split one game
  into an "archived only" row beside a "playable only" row for the same bytes.
  The recorded output path is what binds a scanned library row back to the
  carrier that produced it, and nothing re-adopted a moved file. Moved outputs
  are now found again by their recorded SHA-256 and the new location is
  appended as build evidence, so the repair happens once instead of on every
  projection — and a deliberate rename finally tells the archive about itself.
  This runs as its own convergence stage *before* builds, so a file that
  merely moved is never rebuilt beside the copy the library already holds, and
  it runs unattended regardless of the automation policy: every other switch
  asks permission to *produce* something, while adoption only corrects where
  the archive believes an existing file is. So renaming a game in the GUI now
  picks the canonical name and re-attaches itself to its archived release
  without any further action.
- Fixed a superseded build leaving its old output path behind forever. Build
  currency was keyed on the output path, so a rebuild (or an adoption) that
  wrote a *different* name left the previous path projecting as a permanently
  missing playable, which kept deriving work for a release that was already
  whole. Currency now follows the build lineage — what a derivative was built
  from, and the format it produced — and superseded records stay in the
  archive as history, as before.

- Fixed a copy of an archive being invisible on the machine that copied it.
  Archive identity is recorded in the portable root manifest, but a profile
  created for an existing archive minted a *fresh* id instead of adopting it,
  while the rebuildable SQLite projection is keyed on the manifest's id. So
  pointing a new profile at an rsynced collection reindexed 241 releases and
  then reported "No archived releases are indexed yet", because the UI queried
  an id the reconciler never wrote. Profiles now take their identity from the
  archive they point at, existing profiles whose id has drifted re-adopt it on
  load (CLI, daemon, and GUI alike, so no config surgery is needed), and
  opening the same archive at a second mount re-points the profile it already
  has rather than adding a rival one.
- Fixed one rebuilt playable making the whole archive unprojectable. Build
  evidence is append-only, so rebuilding a derivative in place (a newer
  chdman, a changed recipe) leaves two records naming the same output path —
  but a representation row is the *current* state of one file, and the
  projection admits one row per path, so reindexing aborted with `UNIQUE
  constraint failed: representations.location_role, representations.relative_path`
  and left no archive index at all. The newest build for each output is now
  projected, superseded records are logged rather than silently dropped, and
  every record stays in the archive as history.
- Stopped macOS AppleDouble sidecars from being treated as collection content.
  Copying a library onto exFAT, FAT, or SMB leaves a `._<name>` file beside
  every file carrying extended attributes, and those sidecars keep the
  extension they shadow — so each real game scanned as a phantom twin, and
  inside a preservation dump each one read as a file the manifest never
  recorded, reporting a healthy mirror as an integrity failure. Library
  scanning, archive verification, and ingest now share one rule for host
  filesystem metadata (`._*`, `.DS_Store`, `Thumbs.db`, `desktop.ini`).
- Fixed no playable being buildable from a Redumper raw master held on exFAT
  or SMB. The same `._` sidecars reached two paths the rule had not been
  applied to: staging copied them into the scratch workspace, and the split
  step names the image after the first `.scram`/`.scrap`/`.sdram`/`.sbram` it
  finds — `._disc.scram` sorts ahead of `disc.scram`, so redumper was pointed
  at a 4 KiB resource fork, read garbage track geometry out of `._disc.toc`,
  and failed with `error: unable to establish base LBA`. Every PS1 disc
  archived as a raw master was affected, and neither the dump nor the redumper
  version was ever at fault. Package staging and Redumper file discovery now
  go through the same host-metadata rule, so image-name discovery, log
  collection, and intermediate retention all see only dump content.

- Fixed every archive write failing on an SMB share. macOS smbfs answers each
  directory `fsync` with `ENOTSUP` (verified empirically), and the atomic
  manifest write treated that as a failed write — so an import published its
  package, then rolled the whole release back, leaving no trace but a touched
  directory. Flushing a directory is now a durability hint: refusal by the
  filesystem is accepted, genuine I/O failures (`ENOSPC`, `EIO`) still fail,
  and a completed publish is never rolled back because the hint failed.
- Made whole-archive indexing about 6× faster over a network share (241
  releases: 15.3 s → 2.5 s warm, 23.8 s → 5.1 s cold). Scanning read every
  manifest twice — once to parse, once to digest — and walked releases one at
  a time, so the cost was almost entirely round-trip latency. It now reads
  each manifest once and scans releases concurrently.

- Fixed the Library listing the same playable file twice — once inside its
  archived release and again as an unarchived "playable only" row. A playable
  belongs to the archived carrier whose build evidence produced it, but the
  binding was keyed on that carrier's *catalog medium*, so an archive that is
  unbound, on a platform whose DAT was never imported, or bound to a catalog id
  a later import re-slugged could not own its own playable. Bindings are now
  keyed on the carrier (schema v25, re-derived in place on upgrade), and a
  multi-disc row now owns every archived disc image inside the directory it
  stands for instead of only an exact file-name match.

- Stopped re-reading files for hashes the archive already recorded. Dump
  manifests carry CRC32/MD5/SHA-1 beside SHA-256 for every archived file, but
  the projection kept only SHA-256; it now carries all four, and a library row
  holding a byte-identical mirror of a single-file master is filled from that
  record and named from the catalog medium those digests identify — no second
  read of the file. Adoption requires the recorded digests to match exactly one
  catalog medium (the archive stores raw digests while the library hashes
  format-aware payloads), and such rows record `hash_source='archive_evidence'`
  so a later local hash pass replaces them with digests actually read here.
- Made platform names separator-insensitive, so archive and frontend directory
  spellings (`super-famicom`) resolve to the same platform as the spaced alias
  instead of failing to parse and splitting one platform in two.
- Made library identity survive a machine without a catalog: the archive's own
  catalog verification records which game a dump matched, so a scanned playable
  file whose build evidence points at a current, catalog-verified dump is now
  named and shown as verified (match method `archive evidence`) even when no DAT
  has ever been imported locally. A live catalog hash comparison still wins, and
  user tags are never overwritten.
- Fixed playable relocation losing the file name when build evidence recorded a
  bare output path (written before playable outputs were filed under a platform
  directory): the projection replaced the *file name* with the platform
  directory, so present files were projected as missing and never bound to their
  library rows.

- Made Library archive state release-aware: catalog-analysis bindings now
  connect CHD/M3U entries to preservation carriers, incomplete archives remain
  visibly incomplete, and one action verifies and builds every missing disc
  before projecting a multi-disc M3U. Existing loose playable discs can be
  consolidated by a release playlist without recompression or duplicate rows.
- Reused byte-progress-reporting local Redumper staging between catalog
  verification and CHD creation, avoiding a second archive read over the
  network.
- Added a portable preservation archive with release, physical-copy, carrier, dump, representation, verification, and derivation identities.
- Added catalog-driven `archive import` and a blocking GUI import dialog that discover serial-named dump folders, hash and identify packages, resolve physical copies, retain sources by default, and optionally remove sources only after verification.
- Added `archive import-playable` and matching GUI workflow to promote existing loose-ROM libraries into preservation masters while adopting the original files as byte-identical playable representations. Cartridge matching applies platform-aware header removal and byte-order normalization without changing archived source bytes.
- Added verified atomic ingest that retains source files and rejects symlinks and traversal.
- Added separate archive, playable, and scratch roots through collection profiles.
- Added persistent per-carrier desired playable policies, including retain-intermediate and unverified-build controls.
- Added append-only integrity, Redumper reproduction/catalog, and CHD build evidence.
- Added raw Redumper auditing from disposable copies with complete-track catalog matching.
- Added catalog-gated CHD derivation with mandatory chdman round-trip verification and an explicit unverified opt-in.
- Added a rebuildable SQLite archive projection and release-centric Collection GUI view.
- Added Library availability states for playable-only, archived-and-playable, archived-without-playable, and non-preferred playable formats; per-console playable defaults; and an in-app queue that creates byte-verified cartridge mirrors or round-trip-verified CHDs from preservation masters.
- Made per-console playable-policy changes update only the root manifest and affected SQLite policy rows instead of rescanning and rebuilding the entire archive projection.
- Limited the blocking startup modal to actual catalog location/schema migrations; routine archive reconciliation and saved network-root probing now continue in tracked background work, and ordinary index refresh no longer scans the archive twice.
- Filtered console-default and per-copy preferred playable formats to the conservative set accepted by mainstream emulators, while retaining unsupported legacy selections until explicitly changed.
- Moved the durable catalog database from the cache directory to the platform data directory with validated first-run migration.
- Added policy-driven resumable CHD/RVZ/mirror builds, retained canonical intermediates, multi-disc playlist projection, and explicit integrity/reproduction/catalog/round-trip evidence.
- Added general CRC32/MD5/SHA-1 catalog verification, per-device present/missing/partial/modified/stale state, legacy playable adoption with an Inbox, and recoverable archive locking/staging recovery.
- Added authoritative ScreenScraper supporting-file adoption and frontend projection, plus physical-copy photo/provenance/document manifests and GUI provenance editing.

## 0.3.0

- Made SQLite the authoritative GUI library store, with durable entry IDs,
  command-first edits, catalog-backed matching, and automatic migration and
  repair of existing libraries.
- Moved startup loading, scans, filesystem refreshes, hashing, and media
  discovery off the UI thread while keeping list metadata stable as entries
  are selected.
- Bounded GUI list projections and media memory use: list rows retain only
  lightweight asset-presence state, while image data is loaded and retained
  only for the currently focused detail view.
- Tailored game-list columns to each console's identification capabilities so
  unsupported serial, internal-name, region, and DAT fields are omitted.
- Fixed intermittent debug-build red outlines while scrolling large,
  virtualized game lists without disabling true same-frame ID collision
  warnings.
- The main GUI release now includes full Japanese, Chinese, and Korean font
  support; the separate `retro-junk-gui-cjk` release variant was removed.

## 0.2.0

- CUE sheet compatibility issues are now detected during scan and displayed as warning triangles in the game table and detailed messages in the detail panel, with clear "fixable" vs "re-dump required" messaging
- Added `retro-junk fix-cue` command to detect and convert CDRWin-format CUE sheets to standard CUE format for wider emulator compatibility (e.g., DuckStation rejecting `CD_ROM_XA` headers)
- Added `retro-junk systems` command listing all 25 supported systems with DAT/GDB capability tags, grouped by manufacturer, with optional `--manufacturer` filter
- Multi-system database commands (`catalog import`, `catalog enrich`, `catalog enrich-gdb`) now default to all systems when no arguments are given (was: "No systems specified")
- Unified system name validation across all commands into shared helpers (`resolve_systems`, `resolve_single_system`, `resolve_platform_ids`), replacing ~120 lines of duplicated ad-hoc logic
- All "unknown system" errors now consistently suggest `retro-junk systems` for discoverability
- `catalog gaps` now validates the system name (was: passed raw string to DB with no check)
- Updated help text on all system-accepting commands with examples and `retro-junk systems` hints
- Added Sega Saturn disc identification (ISO, BIN/CUE, CHD) with serial, region, and game name extraction
- Added CHD support for Saturn disc images (all compression codecs supported)
- Added `Region::Asia` and `Region::LatinAmerica` to the region enum
- Added `saturnjp` folder alias for Saturn
- Extracted shared disc utilities into `retro-junk-disc` crate for reuse across Sony, Sega, and future disc-based consoles
- Hardened ROM/disc parsing against malformed input
- Fixed WiiU DAT source, ClrMamePro size parsing, and miximage panic
- Added GUI keyboard navigation (arrow keys, Home/End, Page Up/Down, Ctrl+1/2/3 view switching, Shift+arrow selection)
- Fixed background operations (scan, hash, scrape, rename) targeting the wrong entry when the list changed mid-operation
- Fixed multi-disc `.m3u` folder scanning miscounting discs
- Added in-app log viewer and error dialogs for failed operations
- Added right-click "Copy" context menu to all value labels in the GUI detail panel
- Fixed CHD hashing using hardcoded 2448-byte sector stride instead of the CHD header's actual `unit_bytes`; CHDs without subchannel data (SUBTYPE:NONE) use 2352-byte sectors, causing wrong hashes for Saturn and other disc platforms
- Replaced hardcoded sector size literals across disc, Sony, and Sega crates with shared constants (`ISO_SECTOR_SIZE`, `RAW_SECTOR_SIZE`)
- Restructured `cache` subcommands: `cache list/clear/fetch` and `cache gdb-list/gdb-clear/gdb-fetch` are now `cache dat list/clear/fetch` and `cache gdb list/clear/fetch`
- Added `--force` flag to `cache gdb fetch` (skips re-download when cached, matching `cache dat fetch` behavior)
- Removed `config` alias from `credentials` command (conflicted with `settings`)
- Removed `--root` alias from `--library-path`
- Fixed hash matching for BIN dumps where audio tracks were written as zero-filled Mode 2 sectors instead of raw PCM. A secondary boundary detection now finds the data/filler boundary and warns about the incomplete dump.

## 0.1.2

- Added GUI to cargo-dist releases with per-platform builds (macOS, Linux, Windows)
- Added separate `retro-junk-gui-cjk` download variant with full CJK font support (~16MB larger); base `retro-junk-gui` ships without CJK fonts for a smaller download
- Added GameCube and Wii disc identification with RVZ/WBFS/CISO/GCZ compressed format support
- Added PS2 disc identification and hashing
- Added initial database viewer in GUI Tools view for browsing platforms, works, and releases
- Added `works_for_platform` query to catalog database
- Fixed GUI renames losing file extensions (e.g., PS2 `.iso` becoming `.bin`, GC `.rvz` becoming `.iso`) by centralizing extension handling in a single `target_filename_for_rename()` function used by both CLI and GUI
- Fixed auto-correction of previously damaged file extensions: renames now detect the actual file format at rename time, so misnamed files (e.g., RVZ named `.iso`) get the correct extension
- Fixed compressed disc analysis (RVZ, WIA, etc.) failing silently when `file_path` was missing from `AnalysisOptions` — affected both CLI serial matching and GUI format detection
- Fixed hashing of compressed GameCube/Wii disc images (RVZ, WIA, WBFS, CISO, GCZ) to decompress before hashing for correct Redump DAT matching
- Fixed DAT download URLs for GameCube, Wii, and PS2 (was requesting wrong filenames from LibRetro GitHub)
- Fixed serial matching for Redump product codes (e.g., `DL-DOL-GBIE-0-USA` now matchable by 4-char game code)
- Fixed disc-based games reverting to "Ambiguous" status after rescan
- Fixed "Ambiguous" status showing no explanation in GUI detail panel
- Refactored hashing code to share disc-hashing logic across PS1 and PS2

## 0.1.1

- Set up automated GitHub releases via cargo-dist
- Updated README with install instructions and current command reference
- Embedded ScreenScraper dev credentials in release builds

## 0.1.0

- Initial release
- ROM analysis with header parsing for NES, SNES, N64, GB, GBA, DS, 3DS, Genesis, PS1
- Rename ROMs to canonical No-Intro / Redump names via serial or hash matching
- Scrape metadata and media from ScreenScraper (covers, screenshots, videos, marquees)
- ES-DE frontend output (gamelist.xml)
- DAT file caching from No-Intro and Redump
- Multi-disc game support via .m3u folders
- Catalog database with enrichment from ScreenScraper and GameDataBase
- GUI with library management (early)
- 23 consoles across Nintendo, Sony, Sega, and Microsoft
