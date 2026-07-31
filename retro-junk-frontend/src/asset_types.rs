use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Visual asset types that can be scraped and used by frontends.
///
/// Ordered so collections of types read in the order a detail panel shows
/// them rather than in an arbitrary hash order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AssetType {
    /// In-game screenshot
    Screenshot,
    /// Title screen capture
    TitleScreen,
    /// Front box art (2D)
    Cover,
    /// 3D rendered box art
    Cover3D,
    /// Logo / marquee / wheel image
    Marquee,
    /// Gameplay or promotional video
    Video,
    /// Fan-created artwork
    Fanart,
    /// Physical media image (cartridge/disc)
    PhysicalMedia,
    /// Composite miximage (screenshot + box + marquee + physical media)
    Miximage,
}

impl fmt::Display for AssetType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetType::Screenshot => write!(f, "screenshot"),
            AssetType::TitleScreen => write!(f, "title screen"),
            AssetType::Cover => write!(f, "cover"),
            AssetType::Cover3D => write!(f, "3D box"),
            AssetType::Marquee => write!(f, "marquee"),
            AssetType::Video => write!(f, "video"),
            AssetType::Fanart => write!(f, "fanart"),
            AssetType::PhysicalMedia => write!(f, "physical media"),
            AssetType::Miximage => write!(f, "miximage"),
        }
    }
}

impl AssetType {
    /// Parse any written name for an asset type.
    ///
    /// One parser for three vocabularies that all appear in real files: the
    /// semantic names in archive supporting-file manifests, the plural
    /// directory slugs the CLI and settings use, and the display names. They
    /// have to round-trip — a settings file that writes `3dboxes` and a
    /// manifest that writes `3D box` must both come back as the same type, or
    /// a release is reported as missing artwork it already holds.
    #[must_use]
    pub fn from_name(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cover" | "covers" | "box-front" => Some(Self::Cover),
            "3d box" | "3dbox" | "3dboxes" | "cover3d" | "cover-3d" => Some(Self::Cover3D),
            "screenshot" | "screenshots" => Some(Self::Screenshot),
            "title screen" | "titlescreen" | "titlescreens" => Some(Self::TitleScreen),
            "marquee" | "marquees" => Some(Self::Marquee),
            "video" | "videos" => Some(Self::Video),
            "fanart" => Some(Self::Fanart),
            "physical media" | "physicalmedia" => Some(Self::PhysicalMedia),
            "miximage" | "miximages" => Some(Self::Miximage),
            _ => None,
        }
    }

    /// Parse the stable semantic names stored in archive supporting-file
    /// manifests.
    #[must_use]
    pub fn from_archive_name(value: &str) -> Option<Self> {
        Self::from_name(value)
    }

    /// ES-DE-compatible media subdirectory used by both CLI and GUI
    /// projections.
    #[must_use]
    pub const fn subdirectory(self) -> &'static str {
        match self {
            Self::Cover => "covers",
            Self::Cover3D => "3dboxes",
            Self::Screenshot => "screenshots",
            Self::TitleScreen => "titlescreens",
            Self::Marquee => "marquees",
            Self::Video => "videos",
            Self::Fanart => "fanart",
            Self::PhysicalMedia => "physicalmedia",
            Self::Miximage => "miximages",
        }
    }

    /// File extension for this asset type.
    #[must_use]
    pub fn default_extension(&self) -> &'static str {
        match self {
            AssetType::Video => "mp4",
            _ => "png",
        }
    }

    /// All file extensions to check when discovering assets on disk.
    ///
    /// `ScreenScraper` may return media in different formats (e.g., JPG instead
    /// of PNG for screenshots), so discovery must check all plausible extensions.
    /// The default extension is always first.
    #[must_use]
    pub fn discovery_extensions(&self) -> &'static [&'static str] {
        match self {
            AssetType::Video => &["mp4"],
            _ => &["png", "jpg"],
        }
    }
}

/// Asset types shown in the detail panel, in display order. Video is absent:
/// it is scraped and projected, but not previewed as an image.
pub const DISPLAY_ASSET_TYPES: &[AssetType] = &[
    AssetType::Cover,
    AssetType::Cover3D,
    AssetType::Screenshot,
    AssetType::TitleScreen,
    AssetType::Marquee,
    AssetType::PhysicalMedia,
    AssetType::Fanart,
    AssetType::Miximage,
];

/// Which asset types an operation covers.
///
/// One vocabulary for three questions that must agree: what a scrape
/// downloads, what a projection writes to the frontend tree, and what
/// "complete artwork" means for a release. Anything that answers one of
/// those by listing asset types by hand will drift from the other two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSelection {
    pub types: Vec<AssetType>,
}

impl Default for AssetSelection {
    /// What a scrape fetches unless told otherwise.
    fn default() -> Self {
        Self {
            types: vec![
                AssetType::Cover,
                AssetType::Cover3D,
                AssetType::Screenshot,
                AssetType::Marquee,
                AssetType::PhysicalMedia,
                AssetType::Video,
            ],
        }
    }
}

impl AssetSelection {
    /// Every downloadable type. `Miximage` is excluded by construction: it is
    /// composed locally from the others, never fetched.
    #[must_use]
    pub fn all() -> Self {
        Self {
            types: vec![
                AssetType::Cover,
                AssetType::Cover3D,
                AssetType::Screenshot,
                AssetType::TitleScreen,
                AssetType::Marquee,
                AssetType::Video,
                AssetType::Fanart,
                AssetType::PhysicalMedia,
            ],
        }
    }

    /// The types a detail panel previews, for on-disk discovery.
    #[must_use]
    pub fn display() -> Self {
        Self {
            types: DISPLAY_ASSET_TYPES.to_vec(),
        }
    }

    /// Parse from a list of written names (e.g., "covers,screenshots,videos").
    /// Unknown names are ignored rather than failing the run.
    ///
    /// `Miximage` is dropped: it is composed locally, so a selection that
    /// contained it would ask a scraper for a media type that does not exist
    /// and mark every release permanently incomplete.
    #[must_use]
    pub fn from_names(names: &[String]) -> Self {
        let types = names
            .iter()
            .filter_map(|name| AssetType::from_name(name))
            .filter(|asset_type| *asset_type != AssetType::Miximage)
            .collect();
        Self { types }
    }

    /// The canonical written names for this selection, as `from_names` parses
    /// them back.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.types
            .iter()
            .map(|asset_type| asset_type.subdirectory().to_owned())
            .collect()
    }

    #[must_use]
    pub fn contains(&self, asset_type: AssetType) -> bool {
        self.types.contains(&asset_type)
    }

    /// Keep only the types matching `predicate`.
    #[must_use]
    pub fn filtered(&self, predicate: impl Fn(AssetType) -> bool) -> Self {
        Self {
            types: self
                .types
                .iter()
                .copied()
                .filter(|asset_type| predicate(*asset_type))
                .collect(),
        }
    }

    /// Files already on disk for this ROM stem, for the selected types.
    #[must_use]
    pub fn collect_existing(
        &self,
        media_dir: &Path,
        rom_stem: &str,
    ) -> HashMap<AssetType, PathBuf> {
        collect_existing_assets(&self.types, media_dir, rom_stem)
    }
}

/// Discover on-disk assets for one ROM stem.
///
/// Takes a slice so callers with a `const` type list don't have to build an
/// [`AssetSelection`] just to look; [`AssetSelection::collect_existing`] is the
/// owned-selection form. Returns only types that actually have a file.
#[must_use]
pub fn collect_existing_assets(
    types: &[AssetType],
    media_dir: &Path,
    rom_stem: &str,
) -> HashMap<AssetType, PathBuf> {
    let mut found = HashMap::new();
    for &asset_type in types {
        let subdir = media_dir.join(asset_type.subdirectory());
        // Check every plausible extension — ScreenScraper may return JPG
        // where the default is PNG.
        for extension in asset_type.discovery_extensions() {
            let path = subdir.join(format!("{rom_stem}.{extension}"));
            if path.exists() {
                found.insert(asset_type, path);
                break;
            }
        }
    }
    found
}

#[cfg(test)]
#[path = "tests/asset_types_tests.rs"]
mod tests;
