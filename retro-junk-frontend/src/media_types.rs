/// Media types that can be scraped and used by frontends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

impl MediaType {
    /// File extension for this media type.
    pub fn default_extension(&self) -> &'static str {
        match self {
            MediaType::Video => "mp4",
            _ => "png",
        }
    }
}
