//! Original CUT product badge, shared by offline preview and maintainer asset export.
use clipcraft_export::ExportError;
use image::{Rgba, RgbaImage};

pub const ASSET_KEYS: [&str; 1] = ["clipcraft"];

/// Fixed keys and a bounded canvas prevent input-driven paths or unbounded allocations.
pub fn badge(key: &str, edge: u32) -> Result<RgbaImage, ExportError> {
    if !(64..=1024).contains(&edge) || !ASSET_KEYS.contains(&key) {
        return Err(ExportError::Unsupported(
            "unknown Discord asset or invalid badge size",
        ));
    }
    let mut image = RgbaImage::from_pixel(edge, edge, Rgba([23, 28, 30, 255]));
    image::imageops::overlay(
        &mut image,
        &crate::brand::icon(edge, crate::brand::LIME)?,
        0,
        0,
    );
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
