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
    look: Look,
    canvas: [u32; 2],
    dimensions: [usize; 2],
    rgba: Vec<u8>,
}

/// What of a text layer changes its pixels (everything but timing, place and opacity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Look {
    font: crate::TextFont,
    weight: crate::TextWeight,
    italic: bool,
    caps: bool,
    underline: bool,
    tracking: i16,
    shadow: bool,
    outline: bool,
    backdrop: bool,
}
impl Look {
    fn of(text: &SequenceText) -> Self {
        Self {
            font: text.font,
            weight: text.weight,
            italic: text.italic,
            caps: text.caps,
            underline: text.underline,
            tracking: text.tracking,
            shadow: text.shadow,
            outline: text.outline,
            backdrop: text.backdrop,
        }
    }
    /// The style name for the text engine: the weight, with italic where the face has it.
    fn style_name(self) -> String {
        match (self.weight, self.italic) {
            (weight, false) => weight.style().to_owned(),
            (crate::TextWeight::Regular, true) => "Italic".to_owned(),
            (weight, true) => format!("{} Italic", weight.style()),
        }
    }
}

impl TextRaster {
    pub fn render(
        text: &SequenceText,
        width: u32,
        height: u32,
    ) -> Result<Option<Self>, ExportError> {
        if !text.is_valid() {
            return Err(ExportError::InvalidSequence);
        }
        if !(2..=crate::sequence_quality::MAX_OUTPUT_EDGE).contains(&width)
            || !(2..=crate::sequence_quality::MAX_OUTPUT_EDGE).contains(&height)
            || u64::from(width) * u64::from(height) > crate::sequence_quality::MAX_OUTPUT_PIXELS
        {
            return Err(ExportError::Unsupported(
                "text needs a canvas within the 4K output limits",
            ));
        }
        let line = text.text.trim();
        if line.is_empty() {
            return Ok(None);
        }
        let look = Look::of(text);
        let pixel_size = (height as f32 * f32::from(text.size) / 1000.0).max(8.0);
        let style = filmcraft_text::TextStyle {
            family: text.font.family().into(),
            style: look.style_name(),
            size: pixel_size,
            tracking: f32::from(text.tracking),
            caps: if text.caps {
                filmcraft_text::Caps::All
            } else {
                filmcraft_text::Caps::Normal
            },
            underline: text.underline,
            ..Default::default()
        };
        let paragraph = filmcraft_text::ParagraphStyle {
            align: filmcraft_text::Align::Center,
            width: Some(width as f32 * 0.9),
            // Measure content height; a full-canvas paragraph would allocate empty 4K rows.
            // The mask below still clips long text to the canvas and its fixed pixel budget.
            height: None,
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
        let shadow = if look.shadow {
            (height as f32 / 12.0).clamp(1.0, 4.0) as usize
        } else {
            0
        };
        // The outline grows the picture around the letters; the box adds room around them too.
        let ring = if look.outline {
            (pixel_size / 14.0).round().clamp(1.0, 4.0) as usize
        } else {
            0
        };
        let pad = if look.backdrop {
            (pixel_size * 0.3).round().clamp(2.0, 64.0) as usize
        } else {
            0
        };
        let inset = ring + pad;
        let dimensions = [
            w.saturating_add(shadow).saturating_add(inset * 2),
            h.saturating_add(shadow).saturating_add(inset * 2),
        ];
        let pixels = dimensions[0]
            .checked_mul(dimensions[1])
            .filter(|&n| n <= MAX_RASTER_PIXELS)
            .ok_or(ExportError::Unsupported(
                "text exceeds its raster budget; shorten it or reduce its size",
            ))?;
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
        // One source-over blend of `ink` at `alpha` onto the pixel at (`x`, `y`).
        let mut put = |x: usize, y: usize, ink: [u8; 3], alpha: f32| {
            if x >= dimensions[0] || y >= dimensions[1] || alpha <= 0.0 {
                return;
            }
            let alpha = alpha.clamp(0.0, 1.0);
            let index = (y * dimensions[0] + x) * 4;
            let pixel = &mut rgba[index..index + 4];
            let previous = f32::from(pixel[3]) / 255.0;
            let combined = alpha + previous * (1.0 - alpha);
            if combined > 0.0 {
                for (slot, ink) in pixel.iter_mut().take(3).zip(ink) {
                    *slot = ((f32::from(ink) * alpha + f32::from(*slot) * previous * (1.0 - alpha))
                        / combined)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
            pixel[3] = (combined * 255.0).round().clamp(0.0, 255.0) as u8;
        };
        let dark = recolour([0, 0, 0]);
        // A box behind everything, with the outline's room inside it.
        if look.backdrop {
            for y in ring..ring + h + pad * 2 {
                for x in ring..ring + w + pad * 2 {
                    put(x, y, dark, 0.55);
                }
            }
        }
        // The shadow, down and to the right of the letters.
        if look.shadow {
            for row in 0..h {
                for col in 0..w {
                    put(
                        col + inset + shadow,
                        row + inset + shadow,
                        dark,
                        mask.a[row * w + col] * 0.65,
                    );
                }
            }
        }
        // The outline: the letters grown by `ring` pixels (a square growth, one direction at a
        // time), drawn dark under the letters.
        if ring > 0 {
            let (grown_w, grown_h) = (w + ring * 2, h + ring * 2);
            let mut across = vec![0.0_f32; grown_w * h];
            for row in 0..h {
                for col in 0..grown_w {
                    let mut best = 0.0_f32;
                    for source in col.saturating_sub(2 * ring)..=col.min(w.saturating_sub(1)) {
                        best = best.max(mask.a[row * w + source]);
                    }
                    across[row * grown_w + col] = best;
                }
            }
            for row in 0..grown_h {
                for col in 0..grown_w {
                    let mut best = 0.0_f32;
                    for source in row.saturating_sub(2 * ring)..=row.min(h.saturating_sub(1)) {
                        best = best.max(across[source * grown_w + col]);
                    }
                    put(col + pad, row + pad, dark, best * 0.9);
                }
            }
        }
        // The letters themselves.
        let ink = recolour(text.color);
        for row in 0..h {
            for col in 0..w {
                put(col + inset, row + inset, ink, mask.a[row * w + col]);
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
            look,
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
            && self.look == Look::of(text)
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

    fn layer(text: &str) -> SequenceText {
        SequenceText::new(Tick::ZERO, Tick::from_units(1, 1), text)
    }

    #[test]
    fn a_new_layer_looks_as_text_always_did_and_each_style_changes_the_picture() {
        let plain = layer("Style test");
        let base = TextRaster::render(&plain, 1280, 720).unwrap().unwrap();
        // Every option draws something different from the plain layer, and is noticed by the cache.
        let variants: Vec<(&str, SequenceText)> = vec![
            (
                "serif",
                SequenceText {
                    font: crate::TextFont::Serif,
                    ..plain.clone()
                },
            ),
            (
                "mono",
                SequenceText {
                    font: crate::TextFont::Mono,
                    ..plain.clone()
                },
            ),
            (
                "regular",
                SequenceText {
                    weight: crate::TextWeight::Regular,
                    ..plain.clone()
                },
            ),
            (
                "bold",
                SequenceText {
                    weight: crate::TextWeight::Bold,
                    ..plain.clone()
                },
            ),
            (
                "italic",
                SequenceText {
                    italic: true,
                    ..plain.clone()
                },
            ),
            (
                "caps",
                SequenceText {
                    caps: true,
                    ..plain.clone()
                },
            ),
            (
                "underline",
                SequenceText {
                    underline: true,
                    ..plain.clone()
                },
            ),
            (
                "tracking",
                SequenceText {
                    tracking: 200,
                    ..plain.clone()
                },
            ),
            (
                "no shadow",
                SequenceText {
                    shadow: false,
                    ..plain.clone()
                },
            ),
            (
                "outline",
                SequenceText {
                    outline: true,
                    ..plain.clone()
                },
            ),
            (
                "backdrop",
                SequenceText {
                    backdrop: true,
                    ..plain.clone()
                },
            ),
        ];
        for (name, variant) in &variants {
            assert!(variant.is_valid(), "{name}");
            let raster = TextRaster::render(variant, 1280, 720).unwrap().unwrap();
            assert!(
                raster.rgba() != base.rgba() || raster.size() != base.size(),
                "{name} looks like the plain layer"
            );
            assert!(!base.matches(variant, 1280, 720), "{name} is not noticed");
            assert!(
                raster.matches(variant, 1280, 720),
                "{name} does not match itself"
            );
            assert!(raster.bytes() <= MAX_RASTER_PIXELS * 4, "{name}");
        }
        // The box and the outline grow the picture around the letters; no shadow shrinks it.
        let size_of = |name: &str| {
            let raster = TextRaster::render(
                &variants.iter().find(|(n, _)| *n == name).unwrap().1,
                1280,
                720,
            )
            .unwrap()
            .unwrap();
            raster.size()
        };
        assert!(size_of("backdrop")[0] > base.size()[0] && size_of("backdrop")[1] > base.size()[1]);
        assert!(size_of("outline")[0] >= base.size()[0]);
        assert!(size_of("no shadow")[0] <= base.size()[0]);
        // The box is a solid dark rectangle: its corners are dark and see-through, not empty.
        let boxed = TextRaster::render(
            &SequenceText {
                backdrop: true,
                ..plain.clone()
            },
            1280,
            720,
        )
        .unwrap()
        .unwrap();
        let [w, _] = boxed.size();
        let corner = &boxed.rgba()[..4];
        assert!(corner[3] > 100 && corner[0] < 40, "{corner:?}");
        let last = &boxed.rgba()[(w - 1) * 4..w * 4];
        assert!(last[3] > 0);
        // Everything stays inside the pixel budget at the largest size, with every option on.
        let everything = SequenceText {
            text: "W".repeat(60),
            size: 250,
            font: crate::TextFont::Mono,
            weight: crate::TextWeight::Bold,
            italic: true,
            caps: true,
            underline: true,
            tracking: 400,
            outline: true,
            backdrop: true,
            ..plain
        };
        // Too much text for the budget is refused cleanly; anything that does render stays bounded.
        if let Ok(Some(raster)) = TextRaster::render(&everything, 1920, 1080) {
            assert!(raster.bytes() <= MAX_RASTER_PIXELS * 4);
        }
    }

    #[test]
    fn style_ranges_are_enforced_and_opacity_scales_the_layer() {
        let mut text = layer("x");
        assert!(text.is_valid());
        for (name, bad) in [
            (
                "tracking",
                SequenceText {
                    tracking: 401,
                    ..text.clone()
                },
            ),
            (
                "tracking low",
                SequenceText {
                    tracking: -51,
                    ..text.clone()
                },
            ),
            (
                "opacity",
                SequenceText {
                    opacity: 101,
                    ..text.clone()
                },
            ),
            (
                "opacity zero",
                SequenceText {
                    opacity: 0,
                    ..text.clone()
                },
            ),
        ] {
            assert!(!bad.is_valid(), "{name}");
            assert!(TextRaster::render(&bad, 640, 360).is_err(), "{name}");
        }
        assert_eq!(text.opacity_at(Tick::ZERO), 1.0);
        text.opacity = 40;
        assert!((text.opacity_at(Tick::ZERO) - 0.4).abs() < 1e-6);
        // Fades and the layer's own opacity multiply.
        text.fade_in = crate::Fade::Black;
        assert!(text.opacity_at(Tick::ZERO) < 0.01);
    }

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
        let title = SequenceText::new(Tick::ZERO, Tick::from_units(1, 1), "High quality highlight");
        for (w, h) in [(3840, 2160), (2160, 3840)] {
            assert!(
                TextRaster::render(&title, w, h).unwrap().unwrap().bytes() <= MAX_RASTER_PIXELS * 4
            );
        }
        assert!(TextRaster::render(&title, 3840, 3840).is_err());
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
