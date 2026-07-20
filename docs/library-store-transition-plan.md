# SQLite-authoritative library browser plan

## Goal

Make SQLite the authoritative persistent model for a scanned ROM library. The
GUI should retain only disposable read models and interaction state; it must
never be a second mutable copy which is saved back wholesale at exit or root
switch.

```text
filesystem operation / scan result
        -> library-store command commits one SQLite transaction
        -> LibraryChanged { affected IDs, revision }
        -> invalidate affected GUI projection(s)
        -> asynchronously reload only the visible/needed projection
```

This is not a goal to query SQLite for every egui frame. A projection remains
resident until its query inputs or relevant database revision changes.

## Current problem

The current flow is effectively:

```text
load all library_* rows -> LibraryState
mutate LibraryState -> save entry/console/full library back to SQLite
```

`LibraryState`, `ConsoleState.entries`, and `LibraryEntry` consequently form a
second mutable persistent model. `save_library` on exit/root switch is a
particularly risky synchronization boundary. The implementation also uses
vector positions and display names as entity identity, while
`save_console_bulk` deletes and recreates database entries.

## Design principles

- Filesystem bytes and layout are authoritative for installed content.
- `library_*` tables are authoritative for persisted scan/analysis state.
- The catalog tables remain distinct persistent reference data, not a cache of
  the user's filesystem.
- GUI state is a bounded, throwaway read model plus interaction state.
- UI writes are command-oriented database transitions, not mutation followed
  by a later synchronization save.
- Entity identity is a durable ID, never a list index or display name.
- A failed filesystem-to-database transition surfaces a rescan-required state;
  it must not leave a fabricated in-memory update behind.

## Architecture

### Durable IDs and source identity

In `retro-junk-db/src/library.rs`:

- Add `LibraryEntryId(u64)` alongside `LibraryRootId` and
  `LibraryConsoleId`.
- Include IDs in every row or projection which identifies a persisted entity.
- Export IDs and the new query/command APIs in `retro-junk-db/src/lib.rs`.

Add a durable `entry_key` (or clearly named `source_key`) column to
`library_entries`, unique with `console_id`. It is a normalized relative
filesystem identity:

- a single-file entry uses the normalized relative ROM path;
- a multi-disc entry uses the normalized relative playlist/logical entry point;
- `display_name` remains mutable presentation data only.

Selections and operation messages should become:

```rust
selected_console: Option<LibraryConsoleId>
focused_entry: Option<LibraryEntryId>
selected_entries: HashSet<LibraryEntryId>
```

Do not allow `usize` outside a local render-loop position. A row index is only
a temporary table/scrolling position.

### Schema migration

In `retro-junk-db/src/schema.rs`:

1. Add `entry_key` initially nullable.
2. Populate it from existing `game_entry_json` in Rust migration code.
3. Handle malformed legacy rows explicitly: resolve deterministic collisions or
   mark the affected console for a one-time rescan. Do not silently merge rows
   with different cached analysis data.
4. Rebuild `library_entries` with `entry_key NOT NULL` and
   `UNIQUE(console_id, entry_key)`.
5. Remove dependence on `UNIQUE(console_id, display_name)`; a non-unique name
   index may remain for search.
6. Add at least an index on
   `(console_id, display_name COLLATE NOCASE, id)`; add further filter indexes
   only after measuring query plans.

Add root/console revisions, or a monotonic change sequence with affected IDs,
so asynchronous replies can be discarded when their result is stale.

### ID-preserving scan reconciliation

Replace `save_console_bulk` with a single reconciliation transaction:

1. Upsert the console row.
2. Upsert each scanned entry by `(console_id, entry_key)`, preserving its ID.
3. Reuse old derived fields only when source identity and validity allow it.
4. Delete entries absent from a completed authoritative scan.
5. Update fingerprint, scan status/generation, and summary data atomically.
6. Return changed and removed IDs plus the resulting revision.

For app-initiated rename/organize/CHD operations, update `entry_key` and
`game_entry_json` in place using the known `LibraryEntryId`, preserving the ID.
For external changes with no proven old/new mapping, delete/create rather than
pretending identity continuity.

### Projection queries and commands

Do not deserialize the rich entry record merely to render a table. Introduce
focused DB types, preferably split into query and command modules if that is
cleaner:

- `LibraryRootSummary`
- `LibraryConsoleSummary`: ID, folder/platform/path, fingerprint, scan state,
  entry/status counts.
- `LibraryEntryListItem`: ID, display name, status/tag, fields needed by
  filter/sort/badges, and compact hash/identification summaries.
- `LibraryEntryDetail`: ID, console ID, full `GameEntry`, identification and
  diagnostics JSON, hashes, DAT data, titles, and user-edit fields.
- `LibraryEntryListQuery`: console ID, filter, sort, cursor/offset, limit.
- `LibraryEntryListPage`: revision, total/summary counts, entries, next cursor.

Put filtering, sorting, counts, and pagination SQL in the database query
layer, not in `widgets/game_table.rs`. Start with pages even if the first page
size is generous (for example 250--500); virtual-scroll fetching can follow.

Replace generic persistence APIs with intent-specific commands:

- `replace_console_scan` / `reconcile_console_scan`
- `set_entry_region_override`
- `set_entry_tag`
- `apply_entry_analysis`
- `apply_hash_result` and `apply_disc_hash_result`
- `apply_rename`
- `mark_entry_rescan_needed` / `mark_console_stale`
- cache-root deletion and complete cache clearing.

A command returns exactly which root, console, entry, and projection revisions
became stale. The GUI must not use generic `upsert_entry` or
`save_entries(indices)` APIs.

### GUI read model and invalidation

Replace the full application-facing `LibraryState` tree with a browser state
along these lines:

```rust
struct LibraryBrowserState {
    root: Option<LibraryRootId>,
    console_summaries: LoadState<Vec<LibraryConsoleSummary>>,
    selected_console: Option<LibraryConsoleId>,
    entry_list: EntryListState,
    focused_entry: Option<LibraryEntryId>,
    selected_entries: HashSet<LibraryEntryId>,
    entry_detail: LoadState<LibraryEntryDetail>,
    media: HashMap<LibraryEntryId, EphemeralMediaState>,
}
```

`LoadState<T>` should distinguish `Idle`, `Loading { request_id }`,
`Ready { value, revision }`, and `Failed { error }`; empty vectors must not be
loading sentinels.

Keep only transient data here: query/filter/sort/page inputs, ID-based
selection/focus, scroll targets, loading/error state, in-flight IDs, operation
progress, dialogs, and texture/media caches. A user click schedules a detail
query; the detail pane renders loading or last-safe state until its reply
arrives.

On command success:

- invalidate console summaries only if their aggregate data changed;
- invalidate the selected console's list only when its revision is stale;
- invalidate detail only when its entry changed;
- retain selections by ID when they still exist, otherwise clear them;
- schedule the smallest replacement query once, never from every render frame.

### Store service

Create `retro-junk-gui/src/library_store.rs` (or `library_controller.rs`) to
own request generation, stale-response rejection, invalidation, selection
cleanup, root-switch cancellation, and read-model eviction. Widgets become
render/input adapters:

- `console_tree.rs` renders console summaries and selects a console ID.
- `game_table.rs` renders list items and manipulates entry-ID sets.
- `detail_panel.rs` renders detail and submits commands.
- `tag_dialog.rs` stores entry IDs instead of `(console_idx, entry_idx)`.

Run a dedicated store worker owning a SQLite connection opened from `db_path`.
It accepts typed read/command requests and returns typed replies/events through
the existing UI message mechanism. Serialize library-cache writes through this
worker, use WAL, foreign keys, a sensible `busy_timeout`, and short
transactions. Reads can be sequential initially; add read connections only if
profiling justifies them. Requests and replies carry root/session generation,
request ID, and database revision, and stale replies are dropped.

## Scan and filesystem flows

Scanning may build a temporary `ScanSnapshot` off the UI thread. That is
healthy work state, not GUI state. The completion flow should be:

1. Discover console folders and persist/upsert descriptors.
2. Scan/analyze one console off-thread.
3. Commit an atomic reconciliation transaction.
4. Emit `ConsoleChanged { id, revision }`.
5. Invalidate/refetch only that console's summary/list.
6. Apply later hash/DAT results as entry-ID commands.

Rename, organize, CHD, and CUE-fix workflows must complete filesystem work
first and then execute the corresponding DB transition. On DB failure, report
that the filesystem changed and the console needs rescan; do not retain a fake
in-memory update. Keep asset discovery ephemeral and ID-keyed unless there is a
measured cross-run need to persist it; if persisted, use a dedicated table with
a validity fingerprint.

## Staged implementation

### Stage 0: characterize and create seams

- Document field ownership: filesystem, persistent library cache, catalog data,
  or ephemeral UI/media state.
- Add tests demonstrating index-selection shifts, display-name identity breakage,
  and entry-ID loss from `save_console_bulk`.
- Add a `LibraryStore` boundary without changing behavior yet; legacy snapshot
  APIs may temporarily implement it.

### Stage 1: durable database model

Files: `retro-junk-db/src/{library.rs,lib.rs,schema.rs}` and DB tests.

- Add `LibraryEntryId`, source key, revisions, projections, migration tests.
- Implement ID-preserving scan reconciliation.
- Update legacy JSON migration to use the new reconciliation path.

### Stage 2: store executor and read projections

Files: new `retro-junk-gui/src/library_store.rs`, `app.rs`, `state.rs`, and
database query modules.

- Implement typed requests/replies, generations, list/detail models, and
  invalidation.
- At startup/root switch load only root/console summaries.
- Stop using full `cache::load_library` for new code.

### Stage 3: ID-based navigation

Files: `app.rs`, `state.rs`, `console_tree.rs`, `game_table.rs`,
`detail_panel.rs`, `tag_dialog.rs`.

- Convert selection, keyboard navigation, scrolling, context menus, and
  dialogs to IDs.
- Render summaries/list projections and fetch detail on focus.
- Prove root switch, filter, sort, and refresh cannot select the wrong row.

### Stage 4: command-oriented user edits

- Convert region overrides, tags, and similar edits first.
- Write through, then invalidate/refetch exact projections.
- Default to post-commit refresh rather than optimistic UI; any optimistic
  state must be explicit and reversible.

### Stage 5: direct scan and derived-result publishing

- Convert scan, analysis, hash, and DAT completion to ID-keyed commands.
- Move catalog enrichment from UI mutation handlers into store command handling
  or explicit post-commit work.
- Classify media/broken-ref/CUE state as persistent or ephemeral and implement
  that lifecycle consistently.

### Stage 6: filesystem transitions

- Convert rename, organize, CHD compression, and CUE fixes to ID-keyed DB
  transitions.
- Preserve IDs for proven app-initiated transitions; mark stale on partial
  failure.

### Stage 7: delete snapshot synchronization

- Delete `save_library`, `save_console`, `save_entries`, full `load_library`,
  `save_library_cache`, `save_console_cache`, `save_entry_cache`, and
  `AppMessage::CacheLoaded`.
- Remove `LibraryState`/`ConsoleState`/`LibraryEntry` from GUI persistence
  responsibility; retain scanner-domain values only in operation work.
- Remove root-switch and exit-time cache writes.
- Split or rename `cache.rs` into clearer responsibilities such as
  fingerprinting, legacy-cache migration, and DB conversion.

## Acceptance criteria

Database tests must verify:

- migration preserves recoverable cache rows and explicitly invalidates only
  unrecoverable identity rows;
- a rescan and app-initiated rename retain `LibraryEntryId` where appropriate;
- duplicate display names are safe;
- failed reconciliation leaves entries and revisions unchanged;
- direct commands touch only intended rows;
- filtering/sorting/paging and summary counts are deterministic.

GUI/controller tests must verify:

- selection remains on the same ID across refresh, reorder, and filtering;
- stale replies after root switch are ignored;
- successful edits refresh only impacted projections;
- failed edits preserve the previous projection and show an error;
- deleted focus clears cleanly;
- repeated unchanged UI frames issue no database requests.

Integration tests must verify scan -> immediate quit -> reopen preserves data
without an exit-time save, and that filesystem operations plus injected DB
failure leave a visible rescan-required state rather than silent divergence.

## Implementation status

Completed. SQLite is the sole persistent library model: startup and root
switches load console summaries and paged entry projections, user edits and
derived results are serialized store commands, and completed scans reconcile
by durable source key without deleting stable entry IDs. Filesystem operations
publish ID-addressed transitions and persist a stale-console recovery marker if
the database update fails.

The remaining `ConsoleState.entries` values are disposable scan work or the
currently requested detail projection. Display-name lookup is restricted to
analysis of a new scan snapshot before it has database IDs; work on an existing
entry carries `LibraryEntryId`. Media discovery is ephemeral and ID-keyed.

Raw DAT files are read only by the explicit catalog import/update operation,
which pre-populates SQLite. Library browsing, matching, hashing, rename
planning, status summaries, and detail rendering query SQLite and do not load a
console DAT into memory. Folder fingerprinting and legacy JSON migration are
separated from normal library-store persistence.
