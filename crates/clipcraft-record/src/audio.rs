//! Sound for the recorder: sources, per-source level and mute controls, the lanes that bring every
//! source to the clip's sample rate and channel count, and the mixer that turns them into AAC
//! tracks.
//!
//! Everything is timed by the wall clock the video uses. A [`AudioEngine`] is told how much time
//! has passed and produces exactly that much sound per track: a source that runs slow is padded
//! with silence, one that runs fast or has a backlog loses its oldest samples, so a long recording
//! cannot drift out of step with the picture and no queue grows without bound.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use filmcraft_aac::{Encoder, EncoderConfig};

/// Samples in one AAC access unit.
pub const AAC_FRAME: i64 = 1024;
/// How long the mixer waits for sound to arrive before it fills the gap with silence.
const ARRIVAL_SLACK_MS: u64 = 40;
/// The most sound a source may be ahead of the clock before its oldest samples are dropped.
const MAX_LAG_MS: u64 = 120;
/// The smallest batch the engine mixes and encodes at once, in sample frames.
const MIN_BATCH: i64 = 240;
/// Largest gain, in percent.
pub const MAX_GAIN_PERCENT: u32 = 200;
/// The AAC sample rates.
const AAC_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// Sample rate and channel count of interleaved samples.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioFormat {
    /// Sample frames per second.
    pub sample_rate: u32,
    /// Channels per frame.
    pub channels: u16,
}

/// Why a source could not deliver sound.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AudioError {
    /// The device went away (unplugged, or its owner took it back).
    DeviceLost(String),
    /// The device reported an error.
    Failed(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceLost(why) => write!(formatter, "the sound device was lost: {why}"),
            Self::Failed(why) => write!(formatter, "the sound device failed: {why}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Somewhere sound comes from: a microphone, the system's output, a test generator.
pub trait AudioSource: Send {
    /// What to call it in the status.
    fn name(&self) -> String;

    /// The format `read` delivers.
    fn format(&self) -> AudioFormat;

    /// Appends the interleaved samples that arrived since the last call. Must not block.
    fn read(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError>;

    /// Tries to bring a lost device back. Called every couple of seconds while it is lost.
    fn recover(&mut self) -> bool {
        false
    }
}

/// Live controls and the level meter of one source. Cheap to clone; every clone is the same
/// control.
#[derive(Clone, Debug)]
pub struct AudioControl {
    muted: Arc<AtomicBool>,
    gain_percent: Arc<AtomicU32>,
    level: Arc<AtomicU32>,
}

impl Default for AudioControl {
    fn default() -> Self {
        Self {
            muted: Arc::new(AtomicBool::new(false)),
            gain_percent: Arc::new(AtomicU32::new(100)),
            level: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl AudioControl {
    /// Silences or restores the source in clips.
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    /// Whether the source is silenced.
    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    /// Sets the level, 0 to 200 percent.
    pub fn set_gain_percent(&self, percent: u32) {
        self.gain_percent
            .store(percent.min(MAX_GAIN_PERCENT), Ordering::Relaxed);
    }

    /// The level in percent.
    pub fn gain_percent(&self) -> u32 {
        self.gain_percent.load(Ordering::Relaxed)
    }

    /// The loudest sample of the latest block heard (before muting), 0.0 to 1.0.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    fn set_level(&self, level: f32) {
        self.level.store(level.to_bits(), Ordering::Relaxed);
    }

    fn factor(&self) -> f32 {
        if self.muted() {
            0.0
        } else {
            self.gain_percent() as f32 / 100.0
        }
    }
}

/// One source and where its sound goes.
pub struct AudioInput {
    /// Delivers the samples.
    pub source: Box<dyn AudioSource>,
    /// Mute, level, and meter.
    pub control: AudioControl,
    /// Index of the output track it is mixed into.
    pub track: usize,
    /// Fold both channels into one and play it from both sides (a mono microphone on a stereo
    /// device).
    pub mono: bool,
}

impl AudioInput {
    /// A source mixed into `track` with its own controls.
    pub fn new(source: Box<dyn AudioSource>, control: AudioControl, track: usize) -> Self {
        Self {
            source,
            control,
            track,
            mono: false,
        }
    }
}

/// What a source records.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioKind {
    /// A microphone.
    Microphone,
    /// What the computer is playing.
    SystemOutput,
}

type OpenAudio = dyn Fn(AudioKind, &str) -> Result<Box<dyn AudioSource>, String> + Send + Sync;

/// Opens sound sources for the recorder. Cheap to clone.
#[derive(Clone)]
pub struct AudioProvider {
    open: Arc<OpenAudio>,
}

impl AudioProvider {
    /// Wraps a function that opens the named device (empty for the default) or says why not.
    pub fn new(
        open: impl Fn(AudioKind, &str) -> Result<Box<dyn AudioSource>, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            open: Arc::new(open),
        }
    }

    /// Opens a source.
    pub fn open(&self, kind: AudioKind, device: &str) -> Result<Box<dyn AudioSource>, String> {
        (self.open)(kind, device)
    }
}

impl std::fmt::Debug for AudioProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AudioProvider")
    }
}

/// How the clip's sound is laid out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioConfig {
    /// Sample rate of the clip's sound: one of the AAC rates.
    pub sample_rate: u32,
    /// 1 or 2.
    pub channels: u16,
    /// AAC bitrate per track, in kilobits per second.
    pub kbps: u32,
    /// The name of each output track.
    pub tracks: Vec<String>,
}

impl AudioConfig {
    /// Whether the encoder can be created for this layout.
    pub fn valid(&self) -> bool {
        AAC_RATES.contains(&self.sample_rate)
            && (1..=2).contains(&self.channels)
            && (16..=512).contains(&self.kbps)
            && (1..=8).contains(&self.tracks.len())
    }
}

/// All the sound the recorder is asked for.
pub struct AudioSetup {
    /// The layout of the clip's sound.
    pub config: AudioConfig,
    /// The sources, each assigned to a track.
    pub inputs: Vec<AudioInput>,
}

/// One encoded access unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioPacket {
    /// The input sample, counted from the start of the recording, where this unit's sound begins.
    pub start: i64,
    /// Raw AAC.
    pub data: Vec<u8>,
}

/// What the encoder says about a track, needed to describe it in a file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioTrackInfo {
    /// The track's name.
    pub name: String,
    /// Sample rate.
    pub sample_rate: u32,
    /// Channels.
    pub channels: u16,
    /// The AAC `AudioSpecificConfig`.
    pub config: Vec<u8>,
    /// Samples of encoder delay before the first real sample.
    pub priming: u32,
}

/// How one source is doing.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioInputStatus {
    /// The source's name.
    pub name: String,
    /// Track it feeds.
    pub track: usize,
    /// Loudest recent sample, 0.0 to 1.0.
    pub level: f32,
    /// Silenced.
    pub muted: bool,
    /// The device is gone and not back yet.
    pub lost: bool,
    /// Milliseconds of silence inserted because the source was late.
    pub silence_ms: u64,
    /// Milliseconds of sound dropped because the source ran ahead.
    pub dropped_ms: u64,
}

/// Brings one source's samples to the clip's rate and channel count.
struct Lane {
    from: AudioFormat,
    channels: usize,
    /// Input frames consumed per output frame.
    step: f64,
    /// Position in the stream `[previous frame, input...]`.
    position: f64,
    previous: Vec<f32>,
    started: bool,
    /// Interleaved samples at the clip's format, waiting for the clock.
    fifo: VecDeque<f32>,
    mono_mix: bool,
}

impl Lane {
    fn new(from: AudioFormat, rate: u32, channels: u16) -> Self {
        let channels = usize::from(channels);
        Self {
            from,
            channels,
            step: f64::from(from.sample_rate) / f64::from(rate),
            position: 0.0,
            previous: vec![0.0; channels],
            started: false,
            fifo: VecDeque::new(),
            mono_mix: false,
        }
    }

    fn frames(&self) -> usize {
        self.fifo.len() / self.channels
    }

    /// Takes interleaved input samples: converts the channels, resamples, and queues.
    fn push(&mut self, input: &[f32]) {
        let in_channels = usize::from(self.from.channels.max(1));
        let frames = input.len() / in_channels;
        if frames == 0 {
            return;
        }
        // Channel conversion first, at the input rate.
        let mut converted = Vec::with_capacity(frames * self.channels);
        for frame in input.chunks_exact(in_channels).take(frames) {
            let clean = |sample: f32| {
                if sample.is_finite() {
                    sample.clamp(-4.0, 4.0)
                } else {
                    0.0
                }
            };
            let left = clean(frame[0]);
            let right = clean(*frame.get(1).unwrap_or(&frame[0]));
            if self.channels == 1 {
                converted.push((left + right) * 0.5);
            } else if self.mono_mix {
                let both = (left + right) * 0.5;
                converted.push(both);
                converted.push(both);
            } else {
                converted.push(left);
                converted.push(right);
            }
        }
        if !self.started {
            self.previous.copy_from_slice(&converted[..self.channels]);
            self.started = true;
        }
        if (self.step - 1.0).abs() < f64::EPSILON {
            self.fifo.extend(converted);
            return;
        }
        let count = frames as f64;
        let channels = self.channels;
        let frame_at = |this: &Self, index: usize, channel: usize| -> f32 {
            if index == 0 {
                this.previous[channel]
            } else {
                converted[(index - 1) * channels + channel]
            }
        };
        while self.position < count {
            let index = self.position.floor() as usize;
            let fraction = (self.position - index as f64) as f32;
            for channel in 0..channels {
                let a = frame_at(self, index, channel);
                let b = frame_at(self, index + 1, channel);
                self.fifo.push_back(a + (b - a) * fraction);
            }
            self.position += self.step;
        }
        self.position -= count;
        let last = &converted[(frames - 1) * channels..];
        self.previous.copy_from_slice(last);
    }
}

struct LaneState {
    name: String,
    source: Box<dyn AudioSource>,
    control: AudioControl,
    track: usize,
    lane: Lane,
    lost: bool,
    last_recover: Duration,
    silence_frames: u64,
    dropped_frames: u64,
}

struct TrackState {
    name: String,
    encoder: Encoder,
    config: Vec<u8>,
    /// Access units produced so far.
    units: i64,
}

/// Reads the sources, keeps them in step with the clock, mixes them into tracks, and encodes.
pub struct AudioEngine {
    rate: u32,
    channels: u16,
    lanes: Vec<LaneState>,
    tracks: Vec<TrackState>,
    /// Sample frames produced per track so far.
    emitted: i64,
    scratch: Vec<f32>,
}

impl AudioEngine {
    /// Opens the encoders. Errors with a reason when the layout is not valid.
    pub fn new(setup: AudioSetup) -> Result<Self, String> {
        let AudioSetup { config, inputs } = setup;
        if !config.valid() {
            return Err("the audio layout cannot be recorded".to_owned());
        }
        if inputs
            .iter()
            .any(|input| input.track >= config.tracks.len())
        {
            return Err("an audio source feeds a track that does not exist".to_owned());
        }
        let channels = usize::from(config.channels);
        // AAC allows 6144 bits per channel per frame; 6 bits a sample.
        let ceiling = 6 * config
            .sample_rate
            .saturating_mul(u32::from(config.channels));
        let bits = (config.kbps * 1000).min(ceiling);
        let mut tracks = Vec::new();
        for name in &config.tracks {
            let encoder = Encoder::new(EncoderConfig::cbr(config.sample_rate, channels, bits))
                .map_err(|error| error.to_string())?;
            tracks.push(TrackState {
                name: name.clone(),
                config: encoder.audio_specific_config(),
                encoder,
                units: 0,
            });
        }
        let lanes = inputs
            .into_iter()
            .map(|input| {
                let format = input.source.format();
                let mut lane = Lane::new(
                    AudioFormat {
                        sample_rate: format.sample_rate.max(1),
                        channels: format.channels.max(1),
                    },
                    config.sample_rate,
                    config.channels,
                );
                lane.mono_mix = input.mono;
                LaneState {
                    name: input.source.name(),
                    lane,
                    source: input.source,
                    control: input.control,
                    track: input.track,
                    lost: false,
                    last_recover: Duration::ZERO,
                    silence_frames: 0,
                    dropped_frames: 0,
                }
            })
            .collect();
        Ok(Self {
            rate: config.sample_rate,
            channels: config.channels,
            lanes,
            tracks,
            emitted: 0,
            scratch: Vec::new(),
        })
    }

    /// What a file needs to know about each track.
    pub fn tracks(&self) -> Vec<AudioTrackInfo> {
        self.tracks
            .iter()
            .map(|track| AudioTrackInfo {
                name: track.name.clone(),
                sample_rate: self.rate,
                channels: self.channels,
                config: track.config.clone(),
                priming: AAC_FRAME as u32,
            })
            .collect()
    }

    /// How each source is doing.
    pub fn status(&self) -> Vec<AudioInputStatus> {
        let per_ms = f64::from(self.rate) / 1000.0;
        self.lanes
            .iter()
            .map(|lane| AudioInputStatus {
                name: lane.name.clone(),
                track: lane.track,
                level: lane.control.level(),
                muted: lane.control.muted(),
                lost: lane.lost,
                silence_ms: (lane.silence_frames as f64 / per_ms) as u64,
                dropped_ms: (lane.dropped_frames as f64 / per_ms) as u64,
            })
            .collect()
    }

    /// Reads every source and produces the sound due at `elapsed` since the recording started.
    /// Returns the new access units with their track.
    pub fn tick(&mut self, elapsed: Duration) -> Vec<(usize, AudioPacket)> {
        for lane in &mut self.lanes {
            Self::read_lane(lane, elapsed, &mut self.scratch);
        }
        let slack = Duration::from_millis(ARRIVAL_SLACK_MS);
        let due_time = elapsed.saturating_sub(slack);
        let target = i64::try_from(due_time.as_nanos() * u128::from(self.rate) / 1_000_000_000)
            .unwrap_or(i64::MAX);
        let mut out = Vec::new();
        while target - self.emitted >= MIN_BATCH {
            // At most a second at a time, so a stalled thread catches up in bounded pieces.
            let frames = (target - self.emitted).min(i64::from(self.rate));
            self.produce(frames as usize, &mut out);
            self.emitted += frames;
        }
        out
    }

    fn read_lane(lane: &mut LaneState, elapsed: Duration, scratch: &mut Vec<f32>) {
        if lane.lost {
            if elapsed.saturating_sub(lane.last_recover) >= Duration::from_secs(2) {
                lane.last_recover = elapsed;
                if lane.source.recover() {
                    lane.lost = false;
                }
            }
            return;
        }
        scratch.clear();
        match lane.source.read(scratch) {
            Ok(()) => {
                let peak = scratch
                    .iter()
                    .filter(|sample| sample.is_finite())
                    .fold(0.0_f32, |loudest, sample| loudest.max(sample.abs()));
                lane.control.set_level(peak.min(1.0));
                lane.lane.push(scratch);
            }
            Err(_) => {
                lane.lost = true;
                lane.last_recover = elapsed;
                lane.control.set_level(0.0);
            }
        }
    }

    fn produce(&mut self, frames: usize, out: &mut Vec<(usize, AudioPacket)>) {
        let channels = usize::from(self.channels);
        let lag_frames = (u64::from(self.rate) * MAX_LAG_MS / 1000) as usize;
        let mut mixes = vec![vec![0.0_f32; frames * channels]; self.tracks.len()];
        for lane in &mut self.lanes {
            let available = lane.lane.frames();
            // Drop what is more than the allowed lag ahead of the clock.
            if available > frames + lag_frames {
                let excess = available - frames - lag_frames;
                lane.lane.fifo.drain(..excess * channels);
                lane.dropped_frames += excess as u64;
            }
            let take = lane.lane.frames().min(frames);
            lane.silence_frames += (frames - take) as u64;
            let factor = lane.control.factor();
            if let Some(mix) = mixes.get_mut(lane.track) {
                for (slot, sample) in mix.iter_mut().zip(lane.lane.fifo.drain(..take * channels)) {
                    *slot += sample * factor;
                }
            } else {
                lane.lane.fifo.drain(..take * channels);
            }
        }
        for (index, (track, mix)) in self.tracks.iter_mut().zip(mixes).enumerate() {
            let planar: Vec<Vec<f32>> = (0..channels)
                .map(|channel| {
                    mix.chunks_exact(channels)
                        .map(|frame| frame[channel].clamp(-1.0, 1.0))
                        .collect()
                })
                .collect();
            let refs: Vec<&[f32]> = planar.iter().map(Vec::as_slice).collect();
            for data in track.encoder.encode(&refs) {
                // Unit `n` holds the input sound of `[(n - 1) * 1024, n * 1024)`: the first unit
                // is the encoder's delay.
                let start = (track.units - 1) * AAC_FRAME;
                track.units += 1;
                out.push((index, AudioPacket { start, data }));
            }
        }
    }
}

/// The newest encoded sound, bounded by time and by bytes, per track.
#[derive(Debug)]
pub struct AudioBuffer {
    tracks: Vec<VecDeque<AudioPacket>>,
    bytes: usize,
    keep_samples: i64,
    max_bytes: usize,
}

impl AudioBuffer {
    /// Keeps at least `keep_samples` of the newest sound on every track and about `max_bytes` in
    /// all.
    pub fn new(tracks: usize, keep_samples: i64, max_bytes: usize) -> Self {
        Self {
            tracks: (0..tracks).map(|_| VecDeque::new()).collect(),
            bytes: 0,
            keep_samples: keep_samples.max(AAC_FRAME),
            max_bytes: max_bytes.max(1),
        }
    }

    /// Adds a unit to a track, then drops what is too old.
    pub fn push(&mut self, track: usize, packet: AudioPacket) {
        let Some(queue) = self.tracks.get_mut(track) else {
            return;
        };
        self.bytes += packet.data.len();
        let newest = packet.start;
        queue.push_back(packet);
        while let Some(front) = queue.front() {
            let too_old = newest - front.start > self.keep_samples;
            let too_big = self.bytes > self.max_bytes && queue.len() > 1;
            if !too_old && !too_big {
                break;
            }
            self.bytes = self.bytes.saturating_sub(front.data.len());
            queue.pop_front();
        }
    }

    /// The units of `track` that cover the sound from input sample `from` on, starting with the
    /// one before it so the decoder has its overlap.
    pub fn snapshot(&self, track: usize, from: i64) -> Vec<AudioPacket> {
        let Some(queue) = self.tracks.get(track) else {
            return Vec::new();
        };
        let first = queue
            .iter()
            .position(|packet| packet.start + AAC_FRAME > from)
            .unwrap_or(queue.len());
        queue
            .iter()
            .skip(first.saturating_sub(1))
            .cloned()
            .collect()
    }

    /// Bytes stored.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Units stored on a track.
    pub fn len(&self, track: usize) -> usize {
        self.tracks.get(track).map_or(0, VecDeque::len)
    }

    /// True when no track holds anything.
    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(VecDeque::is_empty)
    }
}

/// Samples waiting to be read, bounded, oldest dropped first.
#[derive(Debug)]
pub struct SampleQueue {
    samples: Mutex<VecDeque<f32>>,
    capacity: usize,
    failed: AtomicBool,
}

impl SampleQueue {
    /// A queue that holds at most `capacity` samples.
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
            failed: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<f32>> {
        self.samples.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Appends samples, dropping the oldest ones that no longer fit.
    pub fn push(&self, incoming: impl ExactSizeIterator<Item = f32>) {
        let mut samples = self.lock();
        let incoming_len = incoming.len();
        if incoming_len >= self.capacity {
            samples.clear();
            samples.extend(incoming.skip(incoming_len - self.capacity));
            return;
        }
        let overflow = (samples.len() + incoming_len).saturating_sub(self.capacity);
        let drop_count = overflow.min(samples.len());
        samples.drain(..drop_count);
        samples.extend(incoming);
    }

    /// Moves everything waiting into `out`.
    pub fn drain_into(&self, out: &mut Vec<f32>) {
        out.extend(self.lock().drain(..));
    }

    /// Marks the device as gone.
    pub fn fail(&self) {
        self.failed.store(true, Ordering::SeqCst);
    }

    /// True once the device reported an error.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    /// Samples waiting.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// True when nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A test source: a steady tone generated against the real clock, optionally running fast or
/// slow, going silent, or failing after a while.
pub struct ToneSource {
    format: AudioFormat,
    frequency: f32,
    amplitude: f32,
    /// When the first read happened: the source's time zero.
    started: Option<std::time::Instant>,
    produced: u64,
    /// A 10 ms burst of full-scale tone at this time after the first read.
    pub click_at: Option<Duration>,
    /// Parts per million the device clock runs fast (negative: slow).
    pub clock_error_ppm: i64,
    /// Fails with `DeviceLost` once this many seconds have passed.
    pub fail_after: Option<Duration>,
    /// Set once `recover` has been called and succeeded.
    pub recoverable: bool,
    failed: bool,
}

impl ToneSource {
    /// A tone of `frequency` hertz at `amplitude` (0.0 to 1.0).
    pub fn new(format: AudioFormat, frequency: f32, amplitude: f32) -> Self {
        Self {
            format,
            frequency,
            amplitude,
            started: None,
            produced: 0,
            click_at: None,
            clock_error_ppm: 0,
            fail_after: None,
            recoverable: false,
            failed: false,
        }
    }
}

impl AudioSource for ToneSource {
    fn name(&self) -> String {
        format!("Tone {} Hz", self.frequency)
    }

    fn format(&self) -> AudioFormat {
        self.format
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
        let elapsed = self
            .started
            .get_or_insert_with(std::time::Instant::now)
            .elapsed();
        if self.failed || self.fail_after.is_some_and(|after| elapsed >= after) {
            self.failed = true;
            return Err(AudioError::DeviceLost("test device removed".to_owned()));
        }
        let skew = 1.0 + self.clock_error_ppm as f64 / 1_000_000.0;
        let wanted = (elapsed.as_secs_f64() * skew * f64::from(self.format.sample_rate)) as u64;
        while self.produced < wanted {
            let time = self.produced as f64 / f64::from(self.format.sample_rate);
            let clicking = self
                .click_at
                .is_some_and(|at| time >= at.as_secs_f64() && time < at.as_secs_f64() + 0.010);
            let level = if clicking { 1.0 } else { self.amplitude };
            let value =
                (std::f64::consts::TAU * f64::from(self.frequency) * time).sin() as f32 * level;
            for _ in 0..self.format.channels {
                out.push(value);
            }
            self.produced += 1;
        }
        Ok(())
    }

    fn recover(&mut self) -> bool {
        if self.recoverable {
            self.failed = false;
            self.fail_after = None;
            self.started = None;
            self.produced = 0;
        }
        self.recoverable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_queue_is_bounded_and_keeps_the_newest_samples() {
        let queue = SampleQueue::new(10);
        queue.push((0..6).map(|n| n as f32));
        queue.push((6..12).map(|n| n as f32));
        let mut out = Vec::new();
        queue.drain_into(&mut out);
        assert_eq!(out, (2..12).map(|n| n as f32).collect::<Vec<_>>());
        // A burst bigger than the queue keeps only its tail.
        queue.push((0..100).map(|n| n as f32));
        out.clear();
        queue.drain_into(&mut out);
        assert_eq!(out, (90..100).map(|n| n as f32).collect::<Vec<_>>());
        assert!(queue.is_empty());
    }

    #[test]
    fn a_failed_device_is_remembered() {
        let queue = SampleQueue::new(4);
        assert!(!queue.failed());
        queue.fail();
        assert!(queue.failed());
    }

    const STEREO_48K: AudioFormat = AudioFormat {
        sample_rate: 48_000,
        channels: 2,
    };

    fn config(tracks: usize) -> AudioConfig {
        AudioConfig {
            sample_rate: 48_000,
            channels: 2,
            kbps: 128,
            tracks: (0..tracks).map(|i| format!("Track {i}")).collect(),
        }
    }

    /// A source that hands out whatever the test queued.
    struct Queue {
        format: AudioFormat,
        pending: Vec<f32>,
        lost: bool,
    }

    impl AudioSource for Queue {
        fn name(&self) -> String {
            "queue".to_owned()
        }
        fn format(&self) -> AudioFormat {
            self.format
        }
        fn read(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
            if self.lost {
                return Err(AudioError::DeviceLost("gone".to_owned()));
            }
            out.append(&mut self.pending);
            Ok(())
        }
    }

    fn sine(rate: u32, channels: u16, seconds: f32, hz: f32) -> Vec<f32> {
        let frames = (rate as f32 * seconds) as usize;
        let mut out = Vec::new();
        for n in 0..frames {
            let v = (std::f32::consts::TAU * hz * n as f32 / rate as f32).sin() * 0.5;
            for _ in 0..channels {
                out.push(v);
            }
        }
        out
    }

    fn zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count()
    }

    #[test]
    fn a_lane_changes_the_rate_and_keeps_the_pitch_and_length() {
        for (rate, channels) in [
            (44_100, 2),
            (44_100, 1),
            (16_000, 1),
            (96_000, 2),
            (48_000, 1),
        ] {
            let mut lane = Lane::new(
                AudioFormat {
                    sample_rate: rate,
                    channels,
                },
                48_000,
                2,
            );
            // Pushed in awkward pieces.
            let input = sine(rate, channels, 1.0, 440.0);
            for piece in input.chunks(7 * usize::from(channels)) {
                lane.push(piece);
            }
            let frames = lane.frames();
            assert!(
                (47_990..=48_010).contains(&frames),
                "{rate}/{channels}: {frames}"
            );
            let left: Vec<f32> = lane.fifo.iter().step_by(2).copied().collect();
            let crossings = zero_crossings(&left);
            assert!(
                (439..=441).contains(&crossings),
                "{rate}/{channels}: {crossings} Hz"
            );
            assert!(left.iter().all(|v| v.is_finite() && v.abs() <= 0.51));
        }
    }

    #[test]
    fn channels_are_converted_both_ways() {
        let mut to_mono = Lane::new(STEREO_48K, 48_000, 1);
        to_mono.push(&[1.0, -1.0, 0.5, 0.5]);
        assert_eq!(
            to_mono.fifo.iter().copied().collect::<Vec<_>>(),
            vec![0.0, 0.5]
        );
        let mut to_stereo = Lane::new(
            AudioFormat {
                sample_rate: 48_000,
                channels: 1,
            },
            48_000,
            2,
        );
        to_stereo.push(&[0.25, -0.5]);
        assert_eq!(
            to_stereo.fifo.iter().copied().collect::<Vec<_>>(),
            vec![0.25, 0.25, -0.5, -0.5]
        );
        // Surround keeps its front pair.
        let mut surround = Lane::new(
            AudioFormat {
                sample_rate: 48_000,
                channels: 6,
            },
            48_000,
            2,
        );
        surround.push(&[0.1, 0.2, 9.0, 9.0, 9.0, 9.0]);
        assert_eq!(
            surround.fifo.iter().copied().collect::<Vec<_>>(),
            vec![0.1, 0.2]
        );
    }

    #[test]
    fn garbage_samples_become_silence_or_are_limited() {
        let mut lane = Lane::new(STEREO_48K, 48_000, 2);
        lane.push(&[f32::NAN, f32::INFINITY, 1e9, -1e9]);
        assert!(lane.fifo.iter().all(|v| v.is_finite() && v.abs() <= 4.0));
        // A dangling half frame and an empty push are ignored.
        lane.push(&[0.5]);
        lane.push(&[]);
        assert_eq!(lane.frames(), 2);
    }

    #[test]
    fn the_engine_produces_exactly_what_the_clock_asks_for() {
        let mut engine = AudioEngine::new(AudioSetup {
            config: config(1),
            inputs: vec![AudioInput::new(
                Box::new(Queue {
                    format: STEREO_48K,
                    pending: sine(48_000, 2, 3.0, 440.0),
                    lost: false,
                }),
                AudioControl::default(),
                0,
            )],
        })
        .unwrap();
        let mut units = Vec::new();
        for ms in (0..=2_000).step_by(10) {
            units.extend(engine.tick(Duration::from_millis(ms)));
        }
        // 2 s minus the 40 ms arrival slack, in whole 1024 units (the encoder holds a partial one).
        let produced = engine.emitted;
        assert_eq!(produced, 48_000 * 1_960 / 1_000);
        assert_eq!(units.len() as i64, produced / AAC_FRAME);
        assert_eq!(units[0].1.start, -AAC_FRAME);
        assert_eq!(units[1].1.start, 0);
        assert!(
            units
                .windows(2)
                .all(|w| w[1].1.start - w[0].1.start == AAC_FRAME)
        );
        assert!(units.iter().all(|(_, p)| !p.data.is_empty()));
    }

    #[test]
    fn a_late_source_is_padded_with_silence_and_a_fast_one_loses_its_oldest_sound() {
        let (late_id, fast_id) = (0, 1);
        let mut engine = AudioEngine::new(AudioSetup {
            config: config(2),
            inputs: vec![
                AudioInput::new(
                    Box::new(Queue {
                        format: STEREO_48K,
                        pending: Vec::new(),
                        lost: false,
                    }),
                    AudioControl::default(),
                    late_id,
                ),
                AudioInput::new(
                    Box::new(Queue {
                        format: STEREO_48K,
                        pending: sine(48_000, 2, 2.0, 440.0),
                        lost: false,
                    }),
                    AudioControl::default(),
                    fast_id,
                ),
            ],
        })
        .unwrap();
        for ms in (0..=1_000).step_by(10) {
            engine.tick(Duration::from_millis(ms));
        }
        let status = engine.status();
        assert!(status[0].silence_ms >= 900, "{:?}", status[0]);
        assert_eq!(status[0].dropped_ms, 0);
        // The fast source was ahead by about a second: all but the lag allowance was dropped.
        assert!(status[1].dropped_ms >= 1_500, "{:?}", status[1]);
    }

    #[test]
    fn mute_and_gain_apply_live_and_the_meter_ignores_them() {
        let control = AudioControl::default();
        control.set_gain_percent(5_000);
        assert_eq!(control.gain_percent(), MAX_GAIN_PERCENT);
        control.set_muted(true);
        assert_eq!(control.factor(), 0.0);
        control.set_muted(false);
        control.set_gain_percent(50);
        assert_eq!(control.factor(), 0.5);
        let mut engine = AudioEngine::new(AudioSetup {
            config: config(1),
            inputs: vec![AudioInput::new(
                Box::new(Queue {
                    format: STEREO_48K,
                    pending: sine(48_000, 2, 1.0, 440.0),
                    lost: false,
                }),
                control.clone(),
                0,
            )],
        })
        .unwrap();
        engine.tick(Duration::from_millis(100));
        assert!((control.level() - 0.5).abs() < 0.01, "{}", control.level());
    }

    #[test]
    fn a_lost_device_goes_silent_and_comes_back() {
        struct Flaky {
            lost: bool,
            recovered: bool,
        }
        impl AudioSource for Flaky {
            fn name(&self) -> String {
                "flaky".to_owned()
            }
            fn format(&self) -> AudioFormat {
                STEREO_48K
            }
            fn read(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
                if self.lost {
                    return Err(AudioError::DeviceLost("unplugged".to_owned()));
                }
                out.extend(std::iter::repeat_n(0.1, 960));
                Ok(())
            }
            fn recover(&mut self) -> bool {
                self.recovered = true;
                self.lost = false;
                true
            }
        }
        let mut engine = AudioEngine::new(AudioSetup {
            config: config(1),
            inputs: vec![AudioInput::new(
                Box::new(Flaky {
                    lost: true,
                    recovered: false,
                }),
                AudioControl::default(),
                0,
            )],
        })
        .unwrap();
        engine.tick(Duration::from_millis(10));
        assert!(engine.status()[0].lost);
        engine.tick(Duration::from_millis(500));
        assert!(
            engine.status()[0].lost,
            "recovery is not retried every tick"
        );
        engine.tick(Duration::from_millis(2_100));
        assert!(!engine.status()[0].lost);
    }

    #[test]
    fn bad_layouts_are_refused() {
        for bad in [
            AudioConfig {
                sample_rate: 12_345,
                ..config(1)
            },
            AudioConfig {
                channels: 3,
                ..config(1)
            },
            AudioConfig {
                kbps: 0,
                ..config(1)
            },
            config(0),
        ] {
            assert!(
                AudioEngine::new(AudioSetup {
                    config: bad,
                    inputs: Vec::new()
                })
                .is_err()
            );
        }
        let wrong_track = AudioInput::new(
            Box::new(Queue {
                format: STEREO_48K,
                pending: Vec::new(),
                lost: false,
            }),
            AudioControl::default(),
            3,
        );
        assert!(
            AudioEngine::new(AudioSetup {
                config: config(1),
                inputs: vec![wrong_track]
            })
            .is_err()
        );
    }

    #[test]
    fn the_buffer_is_bounded_and_snapshots_include_the_warm_up_unit() {
        let mut buffer = AudioBuffer::new(1, 10 * AAC_FRAME, usize::MAX);
        for n in 0..100 {
            buffer.push(
                0,
                AudioPacket {
                    start: (n - 1) * AAC_FRAME,
                    data: vec![0; 100],
                },
            );
        }
        assert!(buffer.len(0) <= 12, "{}", buffer.len(0));
        let from = 95 * AAC_FRAME + 10;
        let snapshot = buffer.snapshot(0, from);
        // The unit covering `from` starts at 95 * 1024; the snapshot begins one unit earlier.
        assert_eq!(snapshot[0].start, 94 * AAC_FRAME);
        assert_eq!(snapshot.last().map(|p| p.start), Some(98 * AAC_FRAME));
        assert!(buffer.snapshot(5, 0).is_empty());

        let mut capped = AudioBuffer::new(1, i64::MAX / 4, 1_000);
        for n in 0..100 {
            capped.push(
                0,
                AudioPacket {
                    start: n * AAC_FRAME,
                    data: vec![0; 100],
                },
            );
        }
        assert!(capped.bytes() <= 1_100);
        capped.push(
            9,
            AudioPacket {
                start: 0,
                data: vec![0; 100],
            },
        );
    }
}
