//! Audio for playback: a bounded feed of decoded sound that an output device drains.
//!
//! The feed is the playback clock. A decoder thread keeps a short queue of interleaved samples
//! filled at the output device's rate. The device callback calls [`AudioFeed::fill`], which hands
//! out queued samples and counts only the frames it really delivered, so the clip position is
//! "where the start was, plus the sound actually played". Video follows that position, which keeps
//! picture and sound together and makes an underrun pause the picture instead of desyncing it.
//! This module has no operating-system code; `clipcraft-audio-out` connects a device.

use crate::stretch::{self, Stretcher};
use clipcraft_frame::{BoundedQueue, Overflow};
use clipcraft_time::Tick;
use filmcraft_media::SharedSource;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

/// Frames decoded per chunk.
const CHUNK_FRAMES: usize = 2_048;
/// Sound kept decoded ahead of the device, in milliseconds.
const BUFFER_MILLISECONDS: usize = 400;
/// Most chunks and bytes the queue will hold, however the rates are configured.
const MAX_QUEUED_CHUNKS: usize = 256;
const MAX_QUEUED_BYTES: usize = 16 * 1024 * 1024;
/// Output rates and channel counts outside these are refused rather than allocated for.
const MAX_OUTPUT_RATE: u32 = 384_000;
const MAX_OUTPUT_CHANNELS: u32 = 16;
const IDLE_WAIT: Duration = Duration::from_millis(20);

struct FeedState {
    queue: BoundedQueue<Vec<f32>>,
    /// The chunk being drained and how far into it the device has read.
    current: Vec<f32>,
    cursor: usize,
    /// Frames queued but not yet handed to the device (including the rest of `current`).
    buffered_frames: usize,
    /// Where decoding restarts after a seek.
    start: Tick,
    /// Incremented by every seek so the decoder drops chunks it decoded for an older position.
    generation: u64,
    /// Playback speed (1.0 = normal) and which audio stream of the file is played.
    speed: f64,
    stream: usize,
}

/// Decoded sound waiting for an output device, and the clock derived from it.
pub struct AudioFeed {
    state: Mutex<FeedState>,
    wake: Condvar,
    playing: AtomicBool,
    muted: AtomicBool,
    /// Output gain, 0.0 to 1.0, as `f32` bits.
    volume: AtomicU32,
    /// Playback speed as `f64` bits, mirrored here so the clock can read it without the lock.
    speed: AtomicU64,
    output_active: AtomicBool,
    failed: AtomicBool,
    closed: AtomicBool,
    /// Frames delivered to the device since the last seek.
    played_frames: AtomicU64,
    /// Clip position, in ticks, of the first frame after the last seek.
    base_ticks: AtomicI64,
    rate: AtomicU32,
    channels: AtomicU32,
}

impl AudioFeed {
    fn new() -> Self {
        Self {
            state: Mutex::new(FeedState {
                queue: BoundedQueue::new(MAX_QUEUED_CHUNKS, MAX_QUEUED_BYTES, Overflow::Reject),
                current: Vec::new(),
                cursor: 0,
                buffered_frames: 0,
                start: Tick::ZERO,
                generation: 0,
                speed: 1.0,
                stream: 0,
            }),
            wake: Condvar::new(),
            playing: AtomicBool::new(false),
            muted: AtomicBool::new(false),
            volume: AtomicU32::new(1.0_f32.to_bits()),
            speed: AtomicU64::new(1.0_f64.to_bits()),
            output_active: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            played_frames: AtomicU64::new(0),
            base_ticks: AtomicI64::new(0),
            rate: AtomicU32::new(0),
            channels: AtomicU32::new(0),
        }
    }

    fn lock(&self) -> MutexGuard<'_, FeedState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Called by the output device before it starts: the rate and channel count it will pull.
    /// Returns false (and changes nothing) for values that make no sense.
    pub fn configure_output(&self, rate: u32, channels: u32) -> bool {
        if rate == 0 || rate > MAX_OUTPUT_RATE || channels == 0 || channels > MAX_OUTPUT_CHANNELS {
            return false;
        }
        let mut state = self.lock();
        self.rate.store(rate, Ordering::SeqCst);
        self.channels.store(channels, Ordering::SeqCst);
        // Anything already decoded was for another format.
        let start = Tick(self.base_ticks.load(Ordering::SeqCst));
        Self::restart(&mut state, start);
        self.played_frames.store(0, Ordering::SeqCst);
        drop(state);
        self.wake.notify_all();
        true
    }

    /// Marks the device as running (or stopped). Video follows the sound only while it runs.
    pub fn set_output_active(&self, active: bool) {
        self.output_active.store(active, Ordering::SeqCst);
    }

    /// Whether the picture should follow this feed's clock: a device is running and decoding has
    /// not failed.
    pub fn driving(&self) -> bool {
        self.output_active.load(Ordering::SeqCst)
            && !self.failed.load(Ordering::SeqCst)
            && self.rate.load(Ordering::SeqCst) > 0
    }

    /// Whether the audio stream could not be decoded.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    /// Starts or pauses delivery to the device.
    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Ordering::SeqCst);
        self.wake.notify_all();
    }

    /// Silences the sound without stopping the clock.
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::SeqCst);
    }

    /// Whether the sound is muted.
    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::SeqCst)
    }

    /// Sets the output volume, 0.0 (silent) to 1.0 (full). Out-of-range and non-finite values are
    /// clamped.
    pub fn set_volume(&self, volume: f32) {
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.volume.store(volume.to_bits(), Ordering::SeqCst);
    }

    /// The output volume, 0.0 to 1.0.
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::SeqCst))
    }

    /// Plays at `speed` from `position`, keeping pitch. Clamped to the supported range.
    pub fn set_speed(&self, speed: f64, position: Tick) {
        let speed = if speed.is_finite() {
            speed.clamp(stretch::MIN_SPEED, stretch::MAX_SPEED)
        } else {
            1.0
        };
        let mut state = self.lock();
        state.speed = speed;
        self.speed.store(speed.to_bits(), Ordering::SeqCst);
        Self::restart(&mut state, position);
        self.base_ticks.store(position.0, Ordering::SeqCst);
        self.played_frames.store(0, Ordering::SeqCst);
        drop(state);
        self.wake.notify_all();
    }

    /// The playback speed, 1.0 being normal.
    pub fn speed(&self) -> f64 {
        f64::from_bits(self.speed.load(Ordering::SeqCst))
    }

    /// Switches to audio stream `stream` of the file, continuing from `position`.
    pub fn set_stream(&self, stream: usize, position: Tick) {
        let mut state = self.lock();
        state.stream = stream;
        Self::restart(&mut state, position);
        self.base_ticks.store(position.0, Ordering::SeqCst);
        self.played_frames.store(0, Ordering::SeqCst);
        drop(state);
        self.wake.notify_all();
    }

    /// Jumps to `position`: drops queued sound, restarts decoding there, and restarts the clock.
    pub fn seek(&self, position: Tick) {
        let mut state = self.lock();
        Self::restart(&mut state, position);
        self.base_ticks.store(position.0, Ordering::SeqCst);
        self.played_frames.store(0, Ordering::SeqCst);
        drop(state);
        self.wake.notify_all();
    }

    fn restart(state: &mut FeedState, position: Tick) {
        state.queue = BoundedQueue::new(MAX_QUEUED_CHUNKS, MAX_QUEUED_BYTES, Overflow::Reject);
        state.current = Vec::new();
        state.cursor = 0;
        state.buffered_frames = 0;
        state.start = position;
        state.generation = state.generation.wrapping_add(1);
    }

    /// Frames the device has taken since the last seek. Stays at zero for a stalled device.
    pub fn delivered_frames(&self) -> u64 {
        self.played_frames.load(Ordering::SeqCst)
    }

    /// The clip position: the last seek target plus the sound delivered since.
    pub fn position(&self) -> Tick {
        let base = Tick(self.base_ticks.load(Ordering::SeqCst));
        let rate = i64::from(self.rate.load(Ordering::SeqCst));
        let played = i64::try_from(self.played_frames.load(Ordering::SeqCst)).unwrap_or(i64::MAX);
        if rate == 0 {
            return base;
        }
        let elapsed = Tick::from_units(played, rate);
        let speed = f64::from_bits(self.speed.load(Ordering::SeqCst));
        if (speed - 1.0).abs() < 1e-9 {
            base + elapsed
        } else {
            // Each output frame consumed `speed` source frames.
            base + Tick::from_seconds_f64(elapsed.seconds() * speed)
        }
    }

    /// Fills `out` (interleaved, in the configured channel count) for the device. Silence is
    /// written for anything the queue cannot supply, and only supplied frames advance the clock.
    /// Never blocks: if the decoder holds the lock this callback outputs silence for that period.
    pub fn fill(&self, out: &mut [f32]) {
        out.fill(0.0);
        if !self.playing.load(Ordering::SeqCst) {
            return;
        }
        let channels = self.channels.load(Ordering::SeqCst) as usize;
        if channels == 0 {
            return;
        }
        let Ok(mut state) = self.state.try_lock() else {
            return;
        };
        let gain = if self.muted.load(Ordering::SeqCst) {
            0.0
        } else {
            f32::from_bits(self.volume.load(Ordering::SeqCst))
        };
        let mut written = 0;
        while written < out.len() {
            if state.cursor >= state.current.len() {
                match state.queue.pop() {
                    Some(chunk) => {
                        state.current = chunk;
                        state.cursor = 0;
                    }
                    None => break,
                }
            }
            let available = state.current.len() - state.cursor;
            let count = available.min(out.len() - written);
            let from = state.cursor;
            if let (Some(target), Some(source)) = (
                out.get_mut(written..written + count),
                state.current.get(from..from + count),
            ) {
                for (target, source) in target.iter_mut().zip(source) {
                    *target = *source * gain;
                }
            }
            state.cursor += count;
            written += count;
        }
        let frames = written / channels;
        state.buffered_frames = state.buffered_frames.saturating_sub(frames);
        self.played_frames
            .fetch_add(frames as u64, Ordering::SeqCst);
        drop(state);
        self.wake.notify_all();
    }

    /// Stops the decoder thread. Called when playback ends.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.wake.notify_all();
    }

    #[cfg(test)]
    pub fn buffered_frames(&self) -> usize {
        self.lock().buffered_frames
    }

    /// A feed with no decoder, for tests that push sound by hand.
    #[cfg(test)]
    pub fn detached() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Queues interleaved samples decoded for `generation`. Dropped if a seek happened since.
    fn push(&self, generation: u64, samples: Vec<f32>, channels: usize) -> bool {
        let mut state = self.lock();
        if state.generation != generation || channels == 0 {
            return false;
        }
        let frames = samples.len() / channels;
        let bytes = samples.len() * std::mem::size_of::<f32>();
        if state.queue.push(samples, bytes).is_err() {
            return false;
        }
        state.buffered_frames += frames;
        true
    }

    #[cfg(test)]
    pub fn push_for_test(&self, samples: Vec<f32>) {
        let generation = self.lock().generation;
        let channels = self.channels.load(Ordering::SeqCst) as usize;
        assert!(self.push(generation, samples, channels));
    }
}

/// Starts the decoder thread for `source` and returns the feed it fills.
pub(crate) fn start_feed(source: SharedSource) -> Arc<AudioFeed> {
    let feed = Arc::new(AudioFeed::new());
    let worker = feed.clone();
    let spawned = thread::Builder::new()
        .name("clipcraft-audio-decode".to_owned())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| decode_loop(&worker, &source)));
            if outcome.is_err() {
                worker.failed.store(true, Ordering::SeqCst);
            }
        });
    if spawned.is_err() {
        feed.failed.store(true, Ordering::SeqCst);
    }
    feed
}

/// How the decoder is reading the clip right now.
enum Cursor {
    /// Normal speed: consecutive source chunks, passed through untouched.
    Direct { next_sample: i64 },
    /// Any other speed: time-stretched so pitch is kept.
    Stretch(Box<Stretcher>),
}

fn decode_loop(feed: &AudioFeed, source: &SharedSource) {
    let mut seen_generation = u64::MAX;
    let mut cursor = Cursor::Direct { next_sample: 0 };
    let mut stream = 0;
    let mut finished = false;
    let duration = source.info().duration.max(Tick::ZERO);

    while !feed.closed.load(Ordering::SeqCst) {
        let rate = feed.rate.load(Ordering::SeqCst);
        let channels = feed.channels.load(Ordering::SeqCst) as usize;
        let (generation, start, buffered, speed, wanted_stream) = {
            let state = feed.lock();
            (
                state.generation,
                state.start,
                state.buffered_frames,
                state.speed,
                state.stream,
            )
        };
        if rate == 0 || channels == 0 {
            wait(feed);
            continue;
        }
        if generation != seen_generation {
            seen_generation = generation;
            stream = wanted_stream;
            finished = false;
            let first_sample = start.to_units_floor(i64::from(rate));
            cursor = if (speed - 1.0).abs() < 1e-9 {
                Cursor::Direct {
                    next_sample: first_sample,
                }
            } else {
                let end = duration.to_units_floor(i64::from(rate));
                Cursor::Stretch(Box::new(Stretcher::new(rate, speed, first_sample, end)))
            };
        }
        let target_frames = rate as usize * BUFFER_MILLISECONDS / 1000;
        if finished || buffered >= target_frames {
            wait(feed);
            continue;
        }

        let samples = match &mut cursor {
            Cursor::Direct { next_sample } => {
                if Tick::from_units(*next_sample, i64::from(rate)) >= duration {
                    finished = true;
                    continue;
                }
                let Ok(buffer) = source.audio_stream(stream, *next_sample, CHUNK_FRAMES, rate)
                else {
                    feed.failed.store(true, Ordering::SeqCst);
                    return;
                };
                *next_sample = next_sample.saturating_add(CHUNK_FRAMES as i64);
                interleave(&buffer.channels, buffer.frames(), channels)
            }
            Cursor::Stretch(stretcher) => {
                if stretcher.finished() {
                    finished = true;
                    continue;
                }
                let mut fetch = |from: i64, frames: usize| {
                    source
                        .audio_stream(stream, from, frames, rate)
                        .ok()
                        .map(|buffer| buffer.channels)
                };
                let Some(block) = stretcher.next_block(&mut fetch) else {
                    feed.failed.store(true, Ordering::SeqCst);
                    return;
                };
                interleave(&block, stretcher.hop(), channels)
            }
        };
        if !feed.push(generation, samples, channels) {
            // Either a seek made this chunk stale (the next loop restarts) or the queue is full;
            // give the device time to drain before trying again.
            wait(feed);
        }
    }
}

fn wait(feed: &AudioFeed) {
    let state = feed.lock();
    let _ = feed.wake.wait_timeout(state, IDLE_WAIT);
}

/// Planar to interleaved with exactly `out_channels` channels: missing channels repeat the source
/// channels in turn (mono becomes stereo), extra source channels are dropped. Always returns
/// `frames * out_channels` samples, padding short channels with silence.
pub(crate) fn interleave(planar: &[Vec<f32>], frames: usize, out_channels: usize) -> Vec<f32> {
    let mut out = vec![0.0_f32; frames.saturating_mul(out_channels)];
    if planar.is_empty() || out_channels == 0 {
        return out;
    }
    for channel in 0..out_channels {
        let Some(source) = planar.get(channel % planar.len()) else {
            continue;
        };
        for (frame, sample) in source.iter().take(frames).enumerate() {
            if let Some(slot) = out.get_mut(frame * out_channels + channel) {
                *slot = sample.clamp(-1.0, 1.0);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(rate: u32, channels: u32) -> Arc<AudioFeed> {
        let feed = AudioFeed::detached();
        assert!(feed.configure_output(rate, channels));
        feed.set_output_active(true);
        feed
    }

    #[test]
    fn interleave_maps_channels_and_never_exceeds_the_requested_shape() {
        let mono = vec![vec![0.5, -0.5, 1.5]];
        assert_eq!(interleave(&mono, 3, 2), [0.5, 0.5, -0.5, -0.5, 1.0, 1.0]);
        let stereo = vec![vec![0.1, 0.2], vec![0.3, 0.4]];
        assert_eq!(interleave(&stereo, 2, 2), [0.1, 0.3, 0.2, 0.4]);
        assert_eq!(
            interleave(&stereo, 2, 1),
            [0.1, 0.2],
            "extra channels are dropped"
        );
        // A channel shorter than `frames` is padded with silence; empty input is all silence.
        let short = vec![vec![0.7], vec![0.9, 0.9]];
        assert_eq!(interleave(&short, 2, 2), [0.7, 0.9, 0.0, 0.9]);
        assert_eq!(interleave(&[], 4, 2), vec![0.0; 8]);
        assert!(interleave(&mono, usize::MAX, 0).is_empty());
    }

    #[test]
    fn configure_rejects_nonsense_formats() {
        let feed = AudioFeed::detached();
        assert!(!feed.configure_output(0, 2));
        assert!(!feed.configure_output(48_000, 0));
        assert!(!feed.configure_output(u32::MAX, 2));
        assert!(!feed.configure_output(48_000, 1_000));
        assert!(!feed.driving());
    }

    #[test]
    fn the_clock_counts_only_frames_actually_delivered() {
        let feed = feed(1_000, 2);
        feed.seek(Tick::from_seconds_f64(2.0));
        feed.set_playing(true);
        assert_eq!(feed.position(), Tick::from_seconds_f64(2.0));

        // 100 stereo frames queued; the device asks for 150.
        feed.push_for_test(vec![0.25; 200]);
        let mut out = vec![9.0; 300];
        feed.fill(&mut out);
        assert!(out[..200].iter().all(|s| *s == 0.25));
        assert!(
            out[200..].iter().all(|s| *s == 0.0),
            "the shortfall is silence"
        );
        // Only the 100 real frames advanced the clock: 2.0 s + 100 ms.
        let seconds = feed.position().seconds();
        assert!((seconds - 2.1).abs() < 1e-9, "position {seconds}");

        // An underrun does not move the clock at all.
        feed.fill(&mut vec![0.0; 300]);
        assert!((feed.position().seconds() - 2.1).abs() < 1e-9);
    }

    #[test]
    fn pausing_silences_and_freezes_the_clock_and_resuming_continues() {
        let feed = feed(1_000, 1);
        feed.set_playing(true);
        feed.push_for_test(vec![0.5; 100]);
        let mut out = vec![0.0; 40];
        feed.fill(&mut out);
        let before = feed.position();

        feed.set_playing(false);
        feed.fill(&mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(feed.position(), before);
        assert_eq!(feed.buffered_frames(), 60, "paused sound stays queued");

        feed.set_playing(true);
        feed.fill(&mut out);
        assert!(out.iter().all(|s| *s == 0.5));
        assert_eq!(feed.buffered_frames(), 20);
    }

    #[test]
    fn muting_keeps_the_clock_running() {
        let feed = feed(1_000, 1);
        feed.set_playing(true);
        feed.set_muted(true);
        feed.push_for_test(vec![0.9; 50]);
        let mut out = vec![1.0; 50];
        feed.fill(&mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        assert!(feed.muted());
        assert!((feed.position().seconds() - 0.05).abs() < 1e-9);
    }

    #[test]
    fn seeking_drops_queued_sound_and_stale_decoder_chunks() {
        let feed = feed(1_000, 1);
        feed.set_playing(true);
        let old_generation = feed.lock().generation;
        feed.push_for_test(vec![0.5; 100]);

        feed.seek(Tick::from_seconds_f64(5.0));
        assert_eq!(feed.buffered_frames(), 0);
        // A chunk the decoder produced for the old position must not play after the seek.
        assert!(!feed.push(old_generation, vec![0.7; 10], 1));
        let mut out = vec![1.0; 10];
        feed.fill(&mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(feed.position(), Tick::from_seconds_f64(5.0));
    }

    #[test]
    fn the_feed_drives_the_picture_only_while_a_device_runs_and_decoding_works() {
        let feed = AudioFeed::detached();
        assert!(!feed.driving(), "no device configured yet");
        assert!(feed.configure_output(48_000, 2));
        assert!(!feed.driving(), "configured but not started");
        feed.set_output_active(true);
        assert!(feed.driving());
        feed.failed.store(true, Ordering::SeqCst);
        assert!(
            !feed.driving(),
            "a decode failure hands the clock back to the wall clock"
        );
        feed.failed.store(false, Ordering::SeqCst);
        feed.set_output_active(false);
        assert!(!feed.driving());
    }

    #[test]
    fn the_device_callback_survives_a_poisoned_lock_and_odd_buffer_sizes() {
        let feed = feed(1_000, 2);
        feed.set_playing(true);
        feed.push_for_test(vec![0.1; 10]);
        // Buffer sizes that are not a whole number of frames, and empty ones, must not panic.
        feed.fill(&mut []);
        feed.fill(&mut [0.0; 3]);
        feed.fill(&mut [0.0; 1]);
        let poisoner = feed.clone();
        let _ = thread::spawn(move || {
            let _guard = poisoner.state.lock().expect("lock");
            panic!("poison the lock on purpose");
        })
        .join();
        feed.fill(&mut [0.0; 4]);
        feed.seek(Tick::ZERO);
        assert_eq!(feed.position(), Tick::ZERO);
    }

    #[test]
    fn volume_scales_the_output_and_is_clamped() {
        let feed = feed(1_000, 1);
        feed.set_playing(true);
        assert_eq!(feed.volume(), 1.0);
        feed.set_volume(0.5);
        feed.push_for_test(vec![0.8; 10]);
        let mut out = [0.0; 10];
        feed.fill(&mut out);
        assert!(out.iter().all(|s| (*s - 0.4).abs() < 1e-6));

        feed.set_volume(7.0);
        assert_eq!(feed.volume(), 1.0);
        feed.set_volume(-1.0);
        assert_eq!(feed.volume(), 0.0);
        feed.set_volume(f32::NAN);
        assert_eq!(feed.volume(), 1.0, "a non-finite volume falls back to full");
    }

    #[test]
    fn the_clock_runs_at_the_playback_speed() {
        let feed = feed(1_000, 1);
        feed.set_playing(true);
        feed.set_speed(2.0, Tick::from_seconds_f64(1.0));
        assert_eq!(feed.speed(), 2.0);
        feed.push_for_test(vec![0.1; 500]);
        feed.fill(&mut [0.0; 500]);
        // 500 output frames at 1 kHz is half a second of output, which covered one second of media.
        assert!(
            (feed.position().seconds() - 2.0).abs() < 1e-6,
            "{}",
            feed.position().seconds()
        );

        // Hostile speeds are clamped.
        feed.set_speed(1e9, Tick::ZERO);
        assert_eq!(feed.speed(), stretch::MAX_SPEED);
        feed.set_speed(f64::NAN, Tick::ZERO);
        assert_eq!(feed.speed(), 1.0);
        feed.set_speed(0.0, Tick::ZERO);
        assert_eq!(feed.speed(), stretch::MIN_SPEED);
    }

    #[test]
    fn changing_speed_or_track_drops_queued_sound_and_moves_the_clock_base() {
        let feed = feed(1_000, 1);
        feed.set_playing(true);
        feed.push_for_test(vec![0.5; 100]);
        feed.set_stream(1, Tick::from_seconds_f64(3.0));
        assert_eq!(feed.buffered_frames(), 0);
        assert_eq!(feed.position(), Tick::from_seconds_f64(3.0));
        feed.push_for_test(vec![0.5; 100]);
        feed.set_speed(0.5, Tick::from_seconds_f64(4.0));
        assert_eq!(feed.buffered_frames(), 0);
        assert_eq!(feed.position(), Tick::from_seconds_f64(4.0));
    }
}
