use super::{Pattern, matches};

/// The whole reason this matcher exists rather than a per-component one: the
/// useful groupings in a review queue are directory-shaped, and a person
/// typing `*.txt` at a list of relative paths means every `.txt` anywhere.
#[test]
fn a_star_crosses_directory_separators() {
    assert!(matches("*.txt", "gc/rvz/readme.txt"));
    assert!(matches("*/rvz/*", "gc/rvz/Zelda.rvz"));
    assert!(!matches("*/rvz/*", "gc/iso/Zelda.iso"));
}

/// A bare word is the most common thing someone types, and demanding
/// `*zelda*` would make the filter box feel broken.
#[test]
fn a_word_without_wildcards_matches_anywhere_in_the_path() {
    assert!(matches("zelda", "gc/rvz/Zelda Wind Waker.rvz"));
    assert!(matches("rvz", "gc/rvz/Zelda Wind Waker.rvz"));
    assert!(!matches("metroid", "gc/rvz/Zelda Wind Waker.rvz"));
}

/// Anchoring is still available: once a wildcard appears, the pattern is
/// matched against the whole path rather than searched within it. Without
/// this, `*.txt` and `.txt` would behave identically and there would be no way
/// to say "ends with".
#[test]
fn a_pattern_with_wildcards_is_anchored_to_the_whole_path() {
    assert!(!matches("*.txt", "notes.txt.bak"));
    assert!(matches("*.txt*", "notes.txt.bak"));
    assert!(matches(".txt", "notes.txt.bak"));
}

#[test]
fn case_is_ignored_for_ascii() {
    assert!(matches("*.TXT", "gc/readme.txt"));
    assert!(matches("*.txt", "gc/README.TXT"));
}

/// Backtracking has to keep looking after a partial match fails: the first
/// `.rvz` here is not the last one, and a matcher that gave up at the first
/// candidate would miss the file.
#[test]
fn matching_retries_after_a_failed_partial_match() {
    assert!(matches("*.rvz", "gc/Game.rvz.part/disc.rvz"));
    assert!(!matches("*.rvz", "gc/Game.rvz.part/disc.iso"));
}

#[test]
fn character_sets_select_and_exclude() {
    assert!(matches("psx/disc[0-9].chd", "psx/disc3.chd"));
    assert!(!matches("psx/disc[0-9].chd", "psx/discA.chd"));
    assert!(matches("psx/disc[!0-9].chd", "psx/discA.chd"));
    assert!(!matches("psx/disc[!0-9].chd", "psx/disc3.chd"));
}

/// The filter box parses on every keystroke, so a half-typed set arrives here
/// constantly. It must read as a literal bracket rather than panicking or
/// swallowing the rest of the pattern.
#[test]
fn an_unterminated_set_is_a_literal_bracket() {
    assert!(matches("*[0-9", "psx/disc[0-9"));
    assert!(!matches("*[0-9", "psx/disc3"));
    assert!(Pattern::new("[").matches("["));
}

/// An empty pattern is an empty filter box, which means "show everything" —
/// not "show nothing", which would blank the view the moment someone cleared
/// the box.
#[test]
fn an_empty_pattern_matches_everything() {
    let pattern = Pattern::new("");
    assert!(pattern.is_empty());
    assert!(pattern.matches("anything at all"));
    assert!(pattern.matches(""));
}

/// A long run of wildcards is what someone produces by holding a key down.
/// Collapsing them keeps that from becoming exponential backtracking.
#[test]
fn repeated_wildcards_do_not_multiply_the_work() {
    assert!(matches("**********.txt", "gc/rvz/readme.txt"));
    assert!(!matches(
        "*a*a*a*a*a*a*a*a*a*a*b",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    ));
}
