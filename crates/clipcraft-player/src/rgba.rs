//! Turning a decoded picture into the RGBA the screen takes.
//!
//! FilmCraft's `to_rgba8` does this in floating point on every core of the machine. That keeps the
//! picture right for every colour space it knows, but at 1440p it spends about half a second of
//! processor time every second of 60 pictures a second, most of it in worker threads waiting for
//! work. The common case, 8-bit Y'CbCr in BT.709 or sRGB, is converted here in fixed point on the
//! calling thread (a few milliseconds a picture); everything else still goes to FilmCraft.

use filmcraft_color::{Range, Transfer};
use filmcraft_frame::{PixelData, VideoFrame};

/// Fixed-point fraction bits.
const SHIFT: u32 = 16;
const HALF: i32 = 1 << (SHIFT - 1);

/// The picture as straight-alpha RGBA, `width * height * 4` bytes.
pub(crate) fn to_rgba8(frame: &VideoFrame) -> Vec<u8> {
    match fast(frame) {
        Some(rgba) => rgba,
        None => frame.to_rgba8(),
    }
}

fn fast(frame: &VideoFrame) -> Option<Vec<u8>> {
    let PixelData::Yuv8 {
        planes,
        chroma,
        alpha: None,
    } = &frame.data
    else {
        return None;
    };
    if !matches!(frame.color.transfer, Transfer::Bt709 | Transfer::Srgb) {
        return None;
    }
    let (width, height) = (frame.width as usize, frame.height as usize);
    if width == 0 || height == 0 {
        return None;
    }
    let (shift_x, shift_y) = chroma.shifts();
    let chroma_width = width.div_ceil(1 << shift_x);
    let chroma_height = height.div_ceil(1 << shift_y);
    if planes[0].len() < width * height
        || planes[1].len() < chroma_width * chroma_height
        || planes[2].len() < chroma_width * chroma_height
    {
        return None;
    }
    let (kr, kb) = frame.color.matrix.kr_kb();
    let kg = 1.0 - kr - kb;
    let (luma_scale, luma_offset, chroma_scale) = match frame.color.range {
        Range::Limited => (255.0_f32 / 219.0, 16, 255.0_f32 / 224.0),
        Range::Full => (1.0, 0, 1.0),
    };
    let to_q = |value: f32| (f64::from(value) * f64::from(1_u32 << SHIFT)).round() as i32;
    let red_v = 2.0 * (1.0 - kr) * chroma_scale;
    let blue_u = 2.0 * (1.0 - kb) * chroma_scale;
    let (luma_q, red_v_q, blue_u_q) = (to_q(luma_scale), to_q(red_v), to_q(blue_u));
    let (green_v_q, green_u_q) = (to_q(red_v * kr / kg), to_q(blue_u * kb / kg));

    // Every term depends on one sample, so each is a table lookup instead of a multiplication.
    let mut tables = Tables {
        luma: [0; 256],
        red_v: [0; 256],
        green_u: [0; 256],
        green_v: [0; 256],
        blue_u: [0; 256],
    };
    for value in 0..256_usize {
        let centred = value as i32 - 128;
        tables.luma[value] = (value as i32 - luma_offset) * luma_q + HALF;
        tables.red_v[value] = red_v_q * centred;
        tables.green_u[value] = -green_u_q * centred;
        tables.green_v[value] = -green_v_q * centred;
        tables.blue_u[value] = blue_u_q * centred;
    }

    let mut out = vec![0_u8; width * height * 4];
    let (luma, blue, red) = (&planes[0], &planes[1], &planes[2]);
    // Bands of rows on a few threads: the work is memory bound, so more than that does not help.
    let threads = (height / 64).clamp(1, 4);
    let rows_per_band = height.div_ceil(threads);
    let band = |first_row: usize, rows: &mut [u8]| {
        for (offset, row) in rows.chunks_exact_mut(width * 4).enumerate() {
            let y = first_row + offset;
            let luma_row = &luma[y * width..(y + 1) * width];
            let start = (y >> shift_y) * chroma_width;
            convert_row(
                luma_row,
                &blue[start..start + chroma_width],
                &red[start..start + chroma_width],
                row,
                &tables,
                shift_x,
            );
        }
    };
    if threads == 1 {
        band(0, &mut out);
    } else {
        std::thread::scope(|scope| {
            for (index, rows) in out.chunks_mut(rows_per_band * width * 4).enumerate() {
                let band = &band;
                scope.spawn(move || band(index * rows_per_band, rows));
            }
        });
    }
    Some(out)
}

/// The lookup tables for one picture's colour matrix and range.
struct Tables {
    luma: [i32; 256],
    red_v: [i32; 256],
    green_u: [i32; 256],
    green_v: [i32; 256],
    blue_u: [i32; 256],
}

/// One row of pixels: `blue` and `red` hold one sample per `1 << shift_x` pixels.
fn convert_row(
    luma: &[u8],
    blue: &[u8],
    red: &[u8],
    out: &mut [u8],
    tables: &Tables,
    shift_x: u32,
) {
    let put = |pixel: &mut [u8], base: i32, red_term: i32, green_term: i32, blue_term: i32| {
        pixel[0] = clamp8((base + red_term) >> SHIFT);
        pixel[1] = clamp8((base + green_term) >> SHIFT);
        pixel[2] = clamp8((base + blue_term) >> SHIFT);
        pixel[3] = 255;
    };
    let terms = |u: u8, v: u8| {
        (
            tables.red_v[usize::from(v)],
            tables.green_u[usize::from(u)] + tables.green_v[usize::from(v)],
            tables.blue_u[usize::from(u)],
        )
    };
    if shift_x == 0 {
        for (((&sample, &u), &v), pixel) in
            luma.iter().zip(blue).zip(red).zip(out.chunks_exact_mut(4))
        {
            let (r, g, b) = terms(u, v);
            put(pixel, tables.luma[usize::from(sample)], r, g, b);
        }
        return;
    }
    // Two pixels share each chroma sample, so its terms are worked out once for both.
    let mut pairs = luma.chunks_exact(2);
    let mut pixels = out.chunks_exact_mut(8);
    for (((pair, &u), &v), pixel) in (&mut pairs).zip(blue).zip(red).zip(&mut pixels) {
        let (r, g, b) = terms(u, v);
        let (first, second) = pixel.split_at_mut(4);
        put(first, tables.luma[usize::from(pair[0])], r, g, b);
        put(second, tables.luma[usize::from(pair[1])], r, g, b);
    }
    // An odd last pixel has a chroma sample of its own.
    if let (&[sample], Some(&u), Some(&v)) = (
        pairs.remainder(),
        blue.get(luma.len() / 2),
        red.get(luma.len() / 2),
    ) {
        let (r, g, b) = terms(u, v);
        put(
            pixels.into_remainder(),
            tables.luma[usize::from(sample)],
            r,
            g,
            b,
        );
    }
}

/// Clamps to 0..=255 without a branch: a branch is guessed wrong half the time on real pictures.
#[inline]
fn clamp8(value: i32) -> u8 {
    let low = value & !(value >> 31);
    (low | ((255 - low) >> 31)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_color::ColorInfo;
    use filmcraft_frame::Chroma;
    use std::sync::Arc;

    fn frame(width: u32, height: u32, chroma: Chroma, range: Range) -> VideoFrame {
        let (sx, sy) = chroma.shifts();
        let (w, h) = (width as usize, height as usize);
        let (cw, ch) = (w.div_ceil(1 << sx), h.div_ceil(1 << sy));
        let mut seed = 12_345_u32;
        let mut next = move || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) as u8
        };
        let y: Vec<u8> = (0..w * h).map(|_| next()).collect();
        let u: Vec<u8> = (0..cw * ch).map(|_| next()).collect();
        let v: Vec<u8> = (0..cw * ch).map(|_| next()).collect();
        VideoFrame {
            width,
            height,
            data: PixelData::Yuv8 {
                planes: [Arc::new(y), Arc::new(u), Arc::new(v)],
                chroma,
                alpha: None,
            },
            color: ColorInfo {
                range,
                ..ColorInfo::default()
            },
            par: (1, 1),
            pts: Default::default(),
        }
    }

    /// The fixed-point result may differ from FilmCraft's floating-point one by rounding only.
    fn assert_close(frame: &VideoFrame) {
        let ours = fast(frame).expect("fast path");
        let theirs = frame.to_rgba8();
        assert_eq!(ours.len(), theirs.len());
        let worst = ours
            .iter()
            .zip(&theirs)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        assert!(worst <= 1, "largest difference {worst}");
    }

    #[test]
    fn matches_filmcraft_for_every_layout_and_range() {
        for chroma in [Chroma::C420, Chroma::C422, Chroma::C444] {
            for range in [Range::Limited, Range::Full] {
                assert_close(&frame(64, 48, chroma, range));
            }
        }
    }

    #[test]
    fn odd_sizes_use_the_chroma_that_covers_the_edge() {
        assert_close(&frame(33, 17, Chroma::C420, Range::Limited));
        assert_close(&frame(5, 3, Chroma::C422, Range::Limited));
    }

    #[test]
    fn other_colour_spaces_and_short_planes_are_left_to_filmcraft() {
        let mut pq = frame(8, 8, Chroma::C420, Range::Limited);
        pq.color.transfer = Transfer::Pq;
        assert!(fast(&pq).is_none());
        let mut short = frame(8, 8, Chroma::C420, Range::Limited);
        if let PixelData::Yuv8 { planes, .. } = &mut short.data {
            planes[0] = Arc::new(vec![0; 3]);
        }
        assert!(fast(&short).is_none());
        // Whatever the path, the answer is a full picture.
        assert_eq!(to_rgba8(&pq).len(), 8 * 8 * 4);
    }
}
