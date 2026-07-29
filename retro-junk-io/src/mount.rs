//! Shared mount-kind detection.
//!
//! One detector serves every consumer with a different tolerance for remote
//! storage: the GUI warns before adopting a fragile FUSE gateway as a library
//! root, and the filesystem watcher falls back to polling on network mounts
//! where inotify/FSEvents-style notification is unreliable.

use std::path::Path;

/// How a path's filesystem reaches its bytes, when that matters to a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteMountKind {
    /// Userspace FUSE network gateways (GVFS, KIO-FUSE). These stall or
    /// return wrong data under heavy random-access I/O (CHD/ISO seeking), so
    /// callers should warn before using one as a library root.
    FragileFuse(&'static str),
    /// Ordinary kernel network filesystems (NFS, SMB/CIFS, AFP, `WebDAV`).
    /// Fine for storage, but native filesystem notification is unreliable —
    /// watchers should poll instead.
    Network(&'static str),
}

impl RemoteMountKind {
    /// Short human-readable label ("GVFS", "NFS", "SMB", …).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::FragileFuse(label) | Self::Network(label) => label,
        }
    }

    /// Whether callers should warn before random-access-heavy use.
    #[must_use]
    pub fn is_fragile(self) -> bool {
        matches!(self, Self::FragileFuse(_))
    }
}

/// Detect whether `path` lives on a remote/userspace mount.
///
/// Path-shape sniffing catches the FUSE gateways (they mount under
/// recognizable prefixes); a `statfs` probe classifies kernel network
/// filesystems. Returns `None` for local storage, on probe failure, and on
/// platforms without an implemented probe — absence of a warning must never
/// block use of a path.
#[must_use]
pub fn remote_mount_kind(path: &Path) -> Option<RemoteMountKind> {
    let s = path.to_string_lossy();
    if s.contains("/gvfs/") {
        return Some(RemoteMountKind::FragileFuse("GVFS"));
    }
    if s.contains("/kio-fuse-") || s.contains("/kio-fuse/") {
        return Some(RemoteMountKind::FragileFuse("KIO-FUSE"));
    }
    statfs_network_kind(path).map(RemoteMountKind::Network)
}

#[cfg(target_os = "macos")]
fn statfs_network_kind(path: &Path) -> Option<&'static str> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let encoded = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `encoded` is NUL-terminated and `stats` points to writable,
    // correctly aligned storage for one `statfs` result.
    if unsafe { libc::statfs(encoded.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: a successful `statfs` call initialized the structure.
    let stats = unsafe { stats.assume_init() };
    let name_len = stats
        .f_fstypename
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(stats.f_fstypename.len());
    // SAFETY: reinterpreting the NUL-delimited prefix of the C char array as
    // bytes; f_fstypename is ASCII by contract.
    let bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(stats.f_fstypename.as_ptr().cast(), name_len) };
    match std::str::from_utf8(bytes).ok()? {
        "nfs" => Some("NFS"),
        "smbfs" => Some("SMB"),
        "cifs" => Some("CIFS"),
        "afpfs" => Some("AFP"),
        "webdav" => Some("WebDAV"),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn statfs_network_kind(path: &Path) -> Option<&'static str> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let encoded = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `encoded` is NUL-terminated and `stats` points to writable,
    // correctly aligned storage for one `statfs` result.
    if unsafe { libc::statfs(encoded.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: a successful `statfs` call initialized the structure.
    let stats = unsafe { stats.assume_init() };
    // Magic numbers from linux/magic.h. `f_type`'s width varies by target,
    // so widen before comparing.
    #[allow(clippy::cast_possible_wrap, clippy::cast_lossless)]
    let f_type = stats.f_type as i64;
    match f_type {
        0x6969 => Some("NFS"),
        0x517B => Some("SMB"),
        0xFE53_4D42 => Some("SMB2"),
        0xFF53_4D42 => Some("CIFS"),
        _ => None,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn statfs_network_kind(_path: &Path) -> Option<&'static str> {
    None
}
