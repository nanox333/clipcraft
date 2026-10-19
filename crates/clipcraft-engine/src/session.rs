//! A running recording: starting it from the settings, saving a clip into the capture folder, and
//! keeping the folder inside the storage limit.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use clipcraft_capture::CaptureSource;
use clipcraft_record::{
    ClipFormat, EncoderSpeed, RecordConfig, RecordError, RecordStatus, Recorder, SavedClip,
};

use crate::storage::{StoredClip, cleanup_plan, list_clips};
use crate::{Container, EncoderPreset, EngineError, RateControl, RecordingSettings};

/// Every clip ClipCraft saves starts with this, so cleanup only ever deletes its own clips.
pub const CLIP_PREFIX: &str = "ClipCraft ";

/// What the recorder is asked to do for these settings.
pub fn record_config(settings: &RecordingSettings) -> RecordConfig {
    let bytes_per_second = u64::from(settings.video_kbps) * 1000 / 8;
    // Room for the replay window at twice the average rate (bursts), plus a fixed margin.
    let wanted = bytes_per_second
        .saturating_mul(u64::from(settings.replay_seconds))
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
        replay_seconds: settings.replay_seconds,
        max_buffer_bytes: usize::try_from(wanted.clamp(64 * 1024 * 1024, 2 * 1024 * 1024 * 1024))
            .unwrap_or(64 * 1024 * 1024),
        threads: 0,
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
}

/// The recording in progress, if any. Owned by whoever drives the app.
#[derive(Default)]
pub struct CaptureSession {
    recorder: Option<Arc<Recorder>>,
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

    /// Starts recording `source` with the picture, bitrate, and replay settings.
    /// Command id `capture.start`.
    pub fn start(
        &mut self,
        settings: &RecordingSettings,
        source: Box<dyn CaptureSource>,
    ) -> Result<(), EngineError> {
        if self.recorder.is_some() {
            return Err(EngineError::AlreadyRecording);
        }
        let recorder = Recorder::start(source, record_config(&settings.clone().sanitized()))
            .map_err(EngineError::Record)?;
        self.recorder = Some(Arc::new(recorder));
        Ok(())
    }

    /// Stops recording and drops the buffer. Command id `capture.stop`.
    pub fn stop(&mut self) -> Option<RecordStatus> {
        let recorder = self.recorder.take()?;
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
        Ok(SaveJob {
            recorder,
            format,
            seconds: settings.replay_seconds,
            path: free_clip_path(folder, format, unix_now())?,
            folder: folder.to_path_buf(),
            trim_to: (limit > 0 && settings.delete_oldest_when_full).then_some(limit),
        })
    }
}

/// A clip save that can run on any thread.
pub struct SaveJob {
    recorder: Arc<Recorder>,
    format: ClipFormat,
    seconds: u32,
    path: PathBuf,
    folder: PathBuf,
    trim_to: Option<u64>,
}

impl SaveJob {
    /// The file that will be written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the clip, then removes the oldest ClipCraft clips if the folder is over its limit.
    pub fn run(self) -> Result<SavedClipReport, EngineError> {
        let clip = self
            .recorder
            .save_clip(self.seconds, self.format, &self.path)
            .map_err(EngineError::Record)?;
        let deleted = match self.trim_to {
            Some(limit) => trim_to_limit(&self.folder, &clip.path, limit),
            None => Vec::new(),
        };
        Ok(SavedClipReport { clip, deleted })
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
            .start(&settings, Box::new(SyntheticSource::new(320, 180, 30)))
            .unwrap();
        assert_eq!(
            session.start(&settings, Box::new(SyntheticSource::new(320, 180, 30))),
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
        let second = session.save_clip(&settings, &dir).unwrap();
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
            .start(&settings, Box::new(SyntheticSource::new(160, 90, 30)))
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
}
