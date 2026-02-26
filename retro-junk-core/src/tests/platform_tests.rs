use super::*;

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
