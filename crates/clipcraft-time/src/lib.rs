//! Integer and rational time primitives shared by ClipCraft media and edit code.

use std::fmt;

/// Number of integer ticks in one second.
pub const TICKS_PER_SECOND: u64 = 1_000_000_000;

/// A non-negative media timestamp measured in nanosecond ticks.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Tick(u64);

impl Tick {
    /// The zero timestamp.
    pub const ZERO: Self = Self(0);

    /// Creates a timestamp from a tick count.
    pub const fn from_ticks(ticks: u64) -> Self {
        Self(ticks)
    }

    /// Returns the integer tick count.
    pub const fn ticks(self) -> u64 {
        self.0
    }

    /// Converts to seconds for display only.
    pub fn as_seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// Adds a duration, returning an error on overflow.
    pub fn checked_add(self, duration: Self) -> Result<Self, TimeError> {
        self.0
            .checked_add(duration.0)
            .map(Self)
            .ok_or(TimeError::Overflow)
    }

    /// Subtracts a timestamp, returning an error if the result would be negative.
    pub fn checked_sub(self, earlier: Self) -> Result<Self, TimeError> {
        self.0
            .checked_sub(earlier.0)
            .map(Self)
            .ok_or(TimeError::Underflow)
    }
}

/// A frame rate represented as a positive rational number of frames per second.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameRate {
    numerator: u32,
    denominator: u32,
}

impl FrameRate {
    /// Creates a validated rational frame rate.
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, TimeError> {
        if numerator == 0 {
            return Err(TimeError::ZeroFrameRateNumerator);
        }
        if denominator == 0 {
            return Err(TimeError::ZeroFrameRateDenominator);
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    /// Returns the numerator in frames per second.
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    /// Returns the denominator in frames per second.
    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    /// Converts a zero-based frame index to the timestamp at its start.
    ///
    /// The conversion rounds up to the next nanosecond tick and never uses floating-point math.
    /// Rounding up keeps `frame_at_or_before(frame_start(n)) == n` for fractional rates; rounding
    /// down would map a frame's own start back to the previous frame.
    pub fn frame_start(self, frame_index: u64) -> Result<Tick, TimeError> {
        let scaled = u128::from(frame_index)
            .checked_mul(u128::from(self.denominator))
            .and_then(|value| value.checked_mul(u128::from(TICKS_PER_SECOND)))
            .ok_or(TimeError::Overflow)?;
        let ticks = scaled.div_ceil(u128::from(self.numerator));
        let ticks = u64::try_from(ticks).map_err(|_| TimeError::Overflow)?;
        Ok(Tick::from_ticks(ticks))
    }

    /// Returns the zero-based frame whose start is at or before `tick`.
    pub fn frame_at_or_before(self, tick: Tick) -> Result<u64, TimeError> {
        let scaled = u128::from(tick.ticks())
            .checked_mul(u128::from(self.numerator))
            .ok_or(TimeError::Overflow)?;
        let divisor = u128::from(TICKS_PER_SECOND)
            .checked_mul(u128::from(self.denominator))
            .ok_or(TimeError::Overflow)?;
        let frame = scaled / divisor;
        u64::try_from(frame).map_err(|_| TimeError::Overflow)
    }
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
    /// Computed from the index each time, so errors never accumulate across a recording. Rounds up
    /// to the next tick so `sample_at_or_before(sample_start(n)) == n`.
    pub fn sample_start(self, sample_index: u64) -> Result<Tick, TimeError> {
        let scaled = u128::from(sample_index)
            .checked_mul(u128::from(TICKS_PER_SECOND))
            .ok_or(TimeError::Overflow)?;
        let ticks =
            u64::try_from(scaled.div_ceil(u128::from(self.0))).map_err(|_| TimeError::Overflow)?;
        Ok(Tick::from_ticks(ticks))
    }

    /// Returns the zero-based sample whose start is at or before `tick`.
    pub fn sample_at_or_before(self, tick: Tick) -> Result<u64, TimeError> {
        let scaled = u128::from(tick.ticks())
            .checked_mul(u128::from(self.0))
            .ok_or(TimeError::Overflow)?;
        u64::try_from(scaled / u128::from(TICKS_PER_SECOND)).map_err(|_| TimeError::Overflow)
    }
}

/// Errors returned by checked time operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeError {
    /// An audio sample rate must be non-zero.
    ZeroSampleRate,
    /// A frame-rate numerator must be non-zero.
    ZeroFrameRateNumerator,
    /// A frame-rate denominator must be non-zero.
    ZeroFrameRateDenominator,
    /// The requested result is larger than the supported integer range.
    Overflow,
    /// A timestamp subtraction would produce a negative value.
    Underflow,
}

impl fmt::Display for TimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroFrameRateNumerator => {
                formatter.write_str("frame-rate numerator must be positive")
            }
            Self::ZeroFrameRateDenominator => {
                formatter.write_str("frame-rate denominator must be positive")
            }
            Self::ZeroSampleRate => formatter.write_str("audio sample rate must be positive"),
            Self::Overflow => formatter.write_str("time value exceeds the supported range"),
            Self::Underflow => formatter.write_str("time subtraction would be negative"),
        }
    }
}

impl std::error::Error for TimeError {}

#[cfg(test)]
mod tests {
    use super::{FrameRate, SampleRate, TICKS_PER_SECOND, Tick, TimeError};

    #[test]
    fn integer_frame_rate_round_trips_frame_starts() {
        let rate = FrameRate::new(60, 1).expect("valid frame rate");
        let start = rate.frame_start(600).expect("frame time fits");
        assert_eq!(start.ticks(), 10 * TICKS_PER_SECOND);
        assert_eq!(rate.frame_at_or_before(start).expect("frame fits"), 600);
    }

    #[test]
    fn fractional_frame_rate_uses_rational_math() {
        let rate = FrameRate::new(30_000, 1_001).expect("valid frame rate");
        let start = rate.frame_start(30_000).expect("frame time fits");
        assert_eq!(start.ticks(), 1_001 * TICKS_PER_SECOND);
        assert_eq!(rate.frame_at_or_before(start).expect("frame fits"), 30_000);
    }

    #[test]
    fn fractional_frame_starts_map_back_to_their_own_frame() {
        // Regression: frame 1 of 59.94 fps starts at 16_683_333.33 ns. Rounding down to
        // 16_683_333 made frame_at_or_before return frame 0, so a trim at that frame was one
        // frame early.
        let rate = FrameRate::new(60_000, 1_001).expect("valid frame rate");
        let start = rate.frame_start(1).expect("frame time fits");
        assert_eq!(start.ticks(), 16_683_334);
        assert_eq!(rate.frame_at_or_before(start), Ok(1));
        for frame in 0..10_000 {
            let start = rate.frame_start(frame).expect("frame time fits");
            assert_eq!(rate.frame_at_or_before(start), Ok(frame));
        }
    }

    #[test]
    fn invalid_rates_and_overflow_are_errors() {
        assert_eq!(FrameRate::new(0, 1), Err(TimeError::ZeroFrameRateNumerator));
        assert_eq!(
            FrameRate::new(1, 0),
            Err(TimeError::ZeroFrameRateDenominator)
        );
        let rate = FrameRate::new(1, u32::MAX).expect("valid frame rate");
        assert_eq!(rate.frame_start(u64::MAX), Err(TimeError::Overflow));
    }

    #[test]
    fn tick_arithmetic_checks_bounds() {
        assert_eq!(
            Tick::from_ticks(5).checked_sub(Tick::from_ticks(6)),
            Err(TimeError::Underflow)
        );
        assert_eq!(
            Tick::from_ticks(u64::MAX).checked_add(Tick::from_ticks(1)),
            Err(TimeError::Overflow)
        );
    }

    #[test]
    fn sample_rate_round_trips_and_rejects_zero() {
        let rate = SampleRate::new(48_000).expect("valid rate");
        assert_eq!(SampleRate::new(0), Err(TimeError::ZeroSampleRate));
        assert_eq!(
            rate.sample_start(48_000),
            Ok(Tick::from_ticks(TICKS_PER_SECOND))
        );
        assert_eq!(
            rate.sample_at_or_before(Tick::from_ticks(TICKS_PER_SECOND)),
            Ok(48_000)
        );
        assert_eq!(rate.sample_start(u64::MAX), Err(TimeError::Overflow));
    }

    /// M0 exit check: a synthetic video stream and audio stream share one clock without drift.
    #[test]
    fn synthetic_video_and_audio_streams_stay_aligned_for_an_hour() {
        // 59.94 fps video with 48 kHz audio: the hardest common pairing for integer clocks.
        let video = FrameRate::new(60_000, 1_001).expect("valid frame rate");
        let audio = SampleRate::new(48_000).expect("valid sample rate");
        let one_sample = TICKS_PER_SECOND / u64::from(audio.hertz()) + 1;
        let frames_in_an_hour = 60 * 60 * 60_000 / 1_001;

        let mut previous = Tick::ZERO;
        for frame in (0..=frames_in_an_hour).step_by(997) {
            let video_time = video.frame_start(frame).expect("frame time");
            assert!(video_time >= previous, "video time must be monotonic");
            previous = video_time;

            // Audio sample covering this frame's start, converted back to the shared clock.
            let sample = audio.sample_at_or_before(video_time).expect("sample index");
            let audio_time = audio.sample_start(sample).expect("sample time");
            let skew = video_time
                .checked_sub(audio_time)
                .expect("audio never leads");
            assert!(
                skew.ticks() < one_sample,
                "skew {skew:?} exceeds one sample"
            );
            assert_eq!(video.frame_at_or_before(video_time), Ok(frame));
        }
    }
}
