use super::*;

#[test]
fn test_is_power_of_two() {
    assert!(is_power_of_two(1));
    assert!(is_power_of_two(2));
    assert!(is_power_of_two(1024));
    assert!(is_power_of_two(1048576));
    assert!(!is_power_of_two(0));
    assert!(!is_power_of_two(3));
    assert!(!is_power_of_two(1000));
}

#[test]
fn test_build_strategies_with_expected_size() {
    // File is 2 MB, expected 4 MB — should get append 0x00 and 0xFF strategies
    let strategies = build_strategies(2 * 1024 * 1024, Some(4 * 1024 * 1024), DatSource::NoIntro);
    assert_eq!(strategies.len(), 2);
    assert_eq!(strategies[0].padding.append_size, 2 * 1024 * 1024);
    assert_eq!(strategies[0].padding.fill_byte, 0x00);
    assert_eq!(strategies[1].padding.fill_byte, 0xFF);
}

#[test]
fn test_build_strategies_redump_pregap() {
    // Redump disc image with no expected size
    let strategies = build_strategies(650_000_000, None, DatSource::Redump);
    assert_eq!(strategies.len(), 1);
    assert_eq!(strategies[0].padding.prepend_size, CD_PREGAP_SIZE);
    assert_eq!(strategies[0].padding.fill_byte, 0x00);
}

#[test]
fn test_build_strategies_no_expected_non_pow2() {
    // 3 MB NoIntro ROM with no expected size — should try padding to 4 MB
    let strategies = build_strategies(3 * 1024 * 1024, None, DatSource::NoIntro);
    assert_eq!(strategies.len(), 2);
    assert_eq!(strategies[0].padding.append_size, 1024 * 1024);
    assert_eq!(strategies[0].padding.fill_byte, 0x00);
    assert_eq!(strategies[1].padding.fill_byte, 0xFF);
}

#[test]
fn test_build_strategies_already_pow2_no_expected() {
    // 4 MB NoIntro ROM with no expected size — already power of 2, no strategies
    let strategies = build_strategies(4 * 1024 * 1024, None, DatSource::NoIntro);
    assert!(strategies.is_empty());
}

#[test]
fn test_build_strategies_expected_plus_redump() {
    // Redump with expected size: should get append strategies + pregap strategy
    let strategies = build_strategies(600_000_000, Some(650_000_000), DatSource::Redump);
    assert_eq!(strategies.len(), 3); // append 0x00, append 0xFF, prepend pregap
}

#[test]
fn test_repair_method_description() {
    let m = RepairMethod::AppendPadding {
        fill_byte: 0x00,
        bytes_added: 1048576,
    };
    assert_eq!(m.description(), "append 1 MB of 0x00");

    let m = RepairMethod::PrependPadding {
        fill_byte: 0x00,
        bytes_added: 352800,
    };
    assert_eq!(m.description(), "prepend 352800 bytes of 0x00");
}

#[test]
fn test_backup_extension() {
    // Verify the backup path construction
    let path = PathBuf::from("/roms/snes/game.sfc");
    let bak_path = path.with_extension(format!(
        "{}.bak",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    assert_eq!(bak_path, PathBuf::from("/roms/snes/game.sfc.bak"));
}
