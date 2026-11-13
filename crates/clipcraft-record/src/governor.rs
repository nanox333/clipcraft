//! Automatic quality: when the computer cannot keep up, record fewer pictures per second instead
//! of dropping them at random, and go back when there is room.
//!
//! Only the *rate* of pictures changes. The size, the bitrate and the stream stay as they are, so
//! the replay buffer is never cleared by a change; a clip simply has fewer pictures a second for
//! a while (a "half rate" clip of a 60 fps recording plays at 30).
//!
//! The decision is plain arithmetic on counters, so it is tested without any recording.

use std::collections::VecDeque;

/// The lowest effective picture rate worth recording at.
const MIN_FPS: u32 = 15;
/// How many seconds of counters a decision looks at.
const WINDOW_SECONDS: usize = 5;
/// Share of pictures dropped, over the window, that counts as "cannot keep up".
const DROP_LIMIT: f64 = 0.08;
/// Seconds to wait after any change before lowering again, and before going back up.
const LOWER_AFTER: f64 = 8.0;
const RAISE_AFTER: f64 = 30.0;
/// How full the encoder may be, at the higher rate, for going back to be tried.
const RAISE_BELOW_LOAD: f64 = 0.6;

/// Counters since the recording started.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counters {
    /// Pictures taken from the screen.
    pub captured: u64,
    /// Pictures thrown away because the encoder was still busy.
    pub dropped: u64,
    /// Pictures encoded.
    pub encoded: u64,
    /// Time spent encoding, in nanoseconds.
    pub encode_nanos: u128,
}

/// Decides how many pictures to skip.
pub struct Governor {
    fps: u32,
    max_level: u32,
    level: u32,
    history: VecDeque<(f64, Counters)>,
    last_change: f64,
}

impl Governor {
    /// For a recording aiming at `fps` pictures a second.
    pub fn new(fps: u32) -> Self {
        let mut max_level = 0;
        while fps / (max_level + 2) >= MIN_FPS && max_level < 2 {
            max_level += 1;
        }
        Self {
            fps: fps.max(1),
            max_level,
            level: 0,
            history: VecDeque::new(),
            last_change: f64::NEG_INFINITY,
        }
    }

    /// Goes straight to the lowest rate (the computer is being starved), as if it had been asked
    /// for by the drops. It goes back up by the usual rule, after a calm spell.
    pub fn force_lowest(&mut self, now: f64) -> u32 {
        self.level = self.max_level;
        self.last_change = now;
        self.history.clear();
        self.level
    }

    /// 0 is every picture, 1 every second one, 2 every third.
    pub fn level(&self) -> u32 {
        self.level
    }

    /// The most pictures to skip it will ask for.
    pub fn max_level(&self) -> u32 {
        self.max_level
    }

    /// Looks at the counters at time `now` (seconds, any origin) and returns the new level when it
    /// changed. Call about once a second.
    pub fn observe(&mut self, now: f64, counters: Counters) -> Option<u32> {
        self.history.push_back((now, counters));
        while self.history.len() > WINDOW_SECONDS + 1 {
            self.history.pop_front();
        }
        let (then, oldest) = *self.history.front()?;
        if now - then < (WINDOW_SECONDS as f64) - 1.0 {
            return None;
        }
        let captured = counters.captured.saturating_sub(oldest.captured);
        let dropped = counters.dropped.saturating_sub(oldest.dropped);
        let encoded = counters.encoded.saturating_sub(oldest.encoded);
        let nanos = counters.encode_nanos.saturating_sub(oldest.encode_nanos);
        let seen = captured.max(1) as f64;
        let drop_rate = dropped as f64 / seen;
        if drop_rate > DROP_LIMIT
            && self.level < self.max_level
            && now - self.last_change >= LOWER_AFTER
        {
            self.level += 1;
            self.last_change = now;
            // What came before was at the old rate, so it says nothing about the new one.
            self.history.clear();
            return Some(self.level);
        }
        if self.level > 0 && dropped == 0 && encoded > 0 && now - self.last_change >= RAISE_AFTER {
            // What would the encoder be asked to do at the next higher rate?
            let seconds_per_picture = nanos as f64 / encoded as f64 / 1e9;
            let pictures_per_second = f64::from(self.fps) / f64::from(self.level);
            if seconds_per_picture * pictures_per_second < RAISE_BELOW_LOAD {
                self.level -= 1;
                self.last_change = now;
                self.history.clear();
                return Some(self.level);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds one second at a time: `drop_share` of `fps` pictures dropped, `encode_ms` each.
    fn run(
        governor: &mut Governor,
        counters: &mut Counters,
        from: u32,
        seconds: u32,
        fps: u64,
        drop_share: f64,
        encode_ms: f64,
    ) -> Vec<(u32, u32)> {
        let mut changes = Vec::new();
        for second in from..from + seconds {
            let taken = fps / (u64::from(governor.level()) + 1);
            let dropped = (taken as f64 * drop_share) as u64;
            counters.captured += taken;
            counters.dropped += dropped;
            counters.encoded += taken - dropped;
            counters.encode_nanos += ((taken - dropped) as f64 * encode_ms * 1e6) as u128;
            if let Some(level) = governor.observe(f64::from(second), *counters) {
                changes.push((second, level));
            }
        }
        changes
    }

    #[test]
    fn a_recording_that_keeps_up_never_changes() {
        let mut governor = Governor::new(60);
        let mut counters = Counters::default();
        assert!(run(&mut governor, &mut counters, 0, 120, 60, 0.0, 5.0).is_empty());
        assert_eq!(governor.level(), 0);
    }

    #[test]
    fn dropping_pictures_lowers_the_rate_one_step_at_a_time_and_stops_at_the_floor() {
        let mut governor = Governor::new(60);
        assert_eq!(
            governor.max_level(),
            2,
            "60, 30 and 20 are all worth recording"
        );
        let mut counters = Counters::default();
        let changes = run(&mut governor, &mut counters, 0, 60, 60, 0.4, 30.0);
        let levels: Vec<u32> = changes.iter().map(|(_, level)| *level).collect();
        assert_eq!(levels, [1, 2], "{changes:?}");
        // Changes are spaced out, not made on every second of trouble.
        assert!(changes[1].0 - changes[0].0 >= 8);
        assert_eq!(governor.level(), 2);
    }

    #[test]
    fn forcing_the_lowest_rate_goes_there_at_once_and_comes_back_by_the_usual_rule() {
        let mut governor = Governor::new(60);
        assert_eq!(governor.force_lowest(0.0), 2);
        let mut counters = Counters::default();
        let changes = run(&mut governor, &mut counters, 1, 100, 60, 0.0, 2.0);
        // Calm and light: one step up after the wait, another after the next.
        let levels: Vec<u32> = changes.iter().map(|(_, level)| *level).collect();
        assert_eq!(levels, [1, 0], "{changes:?}");
        assert!(changes[0].0 >= 30);
    }

    #[test]
    fn a_low_target_rate_has_nowhere_lower_to_go() {
        assert_eq!(Governor::new(30).max_level(), 1, "30 and 15");
        assert_eq!(
            Governor::new(24).max_level(),
            0,
            "12 is too slow to bother with"
        );
        let mut governor = Governor::new(24);
        let mut counters = Counters::default();
        assert!(run(&mut governor, &mut counters, 0, 60, 24, 0.5, 30.0).is_empty());
    }

    #[test]
    fn it_goes_back_up_only_after_a_calm_half_minute_with_room_to_spare() {
        let mut governor = Governor::new(60);
        let mut counters = Counters::default();
        run(&mut governor, &mut counters, 0, 5, 60, 0.4, 30.0);
        assert_eq!(governor.level(), 1);
        // Calm and light: back to full rate, but not before the wait is over.
        let changes = run(&mut governor, &mut counters, 5, 80, 60, 0.0, 4.0);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0].1, 0);
        assert!(changes[0].0 >= 34, "{changes:?}");
        // Calm but heavy (the encoder would be near full at the higher rate): stays lowered.
        let mut governor = Governor::new(60);
        let mut counters = Counters::default();
        run(&mut governor, &mut counters, 0, 5, 60, 0.4, 30.0);
        let changes = run(&mut governor, &mut counters, 5, 120, 60, 0.0, 25.0);
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(governor.level(), 1);
    }
}
