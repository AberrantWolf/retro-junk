//! Watched incoming folders: pre-process once, then import or suggest.
//!
//! Every package that appears in an incoming root is hashed and identified
//! immediately, so the user reviews finished identifications, not raw
//! files. Per policy, unambiguous confident matches import themselves;
//! everything else becomes a suggestion or an honest error state
//! (`bad dump`, `no match`, `ambiguous`). A size+mtime fingerprint detects
//! files changing under a stored plan and sends them back to `pending`.
//! Sources are never deleted — `--consume` stays a manual decision forever.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use retro_junk_archive_import::{
    DumpImportPlan, DumpImportRequest, IdentificationMethod, IdentificationResolution,
    ImportDisposition, execute_import, plan_import,
};
use serde::{Deserialize, Serialize};

use crate::executor::{ExecContext, WorkError};
use crate::policy::{AutoImportMode, AutomationPolicy, BindConfidence};

/// What a stored import suggestion shows the reviewer. The expensive
/// pre-processing (hashing + identification) already happened; applying
/// re-validates the fingerprint and executes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSuggestionPayload {
    pub source: String,
    pub title: String,
    pub platform_id: String,
    pub region: String,
    pub identification: String,
    pub disposition: String,
    pub package_sha256: String,
    #[serde(default)]
    pub candidates: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// The suggestion kind for incoming imports.
pub const IMPORT_SUGGESTION_KIND: &str = "import";

/// A package is considered settled once its newest file is this old.
const SETTLE_WINDOW_SECONDS: u64 = 3;

/// Cheap change detector: total size, file count, and newest mtime.
#[must_use]
pub fn package_fingerprint(path: &Path) -> String {
    fn visit(path: &Path, size: &mut u64, count: &mut u64, newest: &mut u64) {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return;
        };
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_secs());
        if metadata.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.filter_map(Result::ok) {
                    visit(&entry.path(), size, count, newest);
                }
            }
        } else {
            *size += metadata.len();
            *count += 1;
            *newest = (*newest).max(mtime);
        }
    }
    let (mut size, mut count, mut newest) = (0, 0, 0);
    visit(path, &mut size, &mut count, &mut newest);
    format!("{size}:{count}:{newest}")
}

fn is_settled(path: &Path) -> bool {
    let fingerprint = package_fingerprint(path);
    let newest: u64 = fingerprint
        .rsplit(':')
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    now.saturating_sub(newest) >= SETTLE_WINDOW_SECONDS
}

/// Normalize an event path to its package root: the direct child of the
/// incoming root it lives under (a bare file or a dump directory).
#[must_use]
pub fn package_root(incoming_root: &Path, event_path: &Path) -> Option<PathBuf> {
    let relative = event_path.strip_prefix(incoming_root).ok()?;
    let first = relative.components().next()?;
    Some(incoming_root.join(first))
}

/// Record a filesystem event under an incoming root.
pub fn note_incoming_event(
    conn: &mut retro_junk_db::Connection,
    profile_id: &str,
    incoming_root: &Path,
    event_path: &Path,
) -> Result<(), WorkError> {
    let Some(package) = package_root(incoming_root, event_path) else {
        return Ok(());
    };
    let package_str = package.to_string_lossy().into_owned();
    if package.exists() {
        let fingerprint = package_fingerprint(&package);
        retro_junk_db::work::observe_incoming_package(
            conn,
            &package_str,
            profile_id,
            &fingerprint,
        )?;
    } else {
        // The package left the folder: forget it and retire its suggestion.
        retro_junk_db::work::remove_incoming_package(conn, &package_str)?;
        supersede_import_suggestion(conn, &package_str)?;
    }
    Ok(())
}

fn supersede_import_suggestion(
    conn: &mut retro_junk_db::Connection,
    package: &str,
) -> Result<(), WorkError> {
    let open = retro_junk_db::work::list_open_suggestions(conn, Some(IMPORT_SUGGESTION_KIND))?;
    for suggestion in open {
        if suggestion.target_id == package {
            retro_junk_db::work::resolve_suggestion(conn, suggestion.id, "superseded")?;
        }
    }
    Ok(())
}

/// Outcome counters for one incoming pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IncomingStats {
    pub imported: usize,
    pub suggested: usize,
    pub errored: usize,
    pub waiting: usize,
}

/// Plan every settled `pending` package: auto-import within policy, file a
/// suggestion, or record the honest error state.
pub fn process_pending_incoming(
    ctx: &ExecContext,
    policy: &AutomationPolicy,
    cancelled: &AtomicBool,
) -> Result<IncomingStats, WorkError> {
    let mut stats = IncomingStats::default();
    let mut conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    let pending = retro_junk_db::work::list_incoming_packages(&conn, Some("pending"))?;
    for package in pending {
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let path = PathBuf::from(&package.path);
        if !path.exists() {
            retro_junk_db::work::remove_incoming_package(&mut conn, &package.path)?;
            supersede_import_suggestion(&mut conn, &package.path)?;
            continue;
        }
        let current = package_fingerprint(&path);
        if current != package.fingerprint || !is_settled(&path) {
            // Still being written; the next sighting/pass re-checks.
            retro_junk_db::work::observe_incoming_package(
                &mut conn,
                &package.path,
                &package.profile_id,
                &current,
            )?;
            stats.waiting += 1;
            continue;
        }
        if policy.auto_import == AutoImportMode::Off {
            // "Track only": the package stays recorded (vanish/settle
            // bookkeeping above still ran), but nothing reads its contents
            // and no suggestion is filed. Counted as waiting because that is
            // what it does — waits for the user to raise the mode.
            stats.waiting += 1;
            continue;
        }
        match plan_and_apply(ctx, policy, &mut conn, &path, cancelled) {
            Ok(PackageOutcome::Imported) => stats.imported += 1,
            Ok(PackageOutcome::Suggested) => stats.suggested += 1,
            Ok(PackageOutcome::Errored) => stats.errored += 1,
            Err(error) => {
                stats.errored += 1;
                retro_junk_db::work::set_incoming_error(
                    &mut conn,
                    &package.path,
                    &error.to_string(),
                )?;
            }
        }
    }
    Ok(stats)
}

enum PackageOutcome {
    Imported,
    Suggested,
    Errored,
}

fn plan_and_apply(
    ctx: &ExecContext,
    policy: &AutomationPolicy,
    conn: &mut retro_junk_db::Connection,
    package: &Path,
    cancelled: &AtomicBool,
) -> Result<PackageOutcome, WorkError> {
    let package_str = package.to_string_lossy().into_owned();
    let plan = plan_import(
        import_request(ctx, package),
        &ctx.analyzers,
        conn,
        cancelled,
        |_, _| {},
        |_| {},
    )
    .map_err(WorkError::msg)?;

    // Every candidate must be importable for the package to auto-import.
    let mut all_auto = policy.auto_import == AutoImportMode::On;
    let mut any_review = false;
    let mut error_detail = String::new();
    let mut payloads = Vec::new();
    for candidate in &plan.candidates {
        let (auto_ok, review, error) = classify(candidate, policy);
        all_auto &= auto_ok;
        any_review |= review;
        if let Some(error) = error {
            error_detail = error;
        }
        payloads.push(payload_for(&package_str, candidate));
    }
    if !error_detail.is_empty() {
        retro_junk_db::work::set_incoming_error(conn, &package_str, &error_detail)?;
        return Ok(PackageOutcome::Errored);
    }
    if plan.candidates.iter().all(|candidate| {
        matches!(
            candidate.disposition,
            ImportDisposition::AlreadyArchived { .. }
        )
    }) {
        retro_junk_db::work::set_incoming_imported(conn, &package_str)?;
        return Ok(PackageOutcome::Imported);
    }

    if all_auto && !any_review && !plan.candidates.is_empty() {
        // Never consume: sources placed by the user stay in place.
        let result =
            execute_import(plan, false, cancelled, |_| {}, |_| {}).map_err(WorkError::msg)?;
        let failed = result
            .results
            .iter()
            .any(|item| item.outcome == retro_junk_archive_import::CandidateImportOutcome::Failed);
        if failed {
            let detail = result
                .results
                .iter()
                .map(|item| item.detail.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            retro_junk_db::work::set_incoming_error(conn, &package_str, &detail)?;
            return Ok(PackageOutcome::Errored);
        }
        retro_junk_db::work::set_incoming_imported(conn, &package_str)?;
        supersede_import_suggestion(conn, &package_str)?;
        log::info!("auto-imported {package_str}");
        return Ok(PackageOutcome::Imported);
    }

    // Ready for one-click review: store the completed identification and
    // open (or refresh) the suggestion.
    let summary = payloads
        .first()
        .map(|payload: &ImportSuggestionPayload| {
            format!("{} [{}]", payload.title, payload.identification)
        })
        .unwrap_or_default();
    let payload_json = serde_json::to_string(&payloads).map_err(WorkError::msg)?;
    retro_junk_db::work::set_incoming_ready(conn, &package_str, &summary, &payload_json)?;
    retro_junk_db::work::open_suggestion(
        conn,
        &retro_junk_db::work::NewSuggestion {
            kind: IMPORT_SUGGESTION_KIND,
            target_kind: "path",
            target_id: &package_str,
            payload_json: &payload_json,
            confidence: confidence_score(&plan),
            provenance: &ctx.owner,
        },
    )?;
    Ok(PackageOutcome::Suggested)
}

/// Re-validate and execute a previously suggested import. Fails honestly if
/// the package changed since it was planned (it re-enters `pending` for the
/// next pass).
pub fn apply_import_suggestion(
    ctx: &ExecContext,
    suggestion_id: i64,
    cancelled: &AtomicBool,
) -> Result<(), WorkError> {
    let mut conn = retro_junk_db::open_database(&ctx.db_path).map_err(WorkError::msg)?;
    let suggestion = retro_junk_db::work::get_suggestion(&conn, suggestion_id)?
        .ok_or_else(|| WorkError::Message(format!("suggestion {suggestion_id} not found")))?;
    if suggestion.resolved_at.is_some() {
        return Err(WorkError::Message(format!(
            "suggestion {suggestion_id} is already {}",
            suggestion.resolution
        )));
    }
    if suggestion.kind != IMPORT_SUGGESTION_KIND {
        return Err(WorkError::Message(format!(
            "suggestion {suggestion_id} is a '{}' suggestion; only imports apply here",
            suggestion.kind
        )));
    }
    let package = PathBuf::from(&suggestion.target_id);
    let stored = retro_junk_db::work::get_incoming_package(&conn, &suggestion.target_id)?;
    if let Some(stored) = &stored
        && package_fingerprint(&package) != stored.fingerprint
    {
        retro_junk_db::work::observe_incoming_package(
            &mut conn,
            &suggestion.target_id,
            &stored.profile_id,
            &package_fingerprint(&package),
        )?;
        return Err(WorkError::Message(
            "package changed since it was identified; it will be re-processed".to_owned(),
        ));
    }
    let plan = plan_import(
        import_request(ctx, &package),
        &ctx.analyzers,
        &conn,
        cancelled,
        |_, _| {},
        |_| {},
    )
    .map_err(WorkError::msg)?;
    let importable = plan.candidates.iter().all(|candidate| {
        matches!(
            candidate.disposition,
            ImportDisposition::Ready
                | ImportDisposition::ReadyUnbound { .. }
                | ImportDisposition::AlreadyArchived { .. }
        )
    });
    if !importable || plan.candidates.is_empty() {
        return Err(WorkError::Message(
            "package is no longer directly importable; review it in the import dialog".to_owned(),
        ));
    }
    let result = execute_import(plan, false, cancelled, |_| {}, |_| {}).map_err(WorkError::msg)?;
    let failed = result
        .results
        .iter()
        .any(|item| item.outcome == retro_junk_archive_import::CandidateImportOutcome::Failed);
    if failed {
        let detail = result
            .results
            .iter()
            .map(|item| item.detail.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        retro_junk_db::work::set_incoming_error(&mut conn, &suggestion.target_id, &detail)?;
        return Err(WorkError::Message(detail));
    }
    retro_junk_db::work::set_incoming_imported(&mut conn, &suggestion.target_id)?;
    retro_junk_db::work::resolve_suggestion(&mut conn, suggestion_id, "applied")?;
    // Refresh the projection so every process (the daemon's next derivation
    // included) sees the new release immediately.
    let snapshot =
        retro_junk_archive::scan_archive(&ctx.profile.archive_root).map_err(WorkError::msg)?;
    retro_junk_db::reconcile_archive_snapshot(
        &mut conn,
        &snapshot,
        &ctx.profile.playable_root,
        &ctx.profile.workspace_root,
    )
    .map_err(WorkError::msg)?;
    Ok(())
}

fn import_request(ctx: &ExecContext, package: &Path) -> DumpImportRequest {
    DumpImportRequest {
        source: package.to_path_buf(),
        archive_root: ctx.profile.archive_root.clone(),
        platform_hint: None,
        owner_id: "default".to_owned(),
        new_physical_copy: false,
        redumper_path: (!ctx.tools.redumper.as_os_str().is_empty())
            .then(|| ctx.tools.redumper.clone()),
        workspace_root: Some(ctx.profile.processing_workspace_root()),
        stage_packages_locally: ctx.profile.network_mode,
        // Incoming packages import to the archive; adopting existing
        // playable files is the separate import-playable flow.
        playable_root: None,
        make_playable: false,
        chdman_path: None,
        discard_redundant_bin_cue: false,
    }
}

/// (may auto-import, needs human review, terminal error detail)
fn classify(
    candidate: &retro_junk_archive_import::DumpImportCandidate,
    policy: &AutomationPolicy,
) -> (bool, bool, Option<String>) {
    let confident = identification_confidence(&candidate.identification)
        .is_some_and(|confidence| confidence >= policy.auto_bind_min_confidence);
    match &candidate.disposition {
        ImportDisposition::Ready => (confident, !confident, None),
        // Unbound imports and choice-needing candidates archive honestly,
        // but only a human decides them.
        ImportDisposition::ReadyUnbound { .. }
        | ImportDisposition::NeedsCatalogChoice { .. }
        | ImportDisposition::NeedsPhysicalCopyChoice { .. } => (false, true, None),
        ImportDisposition::AlreadyArchived { .. } => (true, false, None),
        ImportDisposition::Unresolved { reason } | ImportDisposition::Invalid { reason } => {
            (false, false, Some(reason.clone()))
        }
    }
}

fn identification_confidence(resolution: &IdentificationResolution) -> Option<BindConfidence> {
    let method = match resolution {
        IdentificationResolution::CatalogVerified { method }
        | IdentificationResolution::Identified { method } => method,
        IdentificationResolution::Ambiguous | IdentificationResolution::Unresolved => {
            return None;
        }
    };
    Some(match method {
        IdentificationMethod::CompleteTrackSet
        | IdentificationMethod::ExactFileHash
        | IdentificationMethod::FormatAwareFileHash
        | IdentificationMethod::RedumperLog
        | IdentificationMethod::UserSelection => BindConfidence::ExactHash,
        IdentificationMethod::HeaderSerial => BindConfidence::ExactSerial,
        IdentificationMethod::FolderSerial => BindConfidence::FolderSerial,
    })
}

fn payload_for(
    source: &str,
    candidate: &retro_junk_archive_import::DumpImportCandidate,
) -> ImportSuggestionPayload {
    let (title, region) = candidate.selected_match.as_ref().map_or_else(
        || match &candidate.disposition {
            ImportDisposition::ReadyUnbound { title, .. } => (title.clone(), String::new()),
            _ => (String::new(), String::new()),
        },
        |selected| (selected.title.clone(), selected.region.clone()),
    );
    ImportSuggestionPayload {
        source: source.to_owned(),
        title,
        platform_id: candidate.archive_platform_id.clone(),
        region,
        identification: format!("{:?}", candidate.identification),
        disposition: format!("{:?}", disposition_kind(&candidate.disposition)),
        package_sha256: candidate.package.package_sha256.clone(),
        candidates: match &candidate.disposition {
            ImportDisposition::NeedsCatalogChoice { candidates } => candidates
                .iter()
                .map(|choice| format!("{} ({}) [{}]", choice.title, choice.region, choice.source))
                .collect(),
            _ => Vec::new(),
        },
        warnings: candidate.warnings.clone(),
    }
}

#[derive(Debug)]
enum DispositionKind {
    Ready,
    ReadyUnbound,
    AlreadyArchived,
    NeedsCatalogChoice,
    NeedsPhysicalCopyChoice,
    Unresolved,
    Invalid,
}

fn disposition_kind(disposition: &ImportDisposition) -> DispositionKind {
    match disposition {
        ImportDisposition::Ready => DispositionKind::Ready,
        ImportDisposition::ReadyUnbound { .. } => DispositionKind::ReadyUnbound,
        ImportDisposition::AlreadyArchived { .. } => DispositionKind::AlreadyArchived,
        ImportDisposition::NeedsCatalogChoice { .. } => DispositionKind::NeedsCatalogChoice,
        ImportDisposition::NeedsPhysicalCopyChoice { .. } => {
            DispositionKind::NeedsPhysicalCopyChoice
        }
        ImportDisposition::Unresolved { .. } => DispositionKind::Unresolved,
        ImportDisposition::Invalid { .. } => DispositionKind::Invalid,
    }
}

/// Overall confidence for the suggestion row (weakest candidate wins).
fn confidence_score(plan: &DumpImportPlan) -> f64 {
    plan.candidates
        .iter()
        .map(
            |candidate| match identification_confidence(&candidate.identification) {
                Some(BindConfidence::ExactHash) => 1.0,
                Some(BindConfidence::ExactSerial) => 0.7,
                Some(BindConfidence::FolderSerial) => 0.4,
                None => 0.1,
            },
        )
        .fold(1.0_f64, f64::min)
}
