# Library field ownership

This inventory records the ownership boundary used by the SQLite-authoritative
library transition. `LibraryState` is a temporary compatibility read model; it
must not be treated as durable state.

| Existing field group | Authoritative owner | Projection / lifetime |
|---|---|---|
| Root path and root revision | `library_roots` | Console-summary session |
| Console identity, platform, folder, DAT count, scan state and revision | `library_consoles` | `LibraryConsoleSummary` |
| Entry source identity, serialized `GameEntry`, fingerprint and source revision | `library_entries` | List identity plus on-demand detail |
| Status, hashes, identification, DAT match, catalog titles and ambiguous candidates | `library_entries` | `LibraryEntryDetail`; list exposes badges only |
| Region override and tag | `library_entries` | Detail/list; changed only by typed commands |
| Broken references and CUE compatibility diagnostics | `library_entries`, bound to `source_revision` | Detail |
| Selected/focused entry and selected console | GUI controller, by durable ID | Current UI session |
| Search, filter, sort, offset and visible page | GUI controller | Current root/query |
| DAT indexes and loose-disc operation planning | GUI | Ephemeral |
| Asset paths, decoded textures and media discovery | GUI | Ephemeral and lazily rebuilt |
| Operation progress, cancellation tokens, dialogs and errors | GUI | Ephemeral session state |

`scanning` is intentionally not a persisted scan state. Beginning a scan only
advances `scan_generation`; only reconciliation of the matching token can make
the console `ready` or delete absent entries. A cancelled or failed producer
therefore leaves the previous authoritative rows intact.
