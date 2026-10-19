//! Local clip playback.
//!
//! A [`Player`] opens a clip on a worker thread and decodes frames against a wall clock. Decoding
//! never runs on the caller's thread, so a slow or damaged file cannot freeze the window: when
//! decoding falls behind real time, frames are skipped and the clock keeps its pace. The worker
//! reads the file in place (only the index plus the samples it needs), so a multi-gigabyte
//! recording opens quickly. Anything the decoders panic on is reported as a failure instead of
//! taking the application down.

use clipcraft_time::{FrameRate, Tick};
use filmcraft_media::reader::{FileReader, open_reader_within};
use filmcraft_media::{FrameRequest, MediaError, SharedReader, SharedSource};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Largest file read whole when no streaming reader exists for its format.
const MAX_WHOLE_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Frame period assumed when a file does not state a usable frame rate.
const FALLBACK_FRAME_PERIOD: Duration = Duration::from_millis(33);
/// Longest wait between wake-ups, so a shutdown request is always noticed promptly.
const MAX_IDLE_WAIT: Duration = Duration::from_millis(250);

/// Facts about the opened clip.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipInfo {
    /// Total duration.
    pub duration: Tick,
    /// Picture size in pixels.
    pub width: u32,
    /// Picture size in pixels.
    pub height: u32,
    /// Average frame rate.
    pub frame_rate: FrameRate,
    /// Whether the file has an audio stream (not yet played back).
    pub has_audio: bool,
}

/// One decoded picture, ready to upload as a texture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedFrame {
    /// Presentation time of this picture.
    pub pts: Tick,
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Straight-alpha sRGB RGBA, 4 bytes per pixel.
    pub rgba: Vec<u8>,
}

/// Why playback stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlayerError {
    /// The file could not be opened or read.
    Unreadable,
    /// The file's container or codec is not supported.
    Unsupported,
    /// The decoder rejected the stream.
    DecodeFailed,
    /// The decoder failed unexpectedly. The clip is untouched.
    Crashed,
}

impl fmt::Display for PlayerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unreadable => "The video could not be read.",
            Self::Unsupported => "ClipCraft cannot play this video format yet.",
            Self::DecodeFailed => "The video could not be decoded. The file may be damaged.",
            Self::Crashed => "Playback stopped unexpectedly. Your clip is untouched.",
        })
    }
}

impl std::error::Error for PlayerError {}

/// Something the worker reports to the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayerEvent {
    /// The clip opened.
    Opened(ClipInfoSummary),
    /// A new picture to show.
    Frame(DecodedFrame),
    /// Playback reached the end and stopped on the last frame.
    Ended,
    /// Playback cannot continue.
    Failed(PlayerError),
}

/// [`ClipInfo`] in an `Eq`-friendly form for events: duration in ticks and rate numerator/denominator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipInfoSummary {
    /// Total duration.
    pub duration: Tick,
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Frame rate numerator.
    pub rate_numerator: i64,
    /// Frame rate denominator.
    pub rate_denominator: i64,
    /// Whether the file has an audio stream.
    pub has_audio: bool,
}

impl From<&ClipInfo> for ClipInfoSummary {
    fn from(info: &ClipInfo) -> Self {
        Self {
            duration: info.duration,
            width: info.width,
            height: info.height,
            rate_numerator: info.frame_rate.num,
            rate_denominator: info.frame_rate.den,
            has_audio: info.has_audio,
        }
    }
}

impl ClipInfoSummary {
    /// The frame rate as a [`FrameRate`].
    pub fn frame_rate(&self) -> FrameRate {
        FrameRate::new(self.rate_numerator, self.rate_denominator)
    }
}

enum Command {
    Play,
    Pause,
    Seek(Tick),
    Shutdown,
}

/// Handle to a clip being played on a worker thread. Dropping it stops the worker.
pub struct Player {
    commands: Sender<Command>,
    events: Receiver<PlayerEvent>,
    worker: Option<JoinHandle<()>>,
}

impl Player {
    /// Starts opening `path` on a worker thread. `notify` is called after every event so the UI
    /// can wake up and call [`Player::poll`].
    pub fn start(path: PathBuf, notify: impl Fn() + Send + 'static) -> Self {
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("clipcraft-player".to_owned())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    run(&path, &command_receiver, &event_sender, &notify);
                }));
                if outcome.is_err() {
                    let _ = event_sender.send(PlayerEvent::Failed(PlayerError::Crashed));
                    notify();
                }
            })
            .ok();
        Self {
            commands: command_sender,
            events: event_receiver,
            worker,
        }
    }

    /// Starts or resumes playback from the current position.
    pub fn play(&self) {
        let _ = self.commands.send(Command::Play);
    }

    /// Pauses on the current picture.
    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    /// Jumps to `position` and shows that picture. Playback state is kept.
    pub fn seek(&self, position: Tick) {
        let _ = self.commands.send(Command::Seek(position));
    }

    /// Takes every event that arrived since the last call. Returns a worker failure if the thread
    /// could not even start.
    pub fn poll(&self) -> Vec<PlayerEvent> {
        let mut events: Vec<PlayerEvent> = self.events.try_iter().collect();
        if self.worker.is_none() {
            events.push(PlayerEvent::Failed(PlayerError::Crashed));
        }
        events
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        // The worker is detached rather than joined: a decoder stuck in a long seek must not
        // freeze the window that is closing the clip. It exits at its next command check.
        drop(self.worker.take());
    }
}

/// Opens `path` as a streaming media source.
fn open_source(path: &Path) -> Result<SharedSource, PlayerError> {
    let reader = FileReader::open(path).map_err(|_| PlayerError::Unreadable)?;
    let reader: SharedReader = Arc::new(reader);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    open_reader_within(
        &name,
        reader,
        &filmcraft_codecs::reader_openers(),
        &filmcraft_codecs::openers(),
        MAX_WHOLE_FILE_BYTES,
    )
    .map_err(|error| match error {
        MediaError::Io(_) | MediaError::Offline(_) => PlayerError::Unreadable,
        MediaError::Decode(_) => PlayerError::DecodeFailed,
        MediaError::Unsupported(_) | MediaError::NoStream(_) | MediaError::Cancelled => {
            PlayerError::Unsupported
        }
    })
}

fn clip_info(source: &SharedSource) -> Result<ClipInfo, PlayerError> {
    let info = source.info();
    let video = info.video.as_ref().ok_or(PlayerError::Unsupported)?;
    if video.width == 0 || video.height == 0 {
        return Err(PlayerError::Unsupported);
    }
    Ok(ClipInfo {
        duration: info.duration.max(Tick::ZERO),
        width: video.width,
        height: video.height,
        frame_rate: video.frame_rate.sane(),
        has_audio: info.has_audio(),
    })
}

/// Decodes the picture shown at `position` (clamped into the clip).
fn decode_at(
    source: &SharedSource,
    info: &ClipInfo,
    position: Tick,
) -> Result<DecodedFrame, PlayerError> {
    let last_frame_start = info.duration - info.frame_rate.frame_duration();
    let position = position.clamp(Tick::ZERO, last_frame_start.max(Tick::ZERO));
    let frame = source
        .video_frame(FrameRequest::full(position))
        .map_err(|error| match error {
            MediaError::Unsupported(_) => PlayerError::Unsupported,
            MediaError::Io(_) | MediaError::Offline(_) => PlayerError::Unreadable,
            _ => PlayerError::DecodeFailed,
        })?;
    let rgba = frame.to_rgba8();
    // A decoder that returns fewer bytes than the picture needs would corrupt the texture upload.
    let expected = usize::try_from(frame.width)
        .ok()
        .zip(usize::try_from(frame.height).ok())
        .and_then(|(width, height)| width.checked_mul(height))
        .and_then(|pixels| pixels.checked_mul(4));
    if frame.width == 0 || frame.height == 0 || expected != Some(rgba.len()) {
        return Err(PlayerError::DecodeFailed);
    }
    Ok(DecodedFrame {
        pts: position,
        width: frame.width,
        height: frame.height,
        rgba,
    })
}

fn frame_period(rate: FrameRate) -> Duration {
    let seconds = rate.frame_duration().seconds();
    if seconds.is_finite() && seconds > 0.0 {
        Duration::from_secs_f64(seconds.min(1.0))
    } else {
        FALLBACK_FRAME_PERIOD
    }
}

fn ticks_from(elapsed: Duration) -> Tick {
    Tick::from_seconds_f64(elapsed.as_secs_f64())
}

fn run(
    path: &Path,
    commands: &Receiver<Command>,
    events: &Sender<PlayerEvent>,
    notify: &impl Fn(),
) {
    let send = |event: PlayerEvent| {
        let delivered = events.send(event).is_ok();
        notify();
        delivered
    };
    let (source, info) = match open_source(path).and_then(|source| {
        let info = clip_info(&source)?;
        Ok((source, info))
    }) {
        Ok(opened) => opened,
        Err(error) => {
            send(PlayerEvent::Failed(error));
            return;
        }
    };
    if !send(PlayerEvent::Opened(ClipInfoSummary::from(&info))) {
        return;
    }

    let period = frame_period(info.frame_rate);
    let mut state = State {
        position: Tick::ZERO,
        playing: false,
        clock_start: Instant::now(),
        clock_base: Tick::ZERO,
        show: true, // show the first picture right away
        ended: false,
        next_tick: Instant::now(),
    };

    loop {
        if state.show {
            state.show = false;
            match decode_at(&source, &info, state.position) {
                Ok(frame) => {
                    state.position = frame.pts;
                    if !send(PlayerEvent::Frame(frame)) {
                        return;
                    }
                    // `Ended` follows the final picture so the screen shows the true last frame.
                    if std::mem::take(&mut state.ended) && !send(PlayerEvent::Ended) {
                        return;
                    }
                }
                Err(error) => {
                    send(PlayerEvent::Failed(error));
                    return;
                }
            }
        }

        let wait = if state.playing {
            state.next_tick.saturating_duration_since(Instant::now())
        } else {
            MAX_IDLE_WAIT
        };
        match commands.recv_timeout(wait.min(MAX_IDLE_WAIT)) {
            Ok(command) => {
                // Apply everything already queued before decoding, so scrubbing quickly decodes
                // only the latest position instead of every intermediate one.
                if state.apply(command, &info) {
                    return;
                }
                for command in commands.try_iter() {
                    if state.apply(command, &info) {
                        return;
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }

        if state.playing && !state.show && Instant::now() >= state.next_tick {
            let target = state.clock_base + ticks_from(state.clock_start.elapsed());
            if target + info.frame_rate.frame_duration() >= info.duration {
                // Stop on the true last picture so playing again restarts from the beginning.
                state.playing = false;
                state.position = (info.duration - info.frame_rate.frame_duration()).max(Tick::ZERO);
                state.show = true;
                state.ended = true;
                continue;
            }
            state.position = target;
            state.show = true;
            state.next_tick = Instant::now() + period;
        }
    }
}

/// The worker's playback state.
struct State {
    position: Tick,
    playing: bool,
    clock_start: Instant,
    clock_base: Tick,
    show: bool,
    ended: bool,
    next_tick: Instant,
}

impl State {
    /// Applies one command. Returns true when the worker should stop.
    fn apply(&mut self, command: Command, info: &ClipInfo) -> bool {
        match command {
            Command::Shutdown => return true,
            Command::Play => {
                if !self.playing {
                    // Replaying from the end starts over.
                    if self.position + info.frame_rate.frame_duration() >= info.duration {
                        self.position = Tick::ZERO;
                        self.show = true;
                    }
                    self.playing = true;
                    self.clock_start = Instant::now();
                    self.clock_base = self.position;
                    self.next_tick = Instant::now();
                }
            }
            Command::Pause => self.playing = false,
            Command::Seek(target) => {
                self.position = target.clamp(Tick::ZERO, info.duration);
                self.clock_start = Instant::now();
                self.clock_base = self.position;
                self.show = true;
            }
        }
        false
    }
}

pub mod thumbnail;

#[cfg(test)]
mod tests;
