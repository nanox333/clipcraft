//! Turns a captured BGRA picture into the planar 4:2:0 picture the encoder takes, fitting it into
//! the clip's frame as a [`Reframe`] says (stretch, black bars, or crop) and scaling with
//! bilinear sampling.

use clipcraft_capture::CaptureFrame;
use clipcraft_capture::reframe::{PixelRect, Reframe};

use crate::RecordError;

/// A planar 8-bit 4:2:0 picture with limited-range BT.709 values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct I420 {
    /// Width in pixels; even.
    pub width: u32,
    /// Height in pixels; even.
    pub height: u32,
    /// Luma plane, `width * height` bytes.
    pub y: Vec<u8>,
    /// Blue-difference plane, `width / 2 * height / 2` bytes.
    pub u: Vec<u8>,
    /// Red-difference plane, `width / 2 * height / 2` bytes.
    pub v: Vec<u8>,
}

impl I420 {
    /// A black picture. Odd or zero sizes are refused.
    pub fn black(width: u32, height: u32) -> Result<Self, RecordError> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(RecordError::BadSize);
        }
        let luma = (width as usize)
            .checked_mul(height as usize)
            .ok_or(RecordError::BadSize)?;
        Ok(Self {
            width,
            height,
            y: vec![16; luma],
            u: vec![128; luma / 4],
            v: vec![128; luma / 4],
        })
    }
}

/// One output column or row: the two source indices to blend and the weight of the second, in
/// 1/256ths.
#[derive(Clone, Copy)]
struct Tap {
    first: u32,
    second: u32,
    weight: u32,
}

fn taps(source_start: u32, source_len: u32, output_len: u32) -> Vec<Tap> {
    let mut taps = Vec::with_capacity(output_len as usize);
    for index in 0..output_len {
        // Centre of the output pixel mapped into the source, in 1/256 pixel, minus half a pixel.
        let centre = ((2 * u64::from(index) + 1) * u64::from(source_len) * 256)
            / (2 * u64::from(output_len));
        let position = centre.saturating_sub(128);
        let first = (position >> 8).min(u64::from(source_len.saturating_sub(1))) as u32;
        let second = (first + 1).min(source_len.saturating_sub(1));
        taps.push(Tap {
            first: source_start + first,
            second: source_start + second,
            weight: (position & 255) as u32,
        });
    }
    taps
}

#[inline]
fn lerp(a: u32, b: u32, weight: u32) -> u32 {
    (a * (256 - weight) + b * weight + 128) >> 8
}

#[inline]
fn luma(r: i32, g: i32, b: i32) -> u8 {
    (16 + ((47 * r + 157 * g + 16 * b + 128) >> 8)).clamp(16, 235) as u8
}

#[inline]
fn chroma(r: i32, g: i32, b: i32) -> (u8, u8) {
    let u = 128 + ((-26 * r - 87 * g + 113 * b + 128) >> 8);
    let v = 128 + ((112 * r - 102 * g - 10 * b + 128) >> 8);
    (u.clamp(16, 240) as u8, v.clamp(16, 240) as u8)
}

/// Luma of one row of BGRA pixels.
fn luma_row(row: &[u8], out: &mut [u8]) {
    for (pixel, out) in row.chunks_exact(4).zip(out.iter_mut()) {
        *out = luma(
            i32::from(pixel[2]),
            i32::from(pixel[1]),
            i32::from(pixel[0]),
        );
    }
}

/// Chroma of two rows of BGRA pixels, one value per 2x2 block.
fn chroma_rows(upper: &[u8], lower: &[u8], u: &mut [u8], v: &mut [u8]) {
    let blocks = upper.chunks_exact(8).zip(lower.chunks_exact(8));
    for ((top, bottom), (u, v)) in blocks.zip(u.iter_mut().zip(v.iter_mut())) {
        let (mut r, mut g, mut b) = (2_i32, 2_i32, 2_i32);
        for pixel in top.chunks_exact(4).chain(bottom.chunks_exact(4)) {
            r += i32::from(pixel[2]);
            g += i32::from(pixel[1]);
            b += i32::from(pixel[0]);
        }
        (*u, *v) = chroma(r / 4, g / 4, b / 4);
    }
}

/// A row of the source as BGRA bytes, from `x` for `len` pixels.
fn source_row(frame: &CaptureFrame, y: u32, x: u32, len: u32) -> &[u8] {
    let start = y as usize * frame.stride + x as usize * 4;
    frame
        .bgra
        .get(start..start + len as usize * 4)
        .unwrap_or(&[])
}

/// One output row scaled with bilinear weights into `out` (BGRA), using `blend` as scratch for the
/// vertically blended source row.
fn scale_row(
    frame: &CaptureFrame,
    row: Tap,
    columns: &[Tap],
    source_x: u32,
    source_len: u32,
    blend: &mut Vec<u8>,
    out: &mut [u8],
) {
    let upper = source_row(frame, row.first, source_x, source_len);
    let lower = source_row(frame, row.second, source_x, source_len);
    blend.clear();
    if row.weight == 0 || upper.len() != lower.len() {
        blend.extend_from_slice(upper);
    } else {
        blend.extend(
            upper
                .iter()
                .zip(lower)
                .map(|(&a, &b)| lerp(u32::from(a), u32::from(b), row.weight) as u8),
        );
    }
    for (column, pixel) in columns.iter().zip(out.chunks_exact_mut(4)) {
        let first = (column.first - source_x) as usize * 4;
        let second = (column.second - source_x) as usize * 4;
        let (Some(a), Some(b)) = (blend.get(first..first + 4), blend.get(second..second + 4))
        else {
            continue;
        };
        for channel in 0..4 {
            pixel[channel] =
                lerp(u32::from(a[channel]), u32::from(b[channel]), column.weight) as u8;
        }
    }
}

/// Fills `out` from `frame` as `plan` describes. `out` must already have the plan's output size.
pub fn convert(
    frame: &CaptureFrame,
    plan: &Reframe,
    out: &mut I420,
    threads: usize,
) -> Result<(), RecordError> {
    frame.validate().map_err(|_| RecordError::BadFrame)?;
    if out.width != plan.output_width || out.height != plan.output_height {
        return Err(RecordError::BadSize);
    }
    let PixelRect {
        x: sx,
        y: sy,
        width: sw,
        height: sh,
    } = plan.source;
    let PixelRect {
        x: dx,
        y: dy,
        width: dw,
        height: dh,
    } = plan.destination;
    if sw == 0
        || sh == 0
        || dw == 0
        || dh == 0
        || sx.saturating_add(sw) > frame.width
        || sy.saturating_add(sh) > frame.height
        || dx.saturating_add(dw) > out.width
        || dy.saturating_add(dh) > out.height
        || !dx.is_multiple_of(2)
        || !dy.is_multiple_of(2)
        || !dw.is_multiple_of(2)
        || !dh.is_multiple_of(2)
    {
        return Err(RecordError::BadSize);
    }
    // Black bars: reset the planes, then draw over the destination.
    let full = dx == 0 && dy == 0 && dw == out.width && dh == out.height;
    if !full {
        out.y.fill(16);
        out.u.fill(128);
        out.v.fill(128);
    }
    let columns = taps(sx, sw, dw);
    let rows = taps(sy, sh, dh);

    let out_width = out.width as usize;
    let chroma_width = out_width / 2;
    let first_pair = (dy / 2) as usize;
    let pairs = (dh / 2) as usize;
    let threads = threads.clamp(1, 32).min(pairs.max(1));
    let pairs_per_band = pairs.div_ceil(threads).max(1);

    // Split the planes into bands of row pairs so threads never touch the same bytes.
    let y_region = out
        .y
        .get_mut(first_pair * 2 * out_width..(first_pair + pairs) * 2 * out_width)
        .ok_or(RecordError::BadSize)?;
    let u_region = out
        .u
        .get_mut(first_pair * chroma_width..(first_pair + pairs) * chroma_width)
        .ok_or(RecordError::BadSize)?;
    let v_region = out
        .v
        .get_mut(first_pair * chroma_width..(first_pair + pairs) * chroma_width)
        .ok_or(RecordError::BadSize)?;
    let bands = y_region
        .chunks_mut(pairs_per_band * 2 * out_width)
        .zip(u_region.chunks_mut(pairs_per_band * chroma_width))
        .zip(v_region.chunks_mut(pairs_per_band * chroma_width))
        .enumerate();

    let unscaled = sw == dw && sh == dh;
    let work = |index: usize, y_band: &mut [u8], u_band: &mut [u8], v_band: &mut [u8]| {
        let first_row = index * pairs_per_band * 2;
        let band_pairs = y_band.len() / (2 * out_width);
        // Scratch for scaled rows: two output rows of BGRA and the blended source row.
        let mut scaled = vec![0_u8; if unscaled { 0 } else { 2 * dw as usize * 4 }];
        let mut blend = Vec::new();
        for pair in 0..band_pairs {
            let top = first_row + pair * 2;
            let (upper, lower): (&[u8], &[u8]) = if unscaled {
                (
                    source_row(frame, sy + top as u32, sx, dw),
                    source_row(frame, sy + top as u32 + 1, sx, dw),
                )
            } else {
                let (first, second) = scaled.split_at_mut(dw as usize * 4);
                if let (Some(&row_a), Some(&row_b)) = (rows.get(top), rows.get(top + 1)) {
                    scale_row(frame, row_a, &columns, sx, sw, &mut blend, first);
                    scale_row(frame, row_b, &columns, sx, sw, &mut blend, second);
                }
                (&*first, &*second)
            };
            let y_start = pair * 2 * out_width + dx as usize;
            if let Some(line) = y_band.get_mut(y_start..y_start + dw as usize) {
                luma_row(upper, line);
            }
            let y_start = (pair * 2 + 1) * out_width + dx as usize;
            if let Some(line) = y_band.get_mut(y_start..y_start + dw as usize) {
                luma_row(lower, line);
            }
            let c_start = pair * chroma_width + dx as usize / 2;
            let c_end = c_start + dw as usize / 2;
            if let (Some(u), Some(v)) = (
                u_band.get_mut(c_start..c_end),
                v_band.get_mut(c_start..c_end),
            ) {
                chroma_rows(upper, lower, u, v);
            }
        }
    };

    if threads == 1 {
        for (index, ((y_band, u_band), v_band)) in bands {
            work(index, y_band, u_band, v_band);
        }
    } else {
        std::thread::scope(|scope| {
            for (index, ((y_band, u_band), v_band)) in bands {
                let work = &work;
                scope.spawn(move || work(index, y_band, u_band, v_band));
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipcraft_capture::reframe::{Framing, OutputShape, reframe};
    use clipcraft_time::Tick;

    fn solid(width: u32, height: u32, bgra: [u8; 4]) -> CaptureFrame {
        CaptureFrame {
            width,
            height,
            stride: width as usize * 4,
            bgra: bgra
                .iter()
                .copied()
                .cycle()
                .take((width * height * 4) as usize)
                .collect(),
            captured_at: Tick::from_units(0, 30),
        }
    }

    fn run(
        frame: &CaptureFrame,
        shape: OutputShape,
        framing: Framing,
        height: u32,
    ) -> (Reframe, I420) {
        let plan = reframe(frame.width, frame.height, shape, framing, height).unwrap();
        let mut out = I420::black(plan.output_width, plan.output_height).unwrap();
        convert(frame, &plan, &mut out, 2).unwrap();
        (plan, out)
    }

    #[test]
    fn white_and_black_map_to_the_limits_of_video_range() {
        let (_, white) = run(
            &solid(32, 32, [255, 255, 255, 255]),
            OutputShape::Source,
            Framing::Original,
            32,
        );
        assert!(white.y.iter().all(|&y| y == 235));
        assert!(white.u.iter().chain(&white.v).all(|&c| c == 128));
        let (_, black) = run(
            &solid(32, 32, [0, 0, 0, 255]),
            OutputShape::Source,
            Framing::Original,
            32,
        );
        assert!(black.y.iter().all(|&y| y == 16));
    }

    #[test]
    fn pure_red_has_the_bt709_chroma() {
        // BGRA order: blue 0, green 0, red 255.
        let (_, red) = run(
            &solid(16, 16, [0, 0, 255, 255]),
            OutputShape::Source,
            Framing::Original,
            16,
        );
        assert!(red.v.iter().all(|&v| v > 200), "{:?}", red.v.first());
        assert!(red.u.iter().all(|&u| u < 110));
    }

    #[test]
    fn letterboxing_leaves_black_bars_and_fills_the_middle() {
        let (plan, out) = run(
            &solid(64, 48, [255, 255, 255, 255]),
            OutputShape::Wide16x9,
            Framing::Letterbox,
            36,
        );
        let d = plan.destination;
        assert!(d.x > 0 && d.width < plan.output_width);
        let row = (plan.output_height / 2) as usize * plan.output_width as usize;
        assert_eq!(out.y[row], 16, "left bar is black");
        assert_eq!(
            out.y[row + (d.x + d.width / 2) as usize],
            235,
            "centre is white"
        );
        assert_eq!(
            out.y[row + plan.output_width as usize - 1],
            16,
            "right bar is black"
        );
    }

    #[test]
    fn cropping_uses_only_the_middle_of_the_source() {
        // Top and bottom thirds white, middle band black: a 16:9 crop of a 4:3 picture keeps the middle.
        let (width, height) = (64_u32, 48_u32);
        let mut frame = solid(width, height, [255, 255, 255, 255]);
        for y in 12..36 {
            for x in 0..width {
                let at = (y * width + x) as usize * 4;
                frame.bgra[at..at + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
        let (plan, out) = run(&frame, OutputShape::Wide16x9, Framing::Crop, 36);
        assert!(plan.source.height < height);
        let middle = (plan.output_height / 2) as usize * plan.output_width as usize;
        assert!(
            out.y[middle..middle + plan.output_width as usize]
                .iter()
                .all(|&y| y == 16)
        );
    }

    #[test]
    fn scaling_down_keeps_a_flat_colour_flat() {
        let (_, out) = run(
            &solid(100, 60, [10, 200, 30, 255]),
            OutputShape::Source,
            Framing::Original,
            30,
        );
        let first = out.y[0];
        assert!(out.y.iter().all(|&y| y == first));
    }

    #[test]
    fn single_and_multi_threaded_results_are_identical() {
        let frame = {
            let mut frame = solid(90, 70, [0, 0, 0, 255]);
            for (index, byte) in frame.bgra.iter_mut().enumerate() {
                *byte = (index * 7 % 251) as u8;
            }
            frame
        };
        let plan = reframe(90, 70, OutputShape::Wide16x9, Framing::Letterbox, 48).unwrap();
        let mut one = I420::black(plan.output_width, plan.output_height).unwrap();
        let mut many = one.clone();
        convert(&frame, &plan, &mut one, 1).unwrap();
        convert(&frame, &plan, &mut many, 7).unwrap();
        assert_eq!(one, many);
    }

    /// Prints how long a conversion takes. Run with `--ignored --nocapture` in release mode.
    #[test]
    #[ignore = "timing only"]
    fn timing() {
        for (from, to) in [
            ((1920, 1080), 1080),
            ((2560, 1440), 1080),
            ((1920, 1080), 720),
        ] {
            let frame = solid(from.0, from.1, [10, 20, 30, 255]);
            let plan = reframe(from.0, from.1, OutputShape::Source, Framing::Original, to).unwrap();
            let mut out = I420::black(plan.output_width, plan.output_height).unwrap();
            let started = std::time::Instant::now();
            for _ in 0..10 {
                convert(&frame, &plan, &mut out, 2).unwrap();
            }
            println!("{from:?} -> {to}p: {:?} per frame", started.elapsed() / 10);
        }
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        let frame = solid(32, 32, [1, 2, 3, 255]);
        let plan = reframe(32, 32, OutputShape::Source, Framing::Original, 32).unwrap();
        let mut wrong = I420::black(16, 16).unwrap();
        assert_eq!(
            convert(&frame, &plan, &mut wrong, 1),
            Err(RecordError::BadSize)
        );
        let mut short = frame.clone();
        short.bgra.truncate(10);
        let mut out = I420::black(32, 32).unwrap();
        assert_eq!(
            convert(&short, &plan, &mut out, 1),
            Err(RecordError::BadFrame)
        );
        assert!(I420::black(0, 4).is_err() && I420::black(3, 4).is_err());
        let odd_plan = Reframe {
            destination: PixelRect {
                x: 1,
                ..plan.destination
            },
            ..plan
        };
        assert_eq!(
            convert(&frame, &odd_plan, &mut out, 1),
            Err(RecordError::BadSize)
        );
    }
}
