use retro_junk_catalog::name_parser::{DumpStatus, parse_dat_name, region_to_slug};

#[test]
fn simple_usa_game() {
    let p = parse_dat_name("Super Mario Bros. (USA)");
    assert_eq!(p.title, "Super Mario Bros.");
    assert_eq!(p.regions, vec!["USA"]);
    assert!(p.revision.is_none());
    assert!(p.languages.is_empty());
    assert!(p.flags.is_empty());
    assert_eq!(p.status, DumpStatus::Verified);
}

#[test]
fn japan_game() {
    let p = parse_dat_name("Super Mario Bros. (Japan)");
    assert_eq!(p.title, "Super Mario Bros.");
    assert_eq!(p.regions, vec!["Japan"]);
}

#[test]
fn multi_region() {
    let p = parse_dat_name("Tetris (USA, Europe)");
    assert_eq!(p.title, "Tetris");
    assert_eq!(p.regions, vec!["USA", "Europe"]);
}

#[test]
fn with_revision() {
    let p = parse_dat_name("The Legend of Zelda (USA) (Rev A)");
    assert_eq!(p.title, "The Legend of Zelda");
    assert_eq!(p.regions, vec!["USA"]);
    assert_eq!(p.revision, Some("Rev A".to_string()));
}

#[test]
fn with_numeric_revision() {
    let p = parse_dat_name("Some Game (Europe) (Rev 1)");
    assert_eq!(p.revision, Some("Rev 1".to_string()));
}

#[test]
fn with_languages() {
    let p = parse_dat_name("Game Title (Europe) (En,Fr,De)");
    assert_eq!(p.title, "Game Title");
    assert_eq!(p.regions, vec!["Europe"]);
    assert_eq!(p.languages, vec!["En", "Fr", "De"]);
}

#[test]
fn with_disc_number() {
    let p = parse_dat_name("Final Fantasy VII (USA) (Disc 1)");
    assert_eq!(p.title, "Final Fantasy VII");
    assert_eq!(p.disc_number, Some(1));
    assert!(p.disc_label.is_none());
}

#[test]
fn with_disc_label() {
    let p = parse_dat_name("Resident Evil 2 (USA) (Disc 1 - Leon)");
    assert_eq!(p.title, "Resident Evil 2");
    assert_eq!(p.disc_number, Some(1));
    assert_eq!(p.disc_label, Some("Leon".to_string()));
}

#[test]
fn with_version() {
    let p = parse_dat_name("Game (USA) (v1.1)");
    assert_eq!(p.version, Some("v1.1".to_string()));
}

#[test]
fn prototype_flag() {
    let p = parse_dat_name("Unreleased Game (USA) (Proto)");
    assert_eq!(p.title, "Unreleased Game");
    assert_eq!(p.flags, vec!["Proto"]);
}

#[test]
fn beta_flag() {
    let p = parse_dat_name("Game (Japan) (Beta)");
    assert!(p.flags.contains(&"Beta".to_string()));
}

#[test]
fn unlicensed_flag() {
    let p = parse_dat_name("Tengen Tetris (USA) (Unl)");
    assert!(p.flags.contains(&"Unl".to_string()));
}

#[test]
fn bad_dump_status() {
    let p = parse_dat_name("Game (USA) [b]");
    assert_eq!(p.status, DumpStatus::BadDump);
}

#[test]
fn verified_status() {
    let p = parse_dat_name("Game (USA) [!]");
    assert_eq!(p.status, DumpStatus::Verified);
}

#[test]
fn overdump_status() {
    let p = parse_dat_name("Game (USA) [o]");
    assert_eq!(p.status, DumpStatus::Overdump);
}

#[test]
fn complex_name() {
    let p = parse_dat_name("Legend of Zelda, The - Ocarina of Time (USA) (Rev B) (En,Fr) [!]");
    assert_eq!(p.title, "Legend of Zelda, The - Ocarina of Time");
    assert_eq!(p.regions, vec!["USA"]);
    assert_eq!(p.revision, Some("Rev B".to_string()));
    assert_eq!(p.languages, vec!["En", "Fr"]);
    assert_eq!(p.status, DumpStatus::Verified);
}

#[test]
fn world_region() {
    let p = parse_dat_name("Game (World)");
    assert_eq!(p.regions, vec!["World"]);
}

#[test]
fn no_tags() {
    let p = parse_dat_name("Just a Name");
    assert_eq!(p.title, "Just a Name");
    assert!(p.regions.is_empty());
}

#[test]
fn multiple_discs_disc_2() {
    let p = parse_dat_name("Final Fantasy VII (USA) (Disc 2)");
    assert_eq!(p.disc_number, Some(2));
}

#[test]
fn revision_and_version() {
    let p = parse_dat_name("Game (USA) (Rev A) (v1.2)");
    assert_eq!(p.revision, Some("Rev A".to_string()));
    assert_eq!(p.version, Some("v1.2".to_string()));
}

#[test]
fn region_slug_mapping() {
    assert_eq!(region_to_slug("USA"), "usa");
    assert_eq!(region_to_slug("Japan"), "japan");
    assert_eq!(region_to_slug("Europe"), "europe");
    assert_eq!(region_to_slug("World"), "world");
    assert_eq!(region_to_slug("Australia"), "australia");
    assert_eq!(region_to_slug("Korea"), "korea");
    assert_eq!(region_to_slug("France"), "france");
    assert_eq!(region_to_slug("Germany"), "germany");
    assert_eq!(region_to_slug("United Kingdom"), "united-kingdom");
}

#[test]
fn title_with_parentheses_in_name() {
    // Some games genuinely have parens in the title, but the convention
    // is that the first paren group after the title is region
    let p = parse_dat_name("Game (Part 1) (USA)");
    // "Game" should be the title since "(Part 1)" isn't a region
    // and gets treated as a flag
    assert_eq!(p.title, "Game");
    assert_eq!(p.regions, vec!["USA"]);
}

#[test]
fn sample_flag() {
    let p = parse_dat_name("Game (Japan) (Sample)");
    assert!(p.flags.contains(&"Sample".to_string()));
}

#[test]
fn demo_flag() {
    let p = parse_dat_name("Game (USA) (Demo)");
    assert!(p.flags.contains(&"Demo".to_string()));
}
