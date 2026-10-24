//! ClipCraft CUT identity: one geometry for scalable artwork and native raster icons.
use clipcraft_export::ExportError;
use image::{Rgba, RgbaImage};

pub const LIME: [u8; 3] = [194, 248, 79];
pub const CHARCOAL: [u8; 3] = [23, 28, 30];
pub const PAPER: [u8; 3] = [248, 247, 244];

// A broad C with a play-shaped opening. Coordinates occupy a 400 × 340 canvas.
const OUTLINE: [(f32, f32); 18] = [
    (150., 10.),
    (380., 10.),
    (230., 158.),
    (138., 108.),
    (138., 240.),
    (230., 180.),
    (390., 254.),
    (317., 321.),
    (298., 330.),
    (108., 330.),
    (87., 323.),
    (30., 288.),
    (13., 270.),
    (10., 250.),
    (10., 139.),
    (14., 119.),
    (28., 102.),
    (125., 18.),
];

/// A standalone, font-free SVG. The exact outline is shared with the rasterizer.
pub fn svg(ink: [u8; 3]) -> String {
    let points = OUTLINE
        .iter()
        .map(|(x, y)| format!("{x},{y}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 400 340\" role=\"img\" aria-label=\"ClipCraft CUT logo\"><polygon fill=\"#{:02x}{:02x}{:02x}\" points=\"{points}\"/></svg>\n",
        ink[0], ink[1], ink[2]
    )
}

/// Transparent square with safe padding for window icons and circular Discord crops.
/// Four subpixel samples smooth the silhouette; all work and allocations are bounded.
pub fn icon(edge: u32, ink: [u8; 3]) -> Result<RgbaImage, ExportError> {
    if !(16..=1024).contains(&edge) {
        return Err(ExportError::Unsupported(
            "brand icon size must be 16–1024 pixels",
        ));
    }
    Ok(render_icon(edge, ink))
}

/// Fixed-size native icon; no input-dependent failure or per-frame work.
pub fn window_icon() -> RgbaImage {
    render_icon(256, LIME)
}

fn render_icon(edge: u32, ink: [u8; 3]) -> RgbaImage {
    let mut image = RgbaImage::new(edge, edge);
    let scale = edge as f32 * 0.8 / 400.;
    let left = edge as f32 * 0.1;
    let top = (edge as f32 - 340. * scale) / 2.;
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let mut coverage = 0_u8;
        for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
            let px = (x as f32 + dx - left) / scale;
            let py = (y as f32 + dy - top) / scale;
            let mut inside = false;
            let mut previous = OUTLINE[OUTLINE.len() - 1];
            for current in OUTLINE {
                if (current.1 > py) != (previous.1 > py)
                    && px
                        < (previous.0 - current.0) * (py - current.1) / (previous.1 - current.1)
                            + current.0
                {
                    inside = !inside;
                }
                previous = current;
            }
            coverage += u8::from(inside);
        }
        *pixel = Rgba([
            ink[0],
            ink[1],
            ink[2],
            (u16::from(coverage) * 255 / 4) as u8,
        ]);
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_icons_keep_the_c_stem_and_play_opening() {
        for size in [16, 24, 32, 64, 256] {
            let image = icon(size, LIME).unwrap();
            assert_eq!(image.get_pixel(size / 4, size / 2).0, [194, 248, 79, 255]);
            assert_eq!(image.get_pixel(size * 5 / 8, size / 2).0[3], 0);
            assert_eq!(image.get_pixel(size - 1, size / 2).0[3], 0);
            assert_eq!(image.get_pixel(0, 0).0[3], 0);
            assert!(image.pixels().any(|p| p.0[3] > 0 && p.0[3] < 255));
        }
    }

    #[test]
    fn hostile_dimensions_are_rejected_before_allocation() {
        for edge in [0, 15, 1025, u32::MAX] {
            assert!(icon(edge, LIME).is_err());
        }
    }

    #[test]
    fn committed_vector_masters_match_the_runtime_silhouette() {
        for (actual, colour) in [
            (include_str!("../../../assets/brand/mark-lime.svg"), LIME),
            (
                include_str!("../../../assets/brand/mark-black.svg"),
                CHARCOAL,
            ),
            (include_str!("../../../assets/brand/mark-white.svg"), PAPER),
        ] {
            assert_eq!(actual, svg(colour));
        }
    }
}
