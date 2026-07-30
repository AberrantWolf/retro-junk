//! Cross-process work coordination.
//!
//! Claims let the daemon, CLI, and GUI share one database without stacking
//! the same work: a claim row exists only while an owner works a target, is
//! heartbeat-refreshed, and becomes reapable when stale. Claims coordinate —
//! they never record results; append-only archive evidence and the
//! projection do. Errors, suggestions, the incoming-package ledger, and the
//! runtime singleton live here too; every mutating commit bumps
//! `runtime_state.dirty_tick` so other processes can poll for change
//! cheaply.
//!
//! This is the string-typed storage layer; the convergence layer supplies
//! typed action/target enums that serialize onto it.

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::operations::OperationError;

/// A claim whose heartbeat is older than this is stale: its owner crashed or
/// hung, and anyone may take the work over.
pub const CLAIM_TIMEOUT_MINUTES: i64 = 2;

/// The daemon skips targets that errored more recently than this; explicit
/// `sync` runs and GUI clicks retry regardless.
pub const ERROR_BACKOFF_HOURS: i64 = 6;

/// How a finished (or abandoned) piece of claimed work resolves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// Work completed; any recorded error for the target is cleared.
    Success,
    /// Work failed; the message replaces the target's open error record.
    Failed { error: String },
    /// Work stopped without a verdict; nothing recorded either way.
    Cancelled,
}

/// A live claim row, for "already being built by the daemon" surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldClaim {
    pub owner: String,
    pub since: String,
}

/// Atomically claim `(action_kind, target)` for `owner`.
///
/// Succeeds when no claim exists or the existing claim is stale. Returns
/// `false` when another owner holds a fresh claim.
pub fn try_claim(
    conn: &Connection,
    action_kind: &str,
    target_kind: &str,
    target_id: &str,
    owner: &str,
) -> Result<bool, OperationError> {
    let changed = conn.execute(
        &format!(
            "INSERT INTO work_claims(action_kind,target_kind,target_id,owner)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(action_kind,target_kind,target_id) DO UPDATE
             SET owner=excluded.owner, since=datetime('now')
             WHERE work_claims.since < datetime('now','-{CLAIM_TIMEOUT_MINUTES} minutes')"
        ),
        params![action_kind, target_kind, target_id, owner],
    )?;
    Ok(changed == 1)
}

/// Refresh the heartbeat on a claim this owner still holds.
pub fn refresh_claim(
    conn: &Connection,
    action_kind: &str,
    target_kind: &str,
    target_id: &str,
    owner: &str,
) -> Result<(), OperationError> {
    conn.execute(
        "UPDATE work_claims SET since=datetime('now')
         WHERE action_kind=?1 AND target_kind=?2 AND target_id=?3 AND owner=?4",
        params![action_kind, target_kind, target_id, owner],
    )?;
    Ok(())
}

/// Release a claim, recording or clearing the target's error state per the
/// outcome, and bump `dirty_tick` — all in one transaction.
pub fn release_claim(
    conn: &mut Connection,
    action_kind: &str,
    target_kind: &str,
    target_id: &str,
    outcome: &ClaimOutcome,
) -> Result<(), OperationError> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM work_claims WHERE action_kind=?1 AND target_kind=?2 AND target_id=?3",
        params![action_kind, target_kind, target_id],
    )?;
    match outcome {
        ClaimOutcome::Success => {
            tx.execute(
                "DELETE FROM work_errors WHERE action_kind=?1 AND target_kind=?2 AND target_id=?3",
                params![action_kind, target_kind, target_id],
            )?;
        }
        ClaimOutcome::Failed { error } => {
            tx.execute(
                "INSERT INTO work_errors(action_kind,target_kind,target_id,message)
                 VALUES(?1,?2,?3,?4)
                 ON CONFLICT(action_kind,target_kind,target_id) DO UPDATE
                 SET message=excluded.message, occurred_at=datetime('now')",
                params![action_kind, target_kind, target_id, error],
            )?;
        }
        ClaimOutcome::Cancelled => {}
    }
    bump_dirty_tick(&tx)?;
    tx.commit()?;
    Ok(())
}

/// The fresh claim currently held on a target, if any.
pub fn held_claim(
    conn: &Connection,
    action_kind: &str,
    target_kind: &str,
    target_id: &str,
) -> Result<Option<HeldClaim>, OperationError> {
    let row = conn
        .query_row(
            &format!(
                "SELECT owner, since FROM work_claims
                 WHERE action_kind=?1 AND target_kind=?2 AND target_id=?3
                   AND since >= datetime('now','-{CLAIM_TIMEOUT_MINUTES} minutes')"
            ),
            params![action_kind, target_kind, target_id],
            |row| {
                Ok(HeldClaim {
                    owner: row.get(0)?,
                    since: row.get(1)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// One open failure. Cleared by the next success on the same target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkError {
    pub action_kind: String,
    pub target_kind: String,
    pub target_id: String,
    pub message: String,
    pub occurred_at: String,
}

/// Every open error, newest first.
///
/// The GUI reads the whole set once per refresh and indexes it in memory:
/// a per-row lookup would otherwise be one query per visible badge.
pub fn list_work_errors(conn: &Connection) -> Result<Vec<WorkError>, OperationError> {
    let mut statement = conn.prepare(
        "SELECT action_kind, target_kind, target_id, message, occurred_at
         FROM work_errors ORDER BY occurred_at DESC",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(WorkError {
                action_kind: row.get(0)?,
                target_kind: row.get(1)?,
                target_id: row.get(2)?,
                message: row.get(3)?,
                occurred_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Whether the target errored within the last `backoff_hours` for this
/// action. The daemon consults this; explicit runs don't.
pub fn has_recent_error(
    conn: &Connection,
    action_kind: &str,
    target_kind: &str,
    target_id: &str,
    backoff_hours: i64,
) -> Result<bool, OperationError> {
    let recent: bool = conn.query_row(
        &format!(
            "SELECT EXISTS(
                 SELECT 1 FROM work_errors
                 WHERE action_kind=?1 AND target_kind=?2 AND target_id=?3
                   AND occurred_at >= datetime('now','-{backoff_hours} hours'))"
        ),
        params![action_kind, target_kind, target_id],
        |row| row.get(0),
    )?;
    Ok(recent)
}

// ── Suggestions ────────────────────────────────────────────────────────────

/// A proposed-but-unapplied command awaiting user review.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub id: i64,
    pub kind: String,
    pub target_kind: String,
    pub target_id: String,
    pub payload_json: String,
    pub confidence: f64,
    pub provenance: String,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub resolution: String,
}

/// Inputs for opening a suggestion.
#[derive(Debug, Clone)]
pub struct NewSuggestion<'a> {
    pub kind: &'a str,
    pub target_kind: &'a str,
    pub target_id: &'a str,
    pub payload_json: &'a str,
    pub confidence: f64,
    /// Who proposed it ("daemon", "cli-sync", "gui", …).
    pub provenance: &'a str,
}

/// Open a suggestion, superseding any open one for the same (kind, target)
/// so re-derivation refreshes instead of piling up. History is preserved —
/// superseded rows resolve, they don't vanish.
pub fn open_suggestion(
    conn: &mut Connection,
    suggestion: &NewSuggestion<'_>,
) -> Result<i64, OperationError> {
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE suggestions SET resolved_at=datetime('now'), resolution='superseded'
         WHERE kind=?1 AND target_kind=?2 AND target_id=?3 AND resolved_at IS NULL",
        params![
            suggestion.kind,
            suggestion.target_kind,
            suggestion.target_id
        ],
    )?;
    tx.execute(
        "INSERT INTO suggestions(kind,target_kind,target_id,payload_json,confidence,provenance)
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            suggestion.kind,
            suggestion.target_kind,
            suggestion.target_id,
            suggestion.payload_json,
            suggestion.confidence,
            suggestion.provenance,
        ],
    )?;
    let id = tx.last_insert_rowid();
    bump_dirty_tick(&tx)?;
    tx.commit()?;
    Ok(id)
}

/// All open suggestions, optionally filtered by kind, oldest first.
pub fn list_open_suggestions(
    conn: &Connection,
    kind: Option<&str>,
) -> Result<Vec<Suggestion>, OperationError> {
    let mut statement = conn.prepare(
        "SELECT id,kind,target_kind,target_id,payload_json,confidence,provenance,
                created_at,resolved_at,resolution
         FROM suggestions
         WHERE resolved_at IS NULL AND (?1 IS NULL OR kind=?1)
         ORDER BY created_at, id",
    )?;
    let rows = statement
        .query_map(params![kind], row_to_suggestion)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One suggestion by id, open or resolved.
pub fn get_suggestion(conn: &Connection, id: i64) -> Result<Option<Suggestion>, OperationError> {
    let row = conn
        .query_row(
            "SELECT id,kind,target_kind,target_id,payload_json,confidence,provenance,
                    created_at,resolved_at,resolution
             FROM suggestions WHERE id=?1",
            params![id],
            row_to_suggestion,
        )
        .optional()?;
    Ok(row)
}

/// Resolve an open suggestion (`applied` / `dismissed`). Returns `false` if
/// it was already resolved — the caller lost a race, not an invariant.
pub fn resolve_suggestion(
    conn: &mut Connection,
    id: i64,
    resolution: &str,
) -> Result<bool, OperationError> {
    let tx = conn.transaction()?;
    let changed = tx.execute(
        "UPDATE suggestions SET resolved_at=datetime('now'), resolution=?1
         WHERE id=?2 AND resolved_at IS NULL",
        params![resolution, id],
    )?;
    bump_dirty_tick(&tx)?;
    tx.commit()?;
    Ok(changed == 1)
}

fn row_to_suggestion(row: &rusqlite::Row<'_>) -> rusqlite::Result<Suggestion> {
    Ok(Suggestion {
        id: row.get(0)?,
        kind: row.get(1)?,
        target_kind: row.get(2)?,
        target_id: row.get(3)?,
        payload_json: row.get(4)?,
        confidence: row.get(5)?,
        provenance: row.get(6)?,
        created_at: row.get(7)?,
        resolved_at: row.get(8)?,
        resolution: row.get(9)?,
    })
}

// ── Incoming packages ──────────────────────────────────────────────────────

/// One watched incoming package and its pre-processing state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingPackage {
    pub path: String,
    pub profile_id: String,
    pub first_seen: String,
    pub fingerprint: String,
    pub state: String,
    pub detail: String,
    pub plan_json: String,
}

/// Record a package sighting. A new path, or an existing one whose
/// fingerprint changed, (re)enters `pending` and returns `true` — meaning a
/// planning pass is owed. An unchanged known package returns `false`.
pub fn observe_incoming_package(
    conn: &mut Connection,
    path: &str,
    profile_id: &str,
    fingerprint: &str,
) -> Result<bool, OperationError> {
    let tx = conn.transaction()?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT fingerprint FROM incoming_packages WHERE path=?1",
            params![path],
            |row| row.get(0),
        )
        .optional()?;
    let needs_planning = match existing {
        Some(known) if known == fingerprint => false,
        Some(_) => {
            tx.execute(
                "UPDATE incoming_packages
                 SET fingerprint=?2, state='pending', detail='', plan_json=''
                 WHERE path=?1",
                params![path, fingerprint],
            )?;
            true
        }
        None => {
            tx.execute(
                "INSERT INTO incoming_packages(path,profile_id,fingerprint) VALUES(?1,?2,?3)",
                params![path, profile_id, fingerprint],
            )?;
            true
        }
    };
    if needs_planning {
        bump_dirty_tick(&tx)?;
    }
    tx.commit()?;
    Ok(needs_planning)
}

/// Store a completed plan: the package is ready to import with zero further
/// processing.
pub fn set_incoming_ready(
    conn: &mut Connection,
    path: &str,
    detail: &str,
    plan_json: &str,
) -> Result<(), OperationError> {
    set_incoming_state(conn, path, "ready", detail, plan_json)
}

/// Record a terminal planning problem (bad dump, incomplete, no match,
/// ambiguous).
pub fn set_incoming_error(
    conn: &mut Connection,
    path: &str,
    detail: &str,
) -> Result<(), OperationError> {
    set_incoming_state(conn, path, "error", detail, "")
}

/// Mark a package as imported into the archive.
pub fn set_incoming_imported(conn: &mut Connection, path: &str) -> Result<(), OperationError> {
    set_incoming_state(conn, path, "imported", "", "")
}

fn set_incoming_state(
    conn: &mut Connection,
    path: &str,
    state: &str,
    detail: &str,
    plan_json: &str,
) -> Result<(), OperationError> {
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE incoming_packages SET state=?2, detail=?3, plan_json=?4 WHERE path=?1",
        params![path, state, detail, plan_json],
    )?;
    bump_dirty_tick(&tx)?;
    tx.commit()?;
    Ok(())
}

/// Forget a package (its file left the watched folder).
pub fn remove_incoming_package(conn: &mut Connection, path: &str) -> Result<(), OperationError> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM incoming_packages WHERE path=?1", params![path])?;
    bump_dirty_tick(&tx)?;
    tx.commit()?;
    Ok(())
}

/// One package by path.
pub fn get_incoming_package(
    conn: &Connection,
    path: &str,
) -> Result<Option<IncomingPackage>, OperationError> {
    let row = conn
        .query_row(
            "SELECT path,profile_id,first_seen,fingerprint,state,detail,plan_json
             FROM incoming_packages WHERE path=?1",
            params![path],
            row_to_incoming,
        )
        .optional()?;
    Ok(row)
}

/// All packages, optionally filtered by state, oldest sighting first.
pub fn list_incoming_packages(
    conn: &Connection,
    state: Option<&str>,
) -> Result<Vec<IncomingPackage>, OperationError> {
    let mut statement = conn.prepare(
        "SELECT path,profile_id,first_seen,fingerprint,state,detail,plan_json
         FROM incoming_packages
         WHERE ?1 IS NULL OR state=?1
         ORDER BY first_seen, path",
    )?;
    let rows = statement
        .query_map(params![state], row_to_incoming)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn row_to_incoming(row: &rusqlite::Row<'_>) -> rusqlite::Result<IncomingPackage> {
    Ok(IncomingPackage {
        path: row.get(0)?,
        profile_id: row.get(1)?,
        first_seen: row.get(2)?,
        fingerprint: row.get(3)?,
        state: row.get(4)?,
        detail: row.get(5)?,
        plan_json: row.get(6)?,
    })
}

// ── Runtime state ──────────────────────────────────────────────────────────

/// The runtime singleton.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeState {
    pub daemon_pid: Option<i64>,
    pub daemon_started_at: Option<String>,
    pub daemon_heartbeat_at: Option<String>,
    pub dirty_tick: i64,
}

/// Bump the change counter inside a caller-owned transaction.
pub fn bump_dirty_tick(tx: &Transaction<'_>) -> Result<(), OperationError> {
    tx.execute(
        "UPDATE runtime_state SET dirty_tick = dirty_tick + 1 WHERE id = 1",
        [],
    )?;
    Ok(())
}

/// Stamp daemon startup identity.
pub fn daemon_started(conn: &Connection, pid: i64) -> Result<(), OperationError> {
    conn.execute(
        "UPDATE runtime_state
         SET daemon_pid=?1, daemon_started_at=datetime('now'),
             daemon_heartbeat_at=datetime('now')
         WHERE id=1",
        params![pid],
    )?;
    Ok(())
}

/// Refresh the daemon liveness heartbeat.
pub fn daemon_heartbeat(conn: &Connection) -> Result<(), OperationError> {
    conn.execute(
        "UPDATE runtime_state SET daemon_heartbeat_at=datetime('now') WHERE id=1",
        [],
    )?;
    Ok(())
}

/// Read the whole singleton.
pub fn read_runtime_state(conn: &Connection) -> Result<RuntimeState, OperationError> {
    let state = conn
        .query_row(
            "SELECT daemon_pid, daemon_started_at, daemon_heartbeat_at, dirty_tick
             FROM runtime_state WHERE id=1",
            [],
            |row| {
                Ok(RuntimeState {
                    daemon_pid: row.get(0)?,
                    daemon_started_at: row.get(1)?,
                    daemon_heartbeat_at: row.get(2)?,
                    dirty_tick: row.get(3)?,
                })
            },
        )
        .optional()?
        .unwrap_or_default();
    Ok(state)
}
