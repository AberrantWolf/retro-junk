/// Platform/console identifiers for all supported systems.
///
/// This enum centralizes console identity — short names, display names,
/// manufacturer, and aliases — in one place, replacing ad-hoc string
/// matching throughout the codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    // Nintendo
    Nes,
    Snes,
    N64,
    GameCube,
    Wii,
    WiiU,
    GameBoy,
    Gba,
    Ds,
    N3ds,

    // Sega
    Sg1000,
    MasterSystem,
    Genesis,
    SegaCd,
    Sega32x,
    Saturn,
    Dreamcast,
    GameGear,

    // Sony
    Ps1,
    Ps2,
    Ps3,
    Psp,
    Vita,

    // Microsoft
    Xbox,
    Xbox360,
}

/// All platform variants in registration order.
const ALL_PLATFORMS: &[Platform] = &[
    Platform::Nes,
    Platform::Snes,
    Platform::N64,
    Platform::GameCube,
    Platform::Wii,
    Platform::WiiU,
    Platform::GameBoy,
    Platform::Gba,
    Platform::Ds,
    Platform::N3ds,
    Platform::Sg1000,
    Platform::MasterSystem,
    Platform::Genesis,
    Platform::SegaCd,
    Platform::Sega32x,
    Platform::Saturn,
    Platform::Dreamcast,
    Platform::GameGear,
    Platform::Ps1,
    Platform::Ps2,
    Platform::Ps3,
    Platform::Psp,
    Platform::Vita,
    Platform::Xbox,
    Platform::Xbox360,
];

impl Platform {
    /// Canonical short name used for CLI, folder paths, and identifiers.
    pub fn short_name(&self) -> &'static str {
        match self {
            Self::Nes => "nes",
            Self::Snes => "snes",
            Self::N64 => "n64",
            Self::GameCube => "gamecube",
            Self::Wii => "wii",
            Self::WiiU => "wiiu",
            Self::GameBoy => "gb",
            Self::Gba => "gba",
            Self::Ds => "nds",
            Self::N3ds => "3ds",
            Self::Sg1000 => "sg1000",
            Self::MasterSystem => "sms",
            Self::Genesis => "genesis",
            Self::SegaCd => "segacd",
            Self::Sega32x => "32x",
            Self::Saturn => "saturn",
            Self::Dreamcast => "dreamcast",
            Self::GameGear => "gamegear",
            Self::Ps1 => "ps1",
            Self::Ps2 => "ps2",
            Self::Ps3 => "ps3",
            Self::Psp => "psp",
            Self::Vita => "vita",
            Self::Xbox => "xbox",
            Self::Xbox360 => "xbox360",
        }
    }

    /// Full display name for the platform.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Nes => "Nintendo Entertainment System",
            Self::Snes => "Super Nintendo Entertainment System",
            Self::N64 => "Nintendo 64",
            Self::GameCube => "Nintendo GameCube",
            Self::Wii => "Nintendo Wii",
            Self::WiiU => "Nintendo Wii U",
            Self::GameBoy => "Game Boy / Game Boy Color",
            Self::Gba => "Game Boy Advance",
            Self::Ds => "Nintendo DS",
            Self::N3ds => "Nintendo 3DS",
            Self::Sg1000 => "Sega SG-1000",
            Self::MasterSystem => "Sega Master System",
            Self::Genesis => "Sega Genesis / Mega Drive",
            Self::SegaCd => "Sega CD / Mega CD",
            Self::Sega32x => "Sega 32X",
            Self::Saturn => "Sega Saturn",
            Self::Dreamcast => "Sega Dreamcast",
            Self::GameGear => "Sega Game Gear",
            Self::Ps1 => "Sony PlayStation",
            Self::Ps2 => "Sony PlayStation 2",
            Self::Ps3 => "Sony PlayStation 3",
            Self::Psp => "Sony PlayStation Portable",
            Self::Vita => "Sony PlayStation Vita",
            Self::Xbox => "Microsoft Xbox",
            Self::Xbox360 => "Microsoft Xbox 360",
        }
    }

    /// Console manufacturer.
    pub fn manufacturer(&self) -> &'static str {
        match self {
            Self::Nes
            | Self::Snes
            | Self::N64
            | Self::GameCube
            | Self::Wii
            | Self::WiiU
            | Self::GameBoy
            | Self::Gba
            | Self::Ds
            | Self::N3ds => "Nintendo",

            Self::Sg1000
            | Self::MasterSystem
            | Self::Genesis
            | Self::SegaCd
            | Self::Sega32x
            | Self::Saturn
            | Self::Dreamcast
            | Self::GameGear => "Sega",

            Self::Ps1 | Self::Ps2 | Self::Ps3 | Self::Psp | Self::Vita => "Sony",

            Self::Xbox | Self::Xbox360 => "Microsoft",
        }
    }

    /// All accepted names for this platform (case-insensitive matching).
    ///
    /// Includes the canonical short name plus any common alternatives
    /// used for folder names, CLI arguments, etc.
    pub fn aliases(&self) -> &'static [&'static str] {
        match self {
            Self::Nes => &["nes", "famicom", "fc"],
            Self::Snes => &["snes", "snesna", "sfc", "super famicom", "super nintendo"],
            Self::N64 => &["n64", "nintendo 64", "nintendo64"],
            Self::GameCube => &["gamecube", "gcn", "gc", "ngc"],
            Self::Wii => &["wii"],
            Self::WiiU => &["wiiu", "wii u"],
            Self::GameBoy => &["gb", "gbc", "gameboy", "game boy"],
            Self::Gba => &["gba", "game boy advance", "gameboy advance"],
            Self::Ds => &["nds", "ds", "nintendo ds"],
            Self::N3ds => &["3ds", "nintendo 3ds", "n3ds"],
            Self::Sg1000 => &["sg1000", "sg-1000", "sc3000", "sc-3000"],
            Self::MasterSystem => &["sms", "master system", "mastersystem", "mark iii"],
            Self::Genesis => &[
                "genesis",
                "megadrive",
                "megadrivejp",
                "mega drive",
                "md",
                "gen",
            ],
            Self::SegaCd => &["segacd", "sega cd", "megacd", "mega cd"],
            Self::Sega32x => &["32x", "sega32x", "sega 32x"],
            Self::Saturn => &["saturn", "sega saturn"],
            Self::Dreamcast => &["dreamcast", "dc"],
            Self::GameGear => &["gamegear", "game gear", "gg"],
            Self::Ps1 => &["ps1", "psx", "playstation", "playstation1"],
            Self::Ps2 => &["ps2", "playstation2", "playstation 2"],
            Self::Ps3 => &["ps3", "playstation3", "playstation 3"],
            Self::Psp => &["psp", "playstation portable"],
            Self::Vita => &["vita", "psvita", "ps vita", "playstation vita"],
            Self::Xbox => &["xbox", "xbox1", "ogxbox"],
            Self::Xbox360 => &["xbox360", "xbox 360", "x360"],
        }
    }

    /// All 25 platform variants.
    pub fn all() -> &'static [Platform] {
        ALL_PLATFORMS
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Error returned when a string cannot be parsed into a `Platform`.
#[derive(Debug, Clone)]
pub struct PlatformParseError(pub String);

impl std::fmt::Display for PlatformParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown platform: '{}'", self.0)
    }
}

impl std::error::Error for PlatformParseError {}

impl std::str::FromStr for Platform {
    type Err = PlatformParseError;

    /// Parse a platform from any recognized name (case-insensitive).
    ///
    /// Matches against `short_name()` and all entries in `aliases()`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_lowercase();
        for &platform in ALL_PLATFORMS {
            if platform.short_name() == lower {
                return Ok(platform);
            }
            for alias in platform.aliases() {
                if *alias == lower {
                    return Ok(platform);
                }
            }
        }
        Err(PlatformParseError(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_has_25_variants() {
        assert_eq!(Platform::all().len(), 25);
    }

    #[test]
    fn canonical_names_round_trip() {
        for &platform in Platform::all() {
            let parsed: Platform = platform.short_name().parse().unwrap();
            assert_eq!(parsed, platform, "round-trip failed for {:?}", platform);
        }
    }

    #[test]
    fn aliases_resolve_correctly() {
        // Test a sample of aliases across manufacturers
        let cases = [
            ("gc", Platform::GameCube),
            ("ds", Platform::Ds),
            ("gg", Platform::GameGear),
            ("psx", Platform::Ps1),
            ("sfc", Platform::Snes),
            ("mega drive", Platform::Genesis),
            ("gen", Platform::Genesis),
            ("gbc", Platform::GameBoy),
            ("x360", Platform::Xbox360),
            ("dc", Platform::Dreamcast),
            ("n3ds", Platform::N3ds),
            ("psvita", Platform::Vita),
            ("ogxbox", Platform::Xbox),
            ("sc3000", Platform::Sg1000),
            ("mark iii", Platform::MasterSystem),
        ];
        for (input, expected) in cases {
            let parsed: Platform = input.parse().unwrap();
            assert_eq!(
                parsed, expected,
                "alias '{}' should parse to {:?}",
                input, expected
            );
        }
    }

    #[test]
    fn case_insensitive_parsing() {
        let parsed: Platform = "SNES".parse().unwrap();
        assert_eq!(parsed, Platform::Snes);
        let parsed: Platform = "GameCube".parse().unwrap();
        assert_eq!(parsed, Platform::GameCube);
        let parsed: Platform = "PS1".parse().unwrap();
        assert_eq!(parsed, Platform::Ps1);
    }

    #[test]
    fn unknown_string_returns_err() {
        let result: Result<Platform, _> = "commodore64".parse();
        assert!(result.is_err());
    }

    #[test]
    fn short_name_is_first_alias() {
        for &platform in Platform::all() {
            assert_eq!(
                platform.short_name(),
                platform.aliases()[0],
                "short_name should be first alias for {:?}",
                platform,
            );
        }
    }

    #[test]
    fn display_returns_display_name() {
        assert_eq!(Platform::Nes.to_string(), "Nintendo Entertainment System");
        assert_eq!(Platform::Genesis.to_string(), "Sega Genesis / Mega Drive");
    }
}
