//! Queue one release's playable build as a derived convergence action.
//!
//! This is only the translation from "the user picked this release and this
//! format" into a [`ProposedAction`]; execution, claiming, locking, and the
//! projection refresh all belong to the shared executor
//! ([`crate::execute_action`]), which the CLI, daemon, and GUI reach the
//! same way.
//!
//! [`ProposedAction`]: retro_junk_db::convergence::ProposedAction

use retro_junk_db::ArchivedPlayableGap;
use retro_junk_db::convergence::{ActionKind, ProposedAction, WorkTarget};

/// Build the action for one release plus the human description of what was
/// queued. The user's explicit format choice overrides the stored policy for
/// this run; the executor reads the format from the gap.
#[must_use]
pub fn queue_release_build(
    release: ArchivedPlayableGap,
    format: &retro_junk_archive::RepresentationFormat,
    playable_platform_id: String,
    profile_id: String,
) -> (ProposedAction, String) {
    let release_label = if release.region.trim().is_empty() {
        release.title.clone()
    } else {
        format!("{} ({})", release.title, release.region)
    };
    let description = if release.needs_playable {
        format!("Queued playable build for {release_label}")
    } else {
        format!("Queued archive verification for {release_label}")
    };
    let mut gap = release;
    gap.preferred_format = Some(format.key().to_owned());
    let action = ProposedAction {
        kind: ActionKind::BuildPlayable,
        target: WorkTarget::Release(gap.archive_release_id.clone()),
        profile_id,
        platform_id: String::new(),
        playable_platform_id,
        label: release_label,
        blocked: None,
        build: Some(gap),
    };
    (action, description)
}
