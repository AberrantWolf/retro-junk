//! Bring built playables back to the name the catalog gives them.
//!
//! The completion fold reports each misnamed playable as
//! [`crate::completion::Attention::StaleName`], carrying both the current and
//! the canonical name. This runs the repair for one of them, or for every one
//! in a profile.

use std::path::Path;

use retro_junk_archive::CollectionProfile;
use retro_junk_io::ProgressUnit;

use super::OpCtx;
use crate::completion::{Attention, Completion};

/// One completed rename, for reporting.
#[derive(Debug, PartialEq, Eq)]
pub struct RenamedPlayable {
    pub from: String,
    pub to: String,
    pub media_renamed: usize,
    pub playlists_updated: usize,
}

#[derive(Debug, Default)]
pub struct RenameReport {
    pub renamed: Vec<RenamedPlayable>,
    /// Playables that could not be renamed, with the reason. A collision or a
    /// missing file is reported rather than forced.
    pub failures: Vec<String>,
}

/// Rename every playable in a profile whose name no longer matches the
/// catalog, then re-project so the result is visible immediately.
pub fn rename_stale_playables(
    profile: &CollectionProfile,
    db_path: &Path,
    media_root: Option<&Path>,
    expected_assets: &retro_junk_frontend::AssetSelection,
    ctx: &OpCtx,
) -> Result<RenameReport, String> {
    let _archive_lock = retro_junk_archive::ArchiveLock::acquire(&profile.archive_root)
        .map_err(|error| error.to_string())?;
    let mut connection =
        retro_junk_db::open_database(db_path).map_err(|error| error.to_string())?;

    // What to rename comes from the one fold, so the repair acts on exactly
    // what the UI showed the user.
    let overviews = crate::queries::releases::release_overviews(
        &connection,
        &profile.profile_id.to_string(),
        expected_assets,
    )?;
    let stale = overviews
        .iter()
        .flat_map(|release| &release.completion.attention)
        .filter_map(|attention| match attention {
            Attention::StaleName {
                representation_id,
                current,
                canonical,
            } => Some((
                representation_id.clone(),
                current.clone(),
                canonical.clone(),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();

    let mut report = RenameReport::default();
    if stale.is_empty() {
        return Ok(report);
    }
    let total = stale.len() as u64;
    // The snapshot is re-read per rename: each repair appends evidence, and
    // acting on a stale view would rename against an outdated location.
    for (index, (representation_id, current, canonical)) in stale.into_iter().enumerate() {
        if ctx.cancelled() {
            break;
        }
        (ctx.progress)(
            &format!("Renaming {current}"),
            ProgressUnit::Items,
            index as u64,
            total,
        );
        let snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
            .map_err(|error| error.to_string())?;
        let outcome = retro_junk_lib::rename_playable::rename_playable(
            &retro_junk_lib::rename_playable::RenamePlayableRequest {
                snapshot: &snapshot,
                playable_root: &profile.playable_root,
                representation_id: &representation_id,
                canonical_file_name: &canonical,
                media_root,
            },
        );
        match outcome {
            Ok(renamed) => report.renamed.push(RenamedPlayable {
                from: renamed.from,
                to: renamed.to,
                media_renamed: renamed.media_renamed,
                playlists_updated: renamed.playlists_updated,
            }),
            Err(error) => report.failures.push(format!("{current}: {error}")),
        }
    }

    let snapshot = retro_junk_archive::scan_archive(&profile.archive_root)
        .map_err(|error| error.to_string())?;
    retro_junk_db::reconcile_archive_snapshot(
        &mut connection,
        &snapshot,
        &profile.playable_root,
        &profile.workspace_root,
    )
    .map_err(|error| error.to_string())?;
    Ok(report)
}

/// Misnamed playables in a profile, without changing anything — the preview
/// behind a dry run and the count behind a prompt.
pub fn stale_playable_names(
    conn: &retro_junk_db::Connection,
    profile_id: &str,
    expected_assets: &retro_junk_frontend::AssetSelection,
) -> Result<Vec<(String, String)>, String> {
    Ok(
        crate::queries::releases::release_overviews(conn, profile_id, expected_assets)?
            .iter()
            .flat_map(|release| &release.completion.attention)
            .filter_map(|attention| match attention {
                Attention::StaleName {
                    current, canonical, ..
                } => Some((current.clone(), canonical.clone())),
                _ => None,
            })
            .collect(),
    )
}

/// Whether a release has any misnamed playable, for callers holding a
/// [`Completion`] already.
#[must_use]
pub fn has_stale_names(completion: &Completion) -> bool {
    completion
        .attention
        .iter()
        .any(|attention| matches!(attention, Attention::StaleName { .. }))
}
