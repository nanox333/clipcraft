//! A running recording: starting it from the settings, saving a clip into the capture folder, and
//! keeping the folder inside the storage limit.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clipcraft_capture::CaptureSource;
use clipcraft_record::{
    AudioConfig, AudioControl, AudioInput, AudioKind, AudioProvider, AudioSetup, ClipFormat,
    EncoderSpeed, HardwareProvider, RecordConfig, RecordError, RecordState, RecordStatus, Recorder,
    SavedClip,
};

use crate::storage::{StoredClip, cleanup_plan, list_clips};
use crate::{Container, EncoderPreset, EngineError, RateControl, RecordingSettings, VideoEncoder};

/// Every clip ClipCraft saves starts with this, so cleanup only ever deletes its own clips.
pub const CLIP_PREFIX: &str = "ClipCraft ";

/// What the recorder is asked to do for these settings.
pub fn record_config(settings: &RecordingSettings) -> RecordConfig {
    let bytes_per_second = u64::from(settings.video_kbps) * 1000 / 8;
    // Room for the replay window at twice the average rate (bursts), plus a fixed margin.
    let wanted = bytes_per_second
        .saturating_mul(u64::from(
            settings.replay_seconds + settings.post_roll_seconds,
        ))
        .saturating_mul(2)
        .saturating_add(32 * 1024 * 1024);
    RecordConfig {
        output_height: settings.height,
        shape: settings.shape,
        framing: settings.framing,
        fps: settings.frames_per_second,
        video_kbps: settings.video_kbps,
        constant_bitrate: settings.rate_control == RateControl::Constant,
        speed: match settings.encoder_preset {
            EncoderPreset::Fast => EncoderSpeed::Fast,
            EncoderPreset::Balanced => EncoderSpeed::Balanced,
            EncoderPreset::Quality => EncoderSpeed::Quality,
        },
        keyframe_seconds: settings.keyframe_seconds,
        // The buffer holds the replay and the post-roll that follows the hotkey.
        replay_seconds: settings.replay_seconds + settings.post_roll_seconds,
        max_buffer_bytes: usize::try_from(wanted.clamp(64 * 1024 * 1024, 2 * 1024 * 1024 * 1024))
            .unwrap_or(64 * 1024 * 1024),
        threads: 0,
        hardware: None,
        gap_restart_seconds: 10,
    }
}

/// The file format the settings ask for. WebM cannot be written yet and becomes MP4.
pub fn clip_format(settings: &RecordingSettings) -> ClipFormat {
    match settings.container {
        Container::Mkv => ClipFormat::Mkv,
        Container::Mp4 | Container::Webm => ClipFormat::Mp4,
    }
}

/// A clip that was saved, and any older ClipCraft clips deleted to stay inside the storage limit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedClipReport {
    /// The clip written.
    pub clip: SavedClip,
    /// Older ClipCraft clips removed to make room.
    pub deleted: Vec<PathBuf>,
    /// Game snapped when Save was pressed, independent of later process/session changes.
    pub recorded_game: Option<String>,
}

/// Handles to the live mute and level of each recorded sound source.
#[derive(Clone, Debug, Default)]
struct SoundControls {
    system: Option<AudioControl>,
    microphone: Option<AudioControl>,
}

/// The sound sources the settings ask for, opened through `provider`, and what went wrong with
/// the ones that could not be.
fn audio_setup(
    settings: &RecordingSettings,
    provider: &AudioProvider,
) -> (Option<AudioSetup>, SoundControls, Vec<String>) {
    let mut warnings = Vec::new();
    let mut controls = SoundControls::default();
    let mut sources = Vec::new();
    if settings.capture_game_audio {
        match provider.open(AudioKind::SystemOutput, &settings.output_device) {
            Ok(source) => {
                let control = AudioControl::default();
                control.set_gain_percent(settings.game_audio_percent);
                controls.system = Some(control.clone());
                sources.push((AudioKind::SystemOutput, source, control));
            }
            Err(why) => warnings.push(why),
        }
    }
    if settings.capture_microphone {
        match provider.open(AudioKind::Microphone, &settings.microphone_device) {
            Ok(source) => {
                let control = AudioControl::default();
                control.set_gain_percent(settings.microphone_percent);
                controls.microphone = Some(control.clone());
                sources.push((AudioKind::Microphone, source, control));
            }
            Err(why) => warnings.push(format!("The microphone could not be recorded: {why}")),
        }
    }
    if sources.is_empty() {
        return (None, controls, warnings);
    }
    let separate = settings.separate_audio_tracks && sources.len() > 1;
    let tracks: Vec<String> = if separate {
        sources
            .iter()
            .map(|(kind, ..)| match kind {
                AudioKind::SystemOutput => "Game and system".to_owned(),
                AudioKind::Microphone => "Microphone".to_owned(),
            })
            .collect()
    } else {
        vec!["Audio".to_owned()]
    };
    let channels = if settings.audio_channels == 1 { 1 } else { 2 };
    let sample_rate = settings.audio_sample_rate;
    let inputs = sources
        .into_iter()
        .enumerate()
        .map(|(index, (kind, source, control))| {
            let mut input = AudioInput::new(source, control, if separate { index } else { 0 });
            input.mono = kind == AudioKind::Microphone && settings.microphone_mono;
            input
        })
        .collect();
    let mut config = AudioConfig {
        sample_rate,
        channels,
        kbps: if channels == 1 {
            settings.audio_kbps.min(192)
        } else {
            settings.audio_kbps
        },
        tracks,
    };
    if !config.valid() {
        config.sample_rate = 48_000;
        config.kbps = 160;
    }
    (Some(AudioSetup { config, inputs }), controls, warnings)
}

/// The recording in progress, if any. Owned by whoever drives the app.
#[derive(Default)]
pub struct CaptureSession {
    recorder: Option<Arc<Recorder>>,
    sound: SoundControls,
    warnings: Vec<String>,
    microphone_off: bool,
    push_to_talk: bool,
    talking: bool,
}

impl CaptureSession {
    /// A session that is not recording.
    pub fn new() -> Self {
        Self::default()
    }

    /// True while a recording is running (even if it has failed and can still be saved).
    pub fn is_active(&self) -> bool {
        self.recorder.is_some()
    }

    /// Starts recording `source` with the picture, bitrate, and replay settings. `hardware` is
    /// the computer's hardware encoder, used only when the settings ask for the GPU; if it cannot
    /// be opened the software encoder runs and the status says why. Command id `capture.start`.
    pub fn start(
        &mut self,
        settings: &RecordingSettings,
        source: Box<dyn CaptureSource>,
        hardware: Option<HardwareProvider>,
    ) -> Result<(), EngineError> {
        self.start_with_audio(settings, source, hardware, None)
    }

    /// As [`CaptureSession::start`], and records the sound the settings ask for from `audio`.
    /// Sources that cannot be opened are skipped and listed by [`CaptureSession::warnings`]; the
    /// video is recorded regardless.
    pub fn start_with_audio(
        &mut self,
        settings: &RecordingSettings,
        source: Box<dyn CaptureSource>,
        hardware: Option<HardwareProvider>,
        audio: Option<AudioProvider>,
    ) -> Result<(), EngineError> {
        if self.recorder.is_some() {
            return Err(EngineError::AlreadyRecording);
        }
        let settings = settings.clone().sanitized();
        let mut config = record_config(&settings);
        if settings.video_encoder == VideoEncoder::Gpu {
            config.hardware = Some(hardware.unwrap_or_else(|| {
                HardwareProvider::new(|_| {
                    Err("this computer has no supported hardware encoder".to_owned())
                })
            }));
        }
        let (setup, controls, warnings) = match audio.as_ref() {
            Some(provider) if settings.capture_game_audio || settings.capture_microphone => {
                audio_setup(&settings, provider)
            }
            _ => (None, SoundControls::default(), Vec::new()),
        };
        let recorder =
            Recorder::start_with_audio(source, config, setup).map_err(EngineError::Record)?;
        self.recorder = Some(Arc::new(recorder));
        self.sound = controls;
        self.warnings = warnings;
        self.microphone_off = false;
        self.push_to_talk = settings.push_to_talk;
        self.talking = false;
        self.apply_microphone();
        Ok(())
    }

    /// Sound sources that could not be recorded, and why. Empty when all is well.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Whether the microphone is being recorded at all (opened, and not muted for good).
    pub fn has_microphone(&self) -> bool {
        self.sound.microphone.is_some()
    }

    /// Whether the microphone is silenced right now: muted, or waiting for push-to-talk.
    pub fn microphone_muted(&self) -> bool {
        self.microphone_off || (self.push_to_talk && !self.talking)
    }

    fn apply_microphone(&self) {
        if let Some(control) = self.sound.microphone.as_ref() {
            control.set_muted(self.microphone_muted());
        }
    }

    /// Mutes or unmutes the microphone. Command id `capture.toggle_microphone`.
    pub fn set_microphone_off(&mut self, off: bool) {
        self.microphone_off = off;
        self.apply_microphone();
    }

    /// Flips the microphone mute and returns whether it is now muted.
    pub fn toggle_microphone(&mut self) -> bool {
        self.set_microphone_off(!self.microphone_off);
        self.microphone_off
    }

    /// Tells the session whether the push-to-talk key is held.
    pub fn set_talking(&mut self, held: bool) {
        self.talking = held;
        self.apply_microphone();
    }

    /// Applies the sound levels of `settings` to the running recording.
    pub fn set_levels(&self, settings: &RecordingSettings) {
        if let Some(control) = self.sound.system.as_ref() {
            control.set_gain_percent(settings.game_audio_percent);
        }
        if let Some(control) = self.sound.microphone.as_ref() {
            control.set_gain_percent(settings.microphone_percent);
        }
    }

    /// Stops recording and drops the buffer. Command id `capture.stop`.
    pub fn stop(&mut self) -> Option<RecordStatus> {
        let recorder = self.recorder.take()?;
        self.sound = SoundControls::default();
        recorder.halt();
        let mut status = recorder.status();
        if status.state == clipcraft_record::RecordState::Running {
            status.state = clipcraft_record::RecordState::Stopped;
        }
        // The threads are joined when the last holder, possibly a clip still being saved, lets go.
        Some(status)
    }

    /// How recording is going. Command id `capture.status`.
    pub fn status(&self) -> Option<RecordStatus> {
        self.recorder.as_ref().map(|recorder| recorder.status())
    }

    /// Saves the newest replay seconds into `folder` and then trims the folder to the storage
    /// limit. Command id `capture.save_clip`. Blocks until the file is written; use
    /// [`CaptureSession::prepare_save`] to do the writing on another thread.
    pub fn save_clip(
        &self,
        settings: &RecordingSettings,
        folder: &Path,
    ) -> Result<SavedClipReport, EngineError> {
        self.prepare_save(settings, folder)?.run()
    }

    /// A screenshot of the screen being recorded, to be taken on any thread.
    pub fn screenshot_job(
        &self,
        folder: &Path,
    ) -> Result<crate::screenshot::ScreenshotJob, EngineError> {
        let recorder = self.recorder.clone().ok_or(EngineError::NotRecording)?;
        Ok(crate::screenshot::ScreenshotJob {
            recorder,
            folder: folder.to_path_buf(),
        })
    }

    /// Checks the storage limit and picks the file name, without writing anything.
    pub fn prepare_save(
        &self,
        settings: &RecordingSettings,
        folder: &Path,
    ) -> Result<SaveJob, EngineError> {
        let limit = u64::from(settings.clone().sanitized().storage_limit_gb) * 1_000_000_000;
        self.prepare_save_within(settings, folder, limit)
    }

    fn prepare_save_within(
        &self,
        settings: &RecordingSettings,
        folder: &Path,
        limit: u64,
    ) -> Result<SaveJob, EngineError> {
        let recorder = self.recorder.clone().ok_or(EngineError::NotRecording)?;
        let settings = settings.clone().sanitized();
        let format = clip_format(&settings);
        if limit > 0 && !settings.delete_oldest_when_full {
            let used: u64 = list_clips(folder)
                .0
                .iter()
                .map(|clip| clip.size_bytes)
                .sum();
            if used >= limit {
                return Err(EngineError::StorageLimit);
            }
        }
        // Refuse early, before writing anything, when the disk cannot hold what is buffered.
        // Skipped when the free space is not known.
        let buffered = recorder.status().buffered_bytes;
        // An MP4 is written twice (a crash-safe copy first), so it needs twice the room while it
        // is being saved.
        let copies = if format == ClipFormat::Mp4 { 2 } else { 1 };
        let needed = buffered
            .saturating_add(buffered / 16)
            .saturating_mul(copies)
            .saturating_add(4 * 1024 * 1024);
        if crate::storage::storage_usage(folder)
            .free_bytes
            .is_some_and(|free| free < needed)
        {
            return Err(EngineError::Record(RecordError::DiskFull));
        }
        Ok(SaveJob {
            recorded_game: None,
            recorder,
            format,
            seconds: settings.replay_seconds + settings.post_roll_seconds,
            post_roll: Duration::from_secs(u64::from(settings.post_roll_seconds)),
            path: free_clip_path(folder, format, unix_now())?,
            folder: folder.to_path_buf(),
            trim_to: (limit > 0 && settings.delete_oldest_when_full).then_some(limit),
        })
    }
}

/// A clip save that can run on any thread.
pub struct SaveJob {
    recorded_game: Option<String>,
    recorder: Arc<Recorder>,
    format: ClipFormat,
    seconds: u32,
    post_roll: Duration,
    path: PathBuf,
    folder: PathBuf,
    trim_to: Option<u64>,
}

impl SaveJob {
    /// Snapshots a game name before moving threads. Invalid labels never prevent saving media.
    pub fn with_game(mut self, game: Option<&str>) -> Self {
        self.recorded_game = game.and_then(crate::library_meta::clean_recorded_game);
        self
    }
    /// The file that will be written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How long [`SaveJob::run`] keeps recording before it writes the clip.
    pub fn post_roll(&self) -> Duration {
        self.post_roll
    }

    /// Waits for the post-roll, then writes the clip and removes the oldest ClipCraft clips if
    /// the folder is over its limit. If recording stops or fails while waiting, the clip is
    /// written at once with what there is.
    pub fn run(self) -> Result<SavedClipReport, EngineError> {
        let until = Instant::now() + self.post_roll;
        while Instant::now() < until && self.recorder.status().state == RecordState::Running {
            let left = until.saturating_duration_since(Instant::now());
            std::thread::sleep(left.min(Duration::from_millis(50)));
        }
        let clip = self
            .recorder
            .save_clip(self.seconds, self.format, &self.path)
            .map_err(EngineError::Record)?;
        let deleted = match self.trim_to {
            Some(limit) => trim_to_limit(&self.folder, &clip.path, limit),
            None => Vec::new(),
        };
        Ok(SavedClipReport {
            clip,
            deleted,
            recorded_game: self.recorded_game,
        })
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Days since 1970-01-01 to a civil date (proleptic Gregorian).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// `ClipCraft 2026-10-19 21-04-55` for a Unix time, in UTC.
pub fn clip_stem(unix_seconds: u64) -> String {
    let days = i64::try_from(unix_seconds / 86_400).unwrap_or(0);
    let (year, month, day) = civil_from_days(days);
    let seconds = unix_seconds % 86_400;
    format!(
        "{CLIP_PREFIX}{year:04}-{month:02}-{day:02} {:02}-{:02}-{:02}",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    )
}

fn free_clip_path(
    folder: &Path,
    format: ClipFormat,
    unix_seconds: u64,
) -> Result<PathBuf, EngineError> {
    let stem = clip_stem(unix_seconds);
    for attempt in 1..=999_u32 {
        let name = if attempt == 1 {
            format!("{stem}.{}", format.extension())
        } else {
            format!("{stem} ({attempt}).{}", format.extension())
        };
        let candidate = folder.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(EngineError::Record(RecordError::AlreadyExists))
}

/// Deletes the oldest ClipCraft clips, never `keep`, until the folder fits `limit` bytes. Files
/// ClipCraft did not make are counted but never deleted.
fn trim_to_limit(folder: &Path, keep: &Path, limit: u64) -> Vec<PathBuf> {
    let (clips, _) = list_clips(folder);
    let is_ours = |clip: &StoredClip| {
        clip.path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(CLIP_PREFIX))
            && clip.path != keep
    };
    let others: u64 = clips
        .iter()
        .filter(|clip| !is_ours(clip))
        .map(|clip| clip.size_bytes)
        .sum();
    let ours: Vec<StoredClip> = clips.iter().filter(|clip| is_ours(clip)).cloned().collect();
    let mut deleted = Vec::new();
    for path in cleanup_plan(&ours, limit.saturating_sub(others)) {
        if fs::remove_file(&path).is_ok() {
            deleted.push(path);
        }
    }
    deleted
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipcraft_capture::SyntheticSource;
    use std::time::{Duration, Instant};

    #[test]
    fn clip_names_are_utc_timestamps() {
        assert_eq!(clip_stem(0), "ClipCraft 1970-01-01 00-00-00");
        assert_eq!(
            clip_stem(951_782_400 + 3_661),
            "ClipCraft 2000-02-29 01-01-01"
        );
    }

    #[test]
    fn settings_become_a_recorder_config() {
        let settings = RecordingSettings {
            rate_control: RateControl::Constant,
            encoder_preset: EncoderPreset::Quality,
            ..RecordingSettings::default()
        };
        let config = record_config(&settings);
        assert_eq!(
            (config.output_height, config.fps, config.video_kbps),
            (1080, 60, 10_000)
        );
        assert!(config.constant_bitrate);
        assert_eq!(config.speed, EncoderSpeed::Quality);
        assert_eq!(config.replay_seconds, settings.replay_seconds);
        assert!(config.max_buffer_bytes >= 64 * 1024 * 1024);
        assert_eq!(
            clip_format(&RecordingSettings {
                container: Container::Mkv,
                ..settings.clone()
            }),
            ClipFormat::Mkv
        );
        assert_eq!(
            clip_format(&RecordingSettings {
                container: Container::Webm,
                ..settings
            }),
            ClipFormat::Mp4
        );
    }

    #[test]
    fn trimming_deletes_only_our_oldest_clips_and_never_the_new_one() {
        let dir = std::env::temp_dir().join(format!("clipcraft-trim-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let make = |name: &str, size: usize| {
            let path = dir.join(name);
            fs::write(&path, vec![0_u8; size]).unwrap();
            path
        };
        let old = make("ClipCraft 2020-01-01 00-00-00.mp4", 100);
        let newer = make("ClipCraft 2021-01-01 00-00-00.mp4", 100);
        let foreign = make("my own clip.mp4", 100);
        let fresh = make("ClipCraft 2022-01-01 00-00-00.mp4", 100);
        // Give the files a clear age order.
        for (path, age) in [(&old, 1_000_u64), (&newer, 500), (&fresh, 0)] {
            let when = SystemTime::now() - Duration::from_secs(age);
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        let deleted = trim_to_limit(&dir, &fresh, 350);
        assert_eq!(deleted, vec![old.clone()]);
        assert!(!old.exists() && newer.exists() && foreign.exists() && fresh.exists());
        // A limit smaller than the clips we must keep still never touches the others.
        let deleted = trim_to_limit(&dir, &fresh, 0);
        assert_eq!(deleted, vec![newer.clone()]);
        assert!(foreign.exists() && fresh.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn trimming_leaves_unfinished_files_and_non_videos_alone_and_counts_sub_folders() {
        let dir = std::env::temp_dir().join(format!("clipcraft-trim-edge-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Games")).unwrap();
        let make = |relative: &str, size: usize, age: u64| {
            let path = dir.join(relative);
            fs::write(&path, vec![0_u8; size]).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(SystemTime::now() - Duration::from_secs(age))
                .unwrap();
            path
        };
        // Files that are being written or are not clips must survive any limit.
        let partial = make("ClipCraft 2019-01-01 00-00-00.mp4.partial", 500, 9_000);
        let recovery = make("ClipCraft 2019-01-01 00-00-01.mp4.recovery", 500, 9_000);
        let notes = make("ClipCraft notes.txt", 500, 9_000);
        // A clip the person filed away in a sub-folder is still one of ours, and is the oldest.
        let filed = make("Games/ClipCraft 2020-01-01 00-00-00.mp4", 100, 3_000);
        let newer = make("ClipCraft 2021-01-01 00-00-00.mp4", 100, 2_000);
        let fresh = make("ClipCraft 2022-01-01 00-00-00.mp4", 100, 0);
        // 300 bytes of clips against a 250 byte limit: only the oldest goes, from its sub-folder.
        let deleted = trim_to_limit(&dir, &fresh, 250);
        assert_eq!(deleted, vec![filed.clone()]);
        assert!(newer.exists() && fresh.exists());
        for kept in [&partial, &recovery, &notes] {
            assert!(kept.exists(), "{kept:?} must not be touched");
        }
        // A folder that is already under the limit loses nothing, and a missing one is fine.
        assert!(trim_to_limit(&dir, &fresh, 10_000).is_empty());
        assert!(trim_to_limit(&dir.join("absent"), &fresh, 0).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    fn wait_for_video(session: &CaptureSession) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if session.status().is_some_and(|s| s.buffered_seconds >= 1.2) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("no video buffered");
    }

    #[test]
    fn a_session_records_saves_and_stops() {
        let dir = std::env::temp_dir().join(format!("clipcraft-session-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let settings = RecordingSettings {
            quality: crate::QualityMode::Custom,
            height: 180,
            frames_per_second: 30,
            video_kbps: 1_000,
            replay_seconds: 15,
            keyframe_seconds: 1,
            ..RecordingSettings::default()
        };
        let mut session = CaptureSession::new();
        assert_eq!(
            session.save_clip(&settings, &dir),
            Err(EngineError::NotRecording)
        );
        session
            .start(
                &settings,
                Box::new(SyntheticSource::new(320, 180, 30)),
                None,
            )
            .unwrap();
        assert_eq!(
            session.start(
                &settings,
                Box::new(SyntheticSource::new(320, 180, 30)),
                None
            ),
            Err(EngineError::AlreadyRecording)
        );
        wait_for_video(&session);
        let report = session.save_clip(&settings, &dir).unwrap();
        assert!(report.clip.path.starts_with(&dir));
        assert!(
            report
                .clip
                .path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(CLIP_PREFIX)
        );
        assert_eq!(
            report.clip.path.extension().and_then(|e| e.to_str()),
            Some("mp4")
        );
        // Two saves in the same second get different names and nothing is overwritten.
        let mut game = "A game label longer than forty characters from the allowlist".to_owned();
        let job = session
            .prepare_save(&settings, &dir)
            .unwrap()
            .with_game(Some(&game));
        game.clear(); // The live process/game can change while a save is pending.
        assert_eq!(
            job.recorded_game.as_deref(),
            Some("A game label longer than forty characters from the allowlist")
        );
        let second = job.run().unwrap();
        assert_eq!(
            second.recorded_game.as_deref(),
            Some("A game label longer than forty characters from the allowlist")
        );
        assert_eq!(
            session
                .prepare_save(&settings, &dir)
                .unwrap()
                .with_game(Some("bad\nlabel"))
                .recorded_game,
            None
        );
        assert_ne!(second.clip.path, report.clip.path);
        assert!(report.clip.path.exists() && second.clip.path.exists());
        assert!(session.stop().is_some() && !session.is_active());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_folder_with_the_stop_policy_refuses_to_save_and_keeps_everything() {
        let dir = std::env::temp_dir().join(format!("clipcraft-limit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("big.mp4"), vec![0_u8; 2_000_000]).unwrap();
        let settings = RecordingSettings {
            delete_oldest_when_full: false,
            ..RecordingSettings::default()
        };
        let mut session = CaptureSession::new();
        session
            .start(&settings, Box::new(SyntheticSource::new(160, 90, 30)), None)
            .unwrap();
        wait_for_video(&session);
        assert_eq!(
            session
                .prepare_save_within(&settings, &dir, 1_000_000)
                .and_then(SaveJob::run),
            Err(EngineError::StorageLimit)
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        // With the delete policy the same folder saves, and the foreign file is left alone.
        let delete = RecordingSettings {
            delete_oldest_when_full: true,
            ..settings
        };
        let report = session
            .prepare_save_within(&delete, &dir, 1_000_000)
            .and_then(SaveJob::run)
            .unwrap();
        assert!(report.deleted.is_empty() && dir.join("big.mp4").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn asking_for_the_gpu_on_a_computer_without_one_records_in_software() {
        let settings = RecordingSettings {
            video_encoder: VideoEncoder::Gpu,
            quality: crate::QualityMode::Custom,
            height: 180,
            frames_per_second: 30,
            video_kbps: 1_000,
            ..RecordingSettings::default()
        };
        let mut session = CaptureSession::new();
        session
            .start(
                &settings,
                Box::new(SyntheticSource::new(320, 180, 30)),
                None,
            )
            .unwrap();
        wait_for_video(&session);
        let encoder = session.status().unwrap().encoder;
        assert!(encoder.starts_with("CPU (GPU unavailable"), "{encoder}");
        // And with the CPU chosen the encoder is just the CPU.
        let mut plain = CaptureSession::new();
        plain
            .start(
                &RecordingSettings {
                    video_encoder: VideoEncoder::Cpu,
                    ..settings
                },
                Box::new(SyntheticSource::new(320, 180, 30)),
                None,
            )
            .unwrap();
        wait_for_video(&plain);
        assert_eq!(plain.status().unwrap().encoder, "CPU");
    }

    fn tone_provider(microphone_works: bool) -> AudioProvider {
        use clipcraft_record::{AudioFormat, ToneSource};
        AudioProvider::new(move |kind, _device| match kind {
            AudioKind::SystemOutput => Ok(Box::new(ToneSource::new(
                AudioFormat {
                    sample_rate: 44_100,
                    channels: 2,
                },
                440.0,
                0.3,
            ))),
            AudioKind::Microphone if microphone_works => Ok(Box::new(ToneSource::new(
                AudioFormat {
                    sample_rate: 48_000,
                    channels: 1,
                },
                880.0,
                0.3,
            ))),
            AudioKind::Microphone => Err("no microphone is plugged in".to_owned()),
        })
    }

    fn sound_settings(separate: bool) -> RecordingSettings {
        RecordingSettings {
            capture_game_audio: true,
            capture_microphone: true,
            separate_audio_tracks: separate,
            ..RecordingSettings::default()
        }
    }

    #[test]
    fn the_settings_choose_one_mixed_track_or_separate_ones() {
        for (separate, tracks) in [(false, 1), (true, 2)] {
            let dir = std::env::temp_dir()
                .join(format!("clipcraft-sound-{}-{separate}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            let settings = sound_settings(separate);
            let mut session = CaptureSession::new();
            session
                .start_with_audio(
                    &settings,
                    Box::new(SyntheticSource::new(160, 90, 30)),
                    None,
                    Some(tone_provider(true)),
                )
                .unwrap();
            assert!(session.warnings().is_empty());
            wait_for_video(&session);
            let report = session.save_clip(&settings, &dir).unwrap();
            assert_eq!(report.clip.audio_tracks, tracks);
            assert_eq!(session.status().unwrap().audio.len(), 2);
            session.stop();
            let _ = fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_microphone_that_cannot_open_is_reported_and_the_clip_still_has_game_sound() {
        let dir = std::env::temp_dir().join(format!("clipcraft-nomic-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let settings = sound_settings(true);
        let mut session = CaptureSession::new();
        session
            .start_with_audio(
                &settings,
                Box::new(SyntheticSource::new(160, 90, 30)),
                None,
                Some(tone_provider(false)),
            )
            .unwrap();
        assert_eq!(session.warnings().len(), 1);
        assert!(session.warnings()[0].contains("microphone"));
        assert!(!session.has_microphone());
        wait_for_video(&session);
        assert_eq!(
            session
                .save_clip(&settings, &dir)
                .unwrap()
                .clip
                .audio_tracks,
            1
        );
        // With nothing to record, video still records.
        session.stop();
        let mut silent = CaptureSession::new();
        silent
            .start_with_audio(
                &sound_settings(false),
                Box::new(SyntheticSource::new(160, 90, 30)),
                None,
                Some(AudioProvider::new(|_, _| Err("no devices".to_owned()))),
            )
            .unwrap();
        assert_eq!(silent.warnings().len(), 2);
        wait_for_video(&silent);
        assert_eq!(
            silent
                .save_clip(&sound_settings(false), &dir)
                .unwrap()
                .clip
                .audio_tracks,
            0
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mute_toggle_and_push_to_talk_decide_whether_the_microphone_is_heard() {
        let mut settings = sound_settings(false);
        let mut session = CaptureSession::new();
        session
            .start_with_audio(
                &settings,
                Box::new(SyntheticSource::new(160, 90, 30)),
                None,
                Some(tone_provider(true)),
            )
            .unwrap();
        assert!(session.has_microphone() && !session.microphone_muted());
        assert!(session.toggle_microphone());
        assert!(session.microphone_muted());
        assert!(!session.toggle_microphone());
        session.stop();

        settings.push_to_talk = true;
        session
            .start_with_audio(
                &settings,
                Box::new(SyntheticSource::new(160, 90, 30)),
                None,
                Some(tone_provider(true)),
            )
            .unwrap();
        // Silent until the key is held.
        assert!(session.microphone_muted());
        session.set_talking(true);
        assert!(!session.microphone_muted());
        // A muted microphone stays muted while the key is held.
        session.set_microphone_off(true);
        assert!(session.microphone_muted());
        session.set_microphone_off(false);
        session.set_talking(false);
        assert!(session.microphone_muted());
        // Levels change live.
        settings.microphone_percent = 30;
        session.set_levels(&settings);
        session.stop();
        assert!(!session.has_microphone());
    }

    #[test]
    fn post_roll_keeps_recording_after_the_hotkey_and_the_clip_includes_it() {
        let dir = std::env::temp_dir().join(format!("clipcraft-postroll-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let settings = RecordingSettings {
            replay_seconds: 15,
            post_roll_seconds: 3,
            ..RecordingSettings::default()
        };
        let mut session = CaptureSession::new();
        session
            .start(&settings, Box::new(SyntheticSource::new(160, 90, 30)), None)
            .unwrap();
        wait_for_video(&session);
        // About 1.2 s is buffered when the key is pressed.
        let buffered = session
            .status()
            .map_or(0.0, |status| f64::from(status.buffered_seconds));
        let job = session.prepare_save(&settings, &dir).unwrap();
        assert_eq!(job.post_roll(), Duration::from_secs(3));
        let pressed = Instant::now();
        let report = job.run().unwrap();
        assert!(pressed.elapsed() >= Duration::from_secs(3));
        // A slow machine may drop pictures while the encoder catches up, so the clip is not
        // required to be the full 4.2 s; it must clearly hold more than was buffered at the
        // press, which is the picture taken during the post-roll.
        let seconds = report.clip.duration_frames as f64 / 30.0;
        assert!(
            seconds > buffered + 0.5,
            "the clip is only {seconds} s with {buffered} s buffered at the press"
        );

        // With no post-roll the same press saves what is there at once.
        let now = RecordingSettings {
            post_roll_seconds: 0,
            ..settings.clone()
        };
        let pressed = Instant::now();
        let quick = session.prepare_save(&now, &dir).unwrap().run().unwrap();
        assert!(pressed.elapsed() < Duration::from_secs(2));
        assert!(quick.clip.duration_frames > 0);
        session.stop();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stopping_during_the_post_roll_saves_at_once_with_what_was_recorded() {
        let dir =
            std::env::temp_dir().join(format!("clipcraft-postroll-stop-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let settings = RecordingSettings {
            post_roll_seconds: 15,
            ..RecordingSettings::default()
        };
        let mut session = CaptureSession::new();
        session
            .start(&settings, Box::new(SyntheticSource::new(160, 90, 30)), None)
            .unwrap();
        wait_for_video(&session);
        let job = session.prepare_save(&settings, &dir).unwrap();
        let worker = std::thread::spawn(move || job.run());
        std::thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        session.stop();
        let report = worker.join().unwrap().unwrap();
        // Waiting out the 15 s post-roll would take about 14.7 s more; a slow machine needs a
        // few seconds to write the clip, so the limit sits well between the two.
        assert!(started.elapsed() < Duration::from_secs(11));
        assert!(report.clip.path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn post_roll_is_bounded_and_widens_the_buffer() {
        let settings = RecordingSettings {
            post_roll_seconds: 9_999,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(settings.post_roll_seconds, 30);
        let config = record_config(&settings);
        assert_eq!(config.replay_seconds, settings.replay_seconds + 30);
    }

    /// A small real disk (a tmpfs) for tests that need one to fill. `None` where mounting is not
    /// allowed, and the test is skipped there.
    struct SmallDisk(PathBuf);

    impl SmallDisk {
        fn new(megabytes: u32) -> Option<Self> {
            // Each disk gets its own folder: tests run side by side.
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "clipcraft-smalldisk-{}-{number}",
                std::process::id()
            ));
            fs::create_dir_all(&path).ok()?;
            let mounted = std::process::Command::new("mount")
                .args(["-t", "tmpfs", "-o", &format!("size={megabytes}m"), "tmpfs"])
                .arg(&path)
                .output()
                .ok()
                .is_some_and(|output| output.status.success());
            if mounted {
                Some(Self(path))
            } else {
                let _ = fs::remove_dir(&path);
                None
            }
        }

        /// Fills the disk until `leave` bytes are free.
        fn fill_leaving(&self, leave: u64) {
            let free = crate::storage::storage_usage(&self.0)
                .free_bytes
                .expect("free space is known");
            let filler = vec![0_u8; usize::try_from(free.saturating_sub(leave)).expect("fits")];
            fs::write(self.0.join("filler.bin"), filler).expect("filler writes");
        }
    }

    impl Drop for SmallDisk {
        fn drop(&mut self) {
            let _ = std::process::Command::new("umount").arg(&self.0).output();
            let _ = fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn a_full_disk_refuses_the_save_before_writing_and_keeps_clips_already_saved() {
        let Some(disk) = SmallDisk::new(16) else {
            eprintln!("skipped: this environment cannot mount a small disk");
            return;
        };
        let settings = RecordingSettings::default();
        let mut session = CaptureSession::new();
        session
            .start(&settings, Box::new(SyntheticSource::new(160, 90, 30)), None)
            .unwrap();
        wait_for_video(&session);
        let saved = session.save_clip(&settings, &disk.0).unwrap();
        let before = fs::read(&saved.clip.path).unwrap();

        disk.fill_leaving(200_000);
        let names = |dir: &Path| {
            let mut names: Vec<String> = fs::read_dir(dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        let listing = names(&disk.0);
        assert_eq!(
            session.save_clip(&settings, &disk.0).map(|_| ()),
            Err(EngineError::Record(RecordError::DiskFull))
        );
        assert_eq!(
            names(&disk.0),
            listing,
            "a refused save leaves no file behind"
        );
        assert_eq!(
            fs::read(&saved.clip.path).unwrap(),
            before,
            "the saved clip is untouched"
        );

        // The recording itself is fine, and saving elsewhere still works.
        let elsewhere =
            std::env::temp_dir().join(format!("clipcraft-elsewhere-{}", std::process::id()));
        assert!(session.save_clip(&settings, &elsewhere).is_ok());
        let _ = fs::remove_dir_all(&elsewhere);
        session.stop();
    }

    #[test]
    fn a_save_that_fails_never_deletes_older_clips_to_make_room() {
        let Some(disk) = SmallDisk::new(16) else {
            eprintln!("skipped: this environment cannot mount a small disk");
            return;
        };
        let old: Vec<PathBuf> = (1..=3)
            .map(|n| {
                disk.0
                    .join(format!("{CLIP_PREFIX}2026-01-0{n} 10-00-00.mp4"))
            })
            .collect();
        for path in &old {
            fs::write(path, vec![1_u8; 300_000]).unwrap();
        }
        let settings = RecordingSettings {
            delete_oldest_when_full: true,
            storage_limit_gb: 1,
            ..RecordingSettings::default()
        };
        let mut session = CaptureSession::new();
        session
            .start(&settings, Box::new(SyntheticSource::new(160, 90, 30)), None)
            .unwrap();
        wait_for_video(&session);
        disk.fill_leaving(100_000);
        // A tiny limit would normally delete the old clips after a save; the save fails first.
        let job = session.prepare_save_within(&settings, &disk.0, 400_000);
        assert_eq!(
            job.map(|_| ()),
            Err(EngineError::Record(RecordError::DiskFull))
        );
        assert!(
            old.iter().all(|path| path.exists()),
            "old clips must survive a failed save"
        );
        session.stop();
    }
}
