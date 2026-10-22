//! Shared, bounded text pixels for the editor preview and rendered MP4.

use crate::{ExportError, SequenceText};

const MAX_RASTER_PIXELS: usize = 2 * 1024 * 1024;

/// A rendered text layer, including its shadow, in straight-alpha RGBA.
/// Timing and placement are deliberately separate so moving/fading text reuses its pixels.
pub struct TextRaster {
    text: String,
    size: u16,
    color: [u8; 3],
    effect: crate::Effect,
    canvas: [u32; 2],
    dimensions: [usize; 2],
    rgba: Vec<u8>,
}

impl TextRaster {
    pub fn render(
        text: &SequenceText,
        width: u32,
        height: u32,
    ) -> Result<Option<Self>, ExportError> {
        if !text.is_valid() || !(2..=1280).contains(&width) || !(2..=720).contains(&height) {
            return Err(ExportError::InvalidSequence);
        }
        let line = text.text.trim();
        if line.is_empty() {
            return Ok(None);
        }
        let style = filmcraft_text::TextStyle {
            size: (height as f32 * f32::from(text.size) / 1000.0).max(8.0),
            style: "SemiBold".into(),
            ..Default::default()
        };
        let paragraph = filmcraft_text::ParagraphStyle {
            align: filmcraft_text::Align::Center,
            width: Some(width as f32 * 0.9),
            // Area text stops at the canvas height instead of allocating an arbitrarily tall mask.
            height: Some(height as f32),
            ..Default::default()
        };
        let layout = filmcraft_text::layout(line, &style, &paragraph);
        let transform = filmcraft_text::render::at(0.0, 0.0);
        let bounds = filmcraft_text::render::device_bounds(&layout, &transform);
        if bounds.iter().any(|v| !v.is_finite()) {
            return Err(ExportError::InvalidSequence);
        }
        let w =
            ((bounds[2].ceil() - bounds[0].floor() + 2.0).max(1.0) as usize).min(width as usize);
        let h =
            ((bounds[3].ceil() - bounds[1].floor() + 2.0).max(1.0) as usize).min(height as usize);
        let shadow = (h as f32 / 12.0).clamp(1.0, 4.0) as usize;
        let dimensions = [w.saturating_add(shadow), h.saturating_add(shadow)];
        let pixels = dimensions[0]
            .checked_mul(dimensions[1])
            .filter(|&n| n <= MAX_RASTER_PIXELS)
            .ok_or(ExportError::InvalidSequence)?;
        // Draw directly into a bounded mask: a long unbroken word can overflow the paragraph
        // width, so rasterising its full device bounds would otherwise defeat the canvas limit.
        let mut mask = filmcraft_text::Mask::new(w, h);
        filmcraft_text::render::draw(
            &layout,
            &transform,
            &mut mask,
            (bounds[0].floor() as i32 - 1, bounds[1].floor() as i32 - 1),
        );
        let recolour = |rgb: [u8; 3]| {
            let mut pixel = [rgb[0], rgb[1], rgb[2], 255];
            text.effect.apply_rgba(&mut pixel);
            [pixel[0], pixel[1], pixel[2]]
        };
        let mut rgba = vec![0_u8; pixels * 4];
        for (offset, color, strength) in [
            (shadow, recolour([0, 0, 0]), 0.65_f32),
            (0, recolour(text.color), 1.0),
        ] {
            for row in 0..h {
                for col in 0..w {
                    let alpha = (mask.a[row * w + col] * strength).clamp(0.0, 1.0);
                    let index = ((row + offset) * dimensions[0] + col + offset) * 4;
                    let pixel = &mut rgba[index..index + 4];
                    let previous = f32::from(pixel[3]) / 255.0;
                    let combined = alpha + previous * (1.0 - alpha);
                    if combined > 0.0 {
                        for (slot, ink) in pixel.iter_mut().take(3).zip(color) {
                            *slot = ((f32::from(ink) * alpha
                                + f32::from(*slot) * previous * (1.0 - alpha))
                                / combined)
                                .round()
                                .clamp(0.0, 255.0) as u8;
                        }
                    }
                    pixel[3] = (combined * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        // Keep interaction geometry tight to the visible letters and shadow, rather than the
        // paragraph's wrap box. Positioning near an edge then matches dragging in the preview.
        let mut edges = [dimensions[0], dimensions[1], 0, 0];
        for (i, _) in rgba.chunks_exact(4).enumerate().filter(|(_, p)| p[3] != 0) {
            let x = i % dimensions[0];
            let y = i / dimensions[0];
            edges = [
                edges[0].min(x),
                edges[1].min(y),
                edges[2].max(x + 1),
                edges[3].max(y + 1),
            ];
        }
        if edges[0] >= edges[2] || edges[1] >= edges[3] {
            return Ok(None);
        }
        let tight = [edges[2] - edges[0], edges[3] - edges[1]];
        let mut cropped = Vec::with_capacity(tight[0] * tight[1] * 4);
        for row in edges[1]..edges[3] {
            let first = (row * dimensions[0] + edges[0]) * 4;
            cropped.extend_from_slice(&rgba[first..first + tight[0] * 4]);
        }
        Ok(Some(Self {
            text: text.text.clone(),
            size: text.size,
            color: text.color,
            effect: text.effect,
            canvas: [width, height],
            dimensions: tight,
            rgba: cropped,
        }))
    }
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
    pub fn size(&self) -> [usize; 2] {
        self.dimensions
    }
    pub fn bytes(&self) -> usize {
        self.rgba.len()
    }
    pub fn matches(&self, text: &SequenceText, width: u32, height: u32) -> bool {
        self.canvas == [width, height]
            && self.text == text.text
            && self.size == text.size
            && self.color == text.color
            && self.effect == text.effect
    }
    pub fn position(&self, x: u16, y: u16) -> [i64; 2] {
        text_position(self.dimensions, self.canvas, x, y)
    }
    pub fn draw(&self, canvas: &mut image::RgbaImage, text: &SequenceText, opacity: f32) {
        let [x, y] = self.position(text.x, text.y);
        let opacity = if opacity.is_finite() {
            opacity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        for row in 0..self.dimensions[1] {
            let Ok(py) = u32::try_from(y + row as i64) else {
                continue;
            };
            if py >= canvas.height() {
                continue;
            }
            for col in 0..self.dimensions[0] {
                let Ok(px) = u32::try_from(x + col as i64) else {
                    continue;
                };
                if px >= canvas.width() {
                    continue;
                }
                let index = (row * self.dimensions[0] + col) * 4;
                let source = &self.rgba[index..index + 4];
                let amount = f32::from(source[3]) / 255.0 * opacity;
                let pixel = canvas.get_pixel_mut(px, py);
                for (channel, &ink) in pixel.0.iter_mut().take(3).zip(source) {
                    *channel = (f32::from(*channel) * (1.0 - amount) + f32::from(ink) * amount)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// The same pixel placement for export and the scaled preview's interaction rectangle.
pub fn text_position(size: [usize; 2], canvas: [u32; 2], x: u16, y: u16) -> [i64; 2] {
    let axis = |n: usize, extent: u32, position: u16| {
        let n = i64::try_from(n).unwrap_or(i64::MAX);
        (i64::from(extent) * i64::from(position.min(1000)) / 1000 - n / 2)
            .clamp(0, (i64::from(extent) - n).max(0))
    };
    [axis(size[0], canvas[0], x), axis(size[1], canvas[1], y)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipcraft_time::Tick;

    #[test]
    fn shared_pixels_and_geometry_match_preview_compositing_and_stay_bounded() {
        let mut text = SequenceText::new(Tick::ZERO, Tick::from_units(1, 1), "Shared Inter text");
        text.x = 900;
        let raster = TextRaster::render(&text, 640, 360).unwrap().unwrap();
        assert!(raster.rgba().chunks_exact(4).any(|p| p[3] > 0));
        let mut actual = image::RgbaImage::from_pixel(640, 360, image::Rgba([30, 40, 50, 255]));
        raster.draw(&mut actual, &text, 0.5);
        let [x, y] = raster.position(text.x, text.y);
        for (i, source) in raster.rgba().chunks_exact(4).enumerate() {
            let px = x + (i % raster.size()[0]) as i64;
            let py = y + (i / raster.size()[0]) as i64;
            if px < 640 && py < 360 {
                let pixel = actual.get_pixel(px as u32, py as u32);
                for c in 0..3 {
                    let amount = f32::from(source[3]) / 255.0 * 0.5;
                    let expected = ([30.0, 40.0, 50.0][c] * (1.0 - amount)
                        + f32::from(source[c]) * amount)
                        .round() as u8;
                    assert_eq!(pixel[c], expected);
                }
            }
        }
        text.text = "W".repeat(120);
        text.size = 250;
        let largest = TextRaster::render(&text, 1280, 720).unwrap().unwrap();
        assert!(largest.bytes() <= MAX_RASTER_PIXELS * 4);
        assert!(TextRaster::render(&text, u32::MAX, u32::MAX).is_err());
        text.text.push('W');
        assert!(TextRaster::render(&text, 1280, 720).is_err());
    }

    #[test]
    fn audio_fade_gain_is_shared_without_changing_the_edit_volume() {
        use crate::{Fade, SegmentLook};
        let look = SegmentLook {
            fade_in: Fade::Black,
            fade_out: Fade::White,
            ..Default::default()
        };
        let duration = Tick::from_units(2, 1);
        assert_eq!(look.audio_gain_at(Tick::ZERO, duration, 50), 0.0);
        assert_eq!(
            look.audio_gain_at(Tick::from_units(1, 4), duration, 50),
            0.25
        );
        assert_eq!(
            look.audio_gain_at(Tick::from_units(1, 2), duration, 50),
            0.5
        );
        assert_eq!(
            look.audio_gain_at(Tick::from_units(7, 4), duration, 50),
            0.25
        );
        assert_eq!(look.audio_gain_at(duration, duration, 50), 0.0);
        assert_eq!(
            look.audio_gain_at(Tick::from_units(1, 1), duration, 50),
            0.5
        );
    }
}
