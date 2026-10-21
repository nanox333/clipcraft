//! Timestamp alignment policy for native audio-capture adapters.
//!
//! This is a Rust adaptation of ClipRecorder's `src/audio_timeline.h` at
//! commit `153025b4c88ed42a23a97e030ce140c2fc54949d`, used with permission
//! reported by the ClipCraft maintainer. It handles timestamp gaps, overlap,
//! and microphone recovery; the existing cross-platform `AudioEngine` remains
//! responsible for mixing untimestamped `AudioSource` samples.

const TICKS_PER_SECOND: i64 = 10_000_000;
const DEFAULT_MICROPHONE_GRACE_100NS: i64 = 1_250_000;
const DEFAULT_LOOPBACK_TOLERANCE_100NS: i64 = 200_000;

/// The result of aligning one microphone block with the system-audio clock.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MicTimelineDecision {
    /// Microphone frames that can be mixed now.
    pub mix_frames: usize,
    /// Frames to pad with silence after a missing microphone interval.
    pub silence_frames: usize,
    /// The grace period expired or the device was unavailable.
    pub entered_missing: bool,
    /// A microphone packet arrived after a missing interval.
    pub resumed: bool,
}

/// Stateful policy for aligning timestamped loopback and microphone packets.
#[derive(Clone, Debug)]
pub struct AudioTimelinePolicy {
    sample_rate: u32,
    microphone_grace_100ns: i64,
    loopback_tolerance_100ns: i64,
    microphone_gap_start_100ns: Option<i64>,
    microphone_missing: bool,
}

impl AudioTimelinePolicy {
    /// Creates a policy for `sample_rate` Hz audio with a 125 ms microphone grace
    /// period and 20 ms loopback tolerance.
    pub fn new(sample_rate: u32) -> Self {
        Self::with_timing(
            sample_rate,
            DEFAULT_MICROPHONE_GRACE_100NS,
            DEFAULT_LOOPBACK_TOLERANCE_100NS,
        )
    }

    /// Creates a policy with explicit microphone grace and loopback tolerance.
    pub fn with_timing(
        sample_rate: u32,
        microphone_grace_100ns: i64,
        loopback_tolerance_100ns: i64,
    ) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            microphone_grace_100ns: microphone_grace_100ns.max(0),
            loopback_tolerance_100ns: loopback_tolerance_100ns.max(0),
            microphone_gap_start_100ns: None,
            microphone_missing: false,
        }
    }

    /// Converts sample frames to 100-nanosecond ticks, rounding down.
    pub fn frames_to_100ns(&self, frames: usize) -> i64 {
        let ticks = (frames as u128).saturating_mul(TICKS_PER_SECOND as u128)
            / u128::from(self.sample_rate);
        i64::try_from(ticks).unwrap_or(i64::MAX)
    }

    /// Frames missing from a loopback timeline after queued audio and tolerance expire,
    /// limited to one second per poll.
    pub fn gap_frames(
        &self,
        now_100ns: i64,
        next_audio_100ns: i64,
        audio_frames: usize,
        loopback_frames: usize,
        timeline_started: bool,
        received_loopback_packet: bool,
    ) -> usize {
        if !timeline_started || received_loopback_packet {
            return 0;
        }
        let queued_frames = audio_frames.saturating_add(loopback_frames);
        let queued_end = next_audio_100ns.saturating_add(self.frames_to_100ns(queued_frames));
        if now_100ns <= queued_end.saturating_add(self.loopback_tolerance_100ns) {
            return 0;
        }
        ticks_to_frames(now_100ns.saturating_sub(queued_end), self.sample_rate)
            .min(self.sample_rate as usize)
    }

    /// Missing frames between an expected packet timestamp and a later valid timestamp.
    pub fn packet_gap_frames(
        &self,
        expected_packet_100ns: i64,
        packet_100ns: i64,
        timestamp_valid: bool,
    ) -> usize {
        if !timestamp_valid || expected_packet_100ns < 0 || packet_100ns <= expected_packet_100ns {
            return 0;
        }
        ticks_to_frames(
            packet_100ns.saturating_sub(expected_packet_100ns),
            self.sample_rate,
        )
    }

    /// How many frames of a packet overlap audio already padded through `padded_through_100ns`.
    pub fn overlap_frames(
        &self,
        packet_start_100ns: i64,
        padded_through_100ns: i64,
        packet_frames: usize,
    ) -> usize {
        if packet_start_100ns < 0 || padded_through_100ns <= packet_start_100ns {
            return 0;
        }
        ticks_to_frames(
            padded_through_100ns.saturating_sub(packet_start_100ns),
            self.sample_rate,
        )
        .min(packet_frames)
    }

    /// Aligns one microphone block to loopback, applying the missing-device grace period.
    pub fn decide_microphone(
        &mut self,
        now_100ns: i64,
        loopback_frames: usize,
        microphone_frames: usize,
        received_microphone_packet: bool,
        microphone_available: bool,
    ) -> MicTimelineDecision {
        let mut decision = MicTimelineDecision::default();
        if received_microphone_packet {
            decision.resumed = self.microphone_missing;
            self.microphone_missing = false;
            self.microphone_gap_start_100ns = None;
        }

        if microphone_frames >= loopback_frames {
            decision.mix_frames = loopback_frames;
            return decision;
        }

        if !microphone_available {
            if !self.microphone_missing {
                decision.entered_missing = true;
            }
            self.microphone_missing = true;
        } else if !received_microphone_packet && !self.microphone_missing {
            let gap_start = *self.microphone_gap_start_100ns.get_or_insert(now_100ns);
            if now_100ns.saturating_sub(gap_start) >= self.microphone_grace_100ns {
                self.microphone_missing = true;
                decision.entered_missing = true;
            }
        }

        if self.microphone_missing {
            decision.mix_frames = loopback_frames;
            decision.silence_frames = loopback_frames.saturating_sub(microphone_frames);
        } else {
            decision.mix_frames = microphone_frames;
        }
        decision
    }

    /// Whether the microphone is currently in a missing interval.
    pub fn microphone_missing(&self) -> bool {
        self.microphone_missing
    }
}

fn ticks_to_frames(ticks: i64, sample_rate: u32) -> usize {
    if ticks <= 0 {
        return 0;
    }
    let frames = (ticks as u128).saturating_mul(u128::from(sample_rate)) / TICKS_PER_SECOND as u128;
    usize::try_from(frames).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const GRACE: i64 = 1_250_000;
    const TOLERANCE: i64 = 200_000;

    fn policy() -> AudioTimelinePolicy {
        AudioTimelinePolicy::with_timing(RATE, GRACE, TOLERANCE)
    }

    #[test]
    fn converts_sample_frames_to_100ns_ticks() {
        assert_eq!(policy().frames_to_100ns(RATE as usize), TICKS_PER_SECOND);
        assert_eq!(
            AudioTimelinePolicy::new(RATE).frames_to_100ns(RATE as usize),
            TICKS_PER_SECOND
        );
    }

    #[test]
    fn loopback_gaps_wait_until_the_tolerance_expires() {
        let policy = policy();
        assert_eq!(policy.gap_frames(300_000, 0, 480, 0, true, false), 0);
        assert_eq!(policy.gap_frames(400_000, 0, 480, 0, true, false), 1_440);
        assert_eq!(policy.gap_frames(3_000_000, 0, 480, 0, false, false), 0);
        assert_eq!(policy.gap_frames(3_000_000, 0, 480, 0, true, true), 0);
    }

    #[test]
    fn queued_audio_and_loopback_packets_prevent_duplicate_silence() {
        let policy = policy();
        assert_eq!(
            policy.gap_frames(11_000_000, 10_000_000, 3_840, 960, true, false),
            0
        );
        assert_eq!(
            policy.gap_frames(11_000_000, 10_000_000, 0, 4_800, true, true),
            0
        );
        assert_eq!(
            policy.gap_frames(11_000_000, 10_000_000, 0, 0, true, false),
            4_800
        );
        assert_eq!(
            policy.gap_frames(100_000_000, 0, 0, 0, true, false),
            RATE as usize
        );
    }

    #[test]
    fn packet_gaps_and_overlaps_respect_timestamp_validity_and_packet_bounds() {
        let policy = policy();
        assert_eq!(policy.packet_gap_frames(1_000_000, 1_500_000, true), 2_400);
        assert_eq!(policy.packet_gap_frames(1_500_000, 1_000_000, true), 0);
        assert_eq!(policy.packet_gap_frames(1_000_000, 1_500_000, false), 0);
        assert_eq!(policy.overlap_frames(1_000_000, 1_250_000, 2_000), 1_200);
        assert_eq!(policy.overlap_frames(1_000_000, 2_000_000, 1_000), 1_000);
        assert_eq!(policy.overlap_frames(2_000_000, 1_000_000, 1_000), 0);
    }

    #[test]
    fn microphone_grace_pads_silence_and_a_later_packet_resumes() {
        let mut policy = policy();
        let first = policy.decide_microphone(1_000_000, 480, 0, false, true);
        assert_eq!(first.mix_frames, 0);
        assert!(!first.entered_missing);

        let expired = policy.decide_microphone(2_250_000, 480, 0, false, true);
        assert_eq!(expired.mix_frames, 480);
        assert_eq!(expired.silence_frames, 480);
        assert!(expired.entered_missing);
        assert!(policy.microphone_missing());

        let resumed = policy.decide_microphone(2_300_000, 480, 480, true, true);
        assert_eq!(resumed.mix_frames, 480);
        assert!(resumed.resumed);
        assert!(!policy.microphone_missing());
    }

    #[test]
    fn microphone_jitter_stays_in_grace_until_a_packet_arrives() {
        let mut policy = policy();
        let delayed = policy.decide_microphone(100_000, 480, 0, false, true);
        assert_eq!(delayed.mix_frames, 0);
        assert_eq!(delayed.silence_frames, 0);
        assert!(!delayed.entered_missing);

        let arrived = policy.decide_microphone(200_000, 960, 480, true, true);
        assert_eq!(arrived.mix_frames, 480);
        assert_eq!(arrived.silence_frames, 0);
        assert!(!arrived.resumed);
        assert!(!policy.microphone_missing());
    }

    #[test]
    fn an_unavailable_microphone_enters_missing_immediately() {
        let mut policy = policy();
        let decision = policy.decide_microphone(0, 256, 0, false, false);

        assert!(decision.entered_missing);
        assert_eq!(decision.mix_frames, 256);
        assert_eq!(decision.silence_frames, 256);
        assert!(policy.microphone_missing());
    }

    #[test]
    fn a_full_microphone_block_is_mixed_without_padding() {
        let mut policy = policy();
        let decision = policy.decide_microphone(0, 256, 512, true, true);

        assert_eq!(decision.mix_frames, 256);
        assert_eq!(decision.silence_frames, 0);
        assert!(!decision.entered_missing);
    }
}
