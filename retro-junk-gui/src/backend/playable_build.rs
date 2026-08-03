//! Thin dispatch to `retro_junk_backend::ops::playable_build`. The backend
//! translates the user's release + format pick into a derived action;
//! execution goes through [`crate::backend::convergence`], the same
//! claim → archive-lock → shared-implementation path the CLI and daemon
//! take.

use crate::app::RetroJunkApp;

/// Queue one release's build as a derived convergence action.
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
    let (action, description) = retro_junk_backend::ops::playable_build::queue_release_build(
        release,
        format,
        playable_platform_id,
        profile_id,
    );
    crate::backend::convergence::run_action(app, action, description, ctx);
}
