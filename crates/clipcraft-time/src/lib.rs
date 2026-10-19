//! Exact time primitives shared by ClipCraft media and edit code.
//!
//! ClipCraft uses FilmCraft's clock so clips and edits interoperate with FilmCraft crates: time is
//! an integer [`Tick`] at [`TICKS_PER_SECOND`] (254 016 000 000 per second), a rate that divides
//! evenly into every broadcast frame duration and every common audio sample duration. Edit math
//! never uses floating-point seconds. This crate adds validated constructors for untrusted input
//! and audio sample positions on top of that clock.

use std::fmt;

pub use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

/// Builds a frame rate from untrusted numbers, rejecting zero instead of silently defaulting.
pub fn checked_frame_rate(numerator: u32, denominator: u32) -> Result<FrameRate, TimeError> {
    if numerator == 0 {
        return Err(TimeError::ZeroFrameRateNumerator);
    }
    if denominator == 0 {
        return Err(TimeError::ZeroFrameRateDenominator);
    }
    Ok(FrameRate::new(i64::from(numerator), i64::from(denominator)))
}

/// An audio sample rate in hertz.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SampleRate(u32);

impl SampleRate {
    /// Creates a validated, non-zero sample rate.
    pub fn new(hertz: u32) -> Result<Self, TimeError> {
        if hertz == 0 {
            return Err(TimeError::ZeroSampleRate);
        }
        Ok(Self(hertz))
    }

    /// Returns the rate in samples per second.
    pub const fn hertz(self) -> u32 {
        self.0
    }

    /// Converts a zero-based sample index to the timestamp at its start.
    ///
    /// Computed from the index each time, so errors never accumulate across a recording.
    pub fn sample_start(self, sample_index: i64) -> Tick {
        Tick::from_units(sample_index, i64::from(self.0))
    }

    /// Returns the zero-based sample whose start is at or before `tick`.
    pub fn sample_at_or_before(self, tick: Tick) -> i64 {
        tick.to_units_floor(i64::from(self.0))
    }
}

/// Errors returned when validating untrusted time values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeError {
    /// An audio sample rate must be non-zero.
    ZeroSampleRate,
    /// A frame-rate numerator must be non-zero.
    ZeroFrameRateNumerator,
    /// A frame-rate denominator must be non-zero.
    ZeroFrameRateDenominator,
}

impl fmt::Display for TimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroSampleRate => formatter.write_str("audio sample rate must be positive"),
            Self::ZeroFrameRateNumerator => {
                formatter.write_str("frame-rate numerator must be positive")
            }
            Self::ZeroFrameRateDenominator => {
                formatter.write_str("frame-rate denominator must be positive")
            }
        }
    }
}

impl std::error::Error for TimeError {}

#[cfg(test)]
mod tests {
    use super::{FrameRate, SampleRate, TICKS_PER_SECOND, Tick, TimeError, checked_frame_rate};

    #[test]
    fn invalid_rates_are_errors() {
        assert_eq!(
            checked_frame_rate(0, 1),
            Err(TimeError::ZeroFrameRateNumerator)
        );
        assert_eq!(
            checked_frame_rate(1, 0),
            Err(TimeError::ZeroFrameRateDenominator)
        );
        assert_eq!(SampleRate::new(0), Err(TimeError::ZeroSampleRate));
        assert_eq!(checked_frame_rate(60_000, 1_001), Ok(FrameRate::FPS_59_94));
    }

    #[test]
    fn clock_is_exact_for_fractional_rates() {
        // One frame of 59.94 fps is an exact whole number of ticks, so nothing rounds.
        let rate = FrameRate::FPS_59_94;
        assert_eq!(rate.tick_of(60_000).0, 1_001 * TICKS_PER_SECOND);
        assert_eq!(rate.frame_at(rate.tick_of(60_000)), 60_000);
        assert_eq!(rate.frame_duration().0 * 60_000, 1_001 * TICKS_PER_SECOND);
    }

    #[test]
    fn sample_rate_round_trips() {
        let rate = SampleRate::new(48_000).expect("valid rate");
        assert_eq!(rate.sample_start(48_000), Tick(TICKS_PER_SECOND));
        assert_eq!(rate.sample_at_or_before(Tick(TICKS_PER_SECOND)), 48_000);
        // Hostile extremes saturate instead of panicking or wrapping.
        let _ = rate.sample_start(i64::MAX);
        let _ = rate.sample_at_or_before(Tick(i64::MAX));
        let _ = rate.sample_start(i64::MIN);
    }

    /// M0 exit check: a synthetic video stream and audio stream share one clock without drift.
    #[test]
    fn synthetic_video_and_audio_streams_stay_aligned_for_an_hour() {
        let video = FrameRate::FPS_59_94;
        let audio = SampleRate::new(48_000).expect("valid sample rate");
        let one_sample = TICKS_PER_SECOND / i64::from(audio.hertz()) + 1;
        let frames_in_an_hour = 60 * 60 * 60_000 / 1_001;

        let mut previous = Tick::ZERO;
        for frame in (0..=frames_in_an_hour).step_by(997) {
            let video_time = video.tick_of(frame);
            assert!(video_time >= previous, "video time must be monotonic");
            previous = video_time;

            let sample = audio.sample_at_or_before(video_time);
            let skew = video_time - audio.sample_start(sample);
            assert!(
                skew >= Tick::ZERO && skew.0 < one_sample,
                "skew {skew:?} outside one sample"
            );
            assert_eq!(video.frame_at(video_time), frame);
        }
    }

    #[test]
    fn common_frame_and_sample_rates_round_trip_and_stay_monotonic() {
        for rate in FrameRate::COMMON {
            let mut previous = Tick::ZERO;
            for frame in 0..20_000 {
                let start = rate.tick_of(frame);
                assert!(frame == 0 || start > previous, "{rate} not monotonic");
                assert_eq!(rate.frame_at(start), frame);
                // One tick before a frame start belongs to the previous frame (trim boundary).
                if frame > 0 {
                    assert_eq!(rate.frame_at(start - Tick(1)), frame - 1);
                }
                previous = start;
            }
        }
        for hertz in [8_000, 22_050, 44_100, 48_000, 96_000, 192_000] {
            let rate = SampleRate::new(hertz).expect("valid sample rate");
            for sample in (0..2_000_000).step_by(997) {
                assert_eq!(rate.sample_at_or_before(rate.sample_start(sample)), sample);
            }
        }
    }
}
