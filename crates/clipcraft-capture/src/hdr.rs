//! Turning an HDR picture into the standard-dynamic-range picture the rest of ClipCraft records.
//!
//! Windows hands HDR desktops over as linear scRGB in 16-bit floats: BT.709 primaries, where 1.0
//! is 80 nits and a typical "SDR white" in HDR mode is 203 nits (2.5). This module reads those
//! half floats, scales so reference white becomes 1.0, rolls highlights off smoothly instead of
//! clipping them, and encodes the result with the sRGB curve into BGRA.

/// 203 nits, the reference white of BT.2408, in scRGB units (1.0 = 80 nits).
pub const HDR_REFERENCE_WHITE: f32 = 203.0 / 80.0;

/// Where the highlight roll-off begins, as a fraction of reference white.
const KNEE: f32 = 0.9;

/// Reads an IEEE half-precision float from its bits.
pub fn half_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f32::from(bits & 0x3ff);
    match exponent {
        0 => sign * fraction * 2.0_f32.powi(-24),
        0x1f => {
            if fraction == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + fraction / 1024.0) * 2.0_f32.powi(exponent - 15),
    }
}

/// Maps a linear value (1.0 = reference white) to 0..=1, linear below the knee and rolling off
/// towards 1.0 above it. Negative values (colours outside BT.709) and NaN become 0.
pub fn roll_off(value: f32) -> f32 {
    if value.is_nan() || value <= 0.0 {
        return 0.0;
    }
    if value <= KNEE {
        return value;
    }
    KNEE + (1.0 - KNEE) * ((value - KNEE) / (1.0 - KNEE)).tanh()
}

/// The sRGB encoding of a linear value in 0..=1, as an 8-bit code.
pub fn srgb_code(linear: f32) -> u8 {
    let linear = linear.clamp(0.0, 1.0);
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0 + 0.5) as u8
}

/// Converts rows of RGBA half floats (little-endian, `stride` bytes per row) to 8-bit BGRA.
///
/// `reference_white` is the scRGB value that should become white. Returns `None` when the sizes do
/// not agree with the byte count.
pub fn scrgb_to_bgra8(
    half_rgba: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    reference_white: f32,
) -> Option<Vec<u8>> {
    let row_bytes = (width as usize).checked_mul(8)?;
    if width == 0
        || height == 0
        || stride < row_bytes
        || reference_white.is_nan()
        || reference_white <= 0.0
    {
        return None;
    }
    let needed = stride
        .checked_mul(height as usize - 1)?
        .checked_add(row_bytes)?;
    if half_rgba.len() < needed {
        return None;
    }
    let scale = 1.0 / reference_white;
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for row in half_rgba.chunks(stride).take(height as usize) {
        for pixel in row.get(..row_bytes)?.as_chunks::<8>().0 {
            let channel = |at: usize| {
                let bits = u16::from_le_bytes([pixel[at], pixel[at + 1]]);
                srgb_code(roll_off(half_to_f32(bits) * scale))
            };
            out.extend_from_slice(&[channel(4), channel(2), channel(0), 255]);
        }
    }
    Some(out)
}

/// Picks the reference white for a frame: HDR desktops put interface white at 203 nits (2.5),
/// so a frame with any value clearly above 1.0 is treated as HDR and scaled; a frame that never
/// exceeds 1.0 is already standard range and is left as it is.
pub fn reference_white_for(half_rgba: &[u8]) -> f32 {
    let brightest = half_rgba
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| half_to_f32(u16::from_le_bytes(*pair)))
        .filter(|value| value.is_finite())
        .fold(0.0_f32, f32::max);
    if brightest > 1.05 {
        HDR_REFERENCE_WHITE
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half(value: f32) -> u16 {
        // Enough of an encoder for the values used in these tests.
        if value == 0.0 {
            return 0;
        }
        let bits = value.to_bits();
        let sign = ((bits >> 16) & 0x8000) as u16;
        let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
        let mantissa = ((bits >> 13) & 0x3ff) as u16;
        sign | ((exponent as u16) << 10) | mantissa
    }

    #[test]
    fn half_floats_decode_exactly() {
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0xc000), -2.0);
        assert_eq!(half_to_f32(0x4000), 2.0);
        assert_eq!(half_to_f32(0x3555), 0.333_251_95);
        assert_eq!(half_to_f32(0), 0.0);
        assert_eq!(half_to_f32(0x0001), 2.0_f32.powi(-24));
        assert!(half_to_f32(0x7c00).is_infinite());
        assert!(half_to_f32(0x7e00).is_nan());
        assert_eq!(half_to_f32(half(2.5)), 2.5);
    }

    #[test]
    fn roll_off_is_linear_then_smooth_and_never_exceeds_one() {
        assert_eq!(roll_off(0.5), 0.5);
        assert_eq!(roll_off(-1.0), 0.0);
        assert_eq!(roll_off(f32::NAN), 0.0);
        let mut last = 0.0;
        for step in 0..=400 {
            let value = roll_off(step as f32 / 20.0);
            assert!(value >= last && value <= 1.0, "{step}: {value}");
            last = value;
        }
        assert!(roll_off(1.0) > 0.95 && roll_off(1.0) < 1.0);
        assert!(roll_off(f32::INFINITY) <= 1.0);
    }

    #[test]
    fn srgb_codes_match_the_curve() {
        assert_eq!(srgb_code(0.0), 0);
        assert_eq!(srgb_code(1.0), 255);
        assert_eq!(srgb_code(0.214), 127);
        assert_eq!(srgb_code(2.0), 255);
        assert_eq!(srgb_code(-1.0), 0);
    }

    fn pixel(r: f32, g: f32, b: f32) -> Vec<u8> {
        [half(r), half(g), half(b), half(1.0)]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn reference_white_becomes_white_and_black_stays_black() {
        let mut data = pixel(
            HDR_REFERENCE_WHITE,
            HDR_REFERENCE_WHITE,
            HDR_REFERENCE_WHITE,
        );
        data.extend(pixel(0.0, 0.0, 0.0));
        let out = scrgb_to_bgra8(&data, 2, 1, 16, HDR_REFERENCE_WHITE).unwrap();
        assert!(out[0] >= 250 && out[1] >= 250 && out[2] >= 250, "{out:?}");
        assert_eq!(&out[4..8], &[0, 0, 0, 255]);
    }

    #[test]
    fn a_very_bright_highlight_is_kept_below_clipping_of_the_midtones() {
        let data = pixel(10.0, 10.0, 10.0);
        let out = scrgb_to_bgra8(&data, 1, 1, 8, HDR_REFERENCE_WHITE).unwrap();
        assert_eq!(&out[..3], &[255, 255, 255]);
        let mid = scrgb_to_bgra8(&pixel(1.0, 1.0, 1.0), 1, 1, 8, HDR_REFERENCE_WHITE).unwrap();
        assert!(mid[0] > 100 && mid[0] < 200, "{mid:?}");
    }

    #[test]
    fn channels_come_out_in_bgra_order() {
        let out = scrgb_to_bgra8(&pixel(1.0, 0.0, 0.0), 1, 1, 8, 1.0).unwrap();
        // Red is the third byte; the highlight roll-off leaves it just under full.
        assert_eq!((out[0], out[1], out[3]), (0, 0, 255));
        assert!(out[2] >= 250);
    }

    #[test]
    fn bad_sizes_are_refused() {
        let data = pixel(1.0, 1.0, 1.0);
        assert!(scrgb_to_bgra8(&data, 2, 1, 16, 1.0).is_none());
        assert!(scrgb_to_bgra8(&data, 0, 1, 8, 1.0).is_none());
        assert!(scrgb_to_bgra8(&data, 1, 1, 4, 1.0).is_none());
        assert!(scrgb_to_bgra8(&data, 1, 1, 8, 0.0).is_none());
        assert!(scrgb_to_bgra8(&data, 1, 1, 8, f32::NAN).is_none());
        assert!(scrgb_to_bgra8(&data, u32::MAX, u32::MAX, usize::MAX, 1.0).is_none());
    }

    #[test]
    fn the_reference_white_is_guessed_from_the_brightest_value() {
        assert_eq!(reference_white_for(&pixel(0.5, 1.0, 0.2)), 1.0);
        assert_eq!(
            reference_white_for(&pixel(2.5, 1.0, 0.2)),
            HDR_REFERENCE_WHITE
        );
        assert_eq!(reference_white_for(&[]), 1.0);
    }
}
