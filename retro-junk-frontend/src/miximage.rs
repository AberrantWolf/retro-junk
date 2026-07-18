use std::collections::HashMap;
use std::path::{Path, PathBuf};

use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};

use crate::miximage_layout::{
    AnchorPosition, FrameColor, MiximageLayout, PhysMediaPosition, ShadowConfig,
};
use crate::{AssetType, FrontendError};

/// Where the box art landed on the canvas — used to position physical media
/// relative to it.
struct BoxPlacement {
    /// Bottom-right corner of the box art (x, y).
    bottom_right: (i64, i64),
    /// Bottom edge y-coordinate of the box art.
    bottom_y: i64,
}

/// Generate a composite miximage from component images.
///
/// Returns `Ok(true)` if the miximage was generated, `Ok(false)` if the
/// screenshot was missing (the only required component).
// Linear compositing pipeline (load, scale, frame, place each element); splitting obscures order.
#[allow(clippy::too_many_lines)]
pub fn generate_miximage<S: std::hash::BuildHasher>(
    media: &HashMap<AssetType, PathBuf, S>,
    output_path: &Path,
    layout: &MiximageLayout,
) -> Result<bool, FrontendError> {
    // Screenshot is required
    let screenshot_path = match media.get(&AssetType::Screenshot) {
        Some(p) if p.exists() => p,
        _ => return Ok(false),
    };

    let screenshot = image::open(screenshot_path)?.into_rgba8();

    // Scale screenshot to fill its designated area (up or down)
    let (ss_w, ss_h) = fit_to_bounds(
        screenshot.width(),
        screenshot.height(),
        layout.screenshot.max_width,
        layout.screenshot.max_height,
    );
    let upscaling = ss_w > screenshot.width() || ss_h > screenshot.height();
    let filter = if upscaling {
        FilterType::Nearest // preserve pixel art crispness
    } else {
        FilterType::Lanczos3 // sharpest anti-aliased downscale
    };
    let screenshot = imageops::resize(&screenshot, ss_w, ss_h, filter);

    // Determine frame color
    let frame_color = match &layout.screenshot.frame_color {
        FrameColor::Auto => sample_edge_color(&screenshot),
        FrameColor::Fixed(hex) => parse_hex_color(hex),
    };

    // Create canvas
    let canvas_w = layout.canvas.width;
    let canvas_h = layout.canvas.height;
    let mut canvas = RgbaImage::new(canvas_w, canvas_h);

    // Draw screenshot with frame and rounded corners
    let frame_w = layout.screenshot.frame_width;
    let outer_w = ss_w + frame_w * 2;
    let outer_h = ss_h + frame_w * 2;
    let mut framed = RgbaImage::from_pixel(outer_w, outer_h, frame_color);
    imageops::overlay(
        &mut framed,
        &screenshot,
        i64::from(frame_w),
        i64::from(frame_w),
    );

    if layout.screenshot.corner_radius > 0 {
        apply_rounded_corners(&mut framed, layout.screenshot.corner_radius);
    }

    // Center with x_offset
    let ss_x =
        (i64::from(canvas_w) - i64::from(outer_w)) / 2 + i64::from(layout.screenshot.x_offset);
    let ss_y = (i64::from(canvas_h) - i64::from(outer_h)) / 2;
    imageops::overlay(&mut canvas, &framed, ss_x, ss_y);

    // Track box art placement for physical media positioning
    let mut box_placement: Option<BoxPlacement> = None;

    // Box art (prefer 3D if configured and available)
    let box_path = if layout.box_art.prefer_3d {
        media
            .get(&AssetType::Cover3D)
            .filter(|p| p.exists())
            .or_else(|| media.get(&AssetType::Cover).filter(|p| p.exists()))
    } else {
        media.get(&AssetType::Cover).filter(|p| p.exists())
    };

    if let Some(box_path) = box_path {
        let box_img = image::open(box_path)?.into_rgba8();
        let (bw, bh) = scale_to_fit(
            box_img.width(),
            box_img.height(),
            layout.box_art.max_width,
            layout.box_art.max_height,
        );
        let box_img = imageops::resize(&box_img, bw, bh, FilterType::Lanczos3);

        let (bx, by) = anchor_position(
            layout.box_art.position,
            bw,
            bh,
            canvas_w,
            canvas_h,
            &layout.box_art.shadow,
        );

        if layout.box_art.shadow.enabled {
            let shadow = generate_drop_shadow(&box_img, &layout.box_art.shadow);
            let shadow_offset = i64::from(layout.box_art.shadow.offset);
            imageops::overlay(&mut canvas, &shadow, bx + shadow_offset, by + shadow_offset);
        }

        imageops::overlay(&mut canvas, &box_img, bx, by);
        box_placement = Some(BoxPlacement {
            bottom_right: (bx + i64::from(bw), by + i64::from(bh)),
            bottom_y: by + i64::from(bh),
        });
    }

    // Marquee / logo
    if let Some(marquee_path) = media.get(&AssetType::Marquee).filter(|p| p.exists()) {
        let marquee_img = image::open(marquee_path)?.into_rgba8();
        let (mw, mh) = scale_to_fit(
            marquee_img.width(),
            marquee_img.height(),
            layout.marquee.max_width,
            layout.marquee.max_height,
        );
        let marquee_img = imageops::resize(&marquee_img, mw, mh, FilterType::Lanczos3);

        let (mx, my) = anchor_position(
            layout.marquee.position,
            mw,
            mh,
            canvas_w,
            canvas_h,
            &layout.marquee.shadow,
        );

        if layout.marquee.shadow.enabled {
            let shadow = generate_drop_shadow(&marquee_img, &layout.marquee.shadow);
            let shadow_offset = i64::from(layout.marquee.shadow.offset);
            imageops::overlay(&mut canvas, &shadow, mx + shadow_offset, my + shadow_offset);
        }

        imageops::overlay(&mut canvas, &marquee_img, mx, my);
    }

    // Physical media
    if let Some(phys_path) = media.get(&AssetType::PhysicalMedia).filter(|p| p.exists()) {
        let phys_img = image::open(phys_path)?.into_rgba8();
        let (pw, ph) = scale_to_fit(
            phys_img.width(),
            phys_img.height(),
            layout.physical_media.max_width,
            layout.physical_media.max_height,
        );
        let phys_img = imageops::resize(&phys_img, pw, ph, FilterType::Lanczos3);

        let (px, py) = physical_media_position(
            layout.physical_media.position,
            &ElementGeometry {
                elem_w: pw,
                elem_h: ph,
                canvas_w,
                canvas_h,
            },
            layout.physical_media.gap,
            box_placement.as_ref(),
            &layout.physical_media.shadow,
        );

        if layout.physical_media.shadow.enabled {
            let shadow = generate_drop_shadow(&phys_img, &layout.physical_media.shadow);
            let shadow_offset = i64::from(layout.physical_media.shadow.offset);
            imageops::overlay(&mut canvas, &shadow, px + shadow_offset, py + shadow_offset);
        }

        imageops::overlay(&mut canvas, &phys_img, px, py);
    }

    // Save
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    canvas.save(output_path)?;

    Ok(true)
}

/// Scale dimensions to fit within max bounds, preserving aspect ratio.
/// Never upscales — use `fit_to_bounds` when upscaling is desired.
pub(crate) fn scale_to_fit(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (0, 0);
    }

    if src_w <= max_w && src_h <= max_h {
        return (src_w, src_h);
    }

    let scale_w = f64::from(max_w) / f64::from(src_w);
    let scale_h = f64::from(max_h) / f64::from(src_h);
    let scale = scale_w.min(scale_h);

    let new_w = (f64::from(src_w) * scale).round() as u32;
    let new_h = (f64::from(src_h) * scale).round() as u32;

    (new_w.max(1), new_h.max(1))
}

/// Scale dimensions to fit within max bounds, preserving aspect ratio.
/// Scales both up and down — used for the screenshot which should always fill
/// its designated area.
pub(crate) fn fit_to_bounds(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (0, 0);
    }

    let scale_w = f64::from(max_w) / f64::from(src_w);
    let scale_h = f64::from(max_h) / f64::from(src_h);
    let scale = scale_w.min(scale_h);

    let new_w = (f64::from(src_w) * scale).round() as u32;
    let new_h = (f64::from(src_h) * scale).round() as u32;

    (new_w.max(1), new_h.max(1))
}

/// Sample the average color from edge pixels of an image.
pub(crate) fn sample_edge_color(img: &RgbaImage) -> Rgba<u8> {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Rgba([0, 0, 0, 255]);
    }

    let mut r_sum: u64 = 0;
    let mut g_sum: u64 = 0;
    let mut b_sum: u64 = 0;
    let mut count: u64 = 0;

    // Top and bottom rows
    for x in 0..w {
        let p = img.get_pixel(x, 0);
        r_sum += u64::from(p[0]);
        g_sum += u64::from(p[1]);
        b_sum += u64::from(p[2]);
        count += 1;

        if h > 1 {
            let p = img.get_pixel(x, h - 1);
            r_sum += u64::from(p[0]);
            g_sum += u64::from(p[1]);
            b_sum += u64::from(p[2]);
            count += 1;
        }
    }

    // Left and right columns (excluding corners already counted)
    for y in 1..h.saturating_sub(1) {
        let p = img.get_pixel(0, y);
        r_sum += u64::from(p[0]);
        g_sum += u64::from(p[1]);
        b_sum += u64::from(p[2]);
        count += 1;

        if w > 1 {
            let p = img.get_pixel(w - 1, y);
            r_sum += u64::from(p[0]);
            g_sum += u64::from(p[1]);
            b_sum += u64::from(p[2]);
            count += 1;
        }
    }

    if count == 0 {
        return Rgba([0, 0, 0, 255]);
    }

    Rgba([
        (r_sum / count) as u8,
        (g_sum / count) as u8,
        (b_sum / count) as u8,
        255,
    ])
}

/// Generate a drop shadow for an image based on its alpha channel.
pub(crate) fn generate_drop_shadow(img: &RgbaImage, config: &ShadowConfig) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    let mut shadow = RgbaImage::new(w, h);

    let opacity = (config.opacity * 255.0).clamp(0.0, 255.0) as u8;

    // Create shadow from alpha channel
    for y in 0..h {
        for x in 0..w {
            let src_alpha = img.get_pixel(x, y)[3];
            if src_alpha > 0 {
                let shadow_alpha = ((u16::from(src_alpha) * u16::from(opacity)) / 255) as u8;
                shadow.put_pixel(x, y, Rgba([0, 0, 0, shadow_alpha]));
            }
        }
    }

    // Apply box blur passes
    for _ in 0..config.blur_passes {
        shadow = box_blur(&shadow, 3);
    }

    shadow
}

/// Simple box blur with the given radius.
pub(crate) fn box_blur(img: &RgbaImage, radius: u32) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return img.clone();
    }

    let r = radius as i32;
    let kernel_size = (2 * r + 1) as u32;

    // Horizontal pass
    let mut horiz = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut ra: u32 = 0;
            let mut ga: u32 = 0;
            let mut ba: u32 = 0;
            let mut aa: u32 = 0;
            let mut count: u32 = 0;

            for dx in -r..=r {
                let sx = x as i32 + dx;
                if sx >= 0 && sx < w as i32 {
                    let p = img.get_pixel(sx as u32, y);
                    ra += u32::from(p[0]);
                    ga += u32::from(p[1]);
                    ba += u32::from(p[2]);
                    aa += u32::from(p[3]);
                    count += 1;
                }
            }

            if let Some(pixel) = averaged_pixel([ra, ga, ba, aa], count) {
                horiz.put_pixel(x, y, pixel);
            }
        }
    }

    // Vertical pass
    let mut result = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut ra: u32 = 0;
            let mut ga: u32 = 0;
            let mut ba: u32 = 0;
            let mut aa: u32 = 0;
            let mut count: u32 = 0;

            for dy in -r..=r {
                let sy = y as i32 + dy;
                if sy >= 0 && sy < h as i32 {
                    let p = horiz.get_pixel(x, sy as u32);
                    ra += u32::from(p[0]);
                    ga += u32::from(p[1]);
                    ba += u32::from(p[2]);
                    aa += u32::from(p[3]);
                    count += 1;
                }
            }

            if let Some(pixel) = averaged_pixel([ra, ga, ba, aa], count) {
                result.put_pixel(x, y, pixel);
            }
        }
    }

    let _ = kernel_size; // used conceptually; actual kernel is 2*r+1
    result
}

/// Average accumulated RGBA channel sums over `count` samples.
/// Returns `None` when no samples were taken (`count == 0`).
fn averaged_pixel(sums: [u32; 4], count: u32) -> Option<Rgba<u8>> {
    let avg = |sum: u32| sum.checked_div(count).map(|v| v as u8);
    Some(Rgba([
        avg(sums[0])?,
        avg(sums[1])?,
        avg(sums[2])?,
        avg(sums[3])?,
    ]))
}

/// Apply rounded corners by zeroing alpha outside the rounded rectangle.
pub(crate) fn apply_rounded_corners(img: &mut RgbaImage, radius: u32) {
    let (w, h) = (img.width(), img.height());
    let r = radius.min(w / 2).min(h / 2);

    if r == 0 {
        return;
    }

    let r_sq = i64::from(r * r);

    for y in 0..h {
        for x in 0..w {
            let in_corner = corner_distance_sq(x, y, w, h, r);
            if let Some(dist_sq) = in_corner {
                if dist_sq > r_sq {
                    img.put_pixel(x, y, Rgba([0, 0, 0, 0]));
                } else if dist_sq > (i64::from(r) - 2) * (i64::from(r) - 2) {
                    // Anti-alias the edge
                    let p = img.get_pixel(x, y);
                    let dist = (dist_sq as f64).sqrt();
                    let alpha_factor = (f64::from(r) - dist).clamp(0.0, 1.0);
                    let new_alpha = (f64::from(p[3]) * alpha_factor) as u8;
                    img.put_pixel(x, y, Rgba([p[0], p[1], p[2], new_alpha]));
                }
            }
        }
    }
}

/// If (x,y) is in a corner region, return squared distance from the corner's circle center.
/// Returns None if the pixel is not in any corner region.
fn corner_distance_sq(px: u32, py: u32, width: u32, height: u32, radius: u32) -> Option<i64> {
    let (cx, cy) = if px < radius && py < radius {
        // Top-left
        (radius, radius)
    } else if px >= width - radius && py < radius {
        // Top-right
        (width - radius - 1, radius)
    } else if px < radius && py >= height - radius {
        // Bottom-left
        (radius, height - radius - 1)
    } else if px >= width - radius && py >= height - radius {
        // Bottom-right
        (width - radius - 1, height - radius - 1)
    } else {
        return None;
    };

    let dx = i64::from(px) - i64::from(cx);
    let dy = i64::from(py) - i64::from(cy);
    Some(dx * dx + dy * dy)
}

/// Compute position for an anchored element (box art, marquee).
fn anchor_position(
    position: AnchorPosition,
    elem_w: u32,
    elem_h: u32,
    canvas_w: u32,
    canvas_h: u32,
    shadow: &ShadowConfig,
) -> (i64, i64) {
    let margin = if shadow.enabled {
        i64::from(shadow.offset)
    } else {
        8
    };

    match position {
        AnchorPosition::TopLeft => (margin, margin),
        AnchorPosition::TopRight => (i64::from(canvas_w) - i64::from(elem_w) - margin, margin),
        AnchorPosition::BottomLeft => (margin, i64::from(canvas_h) - i64::from(elem_h) - margin),
        AnchorPosition::BottomRight => (
            i64::from(canvas_w) - i64::from(elem_w) - margin,
            i64::from(canvas_h) - i64::from(elem_h) - margin,
        ),
    }
}

/// Pixel dimensions involved in placing an element on the canvas.
struct ElementGeometry {
    elem_w: u32,
    elem_h: u32,
    canvas_w: u32,
    canvas_h: u32,
}

/// Compute position for physical media relative to the box art.
fn physical_media_position(
    position: PhysMediaPosition,
    geometry: &ElementGeometry,
    gap: u32,
    box_placement: Option<&BoxPlacement>,
    shadow: &ShadowConfig,
) -> (i64, i64) {
    let margin = if shadow.enabled {
        i64::from(shadow.offset)
    } else {
        8
    };
    let elem_w = i64::from(geometry.elem_w);
    let elem_h = i64::from(geometry.elem_h);

    match (position, box_placement) {
        (PhysMediaPosition::RightOfBox, Some(placement)) => {
            let x = placement.bottom_right.0 + i64::from(gap);
            let y = placement.bottom_y - elem_h - margin;
            (x, y)
        }
        (PhysMediaPosition::LeftOfBox, Some(placement)) => {
            // bottom_right.0 is the right edge; for LeftOfBox we'd need the left edge
            // For now, just place it to the left of the box's right edge
            let x = placement.bottom_right.0 - elem_w - i64::from(gap);
            let y = placement.bottom_y - elem_h - margin;
            (x, y)
        }
        // No box art present — place at bottom-center
        _ => {
            let x = (i64::from(geometry.canvas_w) - elem_w) / 2;
            let y = i64::from(geometry.canvas_h) - elem_h - margin;
            (x, y)
        }
    }
}

/// Parse a hex color string like "#RRGGBB" into an Rgba pixel.
fn parse_hex_color(hex: &str) -> Rgba<u8> {
    let hex = hex.trim_start_matches('#');
    if hex.len() >= 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
        Rgba([r, g, b, 255])
    } else {
        Rgba([0, 0, 0, 255])
    }
}

#[cfg(test)]
#[path = "tests/miximage_tests.rs"]
mod tests;
