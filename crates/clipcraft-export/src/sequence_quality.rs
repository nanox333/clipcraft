//! Bounded editor output settings, shared by rendering and the desktop export preview.
use clipcraft_time::Tick;
use serde::{Deserialize, Serialize};

use crate::compress::{source_size, video_budget_with_limit};
use crate::{ExportError, ExportPreset};

pub(crate) const MAX_OUTPUT_EDGE: u32 = 3840;
pub(crate) const MAX_OUTPUT_PIXELS: u64 = 8_294_400;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequenceResolution {
    #[default]
    #[serde(rename = "720p")]
    Hd720,
    #[serde(rename = "1080p")]
    FullHd1080,
    Source,
}

/// The shape of the picture an edit is rendered as.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SequenceAspect {
    /// As before: 16:9 for the 720p and 1080p choices, the first clip's own shape for Source.
    #[default]
    Source,
    /// Width to height, as whole numbers: 16:9, 9:16, 1:1, 4:5 or any other shape between 1:4
    /// and 4:1.
    Ratio(u16, u16),
}
impl SequenceAspect {
    /// Widescreen, for most players and sites.
    pub const WIDE: Self = Self::Ratio(16, 9);
    /// Tall, for phones: shorts, reels and stories.
    pub const VERTICAL: Self = Self::Ratio(9, 16);
    pub const SQUARE: Self = Self::Ratio(1, 1);
    /// A little taller than wide, for feeds.
    pub const PORTRAIT: Self = Self::Ratio(4, 5);
    /// The shapes offered by name, widest first within wide, tall and square.
    pub const PRESETS: [SequenceAspect; 12] = [
        Self::Source,
        Self::Ratio(16, 9),
        Self::Ratio(21, 9),
        Self::Ratio(4, 3),
        Self::Ratio(3, 2),
        Self::Ratio(5, 4),
        Self::Ratio(1, 1),
        Self::Ratio(4, 5),
        Self::Ratio(2, 3),
        Self::Ratio(3, 4),
        Self::Ratio(9, 16),
        Self::Ratio(9, 21),
    ];
    /// The name shown for it: "Original" or "W:H".
    pub fn label(self) -> String {
        match self {
            Self::Source => "Original".to_owned(),
            Self::Ratio(across, down) => format!("{across}:{down}"),
        }
    }
    /// Width and height as a ratio; `None` keeps the earlier behaviour.
    pub fn ratio(self) -> Option<(u32, u32)> {
        match self {
            Self::Source => None,
            Self::Ratio(across, down) => Some((u32::from(across), u32::from(down))),
        }
    }
    /// Whether the shape can be rendered: each side at least 1 and at most 99, and no more than
    /// four times as long as the other.
    pub fn is_valid(self) -> bool {
        match self {
            Self::Source => true,
            Self::Ratio(across, down) => {
                (1..=99).contains(&across)
                    && (1..=99).contains(&down)
                    && u32::from(across) <= u32::from(down) * 4
                    && u32::from(down) <= u32::from(across) * 4
            }
        }
    }
    /// A shape from text such as `16:9`, `16x9`, `16/9` or `original`. `None` when it is not one.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("source") || text.eq_ignore_ascii_case("original") {
            return Some(Self::Source);
        }
        let (across, down) = text.split_once([':', 'x', 'X', '/'])?;
        let aspect = Self::Ratio(across.trim().parse().ok()?, down.trim().parse().ok()?);
        aspect.is_valid().then_some(aspect)
    }
}
impl Serialize for SequenceAspect {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Source => serializer.serialize_str("source"),
            Self::Ratio(..) => serializer.serialize_str(&self.label()),
        }
    }
}
impl<'de> Deserialize<'de> for SequenceAspect {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| {
            serde::de::Error::custom("a picture shape such as \"16:9\", between 1:4 and 4:1")
        })
    }
}

/// What happens when a clip is not the shape of the picture.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum SequenceFraming {
    /// The whole clip, in the middle, with dark bars where it does not fill the picture.
    #[default]
    #[serde(rename = "fit")]
    Fit,
    /// The whole clip in the middle, with a soft, darkened copy of it filling the rest.
    #[serde(rename = "blur")]
    Blur,
    /// The picture is filled: the clip is cropped on its long side, around the middle.
    #[serde(rename = "fill")]
    Fill,
    /// The picture is filled by pulling the clip out to every edge, without cropping it.
    #[serde(rename = "stretch")]
    Stretch,
}
impl SequenceFraming {
    pub const ALL: [SequenceFraming; 4] = [
        SequenceFraming::Fit,
        SequenceFraming::Blur,
        SequenceFraming::Fill,
        SequenceFraming::Stretch,
    ];
    pub fn name(self) -> &'static str {
        match self {
            SequenceFraming::Fit => "Fit",
            SequenceFraming::Blur => "Blur",
            SequenceFraming::Fill => "Fill",
            SequenceFraming::Stretch => "Stretch",
        }
    }
    /// One line on what it does, for a tooltip.
    pub fn about(self) -> &'static str {
        match self {
            SequenceFraming::Fit => "The whole clip in the middle, with dark bars around it",
            SequenceFraming::Blur => "The whole clip in the middle, with a blurred copy behind it",
            SequenceFraming::Fill => "Fills the picture by cropping the clip around its middle",
            SequenceFraming::Stretch => "Pulls the clip out to every edge, so it looks stretched",
        }
    }
}

/// Output motion is sampled at this rate; no interpolation or source-FPS guessing is performed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SequenceQuality {
    pub resolution: SequenceResolution,
    pub fps: u32,
    /// The shape of the picture.
    pub aspect: SequenceAspect,
    /// Bars or a crop where a clip is not that shape.
    pub framing: SequenceFraming,
}
impl Default for SequenceQuality {
    fn default() -> Self {
        Self {
            resolution: SequenceResolution::Hd720,
            fps: 30,
            aspect: SequenceAspect::Source,
            framing: SequenceFraming::Fit,
        }
    }
}
impl SequenceQuality {
    pub fn validate(self) -> Result<(), ExportError> {
        if !matches!(self.fps, 30 | 60) {
            return Err(ExportError::Unsupported(
                "choose 30 or 60 FPS for editor export",
            ));
        }
        if !self.aspect.is_valid() {
            return Err(ExportError::Unsupported(
                "choose a picture shape between 1:4 and 4:1",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SequenceSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub video_kbps: u32,
    pub resolution_limited: bool,
    pub bitrate_limited: bool,
}

fn fit(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    let (w, h) =
        if u64::from(width) * u64::from(max_height) > u64::from(height) * u64::from(max_width) {
            let w = width.min(max_width);
            (
                w,
                (u64::from(height) * u64::from(w) / u64::from(width)) as u32,
            )
        } else {
            let h = height.min(max_height);
            (
                (u64::from(width) * u64::from(h) / u64::from(height)) as u32,
                h,
            )
        };
    ((w / 2 * 2).max(2), (h / 2 * 2).max(2))
}

/// 720p/1080p retain the existing 16:9 canvas. Source follows the first clip, within a 4K
/// landscape/portrait box. Pictures are never upscaled. Size budgets may reduce this canvas;
/// the selected 30/60 FPS is retained, including when the bitrate is constrained.
pub fn sequence_settings(
    first_width: u32,
    first_height: u32,
    preset: Option<ExportPreset>,
    duration: Tick,
    quality: SequenceQuality,
) -> Result<SequenceSettings, ExportError> {
    quality.validate()?;
    source_size(first_width, first_height)?;
    if duration <= Tick::ZERO || duration > Tick::from_units(1800, 1) {
        return Err(ExportError::BadRange);
    }
    let requested = if let Some((across, down)) = quality.aspect.ratio() {
        // A chosen shape: the resolution says how long its short side is (720 or 1080, or the
        // clip's own short side for Source), never longer than the clip's own short side.
        let own_short = first_width.min(first_height);
        let short = match quality.resolution {
            SequenceResolution::Hd720 => own_short.min(720),
            SequenceResolution::FullHd1080 => own_short.min(1080),
            SequenceResolution::Source => own_short,
        }
        .max(2);
        let (w, h) = if across >= down {
            (short * across / down, short)
        } else {
            (short, short * down / across)
        };
        // Within the same pixel budget as everything else: 4K on the long side.
        let bounds = if across >= down {
            (3840, 2160)
        } else {
            (2160, 3840)
        };
        fit(w, h, bounds.0, bounds.1)
    } else {
        match quality.resolution {
            SequenceResolution::Source => {
                let bounds = if first_width >= first_height {
                    (3840, 2160)
                } else {
                    (2160, 3840)
                };
                fit(first_width, first_height, bounds.0, bounds.1)
            }
            resolution => {
                let cap = if resolution == SequenceResolution::Hd720 {
                    720
                } else {
                    1080
                };
                let h = (first_height.min(cap) / 2 * 2).max(2);
                ((h * 16 / 9 / 2 * 2).max(2), h)
            }
        }
    };
    // Keep the legacy baseline, scaling with pixels and motion for the higher-quality modes.
    let standard = (u64::from(requested.0) * u64::from(requested.1) * u64::from(quality.fps) * 4000
        / (1280 * 720 * 30))
        .clamp(4000, 50_000) as u32;
    let mut size = requested;
    let kbps = if let Some(preset) = preset {
        let scale = i128::from(clipcraft_time::TICKS_PER_SECOND);
        let audio_bytes = ((i128::from(duration.0) * 20_000 + scale - 1) / scale) as u64 + 2048;
        let kbps = video_budget_with_limit(preset.target_bytes()?, audio_bytes, duration, 50_000)?
            .min(standard);
        let density = u64::from(kbps) * 30 / u64::from(quality.fps);
        let cap = match density {
            16_000.. => 2160,
            9000.. => 1440,
            6000.. => 1080,
            2500.. => 720,
            900.. => 540,
            350.. => 480,
            _ => 360,
        };
        let bounds = if requested.0 >= requested.1 {
            (cap * 16 / 9, cap)
        } else {
            (cap, cap * 16 / 9)
        };
        size = if quality.resolution == SequenceResolution::Source
            || quality.aspect != SequenceAspect::Source
        {
            fit(requested.0, requested.1, bounds.0, bounds.1)
        } else {
            // Retain legacy even rounding (e.g. 852×480 rather than 852×478).
            let h = requested.1.min(cap) / 2 * 2;
            ((h * 16 / 9 / 2 * 2).max(2), h.max(2))
        };
        kbps
    } else {
        standard
    };
    Ok(SequenceSettings {
        width: size.0,
        height: size.1,
        fps: quality.fps,
        video_kbps: kbps,
        resolution_limited: size != requested,
        bitrate_limited: kbps < standard,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan(
        w: u32,
        h: u32,
        resolution: SequenceResolution,
        fps: u32,
        preset: Option<ExportPreset>,
        seconds: i64,
    ) -> SequenceSettings {
        sequence_settings(
            w,
            h,
            preset,
            Tick::from_units(seconds, 1),
            SequenceQuality {
                resolution,
                fps,
                ..Default::default()
            },
        )
        .unwrap()
    }
    #[test]
    fn requested_quality_preserves_legacy_and_native_framing_with_bounded_output() {
        let legacy = plan(1920, 1080, SequenceResolution::Hd720, 30, None, 60);
        assert_eq!(
            (legacy.width, legacy.height, legacy.video_kbps),
            (1280, 720, 4000)
        );
        let full = plan(1920, 1080, SequenceResolution::FullHd1080, 60, None, 60);
        assert_eq!(
            (full.width, full.height, full.fps, full.video_kbps),
            (1920, 1080, 60, 18_000)
        );
        for (input, expected) in [
            ((3440, 1440), (3440, 1440)),
            ((1080, 1920), (1080, 1920)),
            ((7680, 4320), (3840, 2160)),
            ((4320, 7680), (2160, 3840)),
            ((160, 90), (160, 90)),
        ] {
            let s = plan(input.0, input.1, SequenceResolution::Source, 60, None, 60);
            assert_eq!((s.width, s.height), expected);
            assert!(u64::from(s.width) * u64::from(s.height) <= 8_294_400);
            assert!(!s.resolution_limited);
        }
        let small = plan(640, 360, SequenceResolution::FullHd1080, 60, None, 60);
        assert_eq!((small.width, small.height), (640, 360));
    }
    #[test]
    fn size_targets_reduce_pixels_keep_motion_and_reserve_sound() {
        let limited = plan(
            3840,
            2160,
            SequenceResolution::Source,
            60,
            Some(ExportPreset::Custom { megabytes: 10 }),
            60,
        );
        assert!(limited.resolution_limited);
        assert_eq!(limited.fps, 60);
        assert!(limited.height < 2160 && limited.video_kbps < 1500);
        let roomy = plan(
            1920,
            1080,
            SequenceResolution::FullHd1080,
            60,
            Some(ExportPreset::Medium),
            1,
        );
        assert_eq!((roomy.width, roomy.height), (1920, 1080));
        assert!(!roomy.resolution_limited);
        assert!(!roomy.bitrate_limited);
        let reduced_bitrate = plan(
            1920,
            1080,
            SequenceResolution::FullHd1080,
            60,
            Some(ExportPreset::Medium),
            10,
        );
        assert!(reduced_bitrate.bitrate_limited && !reduced_bitrate.resolution_limited);
        for fps in [0, 1, 24, 59, 120, u32::MAX] {
            assert!(
                sequence_settings(
                    1920,
                    1080,
                    None,
                    Tick::from_units(1, 1),
                    SequenceQuality {
                        resolution: SequenceResolution::Source,
                        fps,
                        ..Default::default()
                    }
                )
                .is_err()
            );
        }
        assert!(
            sequence_settings(
                u32::MAX,
                1080,
                None,
                Tick::from_units(1, 1),
                SequenceQuality::default()
            )
            .is_err()
        );
        assert!(
            sequence_settings(
                1920,
                1080,
                Some(ExportPreset::Custom { megabytes: 1 }),
                Tick::from_units(1800, 1),
                SequenceQuality::default()
            )
            .is_err()
        );
    }

    fn shaped(
        w: u32,
        h: u32,
        resolution: SequenceResolution,
        aspect: SequenceAspect,
    ) -> (u32, u32) {
        let s = sequence_settings(
            w,
            h,
            None,
            Tick::from_units(10, 1),
            SequenceQuality {
                resolution,
                aspect,
                ..Default::default()
            },
        )
        .unwrap();
        (s.width, s.height)
    }

    #[test]
    fn a_chosen_shape_sets_the_output_from_its_short_side_and_stays_within_bounds() {
        use SequenceResolution::{FullHd1080, Hd720, Source};
        // 16:9 source, each shape at each tier: the tier is the short side.
        for (aspect, hd720, hd1080) in [
            (SequenceAspect::WIDE, (1280, 720), (1920, 1080)),
            (SequenceAspect::VERTICAL, (720, 1280), (1080, 1920)),
            (SequenceAspect::SQUARE, (720, 720), (1080, 1080)),
            (SequenceAspect::PORTRAIT, (720, 900), (1080, 1350)),
            (SequenceAspect::Ratio(21, 9), (1680, 720), (2520, 1080)),
            (SequenceAspect::Ratio(4, 3), (960, 720), (1440, 1080)),
            (SequenceAspect::Ratio(3, 2), (1080, 720), (1620, 1080)),
            (SequenceAspect::Ratio(5, 4), (900, 720), (1350, 1080)),
            (SequenceAspect::Ratio(2, 3), (720, 1080), (1080, 1620)),
            (SequenceAspect::Ratio(3, 4), (720, 960), (1080, 1440)),
            (SequenceAspect::Ratio(9, 21), (720, 1680), (1080, 2520)),
        ] {
            assert_eq!(shaped(1920, 1080, Hd720, aspect), hd720, "{aspect:?}");
            assert_eq!(shaped(1920, 1080, FullHd1080, aspect), hd1080, "{aspect:?}");
        }
        // Never longer on the short side than the clip itself, and always even.
        assert_eq!(
            shaped(640, 360, FullHd1080, SequenceAspect::VERTICAL),
            (360, 640)
        );
        assert_eq!(
            shaped(1280, 720, FullHd1080, SequenceAspect::SQUARE),
            (720, 720)
        );
        for aspect in SequenceAspect::PRESETS {
            for (w, h) in [
                (1920, 1080),
                (3840, 2160),
                (7680, 4320),
                (1080, 1920),
                (320, 240),
            ] {
                for resolution in [Hd720, FullHd1080, Source] {
                    let (width, height) = shaped(w, h, resolution, aspect);
                    assert!(width >= 2 && height >= 2 && width % 2 == 0 && height % 2 == 0);
                    assert!(width <= MAX_OUTPUT_EDGE && height <= MAX_OUTPUT_EDGE);
                    assert!(u64::from(width) * u64::from(height) <= MAX_OUTPUT_PIXELS);
                }
            }
        }
        // The original shape keeps what it always did.
        assert_eq!(
            shaped(1920, 1080, Hd720, SequenceAspect::Source),
            (1280, 720)
        );
        assert_eq!(
            shaped(1080, 1920, Source, SequenceAspect::Source),
            (1080, 1920)
        );
        // With a size target the shape is kept and the picture only gets smaller.
        let small = sequence_settings(
            1920,
            1080,
            Some(ExportPreset::Small),
            Tick::from_units(60, 1),
            SequenceQuality {
                aspect: SequenceAspect::VERTICAL,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(small.height > small.width, "{small:?}");
        assert!(small.width <= 720 && small.height <= 1280);
    }

    #[test]
    fn shapes_are_parsed_saved_and_limited() {
        for (text, expected) in [
            ("16:9", Some(SequenceAspect::WIDE)),
            ("9x16", Some(SequenceAspect::VERTICAL)),
            (" 1/1 ", Some(SequenceAspect::SQUARE)),
            ("4 : 5", Some(SequenceAspect::PORTRAIT)),
            ("original", Some(SequenceAspect::Source)),
            ("Source", Some(SequenceAspect::Source)),
            ("1:5", None),
            ("5:1", None),
            ("0:9", None),
            ("100:9", None),
            ("16", None),
            ("", None),
            ("a:b", None),
            ("-1:2", None),
            ("16:9:1", None),
        ] {
            assert_eq!(SequenceAspect::parse(text), expected, "{text:?}");
        }
        // Every shape that is offered is valid and survives a save as text.
        for aspect in SequenceAspect::PRESETS {
            assert!(aspect.is_valid(), "{aspect:?}");
            let text = serde_json::to_string(&aspect).unwrap();
            assert_eq!(
                serde_json::from_str::<SequenceAspect>(&text).unwrap(),
                aspect
            );
        }
        assert_eq!(
            serde_json::to_string(&SequenceAspect::VERTICAL).unwrap(),
            "\"9:16\""
        );
        assert_eq!(
            serde_json::to_string(&SequenceAspect::Source).unwrap(),
            "\"source\""
        );
        assert!(serde_json::from_str::<SequenceAspect>("\"7:1\"").is_err());
        assert!(serde_json::from_str::<SequenceAspect>("3").is_err());
        // A shape that cannot be rendered is refused before any work starts.
        assert!(
            SequenceQuality {
                aspect: SequenceAspect::Ratio(9, 99),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        // Old saved quality (no shape or framing) loads as it always did.
        let old: SequenceQuality =
            serde_json::from_str(r#"{"resolution":"1080p","fps":60}"#).unwrap();
        assert_eq!(old.aspect, SequenceAspect::Source);
        assert_eq!(old.framing, SequenceFraming::Fit);
        let new: SequenceQuality = serde_json::from_str(
            r#"{"resolution":"720p","fps":30,"aspect":"9:16","framing":"blur"}"#,
        )
        .unwrap();
        assert_eq!(new.aspect, SequenceAspect::VERTICAL);
        assert_eq!(new.framing, SequenceFraming::Blur);
        assert!(serde_json::from_str::<SequenceQuality>(r#"{"shape":"9:16"}"#).is_err());
    }
}
