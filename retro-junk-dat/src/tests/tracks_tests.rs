use super::*;

fn rom(name: &str) -> DatRom {
    DatRom {
        name: name.to_owned(),
        size: 1,
        crc: "00000000".to_owned(),
        sha1: None,
        md5: None,
        serial: None,
    }
}

#[test]
fn cue_does_not_make_a_single_track_disc_multi_track() {
    let single = [rom("Game (USA).cue"), rom("Game (USA).bin")];
    assert!(!is_multi_track(&single));

    let multi = [
        rom("Game (USA).cue"),
        rom("Game (USA) (Track 1).bin"),
        rom("Game (USA) (Track 2).bin"),
    ];
    assert!(is_multi_track(&multi));
}

#[test]
fn a_whole_disc_container_is_never_named_after_a_member_track() {
    // The reported case: Media.rom_name for a multi-track Redump game is the
    // largest *track*, so taking its stem named the CHD "(Track 1)".
    let stem = whole_medium_stem(
        "Tenchi Muyou! Ryououki Gokuraku CD-ROM for Sega Saturn (Japan) (1M)",
        "Tenchi Muyou! Ryououki Gokuraku CD-ROM for Sega Saturn (Japan) (1M) (Track 1).bin",
        true,
    );
    assert_eq!(
        stem,
        "Tenchi Muyou! Ryououki Gokuraku CD-ROM for Sega Saturn (Japan) (1M)"
    );
}

#[test]
fn single_file_media_keep_the_rom_name_that_distinguishes_representations() {
    // N64 DATs carry .z64 and .v64 records under one game name; the ROM name
    // is the only thing that tells them apart.
    assert_eq!(
        whole_medium_stem("Game (USA)", "Game (USA).v64", false),
        "Game (USA)"
    );
    assert_eq!(
        whole_medium_stem("Game (USA)", "Game (USA) (Byte Swapped).v64", false),
        "Game (USA) (Byte Swapped)"
    );
}

#[test]
fn a_period_in_a_game_name_is_not_an_extension() {
    // `Path::file_stem` on a DAT game name truncates at the last period, so
    // "Dr. Mario (USA)" became "Dr". Game names are used verbatim.
    assert_eq!(
        whole_medium_stem("Dr. Mario (USA)", "Dr. Mario (USA) (Track 1).bin", true),
        "Dr. Mario (USA)"
    );
    assert_eq!(
        whole_medium_stem("Vol. 2 (Japan)", "", false),
        "Vol. 2 (Japan)"
    );
    // A real filename still loses its real extension, periods and all.
    assert_eq!(
        whole_medium_stem("Dr. Mario (USA)", "Dr. Mario (USA).bin", false),
        "Dr. Mario (USA)"
    );
}

#[test]
fn without_a_game_name_the_track_tag_is_stripped_instead() {
    // The GUI rename path holds only the matched ROM entry for some matches.
    assert_eq!(
        whole_medium_stem("", "Game (USA) (Track 1).bin", true),
        "Game (USA)"
    );
    // Redump appends the tag before nothing, but be robust to a trailing tag.
    assert_eq!(
        strip_track_tag("Game (USA) (Track 12) (Rev 1)"),
        "Game (USA) (Rev 1)"
    );
    assert_eq!(strip_track_tag("Game (USA)"), "Game (USA)");
    assert_eq!(strip_track_tag("Game (Track A)"), "Game (Track A)");
}

#[test]
fn track_numbers_come_from_the_tag_not_the_position() {
    assert_eq!(track_number("Game (USA) (Track 02).bin"), 2);
    assert_eq!(track_number("Game (USA) (Track 11).bin"), 11);
    assert_eq!(track_number("Game (USA).bin"), 0);
    assert!(is_track_member("Game (USA) (Track 1).bin"));
    assert!(!is_track_member("Game (USA).cue"));
    assert!(!is_track_member("Game (USA).bin"));
}
