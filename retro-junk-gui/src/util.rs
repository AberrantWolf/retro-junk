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
