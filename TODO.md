# TODO

## High priority

- [x] **Inbox needs path/name filtering and a bulk dismiss.** Done (2026-08-02), plus the pieces that make a bulk button worth pressing. The filter is glob-shaped and matches the whole relative path, so `*.txt` and `*/rvz/*` both work; the matcher is `retro_junk_io::glob` and is the single answer to "what does this pattern cover" for the list, the bulk buttons, and the adoption sweep alike. Bulk dismiss and bulk apply act on the filtered set with the count in the label and in the confirmation.
  - **Reversibility, as the entry asked:** confirmed and stated in the button's confirmation. Dismissal writes `resolved_at`/`resolution` on the review row and touches no file. Two independent ways back: `reopen_suggestions` puts the exact closed rows back (offered as Undo in the GUI and printed as a ready-to-run command by the CLI), and re-running adoption files anything still unaccounted for again, because `open_suggestion` only supersedes rows that are still *open*.
  - **That last fact is also why dismissal alone was not enough**, and the entry's plan would have been a treadmill: dismiss 900 strays, run the sweep, get 900 rows back. So ignore rules landed with it — a durable per-pattern decision stored beside the collection (`retro_junk_archive::ignore`, one file per rule like marks), consulted before the sweep hashes anything, revocable at any time.
  - Adoption reviews became resolvable at the same time (see the Phase B follow-up below), since "dismiss" was otherwise the only working button on 98% of the rows.

- [ ] **Redumper audits copy the raw dump; on the live setup they copy it disk-to-same-disk.** Raised 2026-08-02. `redumper split` writes BIN/CUE/log beside its input, so `redumper_cache::prepare` copies the raw set to scratch storage instead of letting a tool write into the archive. When `network_mode` is off, `processing_workspace_root()` is `<archive_root>/.retro-junk/work` — the *same volume as the archive*, so a CD audit reads ~1 GB and writes ~1 GB across one USB-C channel before redumper even starts. Options, in rough order of payoff:
  - **Copy-on-write clone.** `clonefile(2)` on macOS / `FICLONE` on Linux would make the staging copy nearly free, but only within one filesystem that supports it. **exFAT supports neither**, and the reference SSD is exFAT — so this pays off only if the archive volume becomes APFS (or btrfs/XFS on Linux). Worth adding to `retro_junk_io::copy_and_hash` as an opportunistic fast path with a fallback, since it also speeds up ingest staging; note that a clone skips the read, so digests the caller needs must be computed separately.
  - **Symlink farm instead of a copy.** Point redumper at a scratch directory of symlinks to the raw files, so it reads through them and writes its output as real files beside them. Zero copy. Two catches: the scratch directory must be on a filesystem that supports symlinks (exFAT does not, so this requires the workspace to sit on the internal drive, i.e. the `network_mode` path), and it trades away the isolation guarantee — anything redumper rewrites in place would land in the archive. `retro_junk_io` deliberately rejects symlinks in staged packages today (`StageError::SymbolicLink`), so this is a policy change, not just an implementation one. Would want the raw files' recorded SHA-256 re-verified after the run to prove nothing was touched.
  - **Cheapest immediate mitigation:** default the processing workspace to the internal drive rather than the archive volume when the archive is on removable/slow storage, so the copy is at least a cross-device read + local write rather than two operations fighting over one bus.

- [ ] **No way to ask for a re-identification after a catalog update.** Since 2026-08-02, a dump whose identification concluded "no single catalog match" projects as `catalog_state='unresolved'` and convergence stops proposing it — correct, because reproducing a disc is expensive and the answer cannot change until its bytes do. But the *catalog* changing (a newly imported DAT) can change the answer, and the only way to act on that is `retro-junk archive redumper-audit`, which re-audits regardless of state. The GUI badge's "run this again" goes through the derivation, so it cannot reach an unresolved dump. Wants either a "re-identify" action that bypasses the derived state, or an import-time pass that clears `unresolved` for platforms whose catalog gained media.

## Bugs

- [ ] **A weak scrape match makes the daemon re-run the same scrape every tick, forever.** Found in the 2026-08-03 review. A filename-tier match returns `Success` and files a review suggestion (`retro-junk-work/src/executor.rs:555`), but nothing records that the release's scrape question is now *waiting on a person* — so `derive_scrape_actions` keeps deriving `Scrape` as pending and, with `auto_scrape` on, the daemon re-executes it every 30-second pass. The per-pass cost is now small (media present → skip; `open_suggestion` keeps an identical open row instead of superseding it, fixed 2026-08-03), but the derivation is still lying about pending work. Wants a derived "weak match filed, awaiting review" state: an open scrape suggestion for the release should suppress the pending `Scrape` action until it is resolved.

- [ ] **`ProjectAssets` and `SyncGamelist` derive as pending forever, and each one dispatches a full archive scan.** Found in the 2026-08-03 review. Every release with a present playable derives both actions unconditionally (`retro-junk-db/src/convergence.rs:618-663`), and `summarize_convergence` computes no `done` for those kinds — so `status` on a fully converged library still reports N pending projections, and an explicit `sync` re-executes all of them. Worse, each such action's dispatch calls `scan(ctx)` (`retro-junk-work/src/executor.rs:572-595`), a full archive walk *per action*: one pass over N releases is ~2N complete scans, which over SMB is hours of redundant I/O. Needs a currency check for projection kinds (asset fingerprints or a projected-at watermark) and one shared scan per run, not per action.

- [ ] **A crashed daemon shows as "Running" because its zombie is never reaped.** Found in the 2026-08-03 review. The GUI spawns the daemon and never waits on the child (`retro-junk-gui/src/backend/daemon.rs:84-95`), so on Unix an exited daemon stays a zombie in the GUI's process table, and `process_alive` — `kill(pid, 0)` — reports zombies as alive. `status()` then says "Running (pid X)" with a frozen heartbeat instead of the Stale state built for exactly this, and Stop fails confusingly. Hold the `Child` and `try_wait` it during status probes, or detach properly (double-fork/setsid) so the PID probe means what it says.
  - Same section, same review: the daemon panel does filesystem and database I/O **every frame** while Settings is visible — `daemon::status()` reads the PID file, probes the process, and reads `runtime_state`; `log_tail(12)` opens and reads up to 16 KiB (`retro-junk-gui/src/views/settings.rs:716,764`). This contradicts the changeset's own per-frame-I/O rule (documented in `backend/inbox.rs`); cache both behind a ~1 s throttle.

- [ ] **Executor claim protocol: a quiet dispatch loses its claim, and nobody notices the theft.** Found in the 2026-08-03 review. The claim heartbeat piggybacks on progress callbacks (`retro-junk-work/src/executor.rs:247-259`), so any dispatch phase quiet for over `CLAIM_TIMEOUT_MINUTES` (2 min) — a long external tool run, a stalled network read — lets another process legitimately steal the claim; `refresh_claim` updates 0 rows and returns `Ok`, so the original owner keeps working (duplicated work; the archive lock prevents actual concurrent mutation). Owner-scoped `release_claim` landed 2026-08-03, so the walking dead can no longer delete the thief's live claim on the way out. Remaining: `refresh_claim` should report whether the claim is still yours (and dispatch should check it at phase boundaries), and/or a timer-based beat for quiet phases.
  - Related fragility, same file (line ~266): cancellation is classified by comparing the error's display text to the literal `"operation cancelled"` (and `retro-junk-work/src/adoption.rs` now emits that string too). Any wording change silently reclassifies cancellations as failures. Wants a typed `WorkError::Cancelled` variant.

- [ ] **Convergence error reporting drops errors it cannot attribute, and mixes scopes.** Found in the 2026-08-03 review. `errors_by_release` silently drops any recorded error whose target no longer resolves to a release (Path-target errors; dumps whose projection rows a later reconcile removed), and `release_for_target` swallows SQL errors with `.ok().flatten()` (`retro-junk-db/src/convergence.rs:240-287`) — so the summary's error count and the visible per-release badges can disagree. Separately, `summarize_convergence`'s `running`/`errored` counts query the claim/error tables globally while `pending`/`blocked`/`done` are scope-filtered (lines ~867-883), so a per-profile status shows other profiles' in-flight work.

- [ ] **A verified-but-unbindable dump re-runs a full redumper audit every 6 hours, forever.** Found in the 2026-08-03 review. A dump whose evidence says catalog-verified but whose recorded media id this catalog cannot resolve — and whose digests the re-derivation can't match either, e.g. a different DAT version — derives `AuditRedumper` on every pass (`retro-junk-db/src/archive.rs:321` with `convergence.rs:405`); each attempt fails "no complete catalog match", records an error, and retries after the 6-hour backoff. `CATALOG_UNRESOLVED` stops only the never-attempted case. The verified-but-unbindable case should park the same way (and the "re-identify after a catalog update" entry above is its escape hatch).

- [ ] **A symlinked playable root never marks consoles stale from watcher events.** Found in the 2026-08-03 review. Watcher roots are canonicalized so event paths compare against them, but `apply_event` passes the *non-canonical* `profile.playable_root` with the *canonical* event path into `console_for_path`, whose SQL does plain string prefix matching on `folder_path` (`retro-junk-work/src/daemon.rs:264-307`). On a root reached through a symlink (`/tmp` → `/private/tmp`, some `/Volumes` setups) the prefix never matches: the archive-side reconcile still runs, but the library view stays stale until a manual rescan. Canonicalize once and compare like with like.

- [ ] **An incoming package with nothing importable is marked "imported".** Found in the 2026-08-03 review. If `plan_import` returns zero candidates, `plan_and_apply` falls into the `all(AlreadyArchived)` check — which is vacuously true on an empty list — and calls `set_incoming_imported` (`retro-junk-work/src/incoming.rs:238-246`): a stray text file gets recorded as an import. Wants an explicit "nothing importable recognized" state (or error) for the empty plan.

- [ ] **Two import-planning behavior changes to confirm or revert** (2026-08-03 review, `retro-junk-archive-import/src/lib.rs:1663-1738`). `Redumper::detect` failure used to be non-fatal (`if let Ok`), falling through to an unresolved-but-importable candidate; it now returns `ImportError::InvalidPackage` → `Invalid` — so a machine without redumper cannot import a raw dump at all. And a RedumperRaw package carrying a `.cue` whose BINs were pruned now fails `validate_cue_references` → Invalid, where the cue was previously ignored and redumper split ran. Both may be deliberate tightening; decide and document, or restore the fallthroughs.

- [ ] **Blocked items no longer fail a non-dry-run `archive build`/`sync`, and are no longer named.** Found in the 2026-08-03 review. The old build queue logged each planning failure ("pending policy cannot be built: …") and exited non-zero when any item couldn't be planned; the executor now skips derivation-blocked actions with only `stats.blocked += 1` (`retro-junk-work/src/worker.rs:194`), and `run_sync` returns `Ok` unless `stats.failed > 0` (`retro-junk-cli/src/commands/sync.rs:186`) — "Sync finished: … 2 blocked", exit 0, no names. The code comment only promises the old contract for `--dry-run`. Decide the intended non-dry-run contract; at minimum, log which items were blocked and why (the derivation knows).

- [ ] **The default gamelist location depends on which surface wrote it.** Found in the 2026-08-03 review. `sync`/`rebuild-playable` default gamelists to the sibling `<playable>-metadata` directory (deliberate, per the comment at `retro-junk-cli/src/commands/sync.rs:567`), while `daemon start` and `suggestions apply` build `FrontendRoots::from_settings`, where an empty `metadata_dir` setting means *inline in the playable root* (`retro-junk-lib/src/archive_ops.rs:77`). Same profile, two gamelist trees, each surface refreshing its own. Pick one default and derive it in one place.

- [ ] **Scrape session polish** (2026-08-03 review, `retro-junk-scraper`). Three smaller ones in the new unified orchestration:
  - Archive-destination targets don't get a miximage composed on the first pass — downloads are still in scratch when `scraped_outcome` composes from `settle.present`, and `publish_and_project` never composes after projecting (`session.rs:712-726`). Playable-destination targets compose in the same run; the archive path self-heals only on the next pass, costing an extra full cycle per game.
  - Non-dry-run skips emit a `ScrapedGame` with `name = filename` and empty metadata even when no media exists at all (`scrape.rs` `fold_outcome`, `Skipped` arm). If the gamelist writer upserts by path and overwrites names, repeated scrapes can degrade previously scraped display names to raw filenames; audit the merge semantics.
  - `ScanComplete.total` excludes secondary discs but `GameGrouped` events carry the full-scan index, so a progress renderer can show `index > total` (`scrape.rs:158`). Cosmetic.

- [ ] **Archive evidence and naming polish** (2026-08-03 review, `retro-junk-archive`). `dump_catalog_evidence` returns the *oldest* current Verified catalog record (`evidence.rs:36-49`); evidence is append-only, so a re-verification against a corrected catalog appends a newer record every consumer then ignores — newest-wins (or superseding) would let corrections take effect. `safe_file_stem` (`layout.rs`) doesn't guard Windows/exFAT reserved device names (`CON`, `NUL`, `COM1`, …) — a title collapsing to one fails to write on exactly the Windows-shared drives the docstring targets. And the legacy lock's 1-minute empty-file staleness window compares server-stamped mtime against the local clock (`lock.rs:lock_age_exceeds`), so SMB server clock skew of a minute defeats it in either direction; the 24-hour window absorbs realistic skew, the 1-minute one doesn't.

- [ ] **GUI shell follow-ups from the Phase B review** (2026-08-03). Four smaller ones:
  - The three inbox dialogs (bulk confirm, ignore-rule editor, candidate picker) bypass the shared modal scaffold and use raw `egui::Window` — no backdrop, no input blocking, and no Escape at all, since the inbox keyboard handler is disabled while they're open (`views/inbox.rs:659,737,801`). `views/collection.rs:860` also builds its own `egui::Modal` and discards `should_close`. Convert them; uniform Escape was the scaffold's stated purpose (a5fe52e).
  - The dirty tick carries no writer identity, so the GUI's own commits trigger a second full refresh pass ~1 s after the completion handler already refreshed (`app.rs:354-389`). A writer token (or "expected tick" bump on own writes) would drop the spurious pass.
  - Native menu events are only drained inside `update()`, and no `muda` event handler wakes egui — menu responsiveness silently rides on the 1 Hz dirty-poll repaint staying alive (`app.rs:395-422`). Register a handler that requests a repaint.
  - `refresh_library_availability` counts open suggestions by fetching *all* open rows on the UI thread and taking `.len()` (`state.rs:2655`); `open_suggestion_counts` already exists — use it (or a `COUNT(*)`), off-thread.

- [ ] **"Scrape Missing Artwork" is disabled on the games most likely to be missing artwork.** Raised 2026-08-02. All three surfaces gate the action on `release.scrape_identity.is_some()` — the archive context menu (`widgets/game_table.rs:857`), the selection menu (`widgets/game_table.rs:1000`), and the detail panel (`widgets/detail_panel.rs:662`) — and `query_archived_scrape_identities` (`retro-junk-db/src/library.rs:2020`) only produces an identity by joining `carriers.catalog_media_id` through to `media`/`releases`. So a release whose carrier never resolved to a catalog medium has no identity, and the button is greyed out with "no reliable catalog scraper identity" — including on releases the user has explicitly marked as needing artwork, which is exactly when they are reaching for it. Two things are conflated: *can we look this up* and *is the lookup trustworthy enough to run unattended*. Derivation-aware scraping (Phase A.5) already answers the first for marked files — a mod resolves through its parent, homebrew by name — without any catalog medium, so the gate is now stricter than the scraper. Needs a decision on what an explicit request should do when identity is weak: run the name-based lookup and file the result as an Inbox review rather than publishing it, or publish with a confirmation. Whatever it does, the greyed-out state should be reserved for cases where there is genuinely nothing to ask.

- [ ] **Cross-host archive lock on SMB could use share-mode locks.** The archive lock (2026-07-30 rework) uses kernel-enforced OS locks where the filesystem honors them and falls back to the existence+PID+age protocol elsewhere. On macOS smbfs, `flock` silently enforces nothing (verified empirically), so SMB shares always use the fallback. macOS `O_EXLOCK`/`O_SHLOCK` open flags map to SMB share-mode (deny) locks enforced server-side, which would give real cross-host exclusion and server-side crash release; needs `OpenOptionsExt::custom_flags` + libc and a Linux-cifs interop check. **This is a live case, not a hypothetical:** the reference archive is written from both an Arch Linux host (cifs) and macOS (smbfs).
  - **The fallback's host attribution landed 2026-08-03** (this had been the dangerous half: the PID probe ran against *local* PIDs regardless of who wrote the record, so machine B could "prove" machine A's live holder dead and delete its lock — two concurrent writers). Lock records now carry `host=`; a PID probe is only authoritative for a record naming this host (or an unattributed record on a local filesystem, where no other machine could have written it). A foreign record reclaims only via the conservative 24-hour age — held rather than never, since a wedge with no recourse is its own failure. Stale-lock reclaim also switched from remove+create to rename-aside, so two contenders judging the same leftover stale can no longer both acquire. Share-mode locks above remain the real fix.

- [ ] **One import re-reads the whole archive several times.** `scan_archive` is now concurrent and single-read (241 releases over SMB: ~2.5 s warm, ~5 s cold), but a single import still pays it repeatedly: `plan_import` scans once, `ingest_new_carrier_dump` scans again *per package* (25 packages = 25 scans), and the GUI scans again — twice if artwork adoption ran — before `reconcile_archive_snapshot`. Measured 2026-07-30: planning a 25-file folder is ~5.7 s of which ~5 s is the one scan; executing it would repeat that per package. Thread one snapshot through the import (planning → ingest → reconcile), invalidating only the release each ingest touched. `ingest_new_carrier_dump` in particular needs release/copy/carrier manifests only, never dumps or evidence — the bulk of the tree.

- [ ] **Schema open path trusts the version stamp.** `open_database` decides "migrated" purely from `schema_version`, so a database whose tables don't match the stamped version (e.g. one written by the pre-rebase divergent branch, which used the same version numbers for a different layout) opens "successfully" and fails later with `no such column` at query time. Add a cheap structural sanity probe on open (e.g. `SELECT scan_state FROM library_consoles LIMIT 0` for a sentinel column per recent version) that produces a clear "incompatible database, delete or restore" error instead. Also note `ensure_catalog_database_location` re-copies the legacy cache DB (`~/Library/Caches/retro-junk/dats/catalog.db`) whenever the target is missing — deleting a bad `catalog.db` silently resurrects an equally old one; the legacy file should probably be renamed once migrated instead of retained under its live name. (Diagnosed 2026-07-30.)


- [x] **Legacy cartridge catalog evidence never claims a complete track set.** Done (2026-08-01) without rewriting a single evidence file. `complete_track_set` is now derived from the dump's shape when the flag is absent: a `rom`-format single-file master is one file and one "track", so a verified match against it was always the complete set — the older path simply predated the flag (244 such records on the reference archive). Deliberately narrow: a single-file `iso` is excluded, because a data-track-only image of a multi-track disc is exactly the case the flag exists to catch, and the one non-`rom` record in that population is such a dump. The **writer** had the opposite bug — `catalog_evidence` recorded `complete_track_set: true` unconditionally on the single-file path, so a single file matched against a multi-track medium on its primary digests claimed a completeness never verified; the match result now carries `medium_has_tracks` and the flag reflects it. Carrier re-resolution also learned the cartridge case (no per-track digests exist, so it matches on the dump manifest's recorded file digests). Combined effect on the reference archive: unresolved carriers **149 → 35** (remaining: 19 saturnjp, 14 ps1, 1 ps2, 1 nes — discs whose track digests resolve to no imported medium, plus one headered dump blocked by the raw-vs-payload digest issue below).

- [x] **Carrier catalog bindings do not survive a re-import on another machine.** Done (2026-08-01) via the cheaper option this entry already identified: re-resolve during projection from the digests the archive recorded. `rederived_catalog_media` feeds a dump's current complete-track catalog evidence back through `match_complete_catalog_media` when the recorded media id is absent, and the carrier projects as `binding_state='rederived'` rather than pretending the id resolved. Fixing this exposed a second bug that made it useless on its own: `match_single_track_catalog_media` required every digest the *catalog* held to be matched by the caller, so a caller carrying only SHA-1 — all the archive's track evidence records — could never match a medium that also had a CRC-32. An absent digest on either side now means "not available to compare", with both sides still required to bring at least one.


- [x] **A renamed playable orphans its build evidence.** Done (2026-07-31). Observed 2026-07-30 on the live archive: a PS1 release's playable representation pointed at `psx/castlevania-symphony-of-the-night-usa.chd` (presence `missing`) while the library held `Castlevania - Symphony of the Night (USA) (Track 1).chd`, so the archive showed "archived only" beside a playable-only row for the same game. Three changes: `retro_junk_archive::adopt` finds a moved output by its recorded output SHA-256 (size-indexed walk of the playable root, one walk per run) and appends adoption build evidence naming the new path; `ActionKind::AdoptPlayable` derives that work from `presence_state='missing'` and runs in its own worker stage *before* builds, so a moved file is never rebuilt beside itself; and build currency is now keyed on the build lineage (parent representation + format) rather than the output path, so the superseded record stops projecting instead of leaving the old path behind as a permanently missing representation. `archive adopt-playable` gained `--release-id`/`--dry-run` and runs the moved-output pass before its existing byte-identical-to-master pass. **This also fixes the naming cause:** `Media.rom_name` for a multi-track Redump game is its largest *track* file, and both playable builds and library renames took that stem for a whole-disc container — hence `… (Track 1).chd`. The rule now lives once in `retro_junk_dat::tracks::whole_medium_stem` (multi-track ⇒ the DAT game name; single-file ⇒ the ROM name's stem) and is documented in `.claude/skills/game-scraping/NoIntroDAT.md`.
  - Existing outputs keep their old names until rebuilt or renamed; a rename now re-adopts cleanly instead of orphaning.

- [x] **Two bugs a live `archive adopt-playable` run surfaced.** Found 2026-08-01 running the sweep against the reference archive. `collect_playable_files` walked macOS AppleDouble sidecars, so every game filed a second bogus "unmatched" review row — the same class the library scanner and dump verification already handle via `retro_junk_io::is_noise_path`, which this path now uses too. And a file the platform's analyzer could not read (`Invalid ROM format: Not a recognized disc format`) aborted the entire run before its reconcile; format-aware hashing now falls back to raw digests and the file is filed for review like any other stranger, which is the whole point of a sweep over unaccounted files.

- [x] **Two test flakes that had nothing to do with the code under test.** Found 2026-07-31 while running the suite twice concurrently. `retro-junk-frontend`'s miximage tests wrote to fixed paths under the system temp directory and `remove_dir_all`'d them on the way out, so overlapping runs deleted each other's working files; they use `tempfile::TempDir` now. `retro-junk-gui`'s `chdman_probe_runs_off_the_ui_thread` asserted the probe was still `Probing` right after the frame, which races the background thread it spawned; it now asserts a probe was *started* (not `Idle`), which is the invariant it meant, and still fails if the probe blocks the UI thread or never settles.

- [x] **An unbound carrier's playable is named in a different scheme from a bound one.** Done (2026-08-02): both use the readable form, and the region is written the way a DAT writes it (`Region::from_slug` maps the lowercase token records store back to `(USA)`/`(Japan)`). Doing this made filename safety load-bearing rather than theoretical — the bound path was already writing raw catalog names to disk, so a title carrying a colon could not be written to exFAT at all — so `retro_junk_archive::safe_file_stem` now guards both paths in one place.

- [x] **A renamed playable's artwork and gamelist entry were left behind — and its old name kept being republished.** Done (2026-08-02). Worse than first diagnosed: `release_media_stems`, `release_media_stems_by_platform`, and `sync_esde_gamelist_for_release` all walked `dump.builds` — every record ever written — instead of `current_build_evidence`. `evidence/` is append-only, so a release rebuilt or re-adopted under a corrected name kept *both* names, and every projection run re-copied its artwork under the abandoned one and gave the frontend a second, dead entry for the same game. All three now read `retro_junk_archive::current_release_builds`, the release-level form of the rule that already governed the database projection, and `superseded_release_builds` names what to clean up: stale media files are deleted on projection and the retired gamelist entries removed by `esde::remove_game_entries`. A name another lineage still publishes under is never retired.

- [ ] **Hash provenance is stored but never shown.** `library_entries.hash_source` records when a row's digests were adopted from archive manifests rather than read on this machine, but nothing surfaces it: the detail panel shows CRC32/SHA-1/MD5 with no indication of where they came from, and there is no "re-read this file to confirm" affordance (the hash action skips rows that already have digests unless the include-cached path is used). Plumb `hash_source` through `LibraryEntryRow`/`LibraryEntry` and label adopted rows.

- [ ] **Archive evidence records the raw digests, not the payload digests it computed.** `verify_catalog_files` hashes each single-file master with the analyzer's format-aware normalization (header-skipped) to match the catalog, then discards those digests — the verification evidence keeps only the catalog verdict. Persisting them (alongside the raw digests already in `dump.toml`) would let headered dumps adopt hashes without needing the catalog to confirm the raw form, and would make the archive self-describing for DAT matching.

- [x] **Evidence dot popover never showed why a blocked action didn't run, and the retry toast said "nothing to do" even when the action had just failed or was blocked.** Done 2026-08-02. `derive_build_actions`/`derive_scrape_actions` (`retro-junk-db/src/convergence.rs`) already computed a `BlockedReason` per action — `worker::run_once` uses it to skip the action before the executor ever sees it, so no `WorkError` gets written and the row's dot color never changes — but nothing read it back out at the per-release level; only an aggregate count reached the GUI. Added `blocked_by_release` (`retro-junk-db/src/convergence.rs`) alongside the existing `errors_by_release`, threaded it through `Backlog` (`retro-junk-gui/src/backend/convergence.rs`), and the evidence popover (`retro-junk-gui/src/widgets/evidence_badges.rs`) now shows the reason and drops the "Run again" button for a blocked class instead of offering an action that would silently no-op again. Separately, `run_release_kind`'s toast collapsed every `stats.completed == 0` outcome to "Nothing to do: already current" regardless of whether the run had actually failed or been blocked — it now distinguishes failed/blocked/busy/truly-nothing-pending and points at the evidence dot for the reason.

- [x] **A playable whose adopt fails (bytes genuinely gone, not just moved) had no path back to a rebuild.** Observed 2026-08-02 on Castlevania: Symphony of the Night; done the same day rather than deferred, since the underlying mechanism (bypass the "already satisfied" check without inventing a format or discs the archive doesn't have) turned out not to need the open design question the entry originally raised. Added an explicit "force rebuild" escape hatch instead of an automatic adopt→build fallback: `query_playable_gaps` (`retro-junk-db/src/library.rs`) gained a `force_release_id` parameter that keeps one release's carriers past the per-carrier "already satisfied" skip and the completeness-based retain, reporting `needs_playable` as true so the build stage that consumes the gap actually acts — exposed as `query_forced_playable_gap`. `forced_build_action` (`retro-junk-db/src/convergence.rs`) turns that into an ordinary `ProposedAction` (still subject to genuine `BlockedReason`s — no preferred format, incomplete archive — since forcing bypasses the belief that nothing is owed, not physical impossibility).
  - **First cut was incomplete**, caught the same day on Brave Fencer Musashi: forcing a build straight into `build_release_playable` collided with `playable_build.rs`'s own "don't silently overwrite" guard whenever a file already sat at the canonical output path with no evidence pointing at it (a carrier the archive never built, sitting beside a file placed there some other way — exactly the case `adopt_unbuilt_playables` exists for, and forcing skipped it entirely). Fixed by giving adoption first crack: `retro_junk_work::force_rebuild_playable` (`retro-junk-work/src/executor.rs`) now always runs `ActionKind::AdoptPlayable` for the release before considering a build — safe unconditionally, since adoption only ever links an existing file to evidence by matching content, never overwrites — reconciles, and only forces a build via `forced_build_action` if the release *still* needs one under the ordinary unforced check (new `release_needs_playable`, `retro-junk-db/src/library.rs`). If a forced build still collides with an unrecognized file after that, the error now says so explicitly (content didn't match, or the carrier isn't catalog-verified yet) instead of a bare "already exists". Both the CLI's `rebuild-playable` (`retro-junk-cli/src/commands/sync.rs`) and the GUI's "Force Rebuild Playable" (`retro-junk-gui/src/backend/convergence.rs`) now call this one function, replacing what had been two independently-written copies of the same outcome-matching logic.

## Features

- [ ] **Database management GUI** screen for all sorts of database tasks, including viewing and merging conflicts, importing and previewing enrichment, and maybe even direct database editing
  - Partially done (2026-07-10): Tools → Data tab (`views/tools_data.rs`, `backend/catalog_ops.rs`) adds catalog import and GDB/ScreenScraper enrichment (plus DAT/GDB cache fetch/clear); Dashboard/Browse tabs already view stats, disagreements, and tables. Still open: enrichment *preview* and direct database editing.

- [x] **Move media and data on rename** — Done (2026-07-10): renames execute as per-game filesystem transactions (`retro-junk-lib/src/fs_txn.rs`) that carry scraped media files and gamelist.xml path/asset rewrites (`retro_junk_frontend::esde::plan_gamelist_rewrite`) along with the game files, with preflight collision checks and rollback on failure.

- [ ] **Figure out multi-file WBFS setups** - I don't know what we're meant to do with them or how to treat them

- [x] **Modded/homebrew marks must survive without the database.** Done
  (2026-08-01), by the first option this entry weighed: a sidecar store beside
  the collection, `<collection>/.retro-junk/marks/`, one file per decision named
  by content digest (`retro_junk_archive::marks`). One file per mark is what
  makes it sync-safe — two machines marking different games never touch the same
  file, and the same decision made twice is byte-identical, so copies converge
  rather than conflict, which is why no timestamp is recorded. A mark carries
  the *inputs* ids are minted from and never the ids, so it survives a re-import
  against a differently versioned DAT: a mod names its parent by DAT game name
  (falling back to canonical name) and waits, keeping the decision, on a machine
  whose catalog does not have that DAT yet. The database is now a cache —
  `apply_collection_marks` runs wherever the projection is rebuilt, and tagging
  through the GUI writes the durable form as well as the row.
  Two bugs this exposed and fixed: the tag *dialog* that asks which game you
  modded wrote no mark at all, while the plain menu wrote a parentless one that
  could never resolve; and "which directory holds this collection" had two
  implementations that disagreed off the sibling layout, so marks could be
  written where nothing read them.

- [x] **Derivation-aware scraping.** Done (2026-08-01). A mod is looked up as
  the work it derives from — never by its own bytes, which are in no scraper's
  database — keeping its own name and media stem; homebrew is looked up by name
  with its meaningless serial suppressed; a mod with no parent named is skipped
  without spending a request. The rule lives once in
  `retro_junk_scraper::derivation` and is applied inside the shared scrape core,
  so all three surfaces share it. Derivation reaches it from the catalog
  (`retro_junk_db::derivation`) or, with no database at hand, from the
  collection's marks — which is what makes `retro-junk scrape` derivation-aware
  on a machine that has never imported a DAT. `ScrapeIdentityTier` now reports a
  mod's *parent's* strength, so automation gates on what it will actually ask.

- [ ] **Custom multi-select view** in the game details panel, rather than showing details for the most-recent selection in the list

- [ ] **Show hash match status in detail panel** — After hashing, the detail panel shows CRC32/SHA1/MD5 values but doesn't visually indicate whether they match known DAT entries. Add a match/mismatch indicator next to hash values.

- [ ] **Try copy-on-write reflinks before physically copying disposable staging data** —
  For same-filesystem staging on reflink-capable filesystems, attempt a native
  COW clone (`FICLONE` on Linux, `clonefile` on macOS), hash the cloned snapshot,
  and transparently fall back to the existing single-pass copy-and-hash path
  for unsupported or cross-device sources. Never substitute hard links: tools
  such as Redumper require an isolated writable workspace. Keep free-space
  checks conservative because later writes can materialize cloned extents, and
  surface whether each staging operation used a reflink or physical copy.

## Disc Sets & Verification (deferred from 2026-07-10 rename work)

- [ ] **Surface per-track verification in analyze output** — Rename now
  verifies every track of a cue/bin set against Redump per-track hashes
  (`retro-junk-lib/src/disc_set.rs`), but `analyze` still hashes only the
  data track (`retro-junk-disc/src/hash.rs` hashes Track 1 / largest data
  track only). Reuse the disc-set verification to report per-track
  match/mismatch during analyze and in the GUI detail panel.

- [ ] **Scraper vs. import hash divergence** — Under `--force-hash`, the
  scraper hashes the *first* data track of a multi-BIN cue while DAT import
  stores the *largest* data track's hashes. Divergent for Saturn (MODE1 boot
  track + larger main track). The default serial path is unaffected. Pick one
  convention and share the implementation.

- [ ] **Saturn `.mds`/`.mdf` advertised but unimplemented** —
  `saturn.rs` lists `mdf`/`mds` in `file_extensions()` but
  `retro-junk-disc/src/format.rs` has no MDS/MDF detection. Either implement
  or stop advertising.

- [ ] **Saturn analyzer test coverage** — `saturn_tests.rs` covers IP.BIN
  parsing and ISO analysis but has no CUE, raw-BIN, CHD, or hashing tests —
  the paths redumper dumps actually exercise.

- [ ] **Make M3U folder rename fully transactional** — Disc sets inside
  `.m3u` folders and companion media/gamelist moves now run as transactions,
  but the folder rename + playlist write in `execute_m3u_rename` are still
  individual operations without rollback.

## Analyzer: Compressed Disc Formats

- [ ] **GameCube NKit support** — NKit is a lossy-compressed format (`.nkit.iso`, `.nkit.gcz`) that removes junk/padding data. Hashes will not match Redump unless converted back to full ISO. May need special handling or a warning that NKit images can't be verified against Redump.

- [ ] **Check nod v2.0 stability** — The `nod` crate v2.0 may bring API changes. Check for stability and migration when it releases.

## Format Conversion (deferred from 2026-07-10 CHD compression work)

CHD *compression* (cue/gdi/iso → chd via chdman, with round-trip verification)
shipped in `retro-junk-lib::chd_convert` + GUI dialog + CLI `compress`.
Deferred follow-ups:

- [ ] **PREGAP/POSTGAP round-trip gap compensation** — `chd_convert::plan_compression`
  now rejects (`ChdConvertError::UnsupportedLayout`) any CUE that declares
  `PREGAP`/`POSTGAP` (gap not stored in the track file), because chdman
  synthesizes those gaps into the CHD and materializes them again on
  extraction, making the extracted track longer than the source span. A
  disc with such a cue currently just can't be compressed. A future
  enhancement could compensate during `verify_round_trip` (e.g. skip the
  synthesized gap region when comparing, using `CueTrack::pregap_frames`/
  `postgap_frames`) instead of rejecting at plan time.

- [ ] **`convert_cue_to_standard` still requires space-separated directives** —
  The CDRWin→standard cue fixer's directive detection (`upper.starts_with("DATAFILE ")`
  etc.) was not updated to the tab-tolerant keyword/rest split that
  `cue::parse_cue` now uses (2026-07 CHD remediation, Phase A3). A
  tab-separated CDRWin cue would fail to auto-fix even though `parse_cue`
  itself now parses tab-separated cues fine. Low priority: CDRWin-format
  cues encountered in practice have been space-separated. Fixing this
  properly means extracting `parse_cue`'s directive-token/rest split into a
  helper both functions share (DRY win alongside the fix).

- [ ] **CHD decompression (chd → cue/bin)** — the reverse operation. Can be
  done natively (retro-junk-disc already decodes hunks + CHT2 track metadata;
  writing bins + generating a cue is a modest extension) or via
  `chdman extractcd`, which `chd_convert::verify_round_trip` already invokes —
  most of the plumbing exists.

- [ ] **RVZ compression/decompression for GameCube/Wii** — `nod` 2.0
  (currently 2.0.0-alpha.10, what nodtool ships on) adds a `DiscWriter` with
  RVZ/WIA/ISO output, compression options, and multithreading. Requires
  upgrading from nod 1.4 (read-only). Native Rust both directions — no
  external tool needed. Verify via the existing Redump hashing path.

- [ ] **CSO/ZSO/DAX support for PSP** — the PSP analyzer lists `cso`/`dax`
  extensions but cannot actually read them (no decompression). CISO is a
  trivial format (block index + per-block deflate; ZSO uses LZ4) — native
  read *and* write is a small job with `flate2`/`lz4`. Fix the read gap
  first, then offer compression.

- [ ] **Batch/whole-library compression job queue** — GUI compression runs
  per-console selection today. A library-wide "compress everything eligible"
  pass with disk-space preflight (verification needs temp space equal to the
  uncompressed size) would suit large collections.

- [ ] **Optional verification skip / quick mode** — round-trip verification
  roughly doubles wall time per disc. Consider a settings toggle
  ("verify: full round-trip / chdman verify only"), keeping full round-trip
  mandatory whenever source deletion is enabled.

- [ ] **chdman codec/hunk tuning knobs** — expose `--compression` /
  `--hunksize` / `--numprocessors` per platform via analyzer hints (e.g.,
  some PPSSPP guidance favors 2048-byte hunks for PSP CHDs). Defaults are
  fine for current emulators; revisit when evidence appears.

- [ ] **CLI compress should honor the GUI's chdman path setting** — the GUI
  stores `general.chdman_path` in `~/.config/retro-junk/settings.toml`; the
  CLI currently only has `--chdman` + PATH. Needs a shared typed settings
  struct in retro-junk-lib instead of the GUI-owned one (see also the DRY
  note below).

### CHD / analyzer-trait follow-ups (deferred from the 2026-07-14 CHD remediation, Phase F)

- [ ] **`DiscSupport` capability object.** `RomAnalyzer` has accumulated ~5
  independent disc-specific optional methods (`dat_source`, `redump_slug`,
  `dat_names`, `compute_container_hashes`, `chd_extensions`) whose defaults
  fail silently — the Sega CD/Dreamcast hashing gap (closed by the C2
  invariant test) was the proof. Proposed shape: `fn disc_support(&self) ->
  Option<&dyn DiscSupport>` returning one bundle so the compiler forces the
  whole set at once. Large cross-crate refactor; the C2 invariant test
  contains the risk until then.

- [ ] **Case-insensitive m3u entry resolution** in `find_correct_m3u_entry`
  (`retro-junk-lib/src/rename.rs`) — `chd_convert::update_m3u_references` now
  delegates to this machinery (2026-07 CHD remediation, Phase B5), but on
  case-**sensitive** filesystems a playlist entry whose case differs from the
  actual file still misses a fix, because the fallback lookup probes the
  directory only with exact-case candidates. Extend it to probe
  case-insensitively before giving up.

- [ ] **GDI-aware `expand_disc_set`.** `chd_convert::plan_compression`'s gdi
  branch inlines resolve-tracks-and-collect-missing that
  `disc_set::expand_disc_set` provides for cues. `DiscSetFiles` is cue-shaped
  (`cue: PathBuf` field); unifying means generalizing that struct — worth
  doing together with any future `.toc`/`.ccd` support, not before.

## Raw Redumper Archival + Dual Representation (design, 2026-07-16)

Goal: ingest **raw redumper dump folders** (`.scram`/`.subcode`/`.state`/`.fulltoc`/`.log`) as
archival sources, and let one logical game hold **multiple physical representations** — a pristine
archival master and a compressed emulator-playable copy — that live in **separate sibling folders**
(`RetroLibrary/archive/` vs `RetroLibrary/roms/`) yet share metadata and scraped assets. Driven by a
Syncthing-synced library with per-device selective sync. Format knowledge is in
`.claude/skills/retro-archive/formats/Redumper.md`; methodology/prior-art in
`PreservationVsPlayable.md`. Design reviewed by a Fable subagent; its ranked risks are folded in below.

**Decide before building (the load-bearing question):**
- [ ] **Pick the persistence world.** The GUI renders from the **`library_entries` cache world**, not
  the catalog `media` world (they're disjoint; joined only via `cover_title`/`screen_title`
  enrichment, `views/library.rs`). A `media_representations` table under `media` would NOT surface in
  the GUI without either migrating the library view onto the catalog or duplicating representation data
  in the cache world.
  - **Direction (per the "authoritative over cache" lean, 2026-07-16):** make the **catalog** the
    authoritative home for representations (a representations table under `media`, plus the YAML
    catalog as it becomes the matching source of truth), and keep **`library_entries` a pure derived
    cache** — rebuildable, and *read* on hot paths only where hitting the catalog live is **observably**
    too slow. Do not treat the cache as a second primary model. This **converges with** the planned
    "Migrate matching source-of-truth from raw DAT to catalog DB" work below, so the dual-representation
    feature becomes a reason to advance that migration rather than route around it.
  - Fable's "cheapest = model it in the cache world" is therefore reframed as a **performance question,
    not the default**: only cache once a real slowdown is measured. The likeliest place that need shows
    up is the selective-sync / multi-device case (many entries, files absent locally) — measure there
    first, don't preemptively cache. Do not straddle both worlds.

**Data model:**
- [ ] **Add a representation/location model** (`kind` = source/archive/playable, `format` =
  redumper/cue-bin/iso/chd/rvz, `location_id`, entry-point `path`, integrity hashes,
  `redumper_build`). `path` = entry point (cue/gdi/iso/folder); keep per-track detail in the existing
  `media_tracks`. Do **not** enumerate each `.bin` as its own representation.
- [ ] **Do NOT relax `collection`'s `UNIQUE(media_id, user_id)`.** `collection` carries ownership
  (owned/condition/notes), which is per-dump, not per-representation; overloading it muddies ownership
  queries and forces a risky 12-step SQLite table rebuild. Add a separate additive table; deprecate
  `collection.rom_path`.
- [ ] **Identity vs integrity hashes.** Logical identity stays the normalized `media.sha1`
  (`compute_container_hashes` already collapses CHD/RVZ to the uncompressed representation, so all
  playable forms resolve to one SHA1). A raw `.scram` matches no DAT — its representation hash is
  **integrity-only** (bit-rot), not identity.

**Storage layout / multi-root:**
- [ ] **Model `archive/` as a derived sibling directory, not a peer library root.** Reuse the existing
  `assets_dir`/`metadata_dir` sibling-resolution pattern (`state.rs:140-182`): for
  `roms/psx/Game (USA).chd`, resolve `archive/psx/Game (USA)/` by convention. Avoids a multi-root
  rewrite, the `find_by_folder` collision (two roots with a `psx/` folder → one silently dropped,
  `state.rs:70`), and cross-root hash correlation. If true multi-root is ever pursued instead, consoles
  MUST be re-keyed by `(root, folder)` and a cross-root identity key (normalized `media.sha1`) chosen.

**Selective-sync correctness (missed in first design pass):**
- [ ] **Per-device presence ≠ catalog existence.** Under selective sync a device routinely has a
  catalog row whose file is absent locally; `verify_collection` currently treats an absent path as an
  error (`scan_import.rs:225`) and would fire constantly. Make "known but not present here" a normal
  state.
- [ ] **Store representation paths root-relative + `location_id`,** not absolute
  (`scan_import.rs:165`) — absolute paths don't survive different mount points across devices.

**Scanner / ingestion:**
- [ ] **Teach the scanner to see raw folders.** `scan_game_entries` only recognizes top-level files by
  extension and `.m3u` dirs (`scanner.rs:110`). Add detection of a directory containing
  `.scram`/`.sdram`/`.sbram` = redumper archive, a `GameEntry::RedumperRaw` variant, and an
  entry-creation path that lists **archive-only games with no playable file**.
- [ ] **Ingest via `redumper split` + `redumper hash` subprocess,** mirroring `Chdman::detect()`
  (`chd_convert.rs:70`). No JSON output exists; the `.log` `dat:` block is clrmamepro `<rom .../>`
  lines — route them through the **existing `retro-junk-dat` parser**, not a bespoke scraper.
- [ ] **Handle split failure as a first-class state.** `redumper split` throws on unrecovered C2/SCSI
  errors or positive combined offset with missing lead-out. "Archive present, playable unrealizable"
  must be a normal state that stores the `.log` error and does not mark the game bad.
- [ ] **Record the redumper build; verify against Redump DB hashes, not byte-identical re-splits.**
  redumper ships rolling builds with no determinism guarantee across versions.

**GUI/UX:**
- [ ] **One row per game; representation badge cluster** (source/archive/playable, filled/hollow) in
  the game table; a **Representations** section in the detail panel with per-row
  Verify/Regenerate/Compress/Reveal actions; context-menu items for ingest/verify/regenerate. Badges
  must roll up across discs for a multi-disc `.m3u` entry.
- [ ] **Reuse `chd_convert::finalize_verified`** (already round-trip-verifies before deleting sources)
  for the Compress action.

**DB / sync hygiene (independent of this feature but surfaced by it):**
- [ ] **Move the catalog DB from XDG cache → XDG data.** It's in `~/.cache/retro-junk/dats/catalog.db`
  (`app.rs:177`) but CLAUDE.md calls it the long-lived store; cache dirs get cleaned.
- [ ] **Never sync a live WAL SQLite DB** through Syncthing (atomic per-file, not across `.db`/`-wal`;
  corruption risk). Keep the DB out of the synced tree (currently true); sync the YAML catalog and
  rebuild the per-device cache. On open, detect sibling `*.sync-conflict-*` DB files and warn.

**Prior art to borrow** (`PreservationVsPlayable.md`): romba/RomVaultX content-addressed depot + built
views; igir `--link-mode` for zero-copy playable projections; MAME merged/split set policy; expose
only the playable tree to frontends.

## DAT Source Coverage

- [ ] **Wii U has no Redump DAT** — Redump.org has no Wii U disc entries or datfile download. The previous LibRetro "Nintendo - Wii U (Digital)" DAT was not real Redump data. DAT support for Wii U is currently disabled. Options: (1) find an alternative DAT source for Wii U, (2) re-enable using LibRetro's DAT with `DatSource::NoIntro` if the data is good enough, or (3) wait for Redump to add Wii U support.

- [ ] **Verify all Redump slugs work** — After switching disc-based DAT downloads from LibRetro to redump.info direct, verify that all slug mappings actually return valid data. Verified 2026-07-10: `psx` and `ss` both return fresh datfile zips. Still to verify: `ps2`, `ps3`, `psp`, `mcd`, `dc`, `gc`, `wii`, `xbox`, `xbox360`. Some systems may have restricted access or different slug conventions on redump.info.

## Data Model & Import Pipeline

- [ ] **Migrate matching source-of-truth from raw DAT files to catalog DB** —
  Currently, hash/serial matching runs against in-memory `DatIndex` built from
  downloaded DAT files each session. Raw DAT files should only seed, update,
  enrich, and fix the catalog DB. The DB should become the authoritative source
  for matching, enabling persistent corrections (e.g., adding missing regional
  entries, resolving cross-region matches) that survive DAT re-downloads.

- [ ] **Re-import after migration v4** — Schema is now at version 4 (`screen_title`, `cover_title` columns added in v3). Run `catalog import all` followed by `catalog enrich` on existing databases to populate `revision`, `variant`, `screen_title`, and `cover_title` fields. This is a one-time user/ops action, not a code gap.

## Deferred module splits (readability, no behavior change)

- [ ] **Finish splitting `retro-junk-lib/src/rename.rs`** — public types now
  live in `rename/types.rs` (2026-07-18). Extract the remaining cohesive
  areas into `serial`, `m3u`, `plan`, `execute`, and `ref_files`, then convert
  the root to `rename/mod.rs` with re-exports so consumer imports stay stable.
- [ ] **Split `retro-junk-db/src/queries.rs` by aggregate** — extract media,
  release, work, collection, and search queries while preserving the existing
  `retro_junk_db` facade.
- [ ] **Split `retro-junk-gui/src/state.rs` by state ownership** — extract
  library/cache, dialogs, operations/jobs, messages, and browse/selection
  state into `state/` submodules; retain one public state facade.
- [ ] **`Override` selector trio → enum** — `entity_id` / `platform_id` /
  `dat_name_pattern` are alternative targeting modes; an `OverrideTarget`
  enum would make illegal combinations unrepresentable (types.rs + YAML
  serde + `apply_overrides`).
- [ ] **CLI disagreement-resolve choice → enum** — `--source-a` /
  `--source-b` / `--custom <value>` are clap-ArgGroup-exclusive bools plus
  an Option; a single value-carrying enum arg would drop the if-chain.
- [ ] **GUI progressive-analysis state → enum** — `LibraryEntry`'s
  `identification` / `hashes` / `dat_match` Options track analysis
  progress alongside `status: EntryStatus`; a full `AnalysisState` enum
  could encode the progression, but each Option carries independently
  consumed data, so this needs a real design pass, not a mechanical swap.
- [ ] **Centralize CLI catalog-db path resolution** — ~25 clap fields
  repeat `db: Option<PathBuf>` + `unwrap_or_else(default_catalog_db_path)`
  (runtime default, not clap-expressible). Resolve once in main and pass
  the resolved path down.
- [ ] **`scan_import::UnmatchedFile.sha1`** stays `Option<String>` because
  upstream `retro_junk_dat::matcher::FileHashes.sha1` is Option (CRC-only
  hashing mode); revisit if FileHashes ever grows an all-or-nothing hash
  group like the scraper's `RomHashes`.
## CLI

- [x] **Flesh out `list` command output** — Resolved: the standalone `list` command was folded into `catalog lookup`. `catalog lookup --type platforms` shows ID, name, manufacturer, year, media type, release/media counts.

## Web Frontend

- [ ] **Create `retro-junk-web` crate** — Web-based frontend for browsing and managing the catalog. Initial scope:
  - Browse platforms, releases, and media with search/filter
  - View release details with associated media assets (box art, screenshots)
  - Collection management (mark owned, add notes)
  - Disagreement review and resolution UI
  - Import/enrichment status and progress
  - Asset coverage dashboard (which releases are missing art)
  - Stack: Axum for HTTP, askama or maud for templates, htmx for interactivity, SQLite read access via shared connection pool. Keep it server-rendered; no SPA framework needed.

## Testing

- [x] **Adopt `egui_kittest` for headless GUI testing** — Done with the egui 0.35 upgrade (2026-07-10): `egui_kittest` dev-dependency with `Harness::new_eframe` smoke tests in `retro-junk-gui/src/app_tests.rs`, built on the hermetic `RetroJunkApp::with_parts` constructor (no settings/DB disk access). Caveat still applies: native `rfd` dialogs live outside the egui scene graph, so kittest can't reach them — flows under test should route confirmations through egui-native modals or an injectable confirm hook.

- [ ] **Expand kittest coverage** — Current tests are startup smoke tests (sidebar present, welcome screen, view switching). Add coverage for root switching, the fragile-mount dialog, game-table selection/filtering, and dialogs; consider snapshot tests (`egui_kittest` `snapshot`+`wgpu` features) for visual regressions.

## Ideas

- [ ] **Handle modded games and homebrew in library** — Games that are modded or homebrew will never match a DAT and show as red (Unrecognized) permanently, cluttering the console list with false-negative indicators. Think about ways to mark or categorize these (e.g., user-applied "homebrew"/"mod" tag, a separate status like `Excluded`, or a filter to hide them from status rollups) so the console list isn't stuck showing red dots.

- [ ] **Auto-resolve trivial disagreements** — Many disagreements are cosmetic (trailing periods, capitalization, date format differences). Add configurable rules to auto-resolve obvious cases during enrichment, reducing manual review noise.

- [ ] **Export to ES-DE / other frontends** — The `scrape` command and GUI already generate ES-DE gamelist.xml for individual systems. Add a `catalog export` command that generates gamelists from the catalog DB for any/all platforms, pulling metadata and asset paths from the database rather than re-scraping.

- [ ] **Collection verification report** — Extend `catalog verify` to produce a summary report: missing ROMs (in DB but not on disk), unmatched files (on disk but not in DB), hash mismatches, and duplicate ROMs across folders.

- [ ] **DAT freshness checking** — Track when each DAT was last downloaded and warn when DATs are stale. Optionally auto-fetch updated DATs before import.

- [ ] **Multi-disc release grouping improvements** — Currently multi-disc games are grouped by title + region + revision + variant. Consider edge cases: different disc counts across regions, bonus discs, demo discs bundled with retail releases.

- [ ] **ROM health dashboard** — Aggregate view across all platforms: total ROMs scanned, verified vs. unverified, trimmed/padded/repaired, missing from known sets (have DAT entry but no matching file in collection).

- [ ] **Overrides YAML expansion** — The overrides system exists but has limited use. Expand with curated override sets for known problem areas: multi-disc serial mismatches (FF7, etc.), regional title corrections, and publisher name normalization.

- [ ] **Apply game mods** - Most mods come as binary modifications to known-good game hashes, and if your game library applies the mod, then it can also automatically flag it as a mod and adjust the metadata correctly and automatically.

- [ ] **Consider using an ORM** crate to help with data types and database management

## Code Health: DRY Violations

Audit findings from 2026-02-26.

### Shared utility functions

- [x] **Consolidate byte-reading helpers within Nintendo crate** — Deleted `ds.rs` private helpers and imported from `n3ds::common` (made `pub(crate)`). Also added bounds checking (`Option<T>` return) to the shared helpers.

- [x] **Extract `get_file_size()` helper** — Added `retro_junk_core::util::file_size()` and replaced ~25 instances of the seek-to-end/seek-to-start pattern across all analyzer crates.

- [ ] **Extract header-reading helper with TooSmall error mapping** — The pattern of `read_exact` + `map_err` converting `UnexpectedEof` to `AnalysisError::TooSmall` appears in `nes.rs:569`, `snes.rs:348`, `gameboy.rs:69`, `gba.rs:61`, `n64.rs:129`, `ds.rs:105`, `ncsd.rs:50`, `genesis.rs:176`, `ps1_disc.rs:161`, and others. Add a helper to `retro-junk-core`:
  ```rust
  pub fn read_header(reader: &mut dyn ReadSeek, buf: &mut [u8], expected: u64) -> Result<(), AnalysisError>
  ```

- [x] **Remove trivial `new()` methods from analyzer structs** — Removed 28 trivial `new()` methods from analyzer structs and `EsDeFrontend`. Updated ~250 call sites to use unit struct literals.

- [ ] **Unify `check_broken_references` and `detect_broken_ref_files`** — `rename.rs` has two functions that both iterate a directory, filter by CUE/M3U extensions, read file contents, call `fmt.extract_reference(line)`, and check `.exists()`. They differ only in return type (`BrokenReference` structs vs. file paths). Unify so `detect_broken_ref_files` is implemented in terms of `check_broken_references`.

- [ ] **Extract GUI semantic color palette** — The same logical colors are hardcoded in 4+ GUI files:
  - Warning orange `Color32::from_rgb(230, 160, 30)` — `status_badge.rs`, `detail_panel.rs`
  - Error red `Color32::from_rgb(220, 50, 50)` — `state.rs`, `app.rs`, `detail_panel.rs`
  - Matched green `Color32::from_rgb(50, 180, 50)` — `state.rs`, `app.rs`
  - Ambiguous yellow `Color32::from_rgb(220, 180, 30)` — `state.rs`, `app.rs`, `detail_panel.rs`

  Extract to named constants in a `theme` or `palette` module. `EntryStatus::color()` in `state.rs` partially centralizes this but other callsites bypass it.

### Test helpers

- [ ] **Extract shared test database setup** — Multiple test files implement similar SQLite test database setup:
  - `retro-junk-db/tests/queries.rs` — `setup_db()`, `setup_db_with_assets()`
  - `retro-junk-import/tests/dat_import.rs` — `setup_db()`
  - `retro-junk-import/tests/scan_import.rs` — `setup_db_with_media()`
  - `retro-junk-import/tests/merge.rs` — `setup_db_with_release()`

  Create a shared `test_helpers` module (e.g., in `retro-junk-db` behind a `#[cfg(test)]` or as a dev-dependency feature) that provides reusable setup functions.

## Code Health: GUI Architecture

Audit findings from 2026-02-27.

- [ ] **Decompose `handle_message`** — `state.rs:handle_message` is 787 lines. Each `AppMessage` match arm should be extracted to a named private handler function for readability and testability.

- [ ] **`check_broken_refs_background` lacks cancellation and progress** — The background thread spawned by `scan.rs:check_broken_refs_background` uses `std::thread::spawn` directly (not `spawn_background_op`) and has no cancel token, no progress messages, and only calls `ctx.request_repaint()` once at the end. On a large library this means multi-second blocking with no feedback. Consider batching repaints every N entries or wrapping in `spawn_background_op` with a cancel token.

- [ ] **`console_tree` and `game_table` duplicate the selectable-list pattern** — Both `widgets/console_tree.rs` and `widgets/game_table.rs` are the same kind of view: a focusable (`FocusedPanel`), keyboard-navigable (shared `keyboard_nav`), status-badge-annotated selectable list with one-shot scroll-into-view. They were implemented independently, and the console tree reintroduced two bugs the table had already solved: (1) it scrolled `scroll_to_me` every frame instead of using a one-shot scroll target — now fixed by mirroring `game_table`'s `scroll_to_row` as `scroll_to_console: Option<usize>`; (2) its hand-rolled `ui.horizontal` + conditional badge + `selectable_label` rows suffered auto-ID churn ("changed id between passes" + scroll resets) — now fixed with `push_id(i)`, whereas `game_table` sidesteps it structurally via `egui_extras::TableBuilder` rows + `paint_cell_text` (which allocates no `WidgetRect`). Consider extracting the shared row/selection/scroll-target lifecycle into a common helper so the two views can't drift again. Note: the console tree also needs manufacturer `CollapsingHeader` grouping that `TableBuilder` doesn't model, so this is a shared-helper refactor, not a switch to `TableBuilder`.

## Code Health: GUI Data Tab (2026-07-10)

Follow-ups from adding the Tools → Data tab (`views/tools_data.rs`,
`backend/catalog_ops.rs`), which surfaced the CLI's catalog data-gathering
pipeline (cache fetch, import, GDB/ScreenScraper enrich) in the GUI.

- [ ] **Promote capability-based console resolution to `retro-junk-lib`** — Both
  the CLI (`retro-junk-cli/src/commands/systems.rs`: `resolve_systems` /
  `SystemCapability`) and the GUI (`backend/catalog_ops.rs`: `targets` / `Cap`)
  independently filter `AnalysisContext` by DAT/GDB capability plus a system
  selection. Extract one shared resolver so the two presentation crates can't
  drift on which systems an operation targets.

- [ ] **Embed catalog seed YAML for self-contained import** — Catalog import
  seeds platforms/companies/overrides from a cwd-relative `./catalog` dir (both
  CLI and GUI; GUI adds a `catalog_data_dir` setting as an escape hatch). An
  installed GUI run from an arbitrary cwd will silently skip seeding and produce
  a catalog with no platforms. Embed the ~156K `catalog/` YAML into the binary
  (e.g. `include_dir` + a `seed_bundled(conn)` in `retro-junk-catalog`/`-db`) so
  import is fully self-contained, then drop the cwd fallback.

- [ ] **Coarse cancellation for ScreenScraper enrich in GUI** —
  `catalog_ops::run_ss_enrich` is only cancel-aware at the connect stage; once
  `enrich_releases` starts it runs to completion (bounded by the per-system
  limit). Thread the cancel token into the enrich loop for mid-run stop, as the
  media scraper in `backend/assets.rs` already does per item.

## Code Health: Cleanup

- [ ] **Decide on user-facing "Media" vs "Assets" terminology** — Rust types were renamed from `MediaType`/`MediaStatus`/etc. to `AssetType`/`AssetStatus`/etc. to disambiguate from physical media types. However, UI strings still say "Scrape Media", "Re-scrape Media", "No scraped media", "Media complete", etc. Decide whether to keep user-facing labels as "Media" (more intuitive to users) or align them with the code terminology ("Assets").

- [x] **Remove dead `CliError` variants** — Removed `DatError` and `Analysis` variants and their constructors.

## Code Health: Safety & Robustness

Audit findings from 2026-03-17. Focus: panic-prone parsing, silent errors, inconsistent patterns.

### Phase 1: Panic-Prone Parsing (Critical)

- [x] **SNES checksum divide-by-zero** — False positive: the else branch only runs when `power != rom_size`, guaranteeing `remainder` is non-empty (`rom_size > power` always holds).

- [x] **N3DS unchecked buffer indexing** — Changed all six helpers in `n3ds/common.rs` to return `Option<T>` with bounds-checked `buf.get()`. Updated all callers in ncsd.rs, ncch.rs, cia.rs, mod.rs. Also consolidated DS duplicate helpers (resolves DRY TODO).

- [x] **Nintendo disc `unwrap()` on slice conversions** — Replaced `try_into().unwrap()` with direct array construction (e.g., `[buf[0x18], buf[0x19], ...]`). Safe because buffer is `[0u8; 0x440]` with all accesses within bounds.

- [x] **ISO 9660 directory record buffer overrun** — Added `if data.len() < 33 { return None; }` upfront check in `parse_directory_record()`.

- [x] **NES header length not validated** — False positive: `parse_ines_header` takes `&[u8; 16]`, a fixed-size array reference. All indexing is compile-time safe.

- [x] **iNES exponent overflow** — Added `if exponent >= 32` guard before both PRG and CHR ROM `1u32 << exponent` shifts, returning `AnalysisError::corrupted_header`.

- [x] **ISO 9660 unbounded memory allocation** — Added `MAX_ISO_FILE_SIZE` (256 MB) constant and validation in both `read_file_content()` and `read_file_from_chd()`.

### Phase 2: Correctness & Data Integrity

- [x] **WiiU missing `dat_source()` override** — Added `fn dat_source() -> DatSource::Redump` to WiiU analyzer.

- [x] **Remove debug `println!` in GameBoy** — Deleted the `println!("gb/c serial: {}", serial)` line.

- [x] **ClrMamePro DAT silent size parsing failure** — Replaced `unwrap_or(0)` with explicit `match` that logs a warning and returns `None` to skip entries with invalid sizes.

- [x] **Miximage `unwrap()` panic** — Replaced `unwrap()` with `if let Some(layout)` pattern.

- [ ] **`unchecked_transaction()` prevents auto-rollback** — `transaction()` requires `&mut Connection` but public API uses `&Connection`. Changing signatures would be a larger refactor. The `unchecked_transaction()` usage is correct for these top-level, non-nested contexts; rollback happens on drop.

- [x] **u64-to-i64 overflow in DAT import** — Replaced bare `as` casts with `i64::try_from().ok()` and `i32::try_from().ok()`.

- [x] **Genesis checksum overflow** — False positive: `u32::MAX as u64 + 1` fits in u64. Replaced `as u64` with explicit `u64::from()` for clarity.

### Phase 3: GUI Silent Error Modes

- [x] **Error dialog for failed operations** — Added `UserError` struct, `error_list` field, `push_error()` helper, and `error_dialog` widget. `HashFailed`, `ScrapeEntryFailed`, `ScrapeFatalError`, `ExportComplete(Err)`, `DatLoadFailed`, and tag dialog DB failures now show a modal error dialog. (Note: these messages were always matched in `handle_message()` and logged, but never surfaced to the user.)

- [ ] **Folder scan errors silent** — `retro-junk-gui/src/backend/scan.rs:40-43`: Scan errors are logged but never shown to users. Empty results are indistinguishable from errors. Low priority — background noise, not user-initiated.

- [ ] **Cache save failures silent** — `retro-junk-gui/src/app.rs:200-228`: `save_library_cache()`, `save_console_cache()`, `save_entry_cache()` failures only produce `log::warn!()`. Low priority — background housekeeping, visible in log viewer.

- [ ] **Loading state can persist forever** — `retro-junk-gui/src/app.rs:73-76`: `loading_library` flag has no timeout. If startup thread crashes, UI shows "Loading..." forever. Same issue with `ScanStatus::Scanning` in `state.rs:95-100`. This is a UI state management issue, not an error dialog issue.

- [ ] **Rate-limit batch error dialogs** — During large batch operations (e.g. hashing hundreds of files), many `HashFailed` errors could flood the dialog. Consider capping displayed errors (e.g. show first N, then "and X more...").

- [ ] **Error dialog "copy to clipboard" button** — Add a button to copy error details for bug reports.

### Phase 4: Analyzer Consistency

- [ ] **Missing `expects_serial()` in 4 analyzers** — NES, SNES, GameBoy, Genesis all implement `extract_dat_game_code()` but don't declare `expects_serial()`, creating ambiguity for DAT diagnostics.

- [ ] **Genesis missing "format" key** — `retro-junk-sega/src/genesis.rs`: All other analyzers insert a `"format"` key into the `extra` HashMap; Genesis does not. Breaks UI display consistency.

- [ ] **Inconsistent seek/rewind in `can_handle()`** — SNES `can_handle()` calls `detect_mapping()` which seeks multiple times but doesn't guarantee reader position reset. Other analyzers (GBA, GameBoy) mix `let _ = reader.seek()` with explicit error checking.

### Phase 5: API & Type Polish

- [ ] **Add missing trait derives** — `FileHashes` and `AnalysisProgress` should derive `PartialEq, Eq`. `DiscGroup` should derive `PartialEq, Eq, Hash`. All fields support these traits.

- [ ] **Complete `RomIdentification` builder pattern** — Only 4 of 11 fields have builder methods (`with_serial`, `with_internal_name`, `with_region`, `with_platform`). Add `with_version()`, `with_file_size()`, `with_expected_size()`, `with_maker_code()`, `with_checksum()`, `with_extra()`.

- [ ] **Make `PlatformParseError` field private** — `retro-junk-core/src/platform.rs:222`: Public `String` field is never accessed directly. Make it `(String)` (private) for encapsulation.

- [ ] **No SQL LIMIT/OFFSET bounds** — `retro-junk-db/src/queries.rs` (8+ functions): Pagination parameters are interpolated without validation. A limit of `u32::MAX` could exhaust memory. Add reasonable caps.

## Code Health: UX Consistency

Audit findings from 2026-03-17.

### Naming & Terminology

- [ ] **"ROM" vs "entry" vs "game" inconsistency** — CLI help says "ROMs" but data model is `GameEntry` which includes multi-disc folders. Standardize to "entries" or "games" in user-facing text.

- [ ] **"Catalog" vs "Database" mixed** — GUI code uses `catalog_db` and "Catalog Tools" but also "Library cache: stored in catalog DB". Pick one term for user-facing text.

- [ ] **"Compute" vs "Calculate" hashes** — Hash backend says "Computing hashes" but buttons say "Calculate All Hashes". Pick one verb.

- [x] **`RomFilterArgs` filters consoles, not ROMs** — Renamed to `ConsoleFilterArgs`.

### Missing User Feedback

- [ ] **Keyboard shortcuts undocumented** — Ctrl+1/2/3 (view switching), Cmd+A (select all), arrow keys, Page Up/Down, Enter, Escape are implemented but never documented in UI. Add a help dialog or tooltips.

- [ ] **Settings path validation absent** — `retro-junk-gui/src/views/settings.rs:110-141`: Invalid metadata/media directory paths accepted without feedback. User discovers the problem only on first use.

- [ ] **DAT status not visible in console tree** — Console tree shows scan status and entry count but no indicator for DAT load state. Users can't tell why serial matching isn't working.

- [ ] **Cancellation lacks confirmation** — Clicking Cancel on an operation provides no visual acknowledgment. Add a brief "Cancelled" state.

## Code Health: DRY Violations (2026-03-17)

- [ ] **CLI `credentials show`/`setup` duplicate credential field metadata** — `retro-junk-cli/src/commands/credentials.rs:78-124` builds its own field list ("dev_id", "dev_password", …) by hand. `retro_junk_scraper::CREDENTIAL_FIELDS` (added 2026-07-15 for the GUI ScreenScraper settings section) is now the single source of field keys/env vars/descriptions; the CLI commands should iterate it instead.

- [ ] **Repeated path extension checking** — `retro-junk-lib/src/scanner.rs` has 4+ copies of the `.extension().and_then().map().unwrap_or(false)` pattern. Extract `has_extension(path, ext)` utility function.

- [x] **Hardcoded disc sector sizes** — Extracted to `retro-junk-disc::sector` as `RAW_SECTOR_SIZE`, `ISO_SECTOR_SIZE`, `MODE1_DATA_OFFSET`, `MODE2_FORM1_DATA_OFFSET`, etc. Used by both Sony and Sega crates.

- [x] **Near-duplicate `read_file_content` / `read_file_from_chd`** — Unified in `retro-junk-disc` crate. Both functions now live in `iso9660.rs` and `chd.rs` respectively, with shared `DirectoryRecord` type and consistent interfaces.

## Code Health: DRY Violations (2026-08-03 review)

- [ ] **Four copies of the `CompleteCatalogMediaMatch` SELECT column list + row mapping** in `retro-junk-db/src/archive.rs` (`match_catalog_serial_any_platform` ~:406, `match_catalog_file_inner` ~:461, `match_single_track_catalog_media` ~:777, `match_complete_catalog_media_inner` ~:844). This week's diff edited all four in lockstep, including a verbatim-duplicated comment block about empty digests. One shared column-list constant + row closure. Note: when this fold happens, `release_disc_count` (added 2026-08-03 as a separate query) could instead become a column on the match itself.

- [ ] **A fourth sibling of the regional folder tables.** `projection_alias_key` (`retro-junk-backend/src/ops/scan.rs`) mirrors the same regional-name distinctions as `regional_archive_platform` (`retro-junk-db/src/library.rs`) and the archive importer's table, but returns a *different* spelling for the same console (`pc-engine`/`turbografx-16` vs `pce`/`tg16`). Folding them is not a pure refactor: these strings key stored console projections, so unifying them needs a migration for existing libraries. Include in the fold below.

- [ ] **Three hand-mirrored region alias tables and two platform+region→directory tables.** `Region::from_slug` (`retro-junk-core/src/region.rs:96` — its own comment says it "mirrors the ones the archive importer already accepts"), the importer's region matches inside `regional_physical_platform`, and the scraper's `region_slug_to_ss_code` must agree by convention. Likewise `esde::system_directory` (`retro-junk-frontend/src/esde.rs`) and `retro_junk_archive::regional_physical_platform` — they agreed only by accident until 2026-08-04, when PC Engine + Europe was settled on `pce`/`pcengine` in both (see `.claude/skills/retro-archive/consoles/PCEngine_Overview.md`) — nothing stops the next divergence. Fold onto `Platform`/`Region` methods. (The two *copies* of `regional_physical_platform` were unified 2026-08-03; this entry is about the remaining sibling tables.)

- [ ] **Small duplications in `retro-junk-work`:** the "join failed candidate details" block appears twice in `incoming.rs` (~:253 and ~:356), and `suggestions.rs:206` clones `ExecContext` field-by-field because the struct doesn't implement `Clone` — derive it and delete the hand copy.

## Enrichment Pipeline Hardening

Audit findings from 2026-02-25. Goal: make `catalog enrich` reliable enough to run hands-off on a server for months.

All 15 items resolved — see commit history for details.

## Phase A follow-ups (automation foundation, 2026-07-30)

- [x] **Phase A.5: scrape orchestration consolidation** — Done (2026-07-31).
  `retro-junk-scraper/src/session.rs` is the one orchestration; `scrape_folder`
  is a folder adapter over it, the GUI builds targets and translates events,
  and `retro_junk_work::scrape` gives the executor a third call site.
  `ActionKind::Scrape` derives from expected-vs-archived artwork, gated by
  `auto_scrape` (off by default) with weak matches filed as appliable
  suggestions. Quota throttling landed first: 429 is retryable and honors
  `Retry-After` against a global gate, and a daily reserve stops a run rather
  than burning the budget.
- [ ] **Instant-apply imports** — `plan_import` re-hashes on suggestion apply.
  The incoming pipeline already computed the full inventory digests at arrival;
  extend `retro-junk-archive-import` to accept precomputed digests so applying
  a suggestion executes with zero re-reads (matters for large disc dumps over
  network mounts).
- [ ] **Per-release incremental reconcile** — `reconcile_archive_snapshot`
  rebuilds the whole projection; the daemon and executor batch it, but the
  biggest remaining network win is reconciling only the releases an action
  touched.
- [ ] **Miximage staleness derivation** — add a `GenerateMiximage` convergence
  kind once component staleness (source artwork vs generated image) is modeled.
- [ ] **`ArchiveLock::acquire_wait` fairness** — daemon+GUI contention is
  fail-fast/wait polling today; add FIFO fairness if contention proves noisy.
- [x] **CLI Ctrl-C for `sync`** — Done (Phase A.5): `sync` and `scrape` both
  install `retro_junk_work::daemon::install_signal_handlers` and thread the
  resulting flag into the executor / scrape core.

## Phase B follow-ups (GUI surfacing, 2026-07-31)

- [x] **Artwork evidence is presence-only** — Done (Phase A.5): the expected
  asset set is `AutomationPolicy::scrape_asset_types`, and
  `convergence::scrape_gaps` is the one comparison behind derivation, the
  summary's done count, and the badge, which now reads "3 of 6" and offers a
  scrape rather than a projection when the set is short.
- [x] **`adopt_playable` suggestions cannot be applied** — Done (2026-08-02) on
  both surfaces at once, through the one dispatch in
  `retro_junk_work::suggestions`. The sweep now records the candidates it found
  and would not choose between, so a review is a question with answers attached
  rather than a note: an ambiguous byte-identical match carries every archived
  master with those bytes, and an ambiguous catalog match carries every medium.
  Accepting one re-proves the claim before writing anything — rescan, re-hash,
  re-compare — so a reviewed adoption is never weaker evidence than an automatic
  one, and a file that moved since the sweep says so instead of recording a
  stale conclusion. Adopting as a master's derivative is shared with the sweep's
  own path (`archive_ops::adopt_identical_playable`) rather than reimplemented.
  Reviews with no candidate stay unappliable, which is honest — the resolutions
  there are to leave it or to ignore it for good — and `offered_actions` is what
  both surfaces ask, so a button never appears that does nothing.
  - Still open: **rename-to-match** as a resolution. A stray whose name is
    merely wrong is a real case and is not covered; it needs the canonical-name
    machinery (`whole_medium_stem`) wired into a review action.
- [ ] **A permanently broken incoming package cannot be silenced.** Noticed
  2026-08-02 while rebuilding the Inbox. A failed package can be retried or
  forgotten (`remove_incoming_package`), but forgetting only clears the row:
  the file is still in the drop folder, so the watcher observes it again and
  files it again. The Inbox's "Forget" button says so in its tooltip rather
  than pretending otherwise, which is honest but not a resolution. Wants the
  same shape adoption reviews just got — a durable per-path decision the
  watcher consults — or a `skipped` package state that survives re-observation
  until the file's fingerprint changes.

- [ ] **The path-pattern matcher has no escape syntax.** `retro_junk_io::glob`
  reads `*`, `?`, and `[…]` as wildcards with no way to mean them literally.
  That is fine for patterns a person types, and mostly fine for the "ignore
  this exact file" path, which builds a rule from a relative path — GoodTools'
  `[!]` survives by accident, because an empty character set falls back to a
  literal bracket. A name containing something like `[a-b]` would not. Either
  add a `\` escape, or have the ignore-one-file path build an
  explicitly-literal rule instead of reusing the pattern syntax.

- [ ] **Backlog scope guesses the platform from the first archived release** —
  a console page with no archived releases falls back to profile-wide scope.
  Project the archive platform onto the library console row so the scope is
  read rather than inferred.
- [ ] **Daemon start has no failure feedback** — `start()` spawns and returns;
  if the CLI exits immediately (bad profile, uninitialized archive) the only
  evidence is the captured log. Poll for the PID file appearing and surface
  the tail on failure.
- [ ] **`retro-junk daemon status` and the GUI section duplicate their
  formatting** — both render the same PID/heartbeat/summary facts. Extract the
  status model into `retro-junk-work` so both callers format one struct.
- [ ] **GUI dirty-tick polling (roadmap B7)** — `runtime_state.dirty_tick`
  already bumps on every coordination commit; add the 1 Hz GUI poll feeding
  `LibraryChangeSet` refresh so daemon writes appear without manual refresh.
- [ ] **Profile editor for `incoming_roots` / `watch_backend`** — the fields
  exist on `CollectionProfile` (settings.toml-editable); add GUI controls next
  to the profile root pickers.
