use crate::app::RetroJunkApp;

/// Render the fragile-network-mount confirmation as a modal window.
///
/// Shown while `app.ui_state.fragile_mount_prompt` is `Some`. GVFS/KIO-FUSE mounts
/// stall or return wrong data under heavy random-access I/O (CHD/ISO
/// seeking), so we ask before adopting one as a library root. Accepting
/// resumes the switch via `switch_to_root_unchecked`.
pub fn show(ctx: &egui::Context, app: &mut RetroJunkApp) {
    let Some(ref prompt) = app.ui_state.fragile_mount_prompt else {
        return;
    };
    let kind = prompt.kind;
    let path_text = prompt.root.display().to_string();

    let mut confirmed = false;
    let mut cancelled = ctx.input(|i| i.key_pressed(egui::Key::Escape));

    egui::Window::new("Network Share Warning")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_max_width(440.0);

            ui.label(format!("This path is a {kind} userspace network mount:"));
            ui.add_space(4.0);
            ui.monospace(&path_text);
            ui.add_space(8.0);
            ui.label(format!(
                "retro-junk performs heavy random-access reads (CHD, ISO, \
                 etc.) and {kind} mounts frequently stall, report wrong \
                 sizes, or drop connections under that load."
            ));
            ui.add_space(4.0);
            ui.label(
                "For best results, mount the share via the kernel (CIFS/NFS) \
                 at a normal filesystem path like /mnt/roms.",
            );
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button("Use Anyway").clicked() {
                    confirmed = true;
                }
                if ui.button("Cancel").clicked() {
                    cancelled = true;
                }
            });
        });

    if confirmed {
        if let Some(prompt) = app.ui_state.fragile_mount_prompt.take() {
            crate::views::library::switch_to_root_unchecked(app, prompt.root, ctx);
        }
    } else if cancelled {
        app.ui_state.fragile_mount_prompt = None;
    }
}
