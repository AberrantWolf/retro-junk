use super::*;

#[test]
fn artwork_only_selection_excludes_video_but_keeps_images() {
    let selection = default_asset_selection(true);
    assert!(!selection.types.contains(&AssetType::Video));
    assert!(selection.types.contains(&AssetType::Cover));
    assert!(selection.types.contains(&AssetType::Screenshot));
}
