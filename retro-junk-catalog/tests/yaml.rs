use retro_junk_catalog::{load_companies, load_overrides, load_platforms};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write_yaml(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

#[test]
fn load_platform_from_yaml() {
    let tmp = TempDir::new().unwrap();
    let platforms_dir = tmp.path().join("platforms");
    fs::create_dir(&platforms_dir).unwrap();

    write_yaml(
        &platforms_dir,
        "nes.yaml",
        r#"
id: nes
display_name: "Nintendo Entertainment System"
short_name: NES
manufacturer: Nintendo
generation: 3
media_type: cartridge
release_year: 1985
core_platform: Nes
regions:
  - region: usa
    release_date: "1985-10-18"
  - region: europe
    release_date: "1986-09-01"
relationships:
  - platform: famicom
    type: regional_variant
"#,
    );

    let platforms = load_platforms(&platforms_dir).unwrap();
    assert_eq!(platforms.len(), 1);
    let nes = &platforms[0];
    assert_eq!(nes.id, "nes");
    assert_eq!(nes.display_name, "Nintendo Entertainment System");
    assert_eq!(nes.generation, 3);
    assert_eq!(nes.regions.len(), 2);
    assert_eq!(nes.relationships.len(), 1);
    assert_eq!(nes.core_platform, "Nes");
    // description is absent from the YAML — defaults to empty, not None
    assert_eq!(nes.description, "");
}

#[test]
fn load_company_from_yaml() {
    let tmp = TempDir::new().unwrap();
    let companies_dir = tmp.path().join("companies");
    fs::create_dir(&companies_dir).unwrap();

    write_yaml(
        &companies_dir,
        "nintendo.yaml",
        r#"
id: nintendo
name: "Nintendo Co., Ltd."
country: Japan
aliases:
  - Nintendo
  - Nintendo EAD
"#,
    );

    let companies = load_companies(&companies_dir).unwrap();
    assert_eq!(companies.len(), 1);
    assert_eq!(companies[0].id, "nintendo");
    assert_eq!(companies[0].aliases.len(), 2);
}

#[test]
fn load_overrides_from_yaml() {
    let tmp = TempDir::new().unwrap();
    let overrides_dir = tmp.path().join("overrides");
    fs::create_dir(&overrides_dir).unwrap();

    write_yaml(
        &overrides_dir,
        "psx-serials.yaml",
        r#"
- entity_type: media
  platform_id: ps1
  dat_name_pattern: "Final Fantasy VII (USA) (Disc *)"
  field: game_serial
  override_value: "SCUS-94163"
  reason: "All FF7 USA discs share catalog serial"
"#,
    );

    let overrides = load_overrides(&overrides_dir).unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].platform_id, "ps1");
    // entity_id is absent from the YAML — defaults to empty, not None
    assert_eq!(overrides[0].entity_id, "");
}

#[test]
fn missing_dir_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("nonexistent");
    let result = load_platforms(&missing).unwrap();
    assert!(result.is_empty());
}

#[test]
fn shipped_platform_relationships_reference_known_platforms() {
    let platforms_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("catalog/platforms");
    let platforms = load_platforms(&platforms_dir).unwrap();
    let ids = platforms
        .iter()
        .map(|platform| platform.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();

    for platform in &platforms {
        for relationship in &platform.relationships {
            assert!(
                ids.contains(relationship.platform.as_str()),
                "{} references missing platform {}",
                platform.id,
                relationship.platform
            );
        }
    }
}
