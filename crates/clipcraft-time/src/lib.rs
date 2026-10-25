//! Exact time primitives shared by ClipCraft media and edit code.
//!
//! ClipCraft uses FilmCraft's clock so clips and edits interoperate with FilmCraft crates: time is
//! an integer [`Tick`] at [`TICKS_PER_SECOND`] (254 016 000 000 per second), a rate that divides
//! evenly into every broadcast frame duration and every common audio sample duration. Edit math
//! never uses floating-point seconds. This crate supplies the shared FilmCraft clock types.

pub use filmcraft_time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};

#[cfg(test)]
mod tests {
    use super::{FrameRate, TICKS_PER_SECOND, Tick};

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
        assert_eq!(Tick::from_units(48_000, 48_000), Tick(TICKS_PER_SECOND));
        assert_eq!(Tick(TICKS_PER_SECOND).to_units_floor(48_000), 48_000);
        // Hostile extremes saturate instead of panicking or wrapping.
        let _ = Tick::from_units(i64::MAX, 48_000);
        let _ = Tick(i64::MAX).to_units_floor(48_000);
        let _ = Tick::from_units(i64::MIN, 48_000);
    }

    /// M0 exit check: a synthetic video stream and audio stream share one clock without drift.
    #[test]
    fn synthetic_video_and_audio_streams_stay_aligned_for_an_hour() {
        let video = FrameRate::FPS_59_94;
        let hertz = 48_000;
        let one_sample = TICKS_PER_SECOND / hertz + 1;
        let frames_in_an_hour = 60 * 60 * 60_000 / 1_001;

        let mut previous = Tick::ZERO;
        for frame in (0..=frames_in_an_hour).step_by(997) {
            let video_time = video.tick_of(frame);
            assert!(video_time >= previous, "video time must be monotonic");
            previous = video_time;

            let sample = video_time.to_units_floor(hertz);
            let skew = video_time - Tick::from_units(sample, hertz);
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
            for sample in (0..2_000_000).step_by(997) {
                assert_eq!(
                    Tick::from_units(sample, hertz).to_units_floor(hertz),
                    sample
                );
            }
        }
    }
}
