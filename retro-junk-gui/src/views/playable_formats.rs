use retro_junk_archive::RepresentationFormat;
use retro_junk_core::Platform;

/// Representation formats which can be handed directly to a mainstream
/// emulator for this platform. This intentionally excludes preservation
/// formats whose filename is misleadingly similar to an emulator input (for
/// example, a Redump Xbox ISO is not the XISO required by xemu).
pub(super) fn supported(platform: Platform) -> Vec<RepresentationFormat> {
    use RepresentationFormat as Format;

    match platform {
        Platform::Ps1 | Platform::SegaCd | Platform::Saturn => {
            vec![Format::Chd, Format::CueBin]
        }
        Platform::Ps2 => vec![Format::Chd, Format::Iso, Format::CueBin],
        Platform::Psp => vec![Format::Chd, Format::Iso],
        Platform::Dreamcast => vec![Format::Chd, Format::CueBin],
        Platform::GameCube | Platform::Wii => vec![Format::Rvz, Format::Iso],
        Platform::Xbox360 => vec![Format::Iso],
        // Xbox needs XISO, PS3 needs an installed/extracted layout or package,
        // and Vita needs a supported install package/layout. None of those are
        // represented honestly by the formats modeled in the manifest yet.
        Platform::Xbox | Platform::Ps3 | Platform::Vita => Vec::new(),
        Platform::Nes
        | Platform::Snes
        | Platform::N64
        | Platform::WiiU
        | Platform::GameBoy
        | Platform::Gba
        | Platform::Ds
        | Platform::N3ds
        | Platform::Sg1000
        | Platform::MasterSystem
        | Platform::Genesis
        | Platform::Sega32x
        | Platform::GameGear => vec![Format::Rom],
    }
}

pub(super) fn label(format: Option<&RepresentationFormat>) -> &'static str {
    match format {
        None => "No preference",
        Some(RepresentationFormat::Rom) => "Native emulator file",
        Some(RepresentationFormat::Chd) => "CHD",
        Some(RepresentationFormat::Rvz) => "RVZ",
        Some(RepresentationFormat::Iso) => "ISO",
        Some(RepresentationFormat::CueBin) => "BIN/CUE",
        Some(RepresentationFormat::RedumperRaw) => "Redumper raw",
        Some(RepresentationFormat::Other(_)) => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::supported;
    use retro_junk_archive::RepresentationFormat as Format;
    use retro_junk_core::Platform;

    #[test]
    fn formats_are_console_specific() {
        assert_eq!(supported(Platform::Nes), vec![Format::Rom]);
        assert_eq!(supported(Platform::Ps1), vec![Format::Chd, Format::CueBin]);
        assert_eq!(
            supported(Platform::GameCube),
            vec![Format::Rvz, Format::Iso]
        );
        assert_eq!(
            supported(Platform::Dreamcast),
            vec![Format::Chd, Format::CueBin]
        );
        assert_eq!(supported(Platform::Psp), vec![Format::Chd, Format::Iso]);
        assert_eq!(supported(Platform::Xbox360), vec![Format::Iso]);
        assert!(supported(Platform::Xbox).is_empty());
        assert!(supported(Platform::Ps3).is_empty());
        assert!(supported(Platform::Vita).is_empty());
    }

    #[test]
    fn every_offered_format_is_modeled_by_the_policy_manifest() {
        for platform in Platform::all() {
            for format in supported(*platform) {
                assert!(!matches!(format, Format::Other(_)));
            }
        }
    }
}
