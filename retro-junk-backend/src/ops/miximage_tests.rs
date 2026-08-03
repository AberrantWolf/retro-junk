use std::collections::HashMap;

use super::*;

#[test]
fn miximage_generation_restores_archived_components_to_playable_media() {
    let temp = tempfile::tempdir().unwrap();
    let screenshot = temp.path().join("archived-screenshot.png");
    image::RgbaImage::from_pixel(64, 64, image::Rgba([20, 40, 60, 255]))
        .save(&screenshot)
        .unwrap();
    let media_dir = temp.path().join("roms-media/nes");
    let archived_assets = HashMap::from([(AssetType::Screenshot, screenshot)]);

    let output = generate_miximage_with_archived_assets(
        &archived_assets,
        &media_dir,
        "Game",
        &retro_junk_frontend::miximage_layout::MiximageLayout::default(),
    )
    .unwrap();

    assert!(media_dir.join("screenshots/Game.png").is_file());
    assert_eq!(output, media_dir.join("miximages/Game.png"));
    assert!(output.is_file());
}
