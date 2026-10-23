//! Original typographic Discord badges, shared by the preview and maintainer asset export.
use clipcraft_export::{ExportError, SequenceText, TextRaster};
use clipcraft_time::Tick;
use image::{Rgba, RgbaImage};

pub const ASSET_KEYS: [&str; 4] = [
    "clipcraft",
    "clipcraft_cs2",
    "clipcraft_dota2",
    "clipcraft_league",
];

/// Fixed keys and a bounded canvas prevent input-driven paths or unbounded allocations.
pub fn badge(key: &str, edge: u32) -> Result<RgbaImage, ExportError> {
    if !(64..=1024).contains(&edge) || !ASSET_KEYS.contains(&key) {
        return Err(ExportError::Unsupported(
            "unknown Discord asset or invalid badge size",
        ));
    }
    let mut image = RgbaImage::from_pixel(edge, edge, Rgba([35, 39, 32, 255]));
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let border = edge / 18;
        if x < border || y < border || x >= edge - border || y >= edge - border {
            *pixel = Rgba([53, 62, 42, 255]);
        }
        // Original offset diamond motif; no game or third-party product logos.
        let distance = x.abs_diff(edge / 2) + y.abs_diff(edge / 3);
        if distance < edge / 7 && distance > edge / 9 {
            *pixel = Rgba([194, 248, 79, 255]);
        }
    }
    let label = match key {
        "clipcraft_cs2" => "CS2",
        "clipcraft_dota2" => "DOTA 2",
        "clipcraft_league" => "LoL",
        _ => "CC",
    };
    let mut text = SequenceText::new(Tick::ZERO, Tick::from_units(1, 1), label);
    text.size = 240;
    text.y = 620;
    text.color = [194, 248, 79];
    if let Some(raster) = TextRaster::render(&text, edge, edge)? {
        raster.draw(&mut image, &text, 1.0);
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn badges_are_bounded_original_and_opaque() {
        for key in ASSET_KEYS {
            let image = badge(key, 128).unwrap();
            assert_eq!(image.dimensions(), (128, 128));
            assert!(image.pixels().all(|p| p.0[3] == 255));
            assert!(image.pixels().any(|p| p.0 == [194, 248, 79, 255]));
        }
        for edge in [0, 63, 1025, u32::MAX] {
            assert!(badge("clipcraft", edge).is_err());
        }
        assert!(badge("../../private", 128).is_err());
    }
}
