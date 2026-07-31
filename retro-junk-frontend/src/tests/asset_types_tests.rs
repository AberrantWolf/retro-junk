use super::*;

/// `ScreenScraper` returns JPG where our default extension is PNG, so discovery
/// has to probe every plausible extension. Every caller of this used to own a
/// copy of the loop; a regression here silently reports "no artwork" for a
/// fully scraped library.
#[test]
fn discovery_finds_a_non_default_extension() {
    let dir = tempfile::tempdir().expect("tempdir");
    let covers = dir.path().join(AssetType::Cover.subdirectory());
    std::fs::create_dir_all(&covers).expect("create covers dir");
    std::fs::write(covers.join("Some Game (USA).jpg"), b"x").expect("write cover");

    let found = collect_existing_assets(&[AssetType::Cover], dir.path(), "Some Game (USA)");

    assert_eq!(
        found.get(&AssetType::Cover).map(PathBuf::as_path),
        Some(covers.join("Some Game (USA).jpg").as_path())
    );
}

/// A type with no file on disk must be absent from the map rather than
/// present-with-a-missing-path: callers treat presence as "have it".
#[test]
fn discovery_omits_types_with_no_file() {
    let dir = tempfile::tempdir().expect("tempdir");

    let found = collect_existing_assets(DISPLAY_ASSET_TYPES, dir.path(), "Some Game (USA)");

    assert!(found.is_empty(), "{found:?}");
}

/// The CLI passes `--media-types` through verbatim. Before consolidation the
/// match was case- and whitespace-sensitive, so `--media-types "Covers, videos"`
/// silently selected nothing at all.
#[test]
fn selection_names_tolerate_case_and_padding() {
    let names = vec!["Covers".to_owned(), " videos ".to_owned()];

    let selection = AssetSelection::from_names(&names);

    assert_eq!(selection.types, vec![AssetType::Cover, AssetType::Video]);
}

/// `Miximage` is composed locally from the other assets. If it ever entered a
/// download selection the scraper would ask `ScreenScraper` for a media type
/// that does not exist there.
#[test]
fn downloadable_selections_never_include_miximage() {
    assert!(!AssetSelection::all().contains(AssetType::Miximage));
    assert!(!AssetSelection::default().contains(AssetType::Miximage));
    assert!(
        AssetSelection::from_names(&["miximage".to_owned(), "miximages".to_owned()])
            .types
            .is_empty()
    );
}
