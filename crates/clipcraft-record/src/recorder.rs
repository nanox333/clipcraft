//! The recording threads and the status the rest of the app reads.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use clipcraft_capture::reframe::{Framing, OutputShape, PixelRect, Reframe, reframe};
use clipcraft_capture::{
    CaptureError, CaptureFrame, CaptureSource, MAX_CAPTURE_EDGE, NativeCaptureFrame,
};
use filmcraft_h264enc::{Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl};

use crate::RecordError;
use crate::audio::{AudioBuffer, AudioEngine, AudioInputStatus, AudioSetup, AudioTrackInfo};
use crate::convert::{convert, convert_rgba};
use crate::hardware::{
    HardwareEncoder, HardwareProvider, HardwareRequest, StreamCodec, SurfaceFit,
};
use crate::mux::{ClipAudio, ClipFormat, SavedClip, VideoInfo, write_clip};
use crate::replay::{EncodedSample, ReplayBuffer};
use crate::software::{Delivery, SoftwareEncoder};

/// How hard the encoder works.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncoderSpeed {
    /// Least processor use.
    Fast,
    /// A balance of speed and size.
    Balanced,
    /// Smallest files, most processor use.
    Quality,
}

/// Everything the recorder needs to know.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordConfig {
    /// Height of the clip's frame in pixels; the width follows from `shape`.
    pub output_height: u32,
    /// The shape of the clip's frame.
    pub shape: OutputShape,
    /// How the picture is fitted into that frame.
    pub framing: Framing,
    /// Frames per second.
    pub fps: u32,
    /// Video bitrate in kilobits per second.
    pub video_kbps: u32,
    /// Hold the bitrate steady instead of letting it follow the action.
    pub constant_bitrate: bool,
    /// How hard the encoder works.
    pub speed: EncoderSpeed,
    /// Seconds between keyframes.
    pub keyframe_seconds: u32,
    /// Seconds of play the buffer keeps.
    pub replay_seconds: u32,
    /// Most encoded video the buffer holds, in bytes.
    pub max_buffer_bytes: usize,
    /// Threads the encoder and the pixel conversion may use (0 chooses half the processors, and
    /// never more than four, so a game keeps the rest).
    pub threads: usize,
    /// The format to record in. Only a hardware encoder can make H.265; without one, or when it
    /// fails, the recording is H.264 and the status says why.
    pub codec: StreamCodec,
    /// Opens a hardware encoder to use instead of the software one. When it cannot, or fails
    /// later, the software encoder takes over.
    pub hardware: Option<HardwareProvider>,
    /// A silence in the pictures this long (the computer slept, or the screen went away) starts
    /// the replay buffer over, instead of leaving a frozen stretch in every clip.
    pub gap_restart_seconds: u32,
}

/// Most threads the software encoder takes when the number is left to the recorder. Encoding in
/// bursts across eight threads finishes each picture sooner than four, which is what keeps 1080p
/// at 60 pictures a second from dropping pictures.
const MAX_AUTO_THREADS: usize = 8;

/// Half the processors, but never more than [`MAX_AUTO_THREADS`]: on a big machine the rest stay
/// free for the game.
fn auto_threads(cores: usize) -> usize {
    (cores / 2).clamp(1, MAX_AUTO_THREADS)
}

impl RecordConfig {
    fn validate(&self) -> Result<(), RecordError> {
        let ok = (1..=240).contains(&self.fps)
            && self.video_kbps >= 1
            && (1..=3_600).contains(&self.replay_seconds)
            && (1..=60).contains(&self.keyframe_seconds)
            && self.max_buffer_bytes >= 1_000_000
            && (1..=3_600).contains(&self.gap_restart_seconds)
            && self.output_height >= 16;
        if ok {
            Ok(())
        } else {
            Err(RecordError::BadConfig)
        }
    }

    fn worker_threads(&self) -> usize {
        if self.threads > 0 {
            return self.threads.min(32);
        }
        auto_threads(std::thread::available_parallelism().map_or(2, usize::from))
    }
}

/// The clip height for a source `source_height` high. A clip that keeps the source's shape is
/// never made taller than the source: enlarging a 1440x900 desktop to "1080p" adds no detail but
/// about 45% more pictures' worth of pixels for the software encoder, which is what makes it drop
/// frames. Other shapes (16:9, 9:16, square...) keep the requested height.
fn clip_height(config: &RecordConfig, source_height: u32) -> u32 {
    if config.shape == OutputShape::Source {
        config.output_height.min(source_height.max(16))
    } else {
        config.output_height
    }
}

/// The clip frame size a source of `width` x `height` is recorded at.
pub fn output_size(config: &RecordConfig, width: u32, height: u32) -> Option<(u32, u32)> {
    reframe(
        width,
        height,
        config.shape,
        config.framing,
        clip_height(config, height),
    )
    .map(|plan| (plan.output_width, plan.output_height))
}

/// Whether the recorder is working.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordState {
    /// Capturing and encoding.
    Running,
    /// Stopped on request.
    Stopped,
    /// Stopped by an error. The buffered video can still be saved.
    Failed(RecordError),
}

/// A snapshot of how recording is going.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordStatus {
    /// Running, stopped, or failed.
    pub state: RecordState,
    /// Name of what is being captured.
    pub source: String,
    /// Pictures taken from the screen.
    pub frames_captured: u64,
    /// Pictures encoded.
    pub frames_encoded: u64,
    /// Pictures skipped because the encoder was still busy, or came too late.
    pub frames_dropped: u64,
    /// Times the stream restarted because the picture size changed.
    pub restarts: u64,
    /// Pictures in the replay buffer.
    pub buffered_frames: u64,
    /// Bytes in the replay buffer.
    pub buffered_bytes: u64,
    /// Seconds of play in the replay buffer.
    pub buffered_seconds: f32,
    /// Complete replay resets that discarded buffered video during this recording.
    pub buffer_resets: u64,
    /// The clip frame size, once the first picture arrived.
    pub output: Option<(u32, u32)>,
    /// Average time to convert and encode one picture, in milliseconds.
    pub average_encode_ms: f32,
    /// Frames per second the recorder is aiming for.
    pub target_fps: u32,
    /// Which encoder is working, and why when it is not the one asked for.
    pub encoder: String,
    /// How each sound source is doing; empty when no sound is recorded.
    pub audio: Vec<AudioInputStatus>,
}

#[derive(Default)]
struct Stats {
    captured: u64,
    encoded: u64,
    dropped: u64,
    restarts: u64,
    encode_nanos: u128,
}

struct Queued {
    frame: QueuedFrame,
    pts: i64,
}

enum QueuedFrame {
    Cpu(CaptureFrame),
    Surface {
        surface: Box<dyn NativeCaptureFrame>,
        readback: Option<CaptureFrame>,
    },
}

impl QueuedFrame {
    fn dimensions(&self) -> Result<(u32, u32), RecordError> {
        let (width, height) = match self {
            Self::Cpu(frame) => {
                frame.validate().map_err(|_| RecordError::BadFrame)?;
                (frame.width, frame.height)
            }
            Self::Surface { surface, .. } => (surface.width(), surface.height()),
        };
        if width == 0 || height == 0 || width > MAX_CAPTURE_EDGE || height > MAX_CAPTURE_EDGE {
            return Err(RecordError::BadFrame);
        }
        Ok((width, height))
    }

    fn readback(&mut self) -> Result<&CaptureFrame, RecordError> {
        let frame = match self {
            Self::Cpu(frame) => frame,
            Self::Surface { surface, readback } => {
                if readback.is_none() {
                    *readback = Some(surface.readback().map_err(RecordError::Capture)?);
                }
                readback.as_ref().ok_or(RecordError::BadFrame)?
            }
        };
        frame.validate().map_err(|_| RecordError::BadFrame)?;
        Ok(frame)
    }
}

#[derive(Default)]
struct Mailbox {
    slot: Option<Queued>,
    capture_done: bool,
}

struct Shared {
    stop: AtomicBool,
    fps: u32,
    source_name: String,
    buffer: Mutex<ReplayBuffer>,
    video: Mutex<Option<VideoInfo>>,
    encoder: Mutex<String>,
    audio_buffer: Mutex<AudioBuffer>,
    audio_tracks: Vec<AudioTrackInfo>,
    audio_status: Mutex<Vec<AudioInputStatus>>,
    stats: Mutex<Stats>,
    state: Mutex<RecordState>,
    mailbox: Mutex<Mailbox>,
    wake: Condvar,
    /// A request for the next captured picture, for a screenshot, and where it is left.
    screenshot_wanted: AtomicBool,
    /// The encoder has opened and taken its first picture. Pictures replaced before that were
    /// only waiting for it to start, so they are not counted as dropped.
    encoder_ready: AtomicBool,
    screenshot: Mutex<Option<CaptureFrame>>,
    /// The requested picture could not be read back, so the waiter need not wait it out.
    screenshot_failed: AtomicBool,
    screenshot_ready: Condvar,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    fn fail(&self, error: RecordError) {
        let mut state = lock(&self.state);
        if *state == RecordState::Running {
            *state = RecordState::Failed(error);
        }
    }
}

/// A running recording. Dropping it stops it.
pub struct Recorder {
    shared: Arc<Shared>,
    capture: Option<JoinHandle<()>>,
    encode: Option<JoinHandle<()>>,
    audio: Option<JoinHandle<()>>,
}

impl Recorder {
    /// Starts capturing `source` into a replay buffer.
    pub fn start(
        source: Box<dyn CaptureSource>,
        config: RecordConfig,
    ) -> Result<Self, RecordError> {
        Self::start_with_audio(source, config, None)
    }

    /// Starts capturing `source`, and the sound sources in `audio`, into a replay buffer.
    pub fn start_with_audio(
        source: Box<dyn CaptureSource>,
        config: RecordConfig,
        audio: Option<AudioSetup>,
    ) -> Result<Self, RecordError> {
        config.validate()?;
        let epoch = Instant::now();
        let (engine, audio_rate, track_count) = match audio {
            Some(setup) => {
                let rate = setup.config.sample_rate;
                let tracks = setup.config.tracks.len();
                let kbps = setup.config.kbps;
                let engine = AudioEngine::new(setup).map_err(|_| RecordError::AudioFailed)?;
                (Some((engine, kbps)), rate, tracks)
            }
            None => (None, 48_000, 0),
        };
        let audio_tracks = engine.as_ref().map_or_else(Vec::new, |(e, _)| e.tracks());
        let seconds_kept =
            i64::from(config.replay_seconds) + i64::from(config.keyframe_seconds) + 3;
        let audio_bytes = engine.as_ref().map_or(0, |(_, kbps)| {
            (u64::from(*kbps) * 125 * seconds_kept as u64).saturating_mul(track_count as u64)
        });
        let window_frames = i64::from(config.replay_seconds) * i64::from(config.fps);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            fps: config.fps,
            source_name: source.target().name.clone(),
            buffer: Mutex::new(ReplayBuffer::new(window_frames, config.max_buffer_bytes)),
            video: Mutex::new(None),
            encoder: Mutex::new(String::from("Starting")),
            audio_buffer: Mutex::new(AudioBuffer::new(
                track_count,
                seconds_kept * i64::from(audio_rate),
                usize::try_from(audio_bytes.saturating_mul(2).max(1 << 20)).unwrap_or(usize::MAX),
            )),
            audio_tracks,
            audio_status: Mutex::new(Vec::new()),
            stats: Mutex::new(Stats::default()),
            state: Mutex::new(RecordState::Running),
            mailbox: Mutex::new(Mailbox::default()),
            wake: Condvar::new(),
            screenshot_wanted: AtomicBool::new(false),
            encoder_ready: AtomicBool::new(false),
            screenshot: Mutex::new(None),
            screenshot_failed: AtomicBool::new(false),
            screenshot_ready: Condvar::new(),
        });
        let encode_shared = shared.clone();
        let encode_config = config.clone();
        let encode = std::thread::Builder::new()
            .name("clipcraft-encode".to_owned())
            .spawn(move || encode_loop(&encode_shared, &encode_config))
            .map_err(|_| RecordError::ThreadFailed)?;
        let capture_shared = shared.clone();
        let fps = config.fps;
        let capture = std::thread::Builder::new()
            .name("clipcraft-capture".to_owned())
            .spawn(move || capture_loop(&capture_shared, source, fps, epoch))
            .map_err(|_| {
                shared.stop.store(true, Ordering::SeqCst);
                RecordError::ThreadFailed
            })?;
        let sound = match engine {
            Some((engine, _)) => {
                let audio_shared = shared.clone();
                let handle = std::thread::Builder::new()
                    .name("clipcraft-audio".to_owned())
                    .spawn(move || audio_loop(&audio_shared, engine, epoch))
                    .map_err(|_| {
                        shared.stop.store(true, Ordering::SeqCst);
                        RecordError::ThreadFailed
                    })?;
                Some(handle)
            }
            None => None,
        };
        Ok(Self {
            shared,
            capture: Some(capture),
            encode: Some(encode),
            audio: sound,
        })
    }

    /// How recording is going.
    pub fn status(&self) -> RecordStatus {
        let stats = lock(&self.shared.stats);
        let (buffered_frames, buffered_bytes, span, buffer_resets) = {
            let buffer = lock(&self.shared.buffer);
            (
                buffer.len() as u64,
                buffer.bytes() as u64,
                buffer.span_frames(),
                buffer.resets(),
            )
        };
        let output = lock(&self.shared.video)
            .as_ref()
            .map(|video| (video.width, video.height));
        let average_encode_ms = if stats.encoded == 0 {
            0.0
        } else {
            (stats.encode_nanos / u128::from(stats.encoded)) as f32 / 1_000_000.0
        };
        RecordStatus {
            state: *lock(&self.shared.state),
            source: self.shared.source_name.clone(),
            frames_captured: stats.captured,
            frames_encoded: stats.encoded,
            frames_dropped: stats.dropped,
            restarts: stats.restarts,
            buffered_frames,
            buffered_bytes,
            buffered_seconds: span as f32 / self.shared.fps.max(1) as f32,
            buffer_resets,
            output,
            average_encode_ms,
            target_fps: self.shared.fps,
            encoder: lock(&self.shared.encoder).clone(),
            audio: lock(&self.shared.audio_status).clone(),
        }
    }

    /// Saves the newest `seconds` of video (a little more, to start on a keyframe) to `path`.
    pub fn save_clip(
        &self,
        seconds: u32,
        format: ClipFormat,
        path: &Path,
    ) -> Result<SavedClip, RecordError> {
        let (samples, video, audio) = self.clip_parts(seconds)?;
        write_clip(&samples, &video, &audio, format, path)
    }

    /// Writes the first `fragments` fragments of the clip's crash-safe copy to `path` and stops,
    /// as if the program had died there. For tests of recovery only.
    #[doc(hidden)]
    pub fn save_clip_interrupted(
        &self,
        seconds: u32,
        path: &Path,
        fragments: usize,
    ) -> Result<(), RecordError> {
        let (samples, video, audio) = self.clip_parts(seconds)?;
        crate::mux::write_clip_interrupted(&samples, &video, &audio, path, fragments)
    }

    /// The pictures, stream description, and sound that make up the newest `seconds`.
    fn clip_parts(
        &self,
        seconds: u32,
    ) -> Result<(Vec<EncodedSample>, VideoInfo, Vec<ClipAudio>), RecordError> {
        let frames = i64::from(seconds.max(1)) * i64::from(self.shared.fps);
        let samples = lock(&self.shared.buffer).snapshot(frames);
        let video = lock(&self.shared.video)
            .clone()
            .ok_or(RecordError::NothingBuffered)?;
        let mut samples = samples;
        let first_pts = samples.first().map_or(0, |sample| sample.pts);
        let audio: Vec<ClipAudio> = {
            let buffer = lock(&self.shared.audio_buffer);
            self.shared
                .audio_tracks
                .iter()
                .enumerate()
                .map(|(index, info)| {
                    let from = i64::try_from(
                        i128::from(first_pts) * i128::from(info.sample_rate)
                            / i128::from(self.shared.fps.max(1)),
                    )
                    .unwrap_or(0);
                    ClipAudio {
                        info: info.clone(),
                        packets: buffer.snapshot(index, from),
                    }
                })
                .collect()
        };
        // Sound is mixed a moment behind the clock; end the picture where the sound ends so the
        // two are the same length.
        let sound_ends = audio
            .iter()
            .filter_map(|track| {
                let last = track.packets.last()?;
                let end = last.start + crate::audio::AAC_FRAME;
                Some(
                    i128::from(end) * i128::from(self.shared.fps)
                        / i128::from(track.info.sample_rate),
                )
            })
            .min();
        if let Some(end_frame) = sound_ends {
            let end_frame = i64::try_from(end_frame).unwrap_or(i64::MAX);
            let keep = samples
                .iter()
                .rposition(|sample| sample.pts < end_frame)
                .map_or(1, |index| index + 1);
            samples.truncate(keep.max(1));
        }
        Ok((samples, video, audio))
    }

    /// The next picture captured, full size and unscaled, waiting up to `wait` for it. `None` when
    /// recording stopped or no picture came in time.
    pub fn screenshot(&self, wait: Duration) -> Option<CaptureFrame> {
        let mut slot = lock(&self.shared.screenshot);
        *slot = None;
        self.shared.screenshot_failed.store(false, Ordering::SeqCst);
        self.shared.screenshot_wanted.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + wait;
        loop {
            if let Some(frame) = slot.take() {
                return Some(frame);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero()
                || self.shared.stop.load(Ordering::SeqCst)
                || self.shared.screenshot_failed.load(Ordering::SeqCst)
            {
                self.shared.screenshot_wanted.store(false, Ordering::SeqCst);
                return None;
            }
            slot = self
                .shared
                .screenshot_ready
                .wait_timeout(slot, left.min(Duration::from_millis(100)))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Asks the threads to stop without waiting for them. Saving still works until the recorder is
    /// dropped. Used when another thread may still be saving a clip.
    pub fn halt(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake.notify_all();
        let mut state = lock(&self.shared.state);
        if *state == RecordState::Running {
            *state = RecordState::Stopped;
        }
    }

    /// Stops recording and returns the final status. The buffer is discarded.
    pub fn stop(mut self) -> RecordStatus {
        self.shutdown();
        let mut status = self.status();
        if status.state == RecordState::Running {
            status.state = RecordState::Stopped;
        }
        status
    }

    fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake.notify_all();
        for handle in [self.capture.take(), self.encode.take(), self.audio.take()]
            .into_iter()
            .flatten()
        {
            if handle.join().is_err() {
                self.shared.fail(RecordError::ThreadFailed);
            }
        }
        let mut state = lock(&self.shared.state);
        if *state == RecordState::Running {
            *state = RecordState::Stopped;
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn capture_loop(shared: &Shared, mut source: Box<dyn CaptureSource>, fps: u32, start: Instant) {
    let nanos_per_second = 1_000_000_000_u128;
    let mut next: u64 = 0;
    // Consecutive picture failures that may pass (a mode switch, a window being resized).
    let mut failures = 0_u32;
    while !shared.stop.load(Ordering::SeqCst) {
        let due = Duration::from_nanos(
            u64::try_from(u128::from(next) * nanos_per_second / u128::from(fps))
                .unwrap_or(u64::MAX),
        );
        let elapsed = start.elapsed();
        if due > elapsed {
            // Sleep in short slices so a stop request is noticed within a frame or two.
            std::thread::sleep((due - elapsed).min(Duration::from_millis(20)));
            continue;
        }
        let captured = match source.capture_surface() {
            Ok(Some(surface)) => Ok(QueuedFrame::Surface {
                surface,
                readback: None,
            }),
            Ok(None) => source.capture().map(QueuedFrame::Cpu),
            Err(error) => Err(error),
        };
        match captured {
            Ok(mut frame) => {
                failures = 0;
                let pts =
                    i64::try_from(start.elapsed().as_nanos() * u128::from(fps) / nanos_per_second)
                        .unwrap_or(i64::MAX);
                lock(&shared.stats).captured += 1;
                if shared.screenshot_wanted.swap(false, Ordering::SeqCst) {
                    match frame.readback().ok().cloned() {
                        Some(picture) => *lock(&shared.screenshot) = Some(picture),
                        None => shared.screenshot_failed.store(true, Ordering::SeqCst),
                    }
                    shared.screenshot_ready.notify_all();
                }
                let mut mailbox = lock(&shared.mailbox);
                if mailbox.slot.replace(Queued { frame, pts }).is_some()
                    && shared.encoder_ready.load(Ordering::SeqCst)
                {
                    lock(&shared.stats).dropped += 1;
                }
                drop(mailbox);
                shared.wake.notify_all();
                // If the encoder or the capture call fell behind, skip the frames already missed.
                next = (next + 1).max(u64::try_from(pts).unwrap_or(0) + 1);
            }
            Err(error @ (CaptureError::Failed | CaptureError::BadFrame)) if failures < 50 => {
                // About five seconds of trying before giving up on something that may be a blip.
                failures += 1;
                let _ = error;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                shared.fail(RecordError::Capture(error));
                break;
            }
        }
    }
    lock(&shared.mailbox).capture_done = true;
    shared.wake.notify_all();
}

fn audio_loop(shared: &Shared, mut engine: AudioEngine, epoch: Instant) {
    while !shared.stop.load(Ordering::SeqCst) {
        let packets = engine.tick(epoch.elapsed());
        if !packets.is_empty() {
            let mut buffer = lock(&shared.audio_buffer);
            for (track, packet) in packets {
                buffer.push(track, packet);
            }
        }
        *lock(&shared.audio_status) = engine.status();
        std::thread::sleep(Duration::from_millis(10));
    }
}

enum Stage {
    Software(SoftwareEncoder),
    Hardware(Box<dyn HardwareEncoder>),
}

struct Active {
    stage: Stage,
    source_size: (u32, u32),
    plan: Reframe,
    rgba: Vec<u8>,
    /// Times of the pictures a hardware encoder has taken and not yet returned.
    pending: VecDeque<i64>,
    avcc_sent: bool,
    last_pts: i64,
    /// While encoding in software although a graphics-card encoder was offered: when to try it
    /// again.
    retry: Option<Retry>,
    /// Times the graphics card's encoder has stopped in this stream, so a card that keeps failing
    /// is asked less and less often and finally left alone.
    failures: u32,
}

/// The schedule for going back to the graphics card after it was unavailable or stopped.
struct Retry {
    at: Instant,
    wait: Duration,
    tries: u32,
}

/// Failed tries after which the recorder stops asking the graphics card.
const MAX_HARDWARE_TRIES: u32 = 8;

impl Retry {
    fn first(provider: &HardwareProvider) -> Self {
        Self::after_failures(provider, 0)
    }

    /// The schedule once the card's encoder has stopped `failures` times.
    fn after_failures(provider: &HardwareProvider, failures: u32) -> Self {
        let wait = provider.retry_after().saturating_mul(1 << failures.min(6));
        Self {
            at: Instant::now() + wait,
            wait,
            tries: 0,
        }
    }
}

fn encoder_config(config: &RecordConfig, plan: &Reframe, threads: usize) -> EncoderConfig {
    let mut encoder = EncoderConfig::new(plan.output_width, plan.output_height, config.fps, 1);
    encoder.profile = Profile::High;
    encoder.preset = match config.speed {
        EncoderSpeed::Fast => Preset::Speed,
        EncoderSpeed::Balanced => Preset::Balanced,
        EncoderSpeed::Quality => Preset::Quality,
    };
    encoder.rate = if config.constant_bitrate {
        RateControl::Cbr {
            kbps: config.video_kbps,
        }
    } else {
        RateControl::Vbr {
            target_kbps: config.video_kbps,
            max_kbps: config.video_kbps.saturating_mul(2),
        }
    };
    encoder.keyint = config.keyframe_seconds.saturating_mul(config.fps).max(1);
    // B-frames would delay every picture and complicate cutting a clip on a keyframe.
    encoder.bframes = 0;
    encoder.format = PacketFormat::LengthPrefixed;
    encoder.threads = threads;
    encoder
}

fn encode_loop(shared: &Shared, config: &RecordConfig) {
    let threads = config.worker_threads();
    let mut active: Option<Active> = None;
    loop {
        let queued = {
            let mut mailbox = lock(&shared.mailbox);
            loop {
                if let Some(queued) = mailbox.slot.take() {
                    break Some(queued);
                }
                if mailbox.capture_done || shared.stop.load(Ordering::SeqCst) {
                    break None;
                }
                mailbox = shared
                    .wake
                    .wait_timeout(mailbox, Duration::from_millis(100))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        };
        let Some(Queued { mut frame, pts }) = queued else {
            break;
        };
        let started = Instant::now();
        if let Err(error) = encode_one(shared, config, threads, &mut active, &mut frame, pts) {
            shared.fail(error);
            break;
        }
        shared.encoder_ready.store(true, Ordering::SeqCst);
        lock(&shared.stats).encode_nanos += started.elapsed().as_nanos();
    }
    if let Some(mut finished) = active
        && let Err(error) = flush(shared, &mut finished)
    {
        let mut state = lock(&shared.state);
        if !matches!(*state, RecordState::Failed(_)) {
            *state = RecordState::Failed(error);
        }
    }
}

fn store_packet(shared: &Shared, data: Vec<u8>, keyframe: bool, pts: i64) {
    // Prepare shared ownership outside the replay lock. Saving clones references, not payloads.
    let data = data.into();
    // A refused sample (out of order, or no keyframe yet) is simply not stored.
    let _ = lock(&shared.buffer).push(EncodedSample {
        pts,
        data,
        keyframe,
    });
}

fn flush(shared: &Shared, active: &mut Active) -> Result<(), RecordError> {
    match &mut active.stage {
        Stage::Software(encoder) => {
            encoder.flush(&mut |delivery| store_delivery(shared, delivery))?;
        }
        Stage::Hardware(encoder) => {
            let packets = encoder.flush().map_err(|why| {
                *lock(&shared.encoder) = format!("GPU (drain failed: {why})");
                RecordError::EncoderFailed
            })?;
            for packet in packets {
                if let Some(pts) = active.pending.pop_front() {
                    store_packet(shared, packet.data, packet.keyframe, pts);
                }
            }
        }
    }
    Ok(())
}

fn software_stage(
    shared: &Shared,
    config: &RecordConfig,
    plan: &Reframe,
    threads: usize,
    label: String,
) -> Result<Stage, RecordError> {
    let encoder = Encoder::new(encoder_config(config, plan, threads))
        .map_err(|_| RecordError::EncoderFailed)?;
    let encoder = SoftwareEncoder::start(encoder, plan.output_width, plan.output_height)?;
    *lock(&shared.video) = Some(VideoInfo {
        width: plan.output_width,
        height: plan.output_height,
        fps: config.fps,
        avcc: encoder.avcc(),
        codec: StreamCodec::H264,
    });
    *lock(&shared.encoder) = label;
    Ok(Stage::Software(encoder))
}

fn hardware_request(config: &RecordConfig, plan: &Reframe) -> HardwareRequest {
    HardwareRequest {
        codec: config.codec,
        width: plan.output_width,
        height: plan.output_height,
        fps: config.fps,
        video_kbps: config.video_kbps,
        constant_bitrate: config.constant_bitrate,
        keyframe_frames: config.keyframe_seconds.saturating_mul(config.fps).max(1),
        effort: config.speed,
    }
}

/// Opens the hardware encoder if one was offered and works; otherwise the software encoder, with
/// the reason in the status.
fn open_stage(
    shared: &Shared,
    config: &RecordConfig,
    plan: &Reframe,
    threads: usize,
) -> Result<Stage, RecordError> {
    let Some(provider) = config.hardware.as_ref() else {
        let label = if config.codec == StreamCodec::H265 {
            "CPU (H.265 needs the graphics card, so this is H.264)"
        } else {
            "CPU"
        };
        return software_stage(shared, config, plan, threads, label.to_owned());
    };
    match provider.open(&hardware_request(config, plan)) {
        Ok(encoder) => {
            *lock(&shared.encoder) = format!("GPU ({})", encoder.name());
            // The decoder configuration is complete only after the first picture.
            *lock(&shared.video) = None;
            Ok(Stage::Hardware(encoder))
        }
        Err(why) => software_stage(
            shared,
            config,
            plan,
            threads,
            if config.codec == StreamCodec::H265 {
                format!("CPU (H.265 unavailable, recording H.264: {why})")
            } else {
                format!("CPU (GPU unavailable: {why})")
            },
        ),
    }
}

fn new_active(
    shared: &Shared,
    config: &RecordConfig,
    threads: usize,
    source_width: u32,
    source_height: u32,
) -> Result<Active, RecordError> {
    let plan = reframe(
        source_width,
        source_height,
        config.shape,
        config.framing,
        clip_height(config, source_height),
    )
    .ok_or(RecordError::BadSize)?;
    // Video from before a size change does not match the new stream's parameters.
    lock(&shared.buffer).clear();
    let stage = open_stage(shared, config, &plan, threads)?;
    let software = matches!(stage, Stage::Software(_));
    let retry = config
        .hardware
        .as_ref()
        .filter(|_| software)
        .map(Retry::first);
    Ok(Active {
        stage,
        source_size: (source_width, source_height),
        rgba: Vec::new(),
        pending: VecDeque::new(),
        avcc_sent: software,
        plan,
        last_pts: -1,
        retry,
        failures: 0,
    })
}

/// The hardware encoder failed: carry on in software, starting a fresh stream.
fn fail_over(
    shared: &Shared,
    config: &RecordConfig,
    threads: usize,
    active: &mut Active,
    why: &str,
) -> Result<(), RecordError> {
    lock(&shared.buffer).clear();
    let stage = software_stage(
        shared,
        config,
        &active.plan,
        threads,
        format!("CPU (GPU stopped: {why})"),
    )?;
    active.stage = stage;
    active.pending.clear();
    active.avcc_sent = true;
    active.last_pts = -1;
    active.failures += 1;
    active.retry = config
        .hardware
        .as_ref()
        .filter(|_| active.failures < MAX_HARDWARE_TRIES)
        .map(|provider| Retry::after_failures(provider, active.failures));
    lock(&shared.stats).restarts += 1;
    Ok(())
}

/// While encoding in software although a graphics-card encoder was offered, asks the card again
/// now and then (waiting twice as long after each failure). When it answers, the stream restarts
/// on the card: the replay buffer starts over, because its pictures belong to the old stream, but
/// the processor stops encoding for the rest of the session instead of until the next restart.
fn try_return_to_hardware(
    shared: &Shared,
    config: &RecordConfig,
    active: &mut Active,
) -> Result<(), RecordError> {
    let (Some(provider), Some(retry)) = (config.hardware.as_ref(), active.retry.as_mut()) else {
        return Ok(());
    };
    if !matches!(active.stage, Stage::Software(_)) || Instant::now() < retry.at {
        return Ok(());
    }
    match provider.open(&hardware_request(config, &active.plan)) {
        Ok(encoder) => {
            let _ = flush(shared, active);
            lock(&shared.buffer).clear();
            active.stage = Stage::Hardware(encoder);
            active.pending.clear();
            active.avcc_sent = false;
            active.last_pts = -1;
            active.retry = None;
            *lock(&shared.video) = None;
            lock(&shared.stats).restarts += 1;
        }
        Err(_) => {
            retry.tries += 1;
            if retry.tries >= MAX_HARDWARE_TRIES {
                active.retry = None;
            } else {
                retry.wait = retry.wait.saturating_mul(2);
                retry.at = Instant::now() + retry.wait;
            }
        }
    }
    Ok(())
}

fn encode_one(
    shared: &Shared,
    config: &RecordConfig,
    threads: usize,
    active: &mut Option<Active>,
    frame: &mut QueuedFrame,
    pts: i64,
) -> Result<(), RecordError> {
    let size = frame.dimensions()?;
    let gap_frames = i64::from(config.gap_restart_seconds) * i64::from(shared.fps);
    if active.as_ref().is_none_or(|current| {
        current.source_size != size
            || (current.last_pts >= 0 && pts - current.last_pts > gap_frames)
    }) {
        let restarted = if let Some(mut old) = active.take() {
            flush(shared, &mut old)?;
            true
        } else {
            false
        };
        let fresh = new_active(shared, config, threads, size.0, size.1)?;
        if restarted {
            lock(&shared.stats).restarts += 1;
        }
        *active = Some(fresh);
    }
    let Some(current) = active.as_mut() else {
        return Err(RecordError::EncoderFailed);
    };
    if pts <= current.last_pts {
        lock(&shared.stats).dropped += 1;
        return Ok(());
    }
    if matches!(current.stage, Stage::Software(_)) {
        try_return_to_hardware(shared, config, current)?;
    }
    if let Stage::Hardware(_) = current.stage {
        match encode_hardware(shared, current, frame, pts, threads) {
            Ok(()) => {}
            Err(why) => {
                fail_over(shared, config, threads, current, &why)?;
            }
        }
        if let Stage::Hardware(_) = current.stage {
            if current.last_pts < 0
                && let Stage::Hardware(encoder) = &current.stage
            {
                *lock(&shared.encoder) = format!("GPU ({})", encoder.name());
            }
            current.last_pts = pts;
            lock(&shared.stats).encoded += 1;
            return Ok(());
        }
        // Fell back to software: this picture is encoded below, as the new stream's first.
    }
    // The encoder works on the last picture while this one is read back and converted.
    let Active { stage, plan, .. } = &mut *current;
    let Stage::Software(encoder) = stage else {
        return Err(RecordError::EncoderFailed);
    };
    let mut deliver = |delivery| store_delivery(shared, delivery);
    encoder.poll(&mut deliver)?;
    let pixels = frame.readback()?;
    let mut picture = encoder.acquire(&mut deliver)?;
    convert(pixels, plan, &mut picture, threads)?;
    encoder.submit(picture, pts)?;
    current.last_pts = pts;
    Ok(())
}

/// Stores what the software encoder finished: packets go to the replay buffer, and a finished
/// picture counts as encoded.
fn store_delivery(shared: &Shared, delivery: Delivery) {
    match delivery {
        Delivery::Frame(packets) => {
            for packet in packets {
                store_packet(shared, packet.data, packet.keyframe, packet.pts);
            }
            lock(&shared.stats).encoded += 1;
        }
        Delivery::Tail(packets) => {
            for packet in packets {
                store_packet(shared, packet.data, packet.keyframe, packet.pts);
            }
        }
    }
}

/// Encodes one picture on the hardware encoder. Any error, or a B-frame, hands the stream back
/// to the software encoder.
fn encode_hardware(
    shared: &Shared,
    current: &mut Active,
    frame: &mut QueuedFrame,
    pts: i64,
    threads: usize,
) -> Result<(), String> {
    let direct_surface = surface_fit(current, frame);
    let Stage::Hardware(encoder) = &mut current.stage else {
        return Err("not a hardware stage".to_owned());
    };
    current.pending.push_back(pts);
    if current.pending.len() > 32 {
        return Err("hardware encoder exceeded its pending-picture bound".to_owned());
    }
    let index = u64::try_from(pts).unwrap_or(0);
    let native_packets = match (direct_surface, &mut *frame) {
        (Some(fit), QueuedFrame::Surface { surface, .. }) => {
            encoder.encode_surface(surface.as_mut(), index, fit)?
        }
        _ => None,
    };
    let packets = match native_packets {
        Some(packets) => packets,
        None => {
            let pixels = frame.readback().map_err(|error| error.to_string())?;
            convert_rgba(pixels, &current.plan, &mut current.rgba, threads)
                .map_err(|error| error.to_string())?;
            encoder
                .encode(&current.rgba, index)
                .map_err(|why| why.clone())?
        }
    };
    for packet in packets {
        if packet.reordered {
            return Err("it reorders pictures".to_owned());
        }
        let Some(time) = current.pending.pop_front() else {
            return Err("it returned more pictures than it was given".to_owned());
        };
        if !current.avcc_sent {
            *lock(&shared.video) = Some(VideoInfo {
                width: current.plan.output_width,
                height: current.plan.output_height,
                fps: shared.fps,
                avcc: encoder.avcc(),
                codec: encoder.codec(),
            });
            current.avcc_sent = true;
        }
        store_packet(shared, packet.data, packet.keyframe, time);
    }
    Ok(())
}

/// How a surface is fitted on the graphics card, or `None` when the picture must take the CPU
/// path. The crop and placement come from the same plan the CPU conversion uses, so both routes
/// give the same picture; the card scales and letterboxes (with black) instead of the processor.
/// The placement must sit on even pixels, which 4:2:0 chroma needs, and the surface must be the
/// size the plan was made for (a resized source restarts the plan).
fn surface_fit(current: &Active, frame: &QueuedFrame) -> Option<SurfaceFit> {
    let QueuedFrame::Surface { surface, .. } = frame else {
        return None;
    };
    let (width, height) = current.source_size;
    let plan = &current.plan;
    let (source, destination) = (plan.source, plan.destination);
    let inside = |rect: PixelRect, bound_width: u32, bound_height: u32| {
        rect.width > 0
            && rect.height > 0
            && rect
                .x
                .checked_add(rect.width)
                .is_some_and(|edge| edge <= bound_width)
            && rect
                .y
                .checked_add(rect.height)
                .is_some_and(|edge| edge <= bound_height)
    };
    let even = |rect: PixelRect| {
        [rect.x, rect.y, rect.width, rect.height]
            .iter()
            .all(|value| value % 2 == 0)
    };
    (surface.width() == width
        && surface.height() == height
        && inside(source, width, height)
        && inside(destination, plan.output_width, plan.output_height)
        && even(destination))
    .then_some(SurfaceFit {
        source,
        destination,
    })
}

#[cfg(test)]
mod tests {
    use super::auto_threads;

    #[test]
    fn the_software_encoder_leaves_most_of_a_big_machine_to_the_game() {
        assert_eq!(auto_threads(1), 1);
        assert_eq!(auto_threads(4), 2);
        assert_eq!(auto_threads(8), 4);
        assert_eq!(auto_threads(16), 8);
        assert_eq!(auto_threads(128), 8);
    }
}
