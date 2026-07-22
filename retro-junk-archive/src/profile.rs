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
    #[serde(default)]
    pub platform_defaults: Vec<PlatformPlayableDefault>,
}

impl CollectionProfile {
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
            platform_defaults: Vec::new(),
        }
    }
}
