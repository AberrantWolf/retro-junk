use serde::{Deserialize, Serialize};
use std::fmt;

/// Visual asset types that can be scraped and used by frontends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// Parse the stable semantic names stored in archive supporting-file
    /// manifests. Older aliases remain accepted so an archive can be
    /// re-projected after the frontend layout evolves.
    #[must_use]
    pub fn from_archive_name(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cover" | "box-front" => Some(Self::Cover),
            "3d box" | "cover3d" | "cover-3d" => Some(Self::Cover3D),
            "screenshot" => Some(Self::Screenshot),
            "title screen" | "titlescreen" => Some(Self::TitleScreen),
            "marquee" => Some(Self::Marquee),
            "video" => Some(Self::Video),
            "fanart" => Some(Self::Fanart),
            "physical media" | "physicalmedia" => Some(Self::PhysicalMedia),
            "miximage" => Some(Self::Miximage),
            _ => None,
        }
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
