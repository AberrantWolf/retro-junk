use super::{IgnoreRule, IgnoreRules, load_rules, remove_rule, write_rule};
use crate::sidecar::{SidecarError, SidecarRecord as _};

/// The property the whole sidecar layout depends on: the same decision made on
/// two machines has to produce the same file with the same bytes, or syncing
/// the collection turns two agreements into a conflict.
#[test]
fn the_same_rule_written_twice_is_the_same_file() {
    let first = tempfile::tempdir().expect("temp dir");
    let second = tempfile::tempdir().expect("temp dir");
    let rule = IgnoreRule::new("*.txt", "");

    let one = write_rule(first.path(), &rule).expect("write");
    let two = write_rule(second.path(), &rule).expect("write");

    assert_eq!(one.file_name(), two.file_name());
    assert_eq!(
        std::fs::read(&one).expect("read"),
        std::fs::read(&two).expect("read"),
        "identical decisions must be byte-identical so copies converge"
    );
}

/// Matching ignores case, so `*.TXT` and `*.txt` are one decision. If they
/// were stored as typed they would become two files covering the same group,
/// and revoking one would appear to do nothing.
#[test]
fn case_variants_of_a_pattern_are_one_rule() {
    let collection = tempfile::tempdir().expect("temp dir");
    write_rule(collection.path(), &IgnoreRule::new("*.TXT", "")).expect("write");
    write_rule(collection.path(), &IgnoreRule::new("*.txt", "")).expect("write");

    let rules = load_rules(collection.path()).expect("load");
    assert_eq!(rules.len(), 1, "one decision should be one file");
    assert_eq!(rules[0].pattern, "*.txt");
}

/// Two patterns that reduce to the same readable stem must still be two rules;
/// the digest in the file name is what keeps them apart.
#[test]
fn patterns_that_slugify_alike_do_not_collide() {
    let collection = tempfile::tempdir().expect("temp dir");
    write_rule(collection.path(), &IgnoreRule::new("*.txt", "")).expect("write");
    write_rule(collection.path(), &IgnoreRule::new("*/txt/*", "")).expect("write");

    assert_eq!(load_rules(collection.path()).expect("load").len(), 2);
}

/// Ignoring has to be reversible, because that is the entire safety case for
/// a bulk button: revoking a rule makes the next sweep file those files again.
#[test]
fn a_revoked_rule_stops_ignoring() {
    let collection = tempfile::tempdir().expect("temp dir");
    let rule = IgnoreRule::new("*.txt", "stray notes");
    write_rule(collection.path(), &rule).expect("write");

    let rules = IgnoreRules::load(collection.path()).expect("load");
    assert!(rules.matching("gc/rvz/readme.txt").is_some());

    assert!(remove_rule(collection.path(), &rule).expect("remove"));
    assert!(
        !remove_rule(collection.path(), &rule).expect("remove"),
        "revoking twice is not an error, it is already gone"
    );
    let rules = IgnoreRules::load(collection.path()).expect("load");
    assert!(rules.matching("gc/rvz/readme.txt").is_none());
}

/// An empty pattern would match every path, so storing one would silently
/// ignore the whole library. It is refused at the point of naming, which is
/// the one place every write goes through.
#[test]
fn an_empty_pattern_is_refused() {
    let collection = tempfile::tempdir().expect("temp dir");
    let rule = IgnoreRule::new("   ", "");
    assert!(rule.is_empty());
    assert!(rule.sidecar_name().is_none());
    assert!(matches!(
        write_rule(collection.path(), &rule),
        Err(SidecarError::Unnamed(_))
    ));
}

/// The sweep reports which decision suppressed a file, so an ignored stray is
/// explainable rather than mysteriously absent.
#[test]
fn a_match_names_the_rule_that_covered_it() {
    let rules = IgnoreRules::from_rules(vec![
        IgnoreRule::new("*.txt", "stray notes"),
        IgnoreRule::new("*/rvz/*", "GameCube, not archivable yet"),
    ]);

    assert_eq!(
        rules
            .matching("gc/rvz/Zelda.rvz")
            .map(|rule| rule.note.as_str()),
        Some("GameCube, not archivable yet")
    );
    assert_eq!(
        rules
            .matching("psx/readme.txt")
            .map(|rule| rule.note.as_str()),
        Some("stray notes")
    );
    assert!(rules.matching("psx/Wipeout.chd").is_none());
}

/// A rule written by a newer build must not take the rest down with it, and
/// must not be silently obeyed under rules this build does not understand.
#[test]
fn a_future_schema_rule_is_skipped_not_fatal() {
    let collection = tempfile::tempdir().expect("temp dir");
    write_rule(collection.path(), &IgnoreRule::new("*.txt", "")).expect("write");
    let directory = super::ignore_directory(collection.path());
    std::fs::write(
        directory.join("from-the-future.toml"),
        "schema_version = 99\npattern = \"*.iso\"\n",
    )
    .expect("write future rule");

    let rules = load_rules(collection.path()).expect("load");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].pattern, "*.txt");
}
