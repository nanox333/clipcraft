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
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Largest file read whole when no streaming reader exists for its format.
const MAX_WHOLE_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Frame period assumed when a file does not state a usable frame rate.
const FALLBACK_FRAME_PERIOD: Duration = Duration::from_millis(33);
/// How long the sound clock may stand still while playing before the picture stops waiting for it.
const AUDIO_STALL: Duration = Duration::from_millis(1500);
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
    /// Whether the file has an audio stream.
    pub has_audio: bool,
    /// One label per audio stream, for a track picker.
    pub audio_tracks: Vec<String>,
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// One label per audio stream, for a track picker.
    pub audio_tracks: Vec<String>,
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
            audio_tracks: info.audio_tracks.clone(),
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
    PlayFromCurrent,
    Pause,
    Seek(Tick),
    SetSpeed(f64),
    SetLoop(bool),
    Step(i64),
    SelectAudio(usize),
    Shutdown,
}

/// Handle to a clip being played on a worker thread. Dropping it stops the worker.
pub struct Player {
    commands: Sender<Command>,
    events: Receiver<PlayerEvent>,
    worker: Option<JoinHandle<()>>,
    audio: Arc<OnceLock<Arc<audio::AudioFeed>>>,
}

impl Player {
    /// Starts opening `path` on a worker thread. `notify` is called after every event so the UI
    /// can wake up and call [`Player::poll`].
    pub fn start(path: PathBuf, notify: impl Fn() + Send + 'static) -> Self {
        let (command_sender, command_receiver) = mpsc::channel();
        let (event_sender, event_receiver) = mpsc::channel();
        let audio = Arc::new(OnceLock::new());
        let worker_audio = audio.clone();
        let worker = thread::Builder::new()
            .name("clipcraft-player".to_owned())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    run(
                        &path,
                        &command_receiver,
                        &event_sender,
                        &notify,
                        &worker_audio,
                    );
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
            audio,
        }
    }

    /// Starts or resumes playback from the current position.
    pub fn play(&self) {
        let _ = self.commands.send(Command::Play);
    }

    /// Resumes the requested position even in the final frame, for a bounded editor range.
    /// Unlike [`Player::play`], this never interprets an end position as a replay request.
    pub fn play_from_current(&self) {
        let _ = self.commands.send(Command::PlayFromCurrent);
    }

    /// Pauses on the current picture.
    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    /// Jumps to `position` and shows that picture. Playback state is kept.
    pub fn seek(&self, position: Tick) {
        let _ = self.commands.send(Command::Seek(position));
    }

    /// Plays at `speed` (0.25 to 4, pitch kept). Takes effect from the current position.
    pub fn set_speed(&self, speed: f64) {
        let _ = self.commands.send(Command::SetSpeed(speed));
    }

    /// When on, playback restarts from the beginning instead of stopping at the end.
    pub fn set_loop(&self, looping: bool) {
        let _ = self.commands.send(Command::SetLoop(looping));
    }

    /// Pauses and moves by `frames` pictures (negative goes back), showing the new picture.
    pub fn step(&self, frames: i64) {
        let _ = self.commands.send(Command::Step(frames));
    }

    /// Plays audio stream `index` (the order of [`ClipInfoSummary::audio_tracks`]).
    pub fn select_audio(&self, index: usize) {
        let _ = self.commands.send(Command::SelectAudio(index));
    }

    /// The clip's sound feed, available once the clip has opened and only when it has audio.
    /// Connect it to an output device (`clipcraft-audio-out`) to hear the clip; until a device
    /// runs, playback keeps time with the wall clock.
    pub fn audio_feed(&self) -> Option<Arc<audio::AudioFeed>> {
        self.audio.get().cloned()
    }

    /// Requests cancellation without blocking the caller on a decoder or file read.
    pub fn request_stop(&self) {
        let _ = self.commands.send(Command::Shutdown);
    }

    /// Whether the worker has acknowledged cancellation (or failed to start).
    /// Callers replacing players can retain stopped workers until this is true to bound opens.
    pub fn worker_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
            && self.audio.get().is_none_or(|feed| feed.worker_finished())
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
        audio_tracks: info
            .audio_streams
            .iter()
            .enumerate()
            .map(|(index, stream)| {
                format!(
                    "Track {} · {} · {} ch · {:.1} kHz",
                    index + 1,
                    stream.codec.to_uppercase(),
                    stream.channels,
                    f64::from(stream.sample_rate) / 1000.0
                )
            })
            .collect(),
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

/// Media time covered by `elapsed` wall time at `speed`.
fn ticks_from(elapsed: Duration, speed: f64) -> Tick {
    Tick::from_seconds_f64(elapsed.as_secs_f64() * speed)
}

fn run(
    path: &Path,
    commands: &Receiver<Command>,
    events: &Sender<PlayerEvent>,
    notify: &impl Fn(),
    audio_slot: &OnceLock<Arc<audio::AudioFeed>>,
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
    // The feed exists before `Opened` is announced, so a listener that reacts to `Opened` can
    // already fetch it. It is closed on every way out of this function.
    let feed = info.has_audio.then(|| audio::start_feed(source.clone()));
    let _close_feed = feed.as_ref().map(|feed| CloseFeed(feed.clone()));
    if let Some(feed) = &feed {
        let _ = audio_slot.set(feed.clone());
    }
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
        audio_synced: false,
        speed: 1.0,
        looping: false,
        audio_stream: 0,
        audio_stalled: false,
        audio_watch: None,
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
                if state.apply(command, &info, feed.as_deref()) {
                    return;
                }
                for command in commands.try_iter() {
                    if state.apply(command, &info, feed.as_deref()) {
                        return;
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }

        state.sync_audio(feed.as_deref());
        state.watch_audio(feed.as_deref());

        if state.playing && !state.show && Instant::now() >= state.next_tick {
            let target = state.clock_position(feed.as_deref());
            if target + info.frame_rate.frame_duration() >= info.duration && state.looping {
                // Loop: start over from the first picture without ever stopping.
                state.position = Tick::ZERO;
                state.clock_base = Tick::ZERO;
                state.clock_start = Instant::now();
                state.show = true;
                if let Some(feed) = &feed {
                    feed.seek(Tick::ZERO);
                }
                continue;
            }
            if target + info.frame_rate.frame_duration() >= info.duration {
                // Stop on the true last picture so playing again restarts from the beginning.
                state.playing = false;
                if let Some(feed) = &feed {
                    feed.set_playing(false);
                }
                state.position = (info.duration - info.frame_rate.frame_duration()).max(Tick::ZERO);
                state.show = true;
                state.ended = true;
                continue;
            }
            // Only decode when the clock has moved on to a different picture.
            if info.frame_rate.frame_at(target) != info.frame_rate.frame_at(state.position) {
                state.position = target;
                state.show = true;
            }
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
    /// The sound feed has been given the current position and is the clock.
    audio_synced: bool,
    speed: f64,
    looping: bool,
    audio_stream: usize,
    /// The sound device is marked active but has stopped taking sound; the wall clock is in use
    /// until it delivers again.
    audio_stalled: bool,
    /// The sound clock reading and when it last changed, for stall detection.
    audio_watch: Option<(Tick, Instant)>,
}

impl State {
    /// Applies one command. Returns true when the worker should stop.
    fn apply(
        &mut self,
        command: Command,
        info: &ClipInfo,
        feed: Option<&audio::AudioFeed>,
    ) -> bool {
        match command {
            Command::Shutdown => return true,
            Command::Play | Command::PlayFromCurrent => {
                if !self.playing {
                    // Replaying from the end starts over.
                    if matches!(command, Command::Play)
                        && self.position + info.frame_rate.frame_duration() >= info.duration
                    {
                        self.position = Tick::ZERO;
                        self.show = true;
                    }
                    self.playing = true;
                    self.clock_start = Instant::now();
                    self.clock_base = self.position;
                    self.next_tick = Instant::now();
                    if let Some(feed) = feed {
                        feed.seek(self.position);
                        feed.set_playing(true);
                    }
                }
            }
            Command::Pause => {
                self.playing = false;
                if let Some(feed) = feed {
                    feed.set_playing(false);
                }
            }
            Command::Seek(target) => {
                self.position = target.clamp(Tick::ZERO, info.duration);
                self.clock_start = Instant::now();
                self.clock_base = self.position;
                self.show = true;
                if let Some(feed) = feed {
                    feed.seek(self.position);
                }
            }
            Command::SetSpeed(speed) => {
                let speed = if speed.is_finite() {
                    speed.clamp(stretch::MIN_SPEED, stretch::MAX_SPEED)
                } else {
                    1.0
                };
                // Re-anchor both clocks at the current position so the change takes effect from
                // here and the position does not jump.
                let now = self.current_position(feed);
                self.speed = speed;
                self.position = now;
                self.clock_base = now;
                self.clock_start = Instant::now();
                if let Some(feed) = feed {
                    feed.set_speed(speed, now);
                }
            }
            Command::SetLoop(looping) => self.looping = looping,
            Command::Step(frames) => {
                // Stepping is a paused operation: stop, then land exactly on a frame boundary.
                self.playing = false;
                let last = info
                    .frame_rate
                    .frame_at((info.duration - info.frame_rate.frame_duration()).max(Tick::ZERO));
                let frame = info
                    .frame_rate
                    .frame_at(self.position)
                    .saturating_add(frames)
                    .clamp(0, last.max(0));
                self.position = info.frame_rate.tick_of(frame);
                self.clock_base = self.position;
                self.clock_start = Instant::now();
                self.show = true;
                if let Some(feed) = feed {
                    feed.set_playing(false);
                    feed.seek(self.position);
                }
            }
            Command::SelectAudio(index) => {
                if index < info.audio_tracks.len() {
                    self.audio_stream = index;
                    let now = self.current_position(feed);
                    if let Some(feed) = feed {
                        feed.set_stream(index, now);
                    }
                }
            }
        }
        false
    }

    /// The position right now: where playback is while playing, the shown picture while paused.
    fn current_position(&self, feed: Option<&audio::AudioFeed>) -> Tick {
        if self.playing {
            self.clock_position(feed)
        } else {
            self.position
        }
    }

    /// The playback position now: the sound clock while a device drives it, else the wall clock.
    fn clock_position(&self, feed: Option<&audio::AudioFeed>) -> Tick {
        match feed {
            Some(feed) if self.audio_synced && !self.audio_stalled && feed.driving() => {
                feed.position()
            }
            _ => self.clock_base + ticks_from(self.clock_start.elapsed(), self.speed),
        }
    }

    /// Notices a sound device that is marked active but has stopped taking sound, so the picture
    /// does not freeze with it. Called every loop while playing.
    fn watch_audio(&mut self, feed: Option<&audio::AudioFeed>) {
        let Some(feed) = feed else {
            return;
        };
        if !(self.playing && self.audio_synced && !self.audio_stalled && feed.driving()) {
            self.audio_watch = None;
            return;
        }
        let position = feed.position();
        match self.audio_watch {
            Some((seen, since)) if seen == position => {
                if since.elapsed() >= AUDIO_STALL {
                    // Carry on from where the sound got to, on the wall clock.
                    self.clock_base = position;
                    self.clock_start = Instant::now();
                    self.audio_stalled = true;
                    self.audio_watch = None;
                }
            }
            _ => self.audio_watch = Some((position, Instant::now())),
        }
    }

    /// Hands the current position to the feed when a sound device starts (or restarts) mid-play,
    /// and goes back to the wall clock when it stops.
    fn sync_audio(&mut self, feed: Option<&audio::AudioFeed>) {
        let Some(feed) = feed else {
            return;
        };
        if self.audio_stalled {
            // Wait for proof of life: a device that delivers again, or one that went away.
            if !feed.driving() || feed.delivered_frames() > 0 {
                self.audio_stalled = false;
                self.audio_synced = false;
                self.audio_watch = None;
            } else {
                return;
            }
        }
        if feed.driving() {
            if !self.audio_synced {
                let now = if self.playing {
                    self.clock_base + ticks_from(self.clock_start.elapsed(), self.speed)
                } else {
                    self.position
                };
                // Give the late device the current speed, track, and position.
                feed.set_speed(self.speed, now);
                if self.audio_stream != 0 {
                    feed.set_stream(self.audio_stream, now);
                }
                feed.set_playing(self.playing);
                self.audio_synced = true;
            }
        } else if self.audio_synced {
            // The device went away: keep time from where the sound got to.
            self.clock_base = feed.position();
            self.clock_start = Instant::now();
            self.audio_synced = false;
        }
    }
}

/// Stops the audio decoder thread when the worker exits, however it exits.
struct CloseFeed(Arc<audio::AudioFeed>);

impl Drop for CloseFeed {
    fn drop(&mut self) {
        self.0.close();
    }
}

pub mod audio;
pub mod stretch;
pub mod thumbnail;
pub mod timeline;

#[cfg(test)]
mod tests;
