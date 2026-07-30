use crate::app::RetroJunkApp;

/// Queue one release's build as a derived convergence action.
///
/// This is only the translation from "the user picked this release and this
/// format" into a [`ProposedAction`]; execution, claiming, locking, and the
/// projection refresh all belong to [`crate::backend::convergence`], which
/// the CLI and daemon reach through the same executor.
///
/// [`ProposedAction`]: retro_junk_db::convergence::ProposedAction
pub fn start(
    app: &mut RetroJunkApp,
    release: retro_junk_db::ArchivedPlayableGap,
    format: &retro_junk_archive::RepresentationFormat,
    playable_platform_id: String,
    ctx: &egui::Context,
) {
    let Some(profile_id) = app
        .settings
        .library
        .active_profile()
        .map(|profile| profile.profile_id.to_string())
    else {
        app.push_error("Archive action", "No active collection profile".to_owned());
        return;
    };
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
    // The user's explicit format choice overrides the stored policy for
    // this run; the executor reads the format from the gap.
    let mut gap = release;
    gap.preferred_format = Some(format.key().to_owned());
    let action = retro_junk_db::convergence::ProposedAction {
        kind: retro_junk_db::convergence::ActionKind::BuildPlayable,
        target: retro_junk_db::convergence::WorkTarget::Release(gap.archive_release_id.clone()),
        profile_id,
        platform_id: String::new(),
        playable_platform_id,
        label: release_label,
        blocked: None,
        build: Some(gap),
    };
    crate::backend::convergence::run_action(app, action, description, ctx);
}
