# UX Roadmap

Living document. Owns the long-term plan for turning retro-junk into a
polished "open the app, browse my collection, fix it up, walk away"
experience — with background automation doing the heavy lifting, and the
collection usable away from the computer (e.g. checking what you own while
standing in a game store).

For the catalog/scraper/analyzer technical backlog, see `TODO.md`. For the
archive/playable-library data model this roadmap builds on, see
`docs/archive-architecture.md` and `docs/collection-library-state-matrix.md`.

## Status (2026-07-30) — Phase A landed

Phase A shipped in seven step-commits: startup latency fixes (~15 s → <1 s
on the network-mounted library by painting from committed projections),
the evidence-currency/mount-detector/dir-helper consolidations,
`retro_junk_lib::archive_ops` (one orchestration per destination for CLI,
GUI, and daemon), schema v22 with the cross-process coordination store
(claims, errors, suggestions, incoming packages, runtime state),
`retro_junk_db::convergence` (the one derivation of pending work),
the `retro-junk-work` crate (automation-first policy, executor, staged
worker), the watcher + pre-processing incoming pipeline + foreground
daemon with CLI `sync`/`status`/`daemon`/`suggestions`, and the GUI
rewired through the shared executor with an `[automation]` Settings
section plus a status-bar suggestion count. Verified live: with
`auto_import = on`, a dump dropped into a watched folder becomes an
archived, integrity- and catalog-verified, canonically-named playable
with projected gamelist — zero interaction. Next: Phase A.5 (scrape
consolidation; TODO.md), then Phase B.

## Original context (2026-07-29) — rebooted atop the v0.4 archive architecture

A previous Phase 1 (watcher + worker + policy + daemon CLI) was fully
implemented against the pre-0.4 schema and is preserved on the
`daemon-phase1-reference` branch. That code targeted a simpler data model
(`library_entries`/`library_media` with per-kind completion-timestamp
columns) that the archive architecture has since superseded. **The concepts
carry forward; the code does not.** This document is the plan for
reimplementing that vision on top of what now exists:

- portable archive manifests + append-only evidence as the source of truth;
- SQLite (schema v21) as a rebuildable projection, mutated command-first;
- the convergence matrix defining every honest starting-state → destination
  route, with CLI and GUI already calling the same implementations;
- a GUI on egui 0.35 with a serialized `LibraryStore` worker and
  revision-aware projections.

What the reference branch proved out and this reboot ports as *designs*:
derive-work-from-state (no queue), claim/heartbeat/reap coordination,
watcher event coalescing and rename detection, conservative auto-action
policy, and suggestions-as-reviewable-actions. See "Mining the reference
branch" below.

## Target user experience

1. **First launch** — point at an archive and/or playable root, browse
   immediately. (Works today; onboarding guidance still to come.)
2. **Drop and walk away** — drop a new dump into a watched incoming folder,
   or a ROM into the playable tree: it is staged, identified, archived,
   verified, built to the preferred playable format, scraped, and projected
   to the frontend — automatically, within policy. Anything ambiguous or
   policy-blocked lands in a review inbox instead of happening silently.
3. **Visible progress** — while browsing, per-row status and a backlog
   summary show what's verified, what's queued, and what failed, so large
   disc verifies don't feel like a hang.
4. **Review, don't babysit** — the inbox presents proposed actions
   (bindings, renames, adoptions, scrapes) as one-click apply/dismiss cards.
5. **CLI and daemon continue where the GUI left off** — same derivation,
   same implementations, same database; quit the GUI mid-convergence and
   nothing is lost.
6. **Collection in your pocket** — export the owned-collection inventory
   (platform, title, region, serial, condition, completeness) to a
   phone-friendly single file, so "do I already own this?" is answerable in
   a store aisle.
7. **ES-DE handoff stays correct** — playable builds, playlists, artwork
   projections, and gamelist entries stay current without manual re-runs.

## Guiding model: automatic convergence, not a queue

The convergence matrix (`docs/collection-library-state-matrix.md`) already
defines the desired state as "the closest honest state supported by the
evidence." The daemon is nothing more than **a third caller of the same
verification/build/projection implementations the CLI and GUI share**,
walking the matrix continuously instead of on demand.

- **Derivation over queueing.** "What to do next" is computed from current
  state — `presence_state`, evidence currency (`input_manifest_sha256`),
  policy tables, the playable inbox — never from a persistent queue. This
  is the reference branch's core idea, retargeted: the archive schema
  already stores the ground truth the old completion-timestamp columns
  approximated. Origin already computes most of it (the Library view's
  per-console queue of carriers missing their preferred representation;
  `archive build --dry-run`'s prerequisite report); derivation extracts
  that into one shared function instead of adding parallel logic.
- **Policy gates every mutation.** Auto-actions run only above a confidence
  threshold; below it they become inbox suggestions. Preservation masters
  are never touched by convergence (existing invariant).
- **A suggestion is a proposed command.** Since all edits are command-first,
  the inbox stores unapplied commands with provenance — applying one is
  exactly the user having clicked the equivalent GUI action.
- **Evidence stays append-only.** Daemon-driven verifies and builds append
  the same JSON evidence records as interactive runs. Automation must not
  invent a second bookkeeping channel.

## Phase A — Automation foundation (daemon-ready)

The load-bearing chunk. Everything else is small follow-on work.

| # | Piece | Scope |
|---|-------|-------|
| A1 | `derive_convergence(conn, scope) -> Vec<ProposedAction>` | One shared function computing pending work from archive/library state, factored out of the existing Library-queue and `build --dry-run` logic rather than written beside them. `Scope`: all profiles / one profile / one platform / one release / explicit selection. Each `ProposedAction` names the existing implementation it would invoke (verify, verify-catalog, audit-redumper, build, project-frontend-files, generate-miximages, scrape-adopt, adopt-playable triage). |
| A2 | `summarize_convergence(conn, scope)` | Counts per action kind: done / pending / blocked-by-policy / errored. Consumed by CLI `status`, the GUI backlog strip (B5), and daemon logging. Single aggregation, no per-view re-derivation. |
| A3 | Worker | Registers handlers that are thin dispatchers to the existing shared implementations — no logic of their own beyond claim/heartbeat/error recording. `run_once(scope)` and `run_continuously(events)`. Cooperates with the existing recoverable process lock and the archive-refresh lock; per-action claims with heartbeat + stale-reap (port the reference branch's SQL pattern) so a crashed run never strands work and GUI/daemon can share a database safely. |
| A4 | Filesystem watcher | Port of `watch/` from the reference branch (notify wrapper, ~500 ms per-path debounce, basename/hash rename pairing, event coalescing — that logic is schema-independent). Watches: (a) configured **incoming** directories → triggers import identification (dry-run first; auto-import within policy, else inbox); (b) playable roots → additions route to adopt/import-playable triage, renames update bindings without re-verifying unchanged bytes, removals become normal `presence_state` transitions on selectively synced devices — never "loss". The archive tree itself is not watched; it only changes through the tool. |
| A5 | Policy | `AutomationPolicy` (TOML, same file GUI Settings edits): `auto_import: On/Suggest/Off`, `auto_bind_min_confidence` (exact-hash / exact-serial / filename), `auto_build: bool`, `auto_scrape` + `only_when_unambiguous`, quiet hours, pause-heavy-work-on-battery. Conservative defaults (suggest, don't act) — carried verbatim from the reference branch. Complements, never bypasses, the matrix's "boundaries where automation must stop". |
| A6 | Suggestions store | Persist proposed-but-unapplied commands with kind, payload, confidence, provenance, and resolution state. Unifies what today is scattered: `.retro-junk/playable-inbox.toml` triage entries, ambiguous import candidates, policy-blocked auto-actions, and (read-only) catalog `disagreements`. Inbox UI lands in Phase B; CLI can list/apply/dismiss from day one. |
| A7 | CLI surface | `retro-junk sync [--scope ...] [--only KIND]` = `run_once` and exit. `retro-junk daemon start [--foreground] / stop / status / reload`. `status` prints `summarize_convergence` + heartbeat age. Daemon stays a CLI subcommand (shared install, creds, config). |

## Phase B — GUI modernization and surfacing

### B-stage 1: presentation polish (independent, cheap, port from reference branch)

Origin is already on egui 0.35; no upgrade PRs needed. The reference branch
landed these on 0.34 and they re-apply nearly mechanically:

- `egui::Modal` for all dialogs (backdrop, focus trap, consistent
  Escape/Enter) — replaces the `egui::Window`-based dialogs.
- Native macOS menubar (`muda`), window persistence, `egui-notify` toasts.
- Game table on `egui_table` (sticky headers, virtualization, per-cell
  click targets) with row identity by durable entry ID.
- `egui-phosphor` icon set replacing ad-hoc unicode glyphs.

Crates added: `muda`, `egui-notify`, `egui_table`, `egui-phosphor`.
Deliberately skipped (evaluated on the reference branch): `egui-modal`
(stale), `egui_dock`/`egui_tiles`, `egui_flex`/`egui_taffy`, `egui_hotkey`.

### B-stage 2: automation surfaced in the GUI

| # | Piece | Scope |
|---|-------|-------|
| B4 | Per-row convergence badges | Clickable dots per evidence class (present / integrity / catalog / playable / artwork), driven by the same projection data the Collection roll-up already loads. Click → popover with "re-run this" + last error. |
| B5 | Backlog summary strip | Chips from `summarize_convergence(current scope)` above the table toolbar. |
| B6 | Suggested-actions inbox view | Top-level view; badge count of open suggestions. Cards grouped by kind, `[Apply] [Edit…] [Dismiss]`, all routing through the existing command implementations + suggestion resolution. Also the new home for playable-inbox triage and a surface for catalog disagreements/overrides (today table-only in Tools → Browse). |
| B7 | Cross-process refresh signal | Today `LibraryProjectionController`/`apply_change_set` refreshes in-process only. Once the daemon writes concurrently, add a `dirty_tick`-style counter bumped on every mutation commit; GUI polls at ~1 Hz and schedules incremental refresh. (Design proven on the reference branch.) Only needed when A3 lands; do not build speculatively. |
| B8 | ScreenScraper onboarding | Settings → Scraper Account: credential entry, "Test login" with inline quota/error, signup link. Turns the current error-toast dead end into a guided path. |
| B9 | Daemon controls | Settings section: status from heartbeat, start/stop (shells to the CLI), backlog widget (reuses B5), log tail. |

## Phase C — Collection on the go, onboarding, polish

- **C1 Owned-collection export** — `retro-junk collection export --format
  html|json|csv`: a single self-contained, phone-friendly file from the
  ownership data (platform, title, region, serial, condition, completeness,
  physical-copy notes) with client-side search. The store-aisle use case.
  Once the daemon exists, keeping the export current is just another
  convergence action. Possible later: "gaps" view (owned vs. catalog).
- **C2 First-run onboarding** — profile setup guidance, folder scaffolding,
  archive-init walkthrough for the matrix's "no archive yet" row.
- **C3 Library health view** — saved queries over evidence + error state:
  unverified masters, stale evidence, incomplete multi-disc, unbound
  releases, ambiguous adoptions, last-error.
- **C4 Keyboard & search** — F2 rename, Cmd+F search/filter by
  name/serial/region/evidence state, Del with confirmation, Enter detail.
- **C5 Storage & diagnostics** — asset disk usage, orphaned-projection
  cleanup, DAT/catalog snapshot versions, scraper quota, daemon status,
  log locations.
- **C6 Throttle & quiet hours enforcement** — policy fields from A5 wired
  into the worker scheduler (don't hammer ScreenScraper overnight; defer
  disc verifies on battery).
- **C7 Rename preview + undo** — builds on the existing per-game
  filesystem transactions; add preview before bulk apply and a
  `rename_history`-style undo window.

## Mining the reference branch (`daemon-phase1-reference`)

Port as designs/tests, not as diffs:

- `retro-junk-lib/src/work/claim.rs` — atomic claim SQL, ~30 s heartbeat,
  2-minute stale-reap. Schema-independent pattern; retarget at
  `ProposedAction` granularity.
- `retro-junk-lib/src/watch/` — watcher wrapper, coalescing rules
  (debounce, latest-type-wins, removed+added rename pairing), and
  `watcher_tests.rs`/`coalesce_tests.rs` intents.
- `policy.rs` defaults and the `AutoActionDecision` shape.
- Test intents throughout `src/tests/` (~4.5k lines): cancel-mid-run,
  claim-already-held, idempotent re-run, rename-during-work keeps results
  attached to the same durable ID, end-to-end drop-a-file daemon test.
- Explicitly **not** ported: the `library_entries`/`library_media` split,
  per-kind completion columns, `query_owned_collection`, the
  archive-legacy-DB rollforward, and handler internals (origin's shared
  implementations replace them all).

## How this stays DRY

- **One derivation function** (A1), factored out of — not written beside —
  the existing Library-queue and `build --dry-run` prerequisite logic.
  GUI queue, CLI `sync`, daemon, and badges all consume it.
- **One implementation per destination** — the daemon dispatches the same
  verification/build/projection code the CLI and GUI already share by
  invariant. Handlers add coordination only, never logic.
- **One aggregation** (A2) behind the backlog strip, `daemon status`, the
  health view, and daemon logs.
- **One suggestion store** (A6) replacing three ad-hoc surfaces
  (playable-inbox TOML, import-ambiguity prompts, policy-blocked actions).
- **One policy file** edited by GUI Settings and read by the daemon.
- **One refresh signal** (B7) reusing the existing
  `LibraryChangeSet`/projection-controller path rather than a second
  update channel.

## How this maintains and improves best practices

- **Command-first everywhere**: automation goes through the same typed
  commands as interactive edits; no ad-hoc writes reappear.
- **Append-only evidence preserved**: daemon runs append the same records;
  history is never rewritten by automation.
- **Honest states**: automation respects the matrix's stop boundaries —
  no completeness inferred from filenames, no silent deletion, masters
  immutable, absent-on-this-device stays a normal state.
- **Crash-safe coordination**: heartbeat claims + stale reap; partial
  progress persists; resume is derivation re-running, not recovery code.
- **Conservative defaults**: suggest-don't-act out of the box; nothing on
  disk changes silently.
- **Cancel-safe workers**: check cancellation at operation boundaries;
  cooperate with the existing recoverable process lock.
- **Platform-native GUI conventions**: real menubar, standard shortcuts,
  modal etiquette, persisted window state, toasts for transient feedback.

## Deferred / TODO.md candidates

- Launch-from-GUI emulator integration (scope creep unless we become a
  frontend).
- Frontend exporters beyond ES-DE (Pegasus, LaunchBox) — `retro-junk-frontend`
  is shaped for it.
- Custom collections (favorites, playing, beat), soft-delete/quarantine
  view, i18n.
- macOS LaunchAgent / systemd unit generation for the daemon.
- Multi-host federation (daemon on NAS, GUI on laptop) — B7's polling
  design should not preclude it.
- Per-handler concurrency tuning; per-asset-type scrape granularity.
- Scheduled DAT/catalog snapshot refresh driven by policy.
