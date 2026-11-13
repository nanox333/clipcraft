//! Keeping ClipCraft from starving the computer it runs on.
//!
//! A [`Watchdog`] watches how much of the processor and memory the program is using while it
//! records, and answers with one of three things: carry on, lighten the recording, or stop it. It
//! is plain arithmetic on samples, so it is tested without a recording or a clock. Heavy jobs the
//! person asks for (an export) are counted by [`HeavyWork`] so they are not mistaken for a runaway.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Share of the whole processor above which the program counts as hogging it.
pub const HOGGING_SHARE: f32 = 0.85;
/// Share below which it counts as having calmed down.
const CALM_SHARE: f32 = 0.60;
/// Seconds of sustained hogging before the recording is lightened.
const LIGHTEN_AFTER_SECONDS: usize = 20;
/// Seconds of hogging after lightening before the recording is stopped.
const STOP_AFTER_SECONDS: usize = 60;
/// Seconds below the calm share before the watch starts over.
const CALM_AFTER_SECONDS: usize = 20;
/// Memory, in bytes, that no recording should reach (the replay buffer is capped far below it).
pub const MEMORY_LIMIT_BYTES: u64 = 6 * 1024 * 1024 * 1024;

/// What the watchdog asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    /// Record fewer pictures a second to take the load off.
    Lighten,
    /// Stop recording; the computer is being starved. Says why.
    Stop(StopReason),
    /// The load is back to normal.
    Recovered,
}

/// Why recording was stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReason {
    /// The program used nearly all of the processor for a long time, even after lightening.
    Processor,
    /// The program used far more memory than a replay needs.
    Memory,
}

/// Watches usage, one sample a second.
#[derive(Default)]
pub struct Watchdog {
    hogging_seconds: usize,
    calm_seconds: usize,
    lightened: bool,
    stopped: bool,
    recent: VecDeque<f32>,
}

impl Watchdog {
    /// Starts watching afresh (when recording starts or a heavy job ends).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// One second's usage. Returns what to do, when anything.
    pub fn observe(&mut self, cpu_share: f32, memory_bytes: u64) -> Option<Verdict> {
        // After a stop there is nothing more to say until the watch is reset.
        if self.stopped {
            return None;
        }
        if memory_bytes >= MEMORY_LIMIT_BYTES {
            self.stopped = true;
            return Some(Verdict::Stop(StopReason::Memory));
        }
        self.recent.push_back(cpu_share);
        if self.recent.len() > CALM_AFTER_SECONDS {
            self.recent.pop_front();
        }
        if cpu_share >= HOGGING_SHARE {
            self.hogging_seconds += 1;
            self.calm_seconds = 0;
        } else if cpu_share < CALM_SHARE {
            self.calm_seconds += 1;
            // A dip in the middle of a long spell does not start the count again.
            if self.calm_seconds >= CALM_AFTER_SECONDS {
                let was_watching = self.lightened || self.hogging_seconds > 0;
                self.hogging_seconds = 0;
                self.calm_seconds = 0;
                if was_watching {
                    self.lightened = false;
                    return Some(Verdict::Recovered);
                }
            }
        }
        if self.lightened && self.hogging_seconds >= STOP_AFTER_SECONDS {
            self.stopped = true;
            return Some(Verdict::Stop(StopReason::Processor));
        }
        if !self.lightened && self.hogging_seconds >= LIGHTEN_AFTER_SECONDS {
            self.lightened = true;
            // Start counting the time since lightening, not the time before it.
            self.hogging_seconds = 0;
            return Some(Verdict::Lighten);
        }
        None
    }
}

static HEAVY: AtomicUsize = AtomicUsize::new(0);

/// Marks heavy work the person asked for (an export) for as long as it is held, so the program is
/// not stopped for being busy with it.
pub struct HeavyWork(());

impl HeavyWork {
    /// Counts one heavy job until the returned value is dropped.
    pub fn begin() -> Self {
        HEAVY.fetch_add(1, Ordering::SeqCst);
        Self(())
    }

    /// Whether any heavy job is running right now.
    pub fn running() -> bool {
        HEAVY.load(Ordering::SeqCst) > 0
    }
}

impl Drop for HeavyWork {
    fn drop(&mut self) {
        HEAVY.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(watchdog: &mut Watchdog, seconds: usize, share: f32) -> Vec<(usize, Verdict)> {
        (0..seconds)
            .filter_map(|second| {
                watchdog
                    .observe(share, 500 * 1024 * 1024)
                    .map(|verdict| (second, verdict))
            })
            .collect()
    }

    #[test]
    fn normal_use_never_triggers_anything() {
        let mut watchdog = Watchdog::default();
        assert!(feed(&mut watchdog, 600, 0.55).is_empty());
        assert!(
            feed(&mut watchdog, 600, 0.80).is_empty(),
            "busy is not hogging"
        );
    }

    #[test]
    fn hogging_the_processor_lightens_first_and_stops_only_if_that_does_not_help() {
        let mut watchdog = Watchdog::default();
        let verdicts = feed(&mut watchdog, 200, 0.97);
        assert_eq!(
            verdicts,
            [
                (LIGHTEN_AFTER_SECONDS - 1, Verdict::Lighten),
                (
                    LIGHTEN_AFTER_SECONDS + STOP_AFTER_SECONDS - 1,
                    Verdict::Stop(StopReason::Processor)
                ),
            ]
        );
    }

    #[test]
    fn lightening_that_works_ends_the_watch_after_a_calm_spell() {
        let mut watchdog = Watchdog::default();
        let mut verdicts = feed(&mut watchdog, 25, 0.95);
        verdicts.extend(feed(&mut watchdog, 40, 0.30));
        let kinds: Vec<Verdict> = verdicts.into_iter().map(|(_, verdict)| verdict).collect();
        assert_eq!(kinds, [Verdict::Lighten, Verdict::Recovered]);
        // And it can lighten again later.
        assert_eq!(feed(&mut watchdog, 25, 0.95).len(), 1);
    }

    #[test]
    fn a_short_dip_does_not_forgive_a_long_spell() {
        let mut watchdog = Watchdog::default();
        let mut verdicts = Vec::new();
        for _ in 0..3 {
            verdicts.extend(feed(&mut watchdog, 9, 0.95));
            verdicts.extend(feed(&mut watchdog, 2, 0.30));
        }
        assert!(
            verdicts
                .iter()
                .any(|(_, verdict)| *verdict == Verdict::Lighten),
            "{verdicts:?}"
        );
    }

    #[test]
    fn far_too_much_memory_stops_at_once() {
        let mut watchdog = Watchdog::default();
        assert_eq!(
            watchdog.observe(0.1, MEMORY_LIMIT_BYTES),
            Some(Verdict::Stop(StopReason::Memory))
        );
    }

    #[test]
    fn heavy_work_is_counted_while_it_is_held() {
        // One test, because the count is shared by the whole process.
        assert!(!HeavyWork::running());
        let first = HeavyWork::begin();
        let second = HeavyWork::begin();
        assert!(HeavyWork::running());
        drop(first);
        assert!(HeavyWork::running());
        drop(second);
        assert!(!HeavyWork::running());
    }
}
