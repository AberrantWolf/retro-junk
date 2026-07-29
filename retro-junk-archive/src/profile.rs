use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{ArchiveProfileId, PlatformPlayableDefault};

/// Device-local roots paired with one portable archive identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionProfile {
    pub profile_id: ArchiveProfileId,
    pub display_name: String,
    pub archive_root: PathBuf,
    pub playable_root: PathBuf,
    pub workspace_root: PathBuf,
    /// Stage large inputs in the device-local workspace before processing.
    ///
    /// This improves seek-heavy work over SMB/NFS at the cost of an additional
    /// full sequential read and local write.
    #[serde(default = "default_network_mode")]
    pub network_mode: bool,
    #[serde(default)]
    pub platform_defaults: Vec<PlatformPlayableDefault>,
    /// Watched drop folders: new dump packages appearing here are
    /// pre-processed (hashed + identified) by the daemon and imported or
    /// suggested per automation policy.
    #[serde(default)]
    pub incoming_roots: Vec<PathBuf>,
    /// Filesystem-notification backend for the daemon's watchers.
    #[serde(default)]
    pub watch_backend: WatchBackend,
}

/// How the daemon watches this profile's roots. Native notification is
/// unreliable on network filesystems; `Auto` falls back to polling there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchBackend {
    #[default]
    Auto,
    Native,
    Poll,
    Off,
}

const fn default_network_mode() -> bool {
    true
}

impl CollectionProfile {
    /// Select disposable scratch storage for conversion and verification work.
    ///
    /// Network mode opts into the configured device-local workspace. When it
    /// is disabled, keep scratch on the archive filesystem so operations do
    /// not make an unrequested device-local copy of large archive inputs.
    #[must_use]
    pub fn processing_workspace_root(&self) -> PathBuf {
        if self.network_mode {
            self.workspace_root.clone()
        } else {
            self.archive_root.join(".retro-junk").join("work")
        }
    }

    #[must_use]
    pub fn from_legacy_playable_root(playable_root: &Path) -> Self {
        let parent = playable_root.parent().unwrap_or_else(|| Path::new("."));
        let display_name = parent
            .file_name()
            .or_else(|| playable_root.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("Retro Collection")
            .to_owned();
        let archive_root = parent.join("archive");
        let profile_id = ArchiveProfileId::new();
        Self {
            profile_id,
            display_name,
            workspace_root: retro_junk_io::default_profile_workspace(profile_id.0),
            archive_root,
            playable_root: playable_root.to_path_buf(),
            network_mode: true,
            platform_defaults: Vec::new(),
            incoming_roots: Vec::new(),
            watch_backend: WatchBackend::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_mode_uses_the_configured_device_local_workspace() {
        let mut profile =
            CollectionProfile::from_legacy_playable_root(Path::new("/collections/roms"));
        profile.workspace_root = PathBuf::from("/local/cache");
        profile.network_mode = true;

        assert_eq!(
            profile.processing_workspace_root(),
            PathBuf::from("/local/cache")
        );
    }

    #[test]
    fn non_network_mode_keeps_processing_scratch_on_the_archive_filesystem() {
        let mut profile =
            CollectionProfile::from_legacy_playable_root(Path::new("/collections/roms"));
        profile.workspace_root = PathBuf::from("/local/cache");
        profile.network_mode = false;

        assert_eq!(
            profile.processing_workspace_root(),
            profile.archive_root.join(".retro-junk/work")
        );
    }
}
