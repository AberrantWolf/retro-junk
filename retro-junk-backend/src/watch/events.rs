//! Coalesced filesystem events the daemon consumes.

use std::path::{Path, PathBuf};

/// One settled filesystem change under a watched root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    FileAdded(PathBuf),
    /// Contents changed — anything derived from the old bytes is stale.
    FileModified(PathBuf),
    /// Same bytes, new name; derived state follows the file.
    FileRenamed {
        from: PathBuf,
        to: PathBuf,
    },
    FileRemoved(PathBuf),
    DirAdded(PathBuf),
    DirRemoved(PathBuf),
}

impl WatchEvent {
    /// The path the event settles on (`to` for renames).
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::FileAdded(path)
            | Self::FileModified(path)
            | Self::FileRemoved(path)
            | Self::DirAdded(path)
            | Self::DirRemoved(path)
            | Self::FileRenamed { to: path, .. } => path,
        }
    }
}
