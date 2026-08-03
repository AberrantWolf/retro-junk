use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{ArchiveProfileId, ArchiveRootManifest, PlatformPlayableDefault};

/// The directory that holds one collection as a whole.
///
/// Marks describe both archived items and playables — a mod exists only as a
/// playable, homebrew may be either — so their store belongs beside both roots
/// rather than inside one of them. When the roots are siblings (the usual
/// `Collection/{archive,roms}` layout) that is their shared parent; otherwise
/// the archive root stands in, since it is the durable one.
///
/// One rule, because two would be worse than either: a writer and a reader
/// that disagree about where marks live lose the decisions silently.
#[must_use]
pub fn collection_root_for(archive_root: &Path, playable_root: &Path) -> PathBuf {
    match (archive_root.parent(), playable_root.parent()) {
        (Some(archive), Some(playable)) if archive == playable => archive.to_path_buf(),
        _ => archive_root.to_path_buf(),
    }
}

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

    /// The directory that holds this collection as a whole.
    #[must_use]
    pub fn collection_root(&self) -> PathBuf {
        collection_root_for(&self.archive_root, &self.playable_root)
    }

    /// Pair device-local roots with an archive identity.
    ///
    /// Identity belongs to the portable archive, not to this device's settings.
    /// An archive copied to another disk keeps the `profile_id` recorded in its
    /// root manifest, and the rebuildable `SQLite` projection is keyed on that
    /// id — so a profile that minted a fresh id for an already-initialized
    /// archive would query for rows the reconciler never writes, and every
    /// archived release would silently disappear from the UI. A new id is
    /// correct only when the archive root has no manifest yet.
    #[must_use]
    pub fn for_roots(archive_root: PathBuf, playable_root: PathBuf) -> Self {
        let identity = ArchiveIdentity::read(&archive_root);
        let display_name = identity.as_ref().map_or_else(
            || default_display_name(&playable_root),
            |identity| identity.display_name.clone(),
        );
        let profile_id = identity
            .as_ref()
            .map_or_else(ArchiveProfileId::new, |identity| identity.profile_id);
        Self {
            profile_id,
            display_name,
            workspace_root: retro_junk_io::default_profile_workspace(profile_id.0),
            archive_root,
            playable_root,
            network_mode: true,
            platform_defaults: identity
                .map(|identity| identity.platform_defaults)
                .unwrap_or_default(),
            incoming_roots: Vec::new(),
            watch_backend: WatchBackend::default(),
        }
    }

    #[must_use]
    pub fn from_legacy_playable_root(playable_root: &Path) -> Self {
        let parent = playable_root.parent().unwrap_or_else(|| Path::new("."));
        Self::for_roots(parent.join("archive"), playable_root.to_path_buf())
    }

    /// Re-adopt the archive root manifest's identity, repairing a profile whose
    /// id drifted from the archive it points at (for example one created
    /// against a copy of an existing archive before identity was adopted).
    ///
    /// Returns `true` when the profile changed. A profile whose archive root
    /// has no manifest keeps its own id: there is no authority to adopt yet.
    pub fn adopt_archive_identity(&mut self) -> bool {
        let Some(identity) = ArchiveIdentity::read(&self.archive_root) else {
            return false;
        };
        if identity.profile_id == self.profile_id {
            return false;
        }
        // A workspace still at the derived default belongs to the old id;
        // an explicitly chosen scratch location is the user's and is kept.
        if self.workspace_root == retro_junk_io::default_profile_workspace(self.profile_id.0) {
            self.workspace_root = retro_junk_io::default_profile_workspace(identity.profile_id.0);
        }
        self.profile_id = identity.profile_id;
        self.display_name = identity.display_name;
        if self.platform_defaults.is_empty() {
            self.platform_defaults = identity.platform_defaults;
        }
        true
    }
}

/// The portable identity an archive root carries in its manifest.
struct ArchiveIdentity {
    profile_id: ArchiveProfileId,
    display_name: String,
    platform_defaults: Vec<PlatformPlayableDefault>,
}

impl ArchiveIdentity {
    /// Read the identity of an initialized archive; `None` when the root has
    /// no readable manifest.
    fn read(archive_root: &Path) -> Option<Self> {
        let manifest: ArchiveRootManifest =
            crate::read_toml(&crate::layout::root_manifest_path(archive_root)).ok()?;
        Some(Self {
            profile_id: manifest.profile_id,
            display_name: manifest.display_name,
            platform_defaults: manifest.platform_defaults,
        })
    }
}

fn default_display_name(playable_root: &Path) -> String {
    playable_root
        .parent()
        .and_then(Path::file_name)
        .or_else(|| playable_root.file_name())
        .and_then(|value| value.to_str())
        .unwrap_or("Retro Collection")
        .to_owned()
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

    /// Initialize a collection at `<root>/archive` and return its identity.
    fn init_collection(root: &Path) -> ArchiveProfileId {
        let archive_root = root.join("archive");
        std::fs::create_dir_all(root.join("roms")).unwrap();
        let manifest = ArchiveRootManifest::new("Original Name");
        crate::initialize_archive(&archive_root, &manifest).unwrap();
        manifest.profile_id
    }

    #[test]
    fn profile_for_an_existing_archive_adopts_its_portable_identity() {
        let temp = tempfile::tempdir().unwrap();
        let archived = init_collection(temp.path());

        let profile = CollectionProfile::from_legacy_playable_root(&temp.path().join("roms"));

        // A fresh id here would query the projection for rows the reconciler
        // keys under the manifest id, hiding every archived release.
        assert_eq!(profile.profile_id, archived);
        assert_eq!(profile.display_name, "Original Name");
    }

    #[test]
    fn a_copied_archive_keeps_the_identity_of_the_original() {
        let source = tempfile::tempdir().unwrap();
        let archived = init_collection(source.path());

        // Stand in for an rsync of the collection to another drive.
        let destination = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(destination.path().join("archive")).unwrap();
        std::fs::copy(
            crate::layout::root_manifest_path(&source.path().join("archive")),
            crate::layout::root_manifest_path(&destination.path().join("archive")),
        )
        .unwrap();

        let profile = CollectionProfile::for_roots(
            destination.path().join("archive"),
            destination.path().join("roms"),
        );

        assert_eq!(profile.profile_id, archived);
    }

    #[test]
    fn profile_for_an_uninitialized_archive_root_mints_an_identity() {
        let temp = tempfile::tempdir().unwrap();

        let first = CollectionProfile::from_legacy_playable_root(&temp.path().join("roms"));
        let second = CollectionProfile::from_legacy_playable_root(&temp.path().join("roms"));

        assert_ne!(first.profile_id, second.profile_id);
    }

    #[test]
    fn adopting_identity_rekeys_a_drifted_profile_and_its_default_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let archived = init_collection(temp.path());
        let mut profile =
            CollectionProfile::for_roots(temp.path().join("archive"), temp.path().join("roms"));
        // Reproduce settings written before identity was adopted.
        let stale = ArchiveProfileId::new();
        profile.profile_id = stale;
        profile.workspace_root = retro_junk_io::default_profile_workspace(stale.0);

        assert!(profile.adopt_archive_identity());

        assert_eq!(profile.profile_id, archived);
        assert_eq!(
            profile.workspace_root,
            retro_junk_io::default_profile_workspace(archived.0)
        );
        assert!(!profile.adopt_archive_identity(), "adoption is idempotent");
    }

    #[test]
    fn adopting_identity_preserves_an_explicitly_chosen_workspace() {
        let temp = tempfile::tempdir().unwrap();
        init_collection(temp.path());
        let mut profile =
            CollectionProfile::for_roots(temp.path().join("archive"), temp.path().join("roms"));
        profile.profile_id = ArchiveProfileId::new();
        profile.workspace_root = PathBuf::from("/fast-scratch/retro-junk");

        assert!(profile.adopt_archive_identity());

        assert_eq!(
            profile.workspace_root,
            PathBuf::from("/fast-scratch/retro-junk")
        );
    }

    #[test]
    fn a_profile_pointing_at_an_uninitialized_root_keeps_its_own_identity() {
        let temp = tempfile::tempdir().unwrap();
        let mut profile = CollectionProfile::from_legacy_playable_root(&temp.path().join("roms"));
        let original = profile.profile_id;

        assert!(!profile.adopt_archive_identity());
        assert_eq!(profile.profile_id, original);
    }
}
