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
    /// File extension for this asset type.
    pub fn default_extension(&self) -> &'static str {
        match self {
            AssetType::Video => "mp4",
            _ => "png",
        }
    }
}
