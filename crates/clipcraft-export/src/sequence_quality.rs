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

/// Output motion is sampled at this rate; no interpolation or source-FPS guessing is performed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SequenceQuality {
    pub resolution: SequenceResolution,
    pub fps: u32,
}
impl Default for SequenceQuality {
    fn default() -> Self {
        Self {
            resolution: SequenceResolution::Hd720,
            fps: 30,
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
    let requested = match quality.resolution {
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
        size = if quality.resolution == SequenceResolution::Source {
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
            SequenceQuality { resolution, fps },
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
                        fps
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
}
