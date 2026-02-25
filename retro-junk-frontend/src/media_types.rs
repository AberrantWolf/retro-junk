use serde::{Deserialize, Serialize};
use std::fmt;

/// Media types that can be scraped and used by frontends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaType {
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

impl fmt::Display for MediaType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaType::Screenshot => write!(f, "screenshot"),
            MediaType::TitleScreen => write!(f, "title screen"),
            MediaType::Cover => write!(f, "cover"),
            MediaType::Cover3D => write!(f, "3D box"),
            MediaType::Marquee => write!(f, "marquee"),
            MediaType::Video => write!(f, "video"),
            MediaType::Fanart => write!(f, "fanart"),
            MediaType::PhysicalMedia => write!(f, "physical media"),
            MediaType::Miximage => write!(f, "miximage"),
        }
    }
}

impl MediaType {
    /// File extension for this media type.
    pub fn default_extension(&self) -> &'static str {
        match self {
            MediaType::Video => "mp4",
            _ => "png",
        }
    }
}
