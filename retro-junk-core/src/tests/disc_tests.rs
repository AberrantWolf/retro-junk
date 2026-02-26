use super::*;

#[test]
fn strip_disc_tag_removes_disc_number() {
    assert_eq!(
        strip_disc_tag("Final Fantasy VII (Disc 1) (USA)"),
        "Final Fantasy VII (USA)"
    );
}

#[test]
fn strip_disc_tag_preserves_non_disc_names() {
    assert_eq!(
        strip_disc_tag("Crash Bandicoot (USA)"),
        "Crash Bandicoot (USA)"
    );
}

#[test]
fn strip_disc_tag_multi_digit() {
    assert_eq!(
        strip_disc_tag("Some Game (Disc 12) (USA)"),
        "Some Game (USA)"
    );
}

#[test]
fn strip_disc_tag_disc_at_end() {
    assert_eq!(strip_disc_tag("Some Game (Disc 1)"), "Some Game");
}

#[test]
fn extract_disc_number_found() {
    assert_eq!(
        extract_disc_number("Final Fantasy VII (Disc 2) (USA).chd"),
        Some(2)
    );
}

#[test]
fn extract_disc_number_not_found() {
    assert_eq!(extract_disc_number("Crash Bandicoot (USA).chd"), None);
}

#[test]
fn extract_disc_number_from_stem() {
    assert_eq!(
        extract_disc_number("Metal Gear Solid (Disc 1) (USA)"),
        Some(1)
    );
}

#[test]
fn detect_disc_groups_basic() {
    let entries = vec![
        (0, "Final Fantasy VII (USA) (Disc 1)"),
        (1, "Final Fantasy VII (USA) (Disc 2)"),
        (2, "Final Fantasy VII (USA) (Disc 3)"),
        (3, "Crash Bandicoot (USA)"),
    ];
    let groups = detect_disc_groups(&entries);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].base_name, "Final Fantasy VII (USA)");
    assert_eq!(groups[0].primary_index, 0);
    assert_eq!(groups[0].member_indices, vec![0, 1, 2]);
}

#[test]
fn detect_disc_groups_ignores_single_disc() {
    let entries = vec![(0, "Some Game (Disc 1)"), (1, "Another Game (USA)")];
    let groups = detect_disc_groups(&entries);
    assert!(groups.is_empty());
}

#[test]
fn detect_disc_groups_multiple_games() {
    let entries = vec![
        (0, "FF7 (USA) (Disc 1)"),
        (1, "FF7 (USA) (Disc 2)"),
        (2, "MGS (USA) (Disc 1)"),
        (3, "MGS (USA) (Disc 2)"),
        (4, "Crash (USA)"),
    ];
    let groups = detect_disc_groups(&entries);
    assert_eq!(groups.len(), 2);

    let ff7 = groups.iter().find(|g| g.base_name == "FF7 (USA)").unwrap();
    assert_eq!(ff7.primary_index, 0);
    assert_eq!(ff7.member_indices, vec![0, 1]);

    let mgs = groups.iter().find(|g| g.base_name == "MGS (USA)").unwrap();
    assert_eq!(mgs.primary_index, 2);
    assert_eq!(mgs.member_indices, vec![2, 3]);
}

#[test]
fn detect_disc_groups_out_of_order_discs() {
    let entries = vec![
        (0, "Game (USA) (Disc 3)"),
        (1, "Game (USA) (Disc 1)"),
        (2, "Game (USA) (Disc 2)"),
    ];
    let groups = detect_disc_groups(&entries);
    assert_eq!(groups.len(), 1);
    // Primary should be the entry with disc 1 (index 1)
    assert_eq!(groups[0].primary_index, 1);
    // Members sorted by disc number: disc 1 (idx 1), disc 2 (idx 2), disc 3 (idx 0)
    assert_eq!(groups[0].member_indices, vec![1, 2, 0]);
}

#[test]
fn extract_disc_number_none_for_scenario_names() {
    // Scenario-named discs (e.g., Resident Evil 2 dual-scenario) have no "(Disc N)" tag
    assert_eq!(
        extract_disc_number("Resident Evil 2 (USA) (Leon Hen)"),
        None
    );
    assert_eq!(
        extract_disc_number("Resident Evil 2 (USA) (Claire Hen)"),
        None
    );
}

#[test]
fn strip_disc_tag_leaves_scenario_names_unchanged() {
    assert_eq!(
        strip_disc_tag("Resident Evil 2 (USA) (Leon Hen)"),
        "Resident Evil 2 (USA) (Leon Hen)"
    );
    assert_eq!(
        strip_disc_tag("Resident Evil 2 (USA) (Claire Hen)"),
        "Resident Evil 2 (USA) (Claire Hen)"
    );
}

// -- derive_base_game_name tests --

#[test]
fn derive_base_game_name_empty() {
    assert_eq!(derive_base_game_name(&[]), "");
}

#[test]
fn derive_base_game_name_single_with_disc_tag() {
    assert_eq!(
        derive_base_game_name(&["Final Fantasy VII (USA) (Disc 1)"]),
        "Final Fantasy VII (USA)"
    );
}

#[test]
fn derive_base_game_name_single_without_disc_tag() {
    assert_eq!(
        derive_base_game_name(&["Crash Bandicoot (USA)"]),
        "Crash Bandicoot (USA)"
    );
}

#[test]
fn derive_base_game_name_numbered_discs() {
    assert_eq!(
        derive_base_game_name(&["FF7 (USA) (Disc 1)", "FF7 (USA) (Disc 2)"]),
        "FF7 (USA)"
    );
}

#[test]
fn derive_base_game_name_scenario_discs() {
    assert_eq!(
        derive_base_game_name(&[
            "Resident Evil 2 (Japan) (Leon Hen)",
            "Resident Evil 2 (Japan) (Claire Hen)"
        ]),
        "Resident Evil 2 (Japan)"
    );
}

#[test]
fn derive_base_game_name_japanese_scenario() {
    assert_eq!(
        derive_base_game_name(&[
            "Biohazard 2 - Dual Shock Ver. (Japan) (Leon Hen)",
            "Biohazard 2 - Dual Shock Ver. (Japan) (Claire Hen)"
        ]),
        "Biohazard 2 - Dual Shock Ver. (Japan)"
    );
}

#[test]
fn derive_base_game_name_divergent_last_group() {
    assert_eq!(
        derive_base_game_name(&["Game (USA) (A Disc)", "Game (USA) (B Disc)"]),
        "Game (USA)"
    );
}
