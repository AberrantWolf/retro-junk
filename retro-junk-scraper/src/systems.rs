use retro_junk_core::{Platform, Region};

/// Map a `Platform` to its ScreenScraper system ID.
///
/// System IDs are ScreenScraper-specific and live here rather than on
/// the `RomAnalyzer` trait, as they're a third-party API detail.
pub fn screenscraper_system_id(platform: Platform) -> Option<u32> {
    match platform {
        // Nintendo
        Platform::Nes => Some(3),
        Platform::Snes => Some(4),
        Platform::N64 => Some(14),
        Platform::GameCube => Some(13),
        Platform::Wii => Some(16),
        Platform::WiiU => Some(18),
        Platform::GameBoy => Some(9),
        Platform::Gba => Some(12),
        Platform::Ds => Some(15),
        Platform::N3ds => Some(17),

        // Sony
        Platform::Ps1 => Some(57),
        Platform::Ps2 => Some(58),
        Platform::Ps3 => Some(59),
        Platform::Psp => Some(61),
        Platform::Vita => Some(62),

        // Sega
        Platform::Sg1000 => Some(109),
        Platform::MasterSystem => Some(2),
        Platform::Genesis => Some(1),
        Platform::SegaCd => Some(20),
        Platform::Sega32x => Some(19),
        Platform::Saturn => Some(22),
        Platform::Dreamcast => Some(23),
        Platform::GameGear => Some(21),

        // Microsoft
        Platform::Xbox => Some(32),
        Platform::Xbox360 => Some(33),
    }
}

/// Additional ScreenScraper system IDs that should be accepted as valid
/// when looking up games for a given platform.
///
/// Some platforms span multiple ScreenScraper system IDs. For example, our
/// `GameBoy` analyzer handles both GB (system 9) and GBC (system 10) ROMs.
/// When we send system ID 9, ScreenScraper may return a GBC-only game as
/// system 10 — that's a valid match, not a platform mismatch.
pub fn acceptable_system_ids(platform: Platform) -> &'static [u32] {
    match platform {
        // Game Boy analyzer handles both GB (9) and GBC (10)
        Platform::GameBoy => &[10],
        _ => &[],
    }
}

/// Whether a console's ROM format normally contains a serial number.
///
/// Consoles that return true are expected to have serials extractable
/// from ROM headers. If analysis fails to find a serial for these,
/// it's worth logging an error rather than silently falling back.
pub fn expects_serial(platform: Platform) -> bool {
    matches!(
        platform,
        Platform::N64
            | Platform::Gba
            | Platform::Ds
            | Platform::N3ds
            | Platform::GameCube
            | Platform::Wii
            | Platform::WiiU
            | Platform::Ps1
            | Platform::Ps2
            | Platform::Ps3
            | Platform::Psp
            | Platform::Vita
            | Platform::SegaCd
            | Platform::Saturn
            | Platform::Dreamcast
            | Platform::Sega32x
            | Platform::Xbox
            | Platform::Xbox360
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
