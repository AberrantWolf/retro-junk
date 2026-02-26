use std::collections::HashMap;

use image::{Rgba, RgbaImage};

use crate::MediaType;
use crate::miximage::{
    box_blur, fit_to_bounds, generate_drop_shadow, generate_miximage, sample_edge_color,
    scale_to_fit,
};
use crate::miximage_layout::{MiximageLayout, ShadowConfig};

#[test]
fn test_scale_to_fit_landscape() {
    // 1920x1080 into 1060x800 -> scale by min(1060/1920, 800/1080) = min(0.552, 0.741) = 0.552
    let (w, h) = scale_to_fit(1920, 1080, 1060, 800);
    assert!(w <= 1060);
    assert!(h <= 800);
    // Aspect ratio preserved
    let ratio_orig = 1920.0 / 1080.0;
    let ratio_new = w as f64 / h as f64;
    assert!((ratio_orig - ratio_new).abs() < 0.02);
}

#[test]
fn test_scale_to_fit_portrait() {
    // 600x900 into 620x600 -> scale by min(620/600, 600/900) = min(1.033, 0.667) = 0.667
    let (w, h) = scale_to_fit(600, 900, 620, 600);
    assert!(w <= 620);
    assert!(h <= 600);
}

#[test]
fn test_scale_to_fit_no_upscale() {
    // 400x300 into 1060x800 -> no upscale, return as-is
    let (w, h) = scale_to_fit(400, 300, 1060, 800);
    assert_eq!(w, 400);
    assert_eq!(h, 300);
}

#[test]
fn test_scale_to_fit_exact_fit() {
    let (w, h) = scale_to_fit(1060, 800, 1060, 800);
    assert_eq!(w, 1060);
    assert_eq!(h, 800);
}

#[test]
fn test_scale_to_fit_zero() {
    let (w, h) = scale_to_fit(0, 0, 100, 100);
    assert_eq!(w, 0);
    assert_eq!(h, 0);
}

#[test]
fn test_fit_to_bounds_upscales() {
    // 320x240 into 1060x800 -> scales up preserving aspect ratio
    let (w, h) = fit_to_bounds(320, 240, 1060, 800);
    assert!(w <= 1060);
    assert!(h <= 800);
    // Should have scaled up significantly
    assert!(w > 320);
    assert!(h > 240);
    // Aspect ratio preserved (4:3)
    let ratio = w as f64 / h as f64;
    assert!((ratio - 4.0 / 3.0).abs() < 0.02);
}

#[test]
fn test_fit_to_bounds_downscales() {
    // 1920x1080 into 1060x800 -> same as scale_to_fit
    let (w, h) = fit_to_bounds(1920, 1080, 1060, 800);
    assert!(w <= 1060);
    assert!(h <= 800);
}

#[test]
fn test_fit_to_bounds_exact() {
    let (w, h) = fit_to_bounds(1060, 800, 1060, 800);
    assert_eq!(w, 1060);
    assert_eq!(h, 800);
}

#[test]
fn test_sample_edge_color_red() {
    // All-red image: edge color should be red
    let img = RgbaImage::from_pixel(10, 10, Rgba([255, 0, 0, 255]));
    let color = sample_edge_color(&img);
    assert_eq!(color[0], 255);
    assert_eq!(color[1], 0);
    assert_eq!(color[2], 0);
    assert_eq!(color[3], 255);
}

#[test]
fn test_sample_edge_color_mixed() {
    // 3x3 image with known edge pixels
    let mut img = RgbaImage::new(3, 3);
    // Fill all with black
    for y in 0..3 {
        for x in 0..3 {
            img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
        }
    }
    // Set all edge pixels to white (center pixel at 1,1 is not an edge)
    // Edge pixels: (0,0), (1,0), (2,0), (0,1), (2,1), (0,2), (1,2), (2,2)
    for &(x, y) in &[
        (0, 0),
        (1, 0),
        (2, 0),
        (0, 1),
        (2, 1),
        (0, 2),
        (1, 2),
        (2, 2),
    ] {
        img.put_pixel(x, y, Rgba([200, 100, 50, 255]));
    }
    let color = sample_edge_color(&img);
    assert_eq!(color[0], 200);
    assert_eq!(color[1], 100);
    assert_eq!(color[2], 50);
}

#[test]
fn test_generate_drop_shadow_creates_image() {
    let img = RgbaImage::from_pixel(50, 50, Rgba([255, 0, 0, 255]));
    let config = ShadowConfig::default();
    let shadow = generate_drop_shadow(&img, &config);
    assert_eq!(shadow.width(), 50);
    assert_eq!(shadow.height(), 50);
    // Center pixel should have some alpha
    let center = shadow.get_pixel(25, 25);
    assert!(center[3] > 0);
    // Shadow color should be black
    assert_eq!(center[0], 0);
}

#[test]
fn test_box_blur_preserves_dimensions() {
    let img = RgbaImage::from_pixel(20, 15, Rgba([100, 100, 100, 255]));
    let blurred = box_blur(&img, 3);
    assert_eq!(blurred.width(), 20);
    assert_eq!(blurred.height(), 15);
}

#[test]
fn test_generate_miximage_no_screenshot() {
    let media = HashMap::new();
    let output = std::env::temp_dir().join("test_miximage_no_ss.png");
    let layout = MiximageLayout::default();
    let result = generate_miximage(&media, &output, &layout).unwrap();
    assert!(!result); // false = no screenshot
}

#[test]
fn test_generate_miximage_screenshot_only() {
    let dir = std::env::temp_dir().join("retro_junk_miximage_test_ss_only");
    std::fs::create_dir_all(&dir).unwrap();

    // Create a test screenshot
    let ss = RgbaImage::from_pixel(320, 240, Rgba([64, 128, 192, 255]));
    let ss_path = dir.join("screenshot.png");
    ss.save(&ss_path).unwrap();

    let mut media = HashMap::new();
    media.insert(MediaType::Screenshot, ss_path);

    let output = dir.join("miximage.png");
    let layout = MiximageLayout::default();
    let result = generate_miximage(&media, &output, &layout).unwrap();
    assert!(result);
    assert!(output.exists());

    // Verify output dimensions
    let generated = image::open(&output).unwrap();
    assert_eq!(generated.width(), 1280);
    assert_eq!(generated.height(), 960);

    // Cleanup
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_generate_miximage_all_components() {
    let dir = std::env::temp_dir().join("retro_junk_miximage_test_all");
    std::fs::create_dir_all(&dir).unwrap();

    // Create test images
    let ss = RgbaImage::from_pixel(640, 480, Rgba([32, 64, 128, 255]));
    let ss_path = dir.join("screenshot.png");
    ss.save(&ss_path).unwrap();

    let cover = RgbaImage::from_pixel(300, 400, Rgba([200, 50, 50, 255]));
    let cover_path = dir.join("cover.png");
    cover.save(&cover_path).unwrap();

    let marquee = RgbaImage::from_pixel(400, 150, Rgba([50, 200, 50, 255]));
    let marquee_path = dir.join("marquee.png");
    marquee.save(&marquee_path).unwrap();

    let phys = RgbaImage::from_pixel(200, 200, Rgba([50, 50, 200, 255]));
    let phys_path = dir.join("physicalmedia.png");
    phys.save(&phys_path).unwrap();

    let mut media = HashMap::new();
    media.insert(MediaType::Screenshot, ss_path);
    media.insert(MediaType::Cover, cover_path);
    media.insert(MediaType::Marquee, marquee_path);
    media.insert(MediaType::PhysicalMedia, phys_path);

    let output = dir.join("miximage_all.png");
    let layout = MiximageLayout::default();
    let result = generate_miximage(&media, &output, &layout).unwrap();
    assert!(result);
    assert!(output.exists());

    let generated = image::open(&output).unwrap();
    assert_eq!(generated.width(), 1280);
    assert_eq!(generated.height(), 960);

    // Cleanup
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_layout_roundtrip() {
    let layout = MiximageLayout::default();
    let yaml = serde_yml::to_string(&layout).unwrap();
    let parsed: MiximageLayout = serde_yml::from_str(&yaml).unwrap();

    assert_eq!(parsed.canvas.width, layout.canvas.width);
    assert_eq!(parsed.canvas.height, layout.canvas.height);
    assert_eq!(parsed.screenshot.max_width, layout.screenshot.max_width);
    assert_eq!(parsed.screenshot.x_offset, layout.screenshot.x_offset);
    assert_eq!(parsed.box_art.max_width, layout.box_art.max_width);
    assert_eq!(parsed.box_art.prefer_3d, layout.box_art.prefer_3d);
    assert_eq!(parsed.marquee.max_width, layout.marquee.max_width);
    assert_eq!(parsed.physical_media.gap, layout.physical_media.gap);
}
