//! The recording threads and the status the rest of the app reads.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use clipcraft_capture::reframe::{Framing, OutputShape, Reframe, reframe};
use clipcraft_capture::{CaptureFrame, CaptureSource};
use filmcraft_h264enc::{
    Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl, YuvFrame,
};

use crate::RecordError;
use crate::audio::{AudioBuffer, AudioEngine, AudioInputStatus, AudioSetup, AudioTrackInfo};
use crate::convert::{I420, convert, convert_rgba};
use crate::hardware::{HardwareEncoder, HardwareProvider, HardwareRequest};
use crate::mux::{ClipAudio, ClipFormat, SavedClip, VideoInfo, write_clip};
use crate::replay::{EncodedSample, ReplayBuffer};

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
    /// Threads the encoder and the pixel conversion may use (0 chooses half the processors).
    pub threads: usize,
    /// Opens a hardware encoder to use instead of the software one. When it cannot, or fails
    /// later, the software encoder takes over.
    pub hardware: Option<HardwareProvider>,
}

impl RecordConfig {
    fn validate(&self) -> Result<(), RecordError> {
        let ok = (1..=240).contains(&self.fps)
            && self.video_kbps >= 1
            && (1..=3_600).contains(&self.replay_seconds)
            && (1..=60).contains(&self.keyframe_seconds)
            && self.max_buffer_bytes >= 1_000_000
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
        let cores = std::thread::available_parallelism().map_or(2, usize::from);
        (cores / 2).clamp(1, 16)
    }
}

/// The clip frame size a source of `width` x `height` is recorded at.
pub fn output_size(config: &RecordConfig, width: u32, height: u32) -> Option<(u32, u32)> {
    reframe(
        width,
        height,
        config.shape,
        config.framing,
        config.output_height,
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
    frame: CaptureFrame,
    pts: i64,
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
        let (buffered_frames, buffered_bytes, span) = {
            let buffer = lock(&self.shared.buffer);
            (
                buffer.len() as u64,
                buffer.bytes() as u64,
                buffer.span_frames(),
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
        match source.capture() {
            Ok(frame) => {
                let pts =
                    i64::try_from(start.elapsed().as_nanos() * u128::from(fps) / nanos_per_second)
                        .unwrap_or(i64::MAX);
                lock(&shared.stats).captured += 1;
                let mut mailbox = lock(&shared.mailbox);
                if mailbox.slot.replace(Queued { frame, pts }).is_some() {
                    lock(&shared.stats).dropped += 1;
                }
                drop(mailbox);
                shared.wake.notify_all();
                // If the encoder or the capture call fell behind, skip the frames already missed.
                next = (next + 1).max(u64::try_from(pts).unwrap_or(0) + 1);
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
    Software(Box<Encoder>),
    Hardware(Box<dyn HardwareEncoder>),
}

struct Active {
    stage: Stage,
    source_size: (u32, u32),
    plan: Reframe,
    picture: I420,
    rgba: Vec<u8>,
    /// Times of the pictures a hardware encoder has taken and not yet returned.
    pending: VecDeque<i64>,
    avcc_sent: bool,
    last_pts: i64,
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
        let Some(Queued { frame, pts }) = queued else {
            break;
        };
        let started = Instant::now();
        if let Err(error) = encode_one(shared, config, threads, &mut active, &frame, pts) {
            shared.fail(error);
            break;
        }
        lock(&shared.stats).encode_nanos += started.elapsed().as_nanos();
    }
    if let Some(mut finished) = active {
        flush(shared, &mut finished);
    }
}

fn store_packet(shared: &Shared, data: Vec<u8>, keyframe: bool, pts: i64) {
    // A refused sample (out of order, or no keyframe yet) is simply not stored.
    let _ = lock(&shared.buffer).push(EncodedSample {
        pts,
        data,
        keyframe,
    });
}

fn flush(shared: &Shared, active: &mut Active) {
    match &mut active.stage {
        Stage::Software(encoder) => {
            for packet in encoder.flush() {
                store_packet(shared, packet.data, packet.keyframe, packet.pts);
            }
        }
        Stage::Hardware(encoder) => {
            if let Ok(packets) = encoder.flush() {
                for packet in packets {
                    if let Some(pts) = active.pending.pop_front() {
                        store_packet(shared, packet.data, packet.keyframe, pts);
                    }
                }
            }
        }
    }
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
    *lock(&shared.video) = Some(VideoInfo {
        width: plan.output_width,
        height: plan.output_height,
        fps: config.fps,
        avcc: encoder.avcc(),
    });
    *lock(&shared.encoder) = label;
    Ok(Stage::Software(Box::new(encoder)))
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
        return software_stage(shared, config, plan, threads, "CPU".to_owned());
    };
    let request = HardwareRequest {
        width: plan.output_width,
        height: plan.output_height,
        fps: config.fps,
        video_kbps: config.video_kbps,
        constant_bitrate: config.constant_bitrate,
        keyframe_frames: config.keyframe_seconds.saturating_mul(config.fps).max(1),
    };
    match provider.open(&request) {
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
            format!("CPU (GPU unavailable: {why})"),
        ),
    }
}

fn new_active(
    shared: &Shared,
    config: &RecordConfig,
    threads: usize,
    frame: &CaptureFrame,
) -> Result<Active, RecordError> {
    let plan = reframe(
        frame.width,
        frame.height,
        config.shape,
        config.framing,
        config.output_height,
    )
    .ok_or(RecordError::BadSize)?;
    // Video from before a size change does not match the new stream's parameters.
    lock(&shared.buffer).clear();
    let stage = open_stage(shared, config, &plan, threads)?;
    let software = matches!(stage, Stage::Software(_));
    Ok(Active {
        stage,
        source_size: (frame.width, frame.height),
        picture: I420::black(
            if software { plan.output_width } else { 2 },
            if software { plan.output_height } else { 2 },
        )?,
        rgba: Vec::new(),
        pending: VecDeque::new(),
        avcc_sent: software,
        plan,
        last_pts: -1,
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
    active.picture = I420::black(active.plan.output_width, active.plan.output_height)?;
    active.pending.clear();
    active.avcc_sent = true;
    active.last_pts = -1;
    lock(&shared.stats).restarts += 1;
    Ok(())
}

fn encode_one(
    shared: &Shared,
    config: &RecordConfig,
    threads: usize,
    active: &mut Option<Active>,
    frame: &CaptureFrame,
    pts: i64,
) -> Result<(), RecordError> {
    frame.validate().map_err(|_| RecordError::BadFrame)?;
    let size = (frame.width, frame.height);
    if active
        .as_ref()
        .is_none_or(|current| current.source_size != size)
    {
        let restarted = if let Some(mut old) = active.take() {
            flush(shared, &mut old);
            true
        } else {
            false
        };
        let fresh = new_active(shared, config, threads, frame)?;
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
    if let Stage::Hardware(_) = current.stage {
        match encode_hardware(shared, current, frame, pts, threads) {
            Ok(()) => {}
            Err(why) => {
                fail_over(shared, config, threads, current, &why)?;
            }
        }
        if let Stage::Hardware(_) = current.stage {
            current.last_pts = pts;
            lock(&shared.stats).encoded += 1;
            return Ok(());
        }
        // Fell back to software: this picture is encoded below, as the new stream's first.
    }
    convert(frame, &current.plan, &mut current.picture, threads)?;
    let picture = &current.picture;
    let yuv = YuvFrame {
        y: &picture.y,
        u: &picture.u,
        v: &picture.v,
        y_stride: picture.width as usize,
        uv_stride: picture.width as usize / 2,
    };
    let Stage::Software(encoder) = &mut current.stage else {
        return Err(RecordError::EncoderFailed);
    };
    let packets = encoder
        .try_encode(&yuv, pts)
        .map_err(|_| RecordError::EncoderFailed)?;
    current.last_pts = pts;
    for packet in packets {
        store_packet(shared, packet.data, packet.keyframe, packet.pts);
    }
    lock(&shared.stats).encoded += 1;
    Ok(())
}

/// Encodes one picture on the hardware encoder. Any error, or a B-frame, hands the stream back
/// to the software encoder.
fn encode_hardware(
    shared: &Shared,
    current: &mut Active,
    frame: &CaptureFrame,
    pts: i64,
    threads: usize,
) -> Result<(), String> {
    convert_rgba(frame, &current.plan, &mut current.rgba, threads)
        .map_err(|error| error.to_string())?;
    let Stage::Hardware(encoder) = &mut current.stage else {
        return Err("not a hardware stage".to_owned());
    };
    current.pending.push_back(pts);
    let packets = encoder
        .encode(&current.rgba, u64::try_from(pts).unwrap_or(0))
        .map_err(|why| why.clone())?;
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
            });
            current.avcc_sent = true;
        }
        store_packet(shared, packet.data, packet.keyframe, time);
    }
    Ok(())
}
