//! Where a captured picture lands in the clip's frame: stretched, letterboxed, or cropped.
//!
//! Pure geometry with no pixels, so every rule is unit-tested without a screen.

use serde::{Deserialize, Serialize};

/// Largest output edge, in pixels.
pub const MAX_OUTPUT_EDGE: u32 = 7_680;
/// Smallest output edge, in pixels.
pub const MIN_OUTPUT_EDGE: u32 = 16;

/// How the captured picture is fitted into the clip's frame.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Framing {
    /// Keep the source's shape. A 4:3 game stays 4:3.
    #[default]
    Original,
    /// Stretch to fill the output frame, distorting the picture (4:3 becomes wide 16:9).
    Stretch,
    /// Keep the shape and add black bars to fill the output frame.
    Letterbox,
    /// Keep the shape and cut off whatever overflows the output frame.
    Crop,
}

impl Framing {
    /// Every framing, in menu order.
    pub const ALL: [Framing; 4] = [
        Framing::Original,
        Framing::Stretch,
        Framing::Letterbox,
        Framing::Crop,
    ];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Stretch => "Stretch",
            Self::Letterbox => "Black bars",
            Self::Crop => "Crop",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Original => "Clip in the game's own shape.",
            Self::Stretch => "Fill the frame. A 4:3 game is stretched wide.",
            Self::Letterbox => "Keep the shape and fill the sides with black.",
            Self::Crop => "Keep the shape and trim the top and bottom.",
        }
    }
}

/// The shape of the clip's frame.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputShape {
    /// Same shape as the capture.
    #[default]
    Source,
    /// 16:9 widescreen.
    Wide16x9,
    /// 4:3 standard.
    Standard4x3,
}

impl OutputShape {
    /// Every shape, in menu order.
    pub const ALL: [OutputShape; 3] = [
        OutputShape::Source,
        OutputShape::Wide16x9,
        OutputShape::Standard4x3,
    ];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Source => "Same as game",
            Self::Wide16x9 => "16:9",
            Self::Standard4x3 => "4:3",
        }
    }

    fn ratio(self) -> Option<(u32, u32)> {
        match self {
            Self::Source => None,
            Self::Wide16x9 => Some((16, 9)),
            Self::Standard4x3 => Some((4, 3)),
        }
    }
}

/// A rectangle of pixels inside a frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelRect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// Where the source picture lands in the output frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reframe {
    /// Output frame width.
    pub output_width: u32,
    /// Output frame height.
    pub output_height: u32,
    /// The part of the source that is used.
    pub source: PixelRect,
    /// Where that part is drawn in the output. Pixels outside it are black.
    pub destination: PixelRect,
}

fn even(value: u32) -> u32 {
    (value & !1).max(2)
}

/// Computes how a `width` x `height` capture is fitted into the output frame.
///
/// Output sizes are even, because H.264 4:2:0 needs even dimensions.
pub fn reframe(
    width: u32,
    height: u32,
    shape: OutputShape,
    framing: Framing,
    output_height: u32,
) -> Option<Reframe> {
    if width == 0 || height == 0 || width > 65_535 || height > 65_535 {
        return None;
    }
    let output_height = even(output_height.min(MAX_OUTPUT_EDGE));
    let whole = PixelRect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let (output_width, framing) = match shape.ratio() {
        None => {
            let scaled = u64::from(output_height) * u64::from(width) / u64::from(height);
            (
                even(scaled.min(u64::from(MAX_OUTPUT_EDGE)) as u32),
                Framing::Original,
            )
        }
        Some((numerator, denominator)) => {
            let scaled = u64::from(output_height) * u64::from(numerator) / u64::from(denominator);
            (even(scaled.min(u64::from(MAX_OUTPUT_EDGE)) as u32), framing)
        }
    };
    let full = PixelRect {
        x: 0,
        y: 0,
        width: output_width,
        height: output_height,
    };
    let (source, destination) = match framing {
        Framing::Original | Framing::Stretch => (whole, full),
        Framing::Letterbox => {
            // Fit inside: compare width/height ratios by cross-multiplying.
            let wide_source = u64::from(width) * u64::from(output_height)
                >= u64::from(height) * u64::from(output_width);
            let fitted = if wide_source {
                let fitted_height = u64::from(output_width) * u64::from(height) / u64::from(width);
                let fitted_height = even(fitted_height as u32).min(output_height);
                PixelRect {
                    x: 0,
                    y: ((output_height - fitted_height) / 2) & !1,
                    width: output_width,
                    height: fitted_height,
                }
            } else {
                let fitted_width = u64::from(output_height) * u64::from(width) / u64::from(height);
                let fitted_width = even(fitted_width as u32).min(output_width);
                PixelRect {
                    x: ((output_width - fitted_width) / 2) & !1,
                    y: 0,
                    width: fitted_width,
                    height: output_height,
                }
            };
            (whole, fitted)
        }
        Framing::Crop => {
            // Fill the output, cutting the source on the axis that overflows.
            let wide_source = u64::from(width) * u64::from(output_height)
                >= u64::from(height) * u64::from(output_width);
            let cut = if wide_source {
                let used_width = (u64::from(height) * u64::from(output_width)
                    / u64::from(output_height))
                .clamp(1, u64::from(width)) as u32;
                PixelRect {
                    x: (width - used_width) / 2,
                    y: 0,
                    width: used_width,
                    height,
                }
            } else {
                let used_height = (u64::from(width) * u64::from(output_height)
                    / u64::from(output_width))
                .clamp(1, u64::from(height)) as u32;
                PixelRect {
                    x: 0,
                    y: (height - used_height) / 2,
                    width,
                    height: used_height,
                }
            };
            (cut, full)
        }
    };
    Some(Reframe {
        output_width,
        output_height,
        source,
        destination,
    })
}
