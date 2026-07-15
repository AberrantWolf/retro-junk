use std::path::Path;

/// If `path` lives under a userspace FUSE-based network mount (GVFS, KIO-FUSE),
/// return a short label naming the kind. These mounts stall or return wrong
/// data under heavy random-access I/O (CHD/ISO seeking), so we warn before
/// using one as a library root.
pub fn fragile_mount_kind(path: &Path) -> Option<&'static str> {
    let s = path.to_string_lossy();
    if s.contains("/gvfs/") {
        Some("GVFS")
    } else if s.contains("/kio-fuse-") || s.contains("/kio-fuse/") {
        Some("KIO-FUSE")
    } else {
        None
    }
}

/// Copy `text` to the clipboard and close the enclosing menu.
///
/// The standard action for a "Copy" context-menu item.
pub fn copy_and_close(ui: &mut egui::Ui, text: String) {
    ui.ctx().copy_text(text);
    ui.close();
}

/// Render `add_contents` inside a `CentralPanel` whose descendant widgets keep
/// **stable ids** regardless of which sibling panels are present this frame.
///
/// A child `Ui` seeds its widgets' auto-ids from the parent's running auto-id
/// counter, and that counter advances once per child `Ui` — including every
/// panel. A plain `CentralPanel::default()` therefore gets a *different* auto-id
/// whenever a conditional sibling panel (a log viewer, a detail pane, an
/// activity bar) appears or disappears, which re-ids every auto-id widget in the
/// panel for a frame. egui's debug-only `warn_if_rect_changes_id` check then
/// flashes red rectangles around those widgets and logs a warning per widget.
///
/// Anchoring the contents under a `UiBuilder::id` — an *explicit*,
/// counter-independent id — pins the whole subtree's ids, so toggling sibling
/// panels no longer disturbs them. Give each call site a unique `id_salt`.
pub fn stable_central_panel<R>(
    ui: &mut egui::Ui,
    id_salt: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::CentralPanel::default()
        .show(ui, |ui| {
            ui.scope_builder(
                egui::UiBuilder::new().id(egui::Id::new(id_salt)),
                add_contents,
            )
            .inner
        })
        .inner
}

/// Platform-appropriate label for the "reveal in file manager" menu item.
pub const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else if cfg!(target_os = "windows") {
    "Show in Explorer"
} else {
    "Show in File Manager"
};

/// Open the OS file manager and highlight the given path.
pub fn reveal_in_file_manager(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .ok();
    }
    #[cfg(target_os = "linux")]
    if let Some(parent) = path.parent() {
        std::process::Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .ok();
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(path)
            .spawn()
            .ok();
    }
}
