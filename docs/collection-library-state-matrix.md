# Collection and library convergence matrix

The desired state is not one particular file layout. It is the closest honest
state supported by the evidence and source material the user actually has:

1. preservation masters are present and byte-integrity verified;
2. every owned carrier is catalog-bound and complete where the source permits;
3. the preferred emulator-ready representation is present and round-trip
   verified;
4. a complete multi-disc release has one correctly ordered playlist;
5. release artwork/video originals are stored in the archive and projected to
   every frontend entry name;
6. physical-copy photos, provenance, and documents remain attached to the
   owned copy rather than being confused with release artwork;
7. missing local files on a selectively synced device remain a normal,
   recoverable presence state.

“Complete” is never inferred from a filename, a `.scram` integrity hash, or a
playable derivative. It requires current catalog evidence for one physical copy
containing every expected carrier.

## Starting states and routes

| Starting state | Closest desirable destination | CLI route | GUI route |
|---|---|---|---|
| No archive; loose playable files | Archive archival-equivalent bytes, retain/adopt playable files, adopt existing artwork | `archive import-playable ... --yes` (uses the sibling media root by default) | Collection → **Import existing playable library…** |
| Archive already exists; matching playable files are unbound | Record byte-identical playable evidence; list unmatched/ambiguous files for review | `archive adopt-playable` | Import existing playable library again; the operation is idempotent |
| New raw/ROM/disc dump, catalog-identifiable | Immutable archive package with provenance and catalog binding | `archive import ... --yes` | Collection → **Import dumps…** |
| New dump is ambiguous | User-selected catalog release/physical copy, then normal import | Run `archive import` interactively and choose from the reported catalog/physical-copy candidates | Import review presents catalog and physical-copy choices |
| New dump is unidentifiable | Honest unbound archive release; no completeness claim | Run `archive import` interactively and confirm an unbound title/platform, or use explicit `archive ingest` | **Import dumps…** → **Archive as an unbound release…**, then confirm title and platform; an existing release also has **Ingest dump…** |
| Archived carrier was imported unbound, but the catalog/tooling can identify it now | Exact carrier binding and current catalog evidence without recopying the preservation master | `archive verify-catalog` for single-file masters or `archive audit-redumper` for Redumper raw masters | Collection → **Identify archived carriers** reproduces unbound Redumper masters from the archive and applies unique complete-track matches |
| One owned multi-disc copy contains compatible discs from different mastering records | Work-level parent with exact mastering/release/media identity on each carrier; completeness by distinct disc position | Import each disc as a separate package under one parent directory, or rerun `audit-redumper` on previously unbound carriers | **Import dumps…** groups compatible carrier matches into one physical copy; **Identify archived carriers** upgrades an existing unbound copy |
| Archive bytes present, integrity unknown/stale | Current SHA-256 integrity evidence | `archive verify` | Collection → **Verify stored bytes** |
| Catalog-bound master unverified | Complete normalized file/track catalog evidence | `archive build` now verifies every prerequisite for a release before publishing any missing derivatives; focused tools remain `verify-catalog` and `audit-redumper` | Library → **Verify archive** or **Verify & make playable** |
| Multi-disc archive is incomplete | Explicit incomplete state; import missing owned carriers when available | `archive build --dry-run` reports the missing count; `archive import` adds available carriers | Library action is disabled with the present/expected count; Collection imports the missing dump |
| Verified archive, no preferred format | Persist an explicit platform or carrier policy | `archive policy-default` or `archive policy` | Library preferred-format selector or Collection carrier policy |
| Preferred playable missing | Verified CHD/RVZ/native mirror with build evidence, projected artwork, and an ES-DE gamelist entry | `archive build` (release-aware); it updates media and ES-DE metadata by default | Library → **Make playable** updates the configured media tree and safely upserts `gamelist.xml` |
| Preferred playable is the wrong format | Preserve the old derivative; build the preferred format | Change policy, then `archive build` | Change preferred format, then **Make playable** |
| Complete multi-disc derivatives, playlist missing | One ordered M3U only after all catalog-expected discs exist | `archive build` recreates the playlist through the shared release-aware builder | Library → **Create multi-disc playlist** |
| Archive-only release has no artwork | Archive ScreenScraper originals or a user-supplied image | Scrape a playable library with `scrape --archive-root`, or `archive add-release-file` | Library → **Scrape Media**, or Collection → choose a semantic type and **Add release artwork…** |
| Frontend artwork exists, archive artwork missing | Adopt frontend files as authoritative release supporting files | `archive import-playable` adopts matching sibling-media files | Importing playable content/dumps adopts matching existing artwork |
| Archive artwork exists, frontend files missing/stale | Integrity-checked projections for single-disc and full `.m3u` entry stems | `archive project-frontend-files`; `archive build` does this automatically unless disabled | Library → **Restore archived media files**; playable builds restore media automatically |
| Artwork components exist, miximage missing/stale | Generated miximage beside the playable entry and an authoritative archived original | `archive generate-miximages` | Right-click the Library release → **Generate Miximage** |
| Physical photos/provenance missing | User-supplied copy-specific supporting files | `archive add-physical-copy-file` | Collection → **Add physical-copy photo…** / **Add provenance document…** |
| Local playable derivative absent due to selective sync | Normal “known, absent here” state; rebuild only when wanted | Re-run `archive build` on that device | Library shows the missing representation and offers the normal build action |
| Stale/lost SQLite projection | Rebuilt disposable index from portable manifests/evidence | `archive reindex` | Collection → **Refresh index** |
| Interrupted staging/work directory | Recoverable quarantine, never silent deletion | `archive recover` | No GUI action yet; this is maintenance rather than a collection/library content destination |

## Boundaries where automation must stop

- A missing physical carrier cannot be manufactured. The system keeps the
  release incomplete and can only optimize the carriers actually present.
- An unbound release cannot honestly be called catalog-complete. It can still
  have integrity evidence, playable mirrors where formats permit, artwork, and
  provenance.
- Original Xbox XISO, PS3, and Vita do not yet have an honest modeled
  preservation-master → mainstream-emulator conversion. The GUI offers **No
  preference** rather than pretending a Redump-style ISO is sufficient.
- ZIP/7z content must currently be unpacked before archive import.
- ScreenScraper acquisition requires credentials, quota, and network access;
  restoring already archived artwork does not.

## Invariants for future transitions

- A CLI and GUI action that claim the same destination must call the same
  verification/build/projection implementation.
- Mastering-specific catalog release/media IDs belong to carriers. A parent
  release may intentionally retain only a work identity when its compatible
  carriers resolve to different mastering records.
- Release-wide prerequisites are checked before the first new disc derivative
  is published.
- Artwork projections include the full `.m3u` directory stem used by ES-DE,
  not only individual carrier filenames.
- Import and projection are idempotent. Existing identical archive originals
  or frontend projections are current, not errors.
- Preservation masters are never deleted by convergence operations.
