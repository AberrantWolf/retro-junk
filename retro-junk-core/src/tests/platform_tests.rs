use super::*;

#[test]
fn canonical_names_round_trip() {
    for &platform in Platform::all() {
        let parsed: Platform = platform.short_name().parse().unwrap();
        assert_eq!(parsed, platform, "round-trip failed for {platform:?}");
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
        ("pcenginecd", Platform::PceCd),
        ("tg-cd", Platform::PceCd),
        // The card system and the CD add-on are separate libraries with
        // separate databases; their names must not bleed into each other.
        ("pce", Platform::Pce),
        ("pc engine", Platform::Pce),
        ("tg16", Platform::Pce),
        ("turbografx-16", Platform::Pce),
    ];
    for (input, expected) in cases {
        let parsed: Platform = input.parse().unwrap();
        assert_eq!(
            parsed, expected,
            "alias '{input}' should parse to {expected:?}"
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
            "short_name should be first alias for {platform:?}",
        );
    }
}

/// Archive and frontend directories spell regional platforms with separators
/// (`super-famicom`); the catalog keys them by canonical platform. A parse that
/// only accepted the spaced alias silently split one platform in two.
#[test]
fn separator_styles_resolve_to_the_same_platform() {
    for name in [
        "super famicom",
        "super-famicom",
        "Super_Famicom",
        "sfc",
        "snesna",
    ] {
        assert_eq!(
            name.parse::<Platform>().unwrap(),
            Platform::Snes,
            "{name} did not resolve to SNES"
        );
    }
    assert_eq!("famicom".parse::<Platform>().unwrap(), Platform::Nes);
    assert_eq!("gbc".parse::<Platform>().unwrap(), Platform::GameBoy);
    // Aliases that are themselves hyphenated keep working.
    assert_eq!("sg-1000".parse::<Platform>().unwrap(), Platform::Sg1000);
    assert_eq!("tg-cd".parse::<Platform>().unwrap(), Platform::PceCd);
}

#[test]
fn catalog_ids_and_equivalence_share_the_alias_model() {
    assert_eq!(catalog_platform_id("saturnjp"), "saturn");
    assert_eq!(catalog_platform_id("super-famicom"), "snes");
    assert_eq!(catalog_platform_id("psx"), "ps1");
    assert_eq!(catalog_platform_id("unknown-system"), "unknown-system");

    assert!(platform_ids_match("saturnjp", "saturn"));
    assert!(platform_ids_match("PSX", "ps1"));
    assert!(!platform_ids_match("saturn", "dreamcast"));
    assert!(!platform_ids_match("unknown-a", "unknown-b"));
}
