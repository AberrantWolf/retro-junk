use retro_junk_core::Region;

/// Map retro-junk short names to ScreenScraper system IDs.
///
/// System IDs are ScreenScraper-specific and live here rather than on
/// the `RomAnalyzer` trait, as they're a third-party API detail.
pub fn screenscraper_system_id(short_name: &str) -> Option<u32> {
    match short_name {
        // Nintendo
        "nes" => Some(3),
        "snes" => Some(4),
        "n64" => Some(14),
        "gc" => Some(13),
        "wii" => Some(16),
        "wiiu" => Some(18),
        "gb" => Some(9),
        "gbc" => Some(10),
        "gba" => Some(12),
        "ds" => Some(15),
        "3ds" => Some(17),

        // Sony
        "ps1" => Some(57),
        "ps2" => Some(58),
        "ps3" => Some(59),
        "psp" => Some(61),
        "vita" => Some(62),

        // Sega
        "sg1000" => Some(109),
        "sms" => Some(2),
        "genesis" => Some(1),
        "segacd" => Some(20),
        "32x" => Some(19),
        "saturn" => Some(22),
        "dreamcast" => Some(23),
        "gg" => Some(21),

        // Microsoft
        "xbox" => Some(32),
        "xbox360" => Some(33),

        _ => None,
    }
}

/// Whether a console's ROM format normally contains a serial number.
///
/// Consoles that return true are expected to have serials extractable
/// from ROM headers. If analysis fails to find a serial for these,
/// it's worth logging an error rather than silently falling back.
pub fn expects_serial(short_name: &str) -> bool {
    matches!(
        short_name,
        "n64" | "gba"
            | "ds"
            | "3ds"
            | "gc"
            | "wii"
            | "wiiu"
            | "ps1"
            | "ps2"
            | "ps3"
            | "psp"
            | "vita"
            | "segacd"
            | "saturn"
            | "dreamcast"
            | "32x"
            | "xbox"
            | "xbox360"
    )
}

/// Map a ScreenScraper region code to a preferred region for name/media lookup.
pub fn preferred_ss_region(region: &str) -> &str {
    match region.to_lowercase().as_str() {
        "us" | "usa" | "united states" => "us",
        "eu" | "europe" => "eu",
        "jp" | "japan" => "jp",
        "wor" | "world" => "wor",
        _ => "us",
    }
}

/// Map a ROM-detected `Region` to the corresponding ScreenScraper region code.
pub fn region_to_ss_code(region: &Region) -> &'static str {
    match region {
        Region::Japan => "jp",
        Region::Usa => "us",
        Region::Europe => "eu",
        Region::Australia => "au",
        Region::Korea => "kr",
        Region::China => "cn",
        Region::Taiwan => "tw",
        Region::Brazil => "br",
        Region::World => "wor",
        Region::Unknown => "us",
    }
}

/// Map a ROM-detected `Region` to a likely description language code.
pub fn region_to_language(region: &Region) -> &'static str {
    match region {
        Region::Japan => "ja",
        Region::Usa => "en",
        Region::Europe => "en",
        Region::Australia => "en",
        Region::Korea => "ko",
        Region::China => "zh",
        Region::Taiwan => "zh",
        Region::Brazil => "pt",
        Region::World => "en",
        Region::Unknown => "en",
    }
}
