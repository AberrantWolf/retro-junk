# Consolidation Plan: One Backend, One Truth, One Completion

This is a full consolidation, not a cleanup. Nothing survives for compatibility's
sake. Every capability lives in exactly one place — the backend — and the CLI and
GUI become thin clients that command it and render its answers. Anything that
duplicated, apologized for, or worked around the old split gets deleted.

---

## 1. The target architecture

```
                 disk (authority for user state)
   archive manifests + evidence + playables + marks + settings
                          |
                    retro-junk-backend          catalog.db (derived from DATs,
   scan / ingest / verify / identify / bind /   keyed by content hash; lookup only)
   build / rename / scrape / converge / query
        |                        |
   retro-junk-cli          retro-junk-gui
   (parse args, call,      (call backend in-process, keep it
    print)                  resident, render its state)
```

**One rule:** if the CLI and the GUI can both do a thing, they do it by calling
the *same backend function*. Neither frontend opens a database connection, walks
a filesystem, computes a hash, derives a name, or decides a status. Ever.

The GUI's only privilege is keeping the backend alive between interactions, so
the backend's in-memory view of the world stays warm and queries are instant.
The CLI constructs the same backend, uses it once, and exits. Same code path,
different lifetime.

### The three stores and what each is allowed to hold

| Store | Holds | Authority? |
|---|---|---|
| **Disk** (archive tree, playable tree, marks) | Everything the user owns or created: dumps, manifests, verification evidence, playables, tags/overrides/ignore rules as portable mark files | **Yes — the only authority for user state** |
| **Catalog DB** (SQLite) | What DATs and scrapers say: works, releases, media, hashes, track digests, assets. Rebuilt by re-import at any time | Authority only for "what does the world call this hash" |
| **Projection** (in-memory in the backend, snapshotted to SQLite) | A queryable index of what's on disk, joined to the catalog | **Never.** Deleting it loses nothing. It is rebuilt from disk + catalog, and it must be *faithful* — it records what disk says even when it can't resolve it |

The projection snapshot carries a single schema-version integer. On mismatch it
is dropped and rebuilt from disk — **no migrations, ever**. Migration arms exist
only for the catalog DB, and even those shrink to "re-import your DATs."

---

## 2. The backend crate

Create `retro-junk-backend`. It absorbs, wholesale:

- `retro-junk-gui/src/backend/` — all 17 modules (scan, hash, rename, organize,
  fix_cue, chd_compress, archive, assets, catalog_ops, convergence, daemon,
  export, inbox, playable_build, worker, library_store). These stop being
  GUI-private and become the *only* implementation.
- `retro-junk-work` — daemon, executor, watcher, adoption, suggestions, policy.
  The daemon is just the backend running with a schedule; it is not a separate
  capability tier.
- The orchestration glue currently split between `retro-junk-lib` high-level ops
  (`archive_ops`, `playable_build`, `rename`, `organize`, `repair`) and the CLI
  command bodies. `retro-junk-lib` shrinks back to what its name promised:
  analyzers, hashing, matching primitives.

The backend exposes two surfaces:

- **Commands** — imperative, idempotent operations: `scan`, `ingest`, `verify`,
  `identify`, `bind`, `build_playable`, `rename`, `organize`, `scrape`,
  `converge`, `import_catalog`. Each returns a typed result and emits progress
  events on a channel (the GUI renders them live; the CLI prints them).
- **Queries** — read-only questions answered from the projection: entry lists,
  release detail, gaps, suggestions, status rollups. All queries return the
  *same typed structs* to both frontends. There is no CLI-shaped answer and no
  GUI-shaped answer.

Every command re-projects the slice of disk it touched before returning, so a
frontend never sees a stale answer after its own action. The watcher and the
scan command feed the same re-projection path — there is one way state enters
the projection.

---

## 3. One completion model

Today there are nine overlapping notions of done-ness (`EntryStatus`,
`DiscVerification`, `AssetStatus`, `RepresentationPresence`, evidence
kinds/outcomes, `dump_events` state strings, summary counters, two disagreeing
disc-count SQL queries, `EvidenceLevel`). All of them are replaced by **one
struct, computed by one function, rendered everywhere**.

```rust
/// Computed by exactly one function in the backend, from disk facts + catalog
/// facts. Every icon, badge, table cell, detail row, and CLI status line is a
/// rendering of this struct. Nothing else may derive a status.
pub struct Completion {
    pub identity: Identity,        // how we know what this is
    pub presence: Fraction,        // dumps on disk vs. expected
    pub integrity: Fraction,       // dumps whose bytes verify vs. present
    pub catalog: Fraction,         // discs matching the catalog vs. expected
    pub playable: Fraction,        // playables built & current vs. desired
    pub artwork: Fraction,         // assets present vs. expected
    pub attention: Vec<Attention>, // actionable problems, each naming its fix
}

pub enum Identity {
    Bound { release_id: ReleaseId },      // catalog-verified identity
    BindingUnresolved { claimed: ReleaseId }, // disk claims an ID the catalog
                                          // lacks — "re-import catalog", NOT
                                          // silently unbound
    Named { name: String },               // serial/header evidence only
    Unknown,
}
```

Rules the struct enforces by construction:

- **A missing denominator is never `0/0`.** `Fraction` is
  `Known { have, want } | Unknown(reason)`. "Verified discs: 0/0" becomes
  impossible; the UI renders the reason ("not catalog-bound — identify to set
  an expectation") because that's what the value *is*.
- **The overall icon is a fold of this struct**, defined once next to it. Gray
  means "identity unknown," never "denominator happened to be zero." Green
  evidence with a gray icon cannot occur, because both render the same value.
- **`existing_id()`-style silent NULLing is deleted.** When `release.toml`
  claims a binding the catalog can't resolve, the projection stores
  `BindingUnresolved` with the claimed ID intact. The old behavior — erasing
  the disk's claim because the DB didn't recognize it — is exactly the
  projection lying about disk, and it dies.
- The two competing disc-count SQL definitions
  (`ARCHIVE_RELEASE_COMPLETENESS_SQL` and the gap query's `copy_counts`) are
  both deleted. Completion is computed in Rust from the projection by the one
  function; SQL stores facts, it does not define semantics.

---

## 4. One naming rule

One function, in the backend, is the entire naming law:

```rust
/// The one place a canonical playable/library name is derived.
/// Everyone — the builder, the rename planner, the conformance checker,
/// the M3U writer — calls this. Nobody else concatenates name parts.
pub fn canonical_name(catalog: &Catalog, release: &BoundRelease, disc: Option<DiscNo>) -> CanonicalName;
```

- The three current layers (`playable_output_stem` in `playable_build.rs`,
  `target_filename_for_rename` in `rename.rs`, the archive-manifest fallback in
  `release_output_name`) collapse into it. The archive-title fallback survives
  only as the explicit answer for `Identity::Named` — clearly labeled
  provisional, never silently mixed with DAT naming.
- **Name conformance becomes a standard convergence check**: for every built
  playable, compare its on-disk name to `canonical_name`; a mismatch yields
  `Attention::StaleName` with a one-click/one-command `rename` fix that moves
  the file, companion assets, playlist entries, and evidence in one atomic
  operation. Your old wrongly-named playables surface immediately and repair
  trivially — because there is finally one definition of "the right name."
- `AdoptPlayable` (relocation by hash) stays; it now feeds the same repair
  path instead of its own bookkeeping.

---

## 5. Binding: hash-keyed and format-blind

- **Catalog IDs stop being title slugs.** IDs derive from content identity
  (track-set digest for discs, file digest for cartridges), with title stored as
  a display attribute. A DAT retitle no longer re-keys the catalog and orphans
  every binding on disk. One-time catalog rebuild; archive manifests re-resolve
  by hash automatically.
- **`identify` works on every dump format.** The `RedumperRaw`-only filter in
  `identify_archived_carriers` is deleted. ISO, CHD, BIN+CUE, RVZ all bind
  through the same hash paths the analyzers already support (decompressing to
  the community-standard bytes first, per the existing rule).
- Binding evidence strictness is **kept** — full track-set or full-file digest
  agreement. We consolidate the plumbing, not the rigor. Serial/header matches
  produce `Identity::Named`, clearly weaker, never a bind.

---

## 6. Ingest records what it proved

`ingest` already re-reads and SHA-256-verifies every published byte. It now
writes `Integrity` verification evidence for that work, and `Catalog` evidence
when the import carried a confirmed catalog match — same evidence format the
explicit verify path writes. Convergence stops re-proposing a re-hash of bytes
verified seconds ago. "Ingest of dumps should be writing the verification" —
yes; it becomes structurally true because ingest and verify share one
evidence-writing function.

---

## 7. Library entries stop being database-only truth

`library_entries` as authoritative SQLite state is abolished.

- **Facts about files** (hashes, disc verification, DAT match) become projection
  rows recomputed from disk + catalog. Hashing is expensive, so hash results are
  cached *keyed by content fingerprint* (size + mtime + path) — a cache entry
  that no longer matches disk is simply dead, never "stale but trusted."
- **User-owned state** (tags, region overrides, ignore rules) moves fully to
  portable on-disk marks — the mechanism `record_entry_mark` already prototyped.
  An external rename can no longer destroy a tag, because the tag travels with
  the files and re-attaches by content hash.
- The legacy `collection` table, the JSON cache migration in `gui/cache.rs`, the
  name-only console fingerprint, `rekey_library_entry`'s silent no-op, and the
  side-channel DB connections opened by the scan worker and rename path are all
  deleted. There is one projection, fed by one scan path, through one store.

---

## 8. The frontends

### CLI
Each command becomes: parse args → call backend → print typed result. Command
bodies with embedded logic (the ~1,800-line `archive.rs` especially) are gutted.

Killed as redundant: `cache` (the projection self-manages; catalog re-import is
the only cache-ish verb left), duplicate status/query variants that answer the
same question differently than the GUI, and any flag that existed to work
around stale projections (`--force`-style re-verify stays, as an honest verb).

### GUI
Every view renders backend query results and issues backend commands. All 17
`gui/src/backend/*` modules are deleted (moved, but *deleted from the GUI* —
the GUI crate ends up with no `backend/` directory at all).

**Promoted to the GUI** (previously CLI-only): catalog import, analyze/deep
inspection, repair, compress, fix-cue, credentials, full daemon control,
settings that only the CLI could set. If the backend can do it, both frontends
can. The GUI additionally shows `Attention` items (stale names, unresolved
bindings, missing evidence) as a first-class list with fix buttons — the
convergence system finally has one visible face.

Confusing GUI-only query paths (the collection view's parallel "Catalog N"
counter that disagreed with "Verified discs", the three renderings of
denominator-zero, the `library_store` request/reply protocol) die with the
modules that produced them.

---

## 9. The kill list

Deleted outright, no shims, no deprecation period:

1. `ARCHIVE_RELEASE_COMPLETENESS_SQL` and `copy_counts` — replaced by the one
   completion function.
2. `existing_id()` NULL-erasure of manifest bindings.
3. `EntryStatus` (and its overload as archive completeness), `DiscVerification`,
   `AssetStatus`, `dump_events.integrity_state`/`catalog_state` strings,
   `EvidenceLevel` — replaced by `Completion`.
4. Title-slug catalog IDs.
5. The `RedumperRaw`-only identify filter.
6. `library_entries` as authoritative store; the legacy `collection` table;
   `gui/cache.rs` JSON migration; name-only console fingerprint.
7. All three naming-rule layers, replaced by `canonical_name`.
8. Every `gui/src/backend/` module (absorbed), the `library_store`
   worker/protocol, side-channel DB opens.
9. Projection-table migration arms — replaced by drop-and-rebuild on version
   mismatch.
10. CLI `cache` commands and duplicate query surfaces.
11. All UI strings that rationalize broken state ("Not catalog-bound;
    completeness is unknown", "Unknown (not catalog-bound)", bare "0 / 0") —
    replaced by renderings of `Identity`/`Fraction` that name the fix.

---

## 10. Work packages

Order is flexible; grouping is by what must land together to avoid a
half-consolidated state.

- **WP1 — Backend crate skeleton.** Create `retro-junk-backend`; move
  `retro-junk-work` and `gui/backend/*` into it; define the command/query API
  and progress-event channel; port CLI `sync`/`status` and the GUI library view
  to it as the proving pair.
- **WP2 — Completion.** Implement `Completion` + the one computing function;
  delete both SQL definitions and all nine status types; re-render icon, badges,
  detail panel, collection column, CLI status from it.
- **WP3 — Faithful projection.** In-memory projection with versioned SQLite
  snapshot, drop-and-rebuild, `BindingUnresolved` preservation, re-project-after-
  command, single scan path, fingerprint-keyed hash cache.
- **WP4 — Identity.** Hash-keyed catalog IDs + one-time catalog rebuild;
  format-blind identify; ingest/import evidence writing.
- **WP5 — Naming.** `canonical_name`; stale-name convergence check + atomic
  rename repair; delete the three old layers.
- **WP6 — Library demotion.** Marks for all user-owned state; delete
  `library_entries`-as-truth, `collection`, cache migration.
- **WP7 — Frontend thinning + promotion.** Gut CLI command bodies; delete GUI
  backend dir; promote CLI-only features into GUI; Attention list UI; delete
  redundant commands/queries.

Each WP ends with the old path *gone*, not disabled.

---

## 11. How this keeps the codebase DRY

This plan is DRY-as-architecture rather than DRY-as-cleanup: every category of
logic ends with exactly one implementation and one call site per frontend.
Completion semantics: one function (was nine types + two SQL queries). Naming:
one function (was three layers). Scanning/projection: one path (was
GUI-worker + daemon-watcher + side-channel connections). Evidence writing: one
function shared by ingest, verify, identify, and build. Command execution: one
backend body shared by CLI, GUI, and daemon (was three parallel orchestrations).
Duplication cannot re-enter easily because the frontends no longer have the
dependencies needed to reimplement anything — the GUI crate ends up unable to
open a database or hash a file even by accident.

## 12. How this maintains and improves best practices

- **Single source of truth, honestly enforced:** disk is authoritative, caches
  are faithful and disposable, and no code path may erase or reinterpret what
  disk says (the `BindingUnresolved` rule makes lossy projection a type error,
  not a code-review hope).
- **Make illegal states unrepresentable:** `Fraction::Unknown(reason)` and
  `Identity` turn the gray-icon/green-badge/0-of-0 contradiction into something
  the type system cannot express.
- **Separation of concerns by crate boundary:** frontends physically lack the
  capability to grow logic, which is stronger than convention.
- **Idempotent, evidence-producing operations:** every command can be re-run
  safely and leaves portable proof of what it verified, preserving the
  archive's existing "evidence on disk" discipline and extending it to ingest.
- **Deletion over deprecation:** no dual paths means no drift, no
  half-migrated states, and tests that assert the one real behavior instead of
  reconciling two.
- Existing preservation rigor (full-digest binding, decompress-before-hash,
  portable TOML/evidence) is retained unchanged — the consolidation removes
  duplication and dishonesty, not strictness.
