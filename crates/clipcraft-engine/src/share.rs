//! Share copies: a clip made small enough for a chat (Discord takes 10, 50 or 500 MB), kept in a
//! `Shared` folder beside the clip and reused until the clip changes. The original is never
//! touched. A clip that already fits is shared as it is. Command `clip.share_copy`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::{EngineError, ExportPreset, export_clip};

/// The folder, beside a clip, that holds its share copies.
pub const SHARE_FOLDER: &str = "Shared";

/// How a clip would be shared at a size.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SharePlan {
    /// The clip already fits; share the file itself.
    Original(PathBuf),
    /// A copy that fits is already made and is newer than the clip.
    Existing(PathBuf),
    /// A copy has to be made, at this path.
    Needed(PathBuf),
}

/// A share copy that is ready.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct SharedCopy {
    /// The file to share.
    pub path: String,
    /// A copy was made now (false when the clip fit or a copy already existed).
    pub made: bool,
}

fn target_bytes(megabytes: u16) -> Result<u64, EngineError> {
    ExportPreset::Custom { megabytes }
        .target_bytes()
        .map_err(EngineError::Export)
}

/// What sharing `source` at `megabytes` takes, without doing it.
pub fn share_plan(source: &Path, megabytes: u16) -> Result<SharePlan, EngineError> {
    let source = crate::existing_clip(source)?;
    let target = target_bytes(megabytes)?;
    let metadata = fs::metadata(&source).map_err(|_| EngineError::FileUnavailable)?;
    if metadata.len() <= target {
        return Ok(SharePlan::Original(source));
    }
    let folder = source
        .parent()
        .ok_or(EngineError::FileUnavailable)?
        .join(SHARE_FOLDER);
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .ok_or(EngineError::FileUnavailable)?;
    let modified = metadata.modified().ok();
    for attempt in 1..=20_u32 {
        let suffix = if attempt == 1 {
            String::new()
        } else {
            format!(" {attempt}")
        };
        let candidate = folder.join(format!("{stem} ({megabytes} MB){suffix}.mp4"));
        match fs::metadata(&candidate) {
            // A copy that fits and was made after the clip last changed is reused.
            Ok(copy)
                if copy.is_file()
                    && copy.len() > 0
                    && copy.len() <= target
                    && copy.modified().ok() >= modified =>
            {
                return Ok(SharePlan::Existing(candidate));
            }
            // An out-of-date or unusable copy is left alone and the next name is tried.
            Ok(_) => {}
            Err(_) => return Ok(SharePlan::Needed(candidate)),
        }
    }
    Err(EngineError::FileUnavailable)
}

/// Makes the share copy of `source` at `megabytes`, or says there is nothing to make. The picture
/// is re-encoded to fit (once more, smaller, if the first try is over), and `progress` runs 0 to 1.
pub fn make_share_copy(
    source: &Path,
    megabytes: u16,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<SharedCopy, EngineError> {
    match share_plan(source, megabytes)? {
        SharePlan::Original(path) | SharePlan::Existing(path) => Ok(SharedCopy {
            path: path.to_string_lossy().into_owned(),
            made: false,
        }),
        SharePlan::Needed(output) => {
            if let Some(folder) = output.parent() {
                fs::create_dir_all(folder).map_err(|_| EngineError::FileUnavailable)?;
            }
            let request = clipcraft_export::TrimRequest {
                input: crate::existing_clip(source)?,
                output,
                start: clipcraft_time::Tick::ZERO,
                end: clipcraft_time::Tick::from_units(i64::from(u32::MAX), 1_000),
            };
            let exported = export_clip(
                &request,
                ExportPreset::Custom { megabytes },
                progress,
                cancel,
            )?;
            Ok(SharedCopy {
                path: exported.clip.path,
                made: true,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn folder() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "clipcraft-share-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_clip_that_fits_is_shared_as_it_is_and_one_that_does_not_gets_a_named_copy() {
        let dir = folder();
        let small = dir.join("small.mp4");
        let big = dir.join("Hades 2026.mp4");
        fs::write(&small, vec![0_u8; 500_000]).unwrap();
        fs::write(&big, vec![0_u8; 3_000_000]).unwrap();
        assert!(matches!(share_plan(&small, 1), Ok(SharePlan::Original(_))));
        let SharePlan::Needed(output) = share_plan(&big, 1).unwrap() else {
            panic!("a 3 MB clip does not fit 1 MB");
        };
        assert_eq!(output, dir.join("Shared").join("Hades 2026 (1 MB).mp4"));
        assert!(!dir.join("Shared").exists(), "planning writes nothing");
        // Non-clips and impossible sizes are refused.
        let notes = dir.join("notes.txt");
        fs::write(&notes, b"x").unwrap();
        assert_eq!(share_plan(&notes, 10), Err(EngineError::NotAClip));
        assert!(share_plan(&big, 0).is_err());
        assert!(share_plan(&big, 501).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_made_copy_is_reused_until_the_clip_changes_and_a_stale_one_is_never_overwritten() {
        let dir = folder();
        let big = dir.join("clip.mp4");
        fs::write(&big, vec![0_u8; 3_000_000]).unwrap();
        let shared = dir.join("Shared");
        fs::create_dir_all(&shared).unwrap();
        let copy = shared.join("clip (1 MB).mp4");
        fs::write(&copy, vec![0_u8; 800_000]).unwrap();
        // The copy was made after the clip: reused.
        assert_eq!(share_plan(&big, 1), Ok(SharePlan::Existing(copy.clone())));
        // The clip changes afterwards: the old copy is left alone and a new name is used.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3_600);
        fs::OpenOptions::new()
            .write(true)
            .open(&big)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(
            share_plan(&big, 1),
            Ok(SharePlan::Needed(shared.join("clip (1 MB) 2.mp4")))
        );
        assert!(copy.exists());
        // The library does not list share copies as clips of their own.
        let scan = crate::scan::scan(&dir, 100, 100, &AtomicBool::new(false)).unwrap();
        assert!(
            scan.entries
                .iter()
                .all(|entry| !entry.path.contains("Shared"))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_real_clip_over_the_size_is_re_encoded_into_a_copy_that_fits_and_is_then_reused() {
        use crate::{CaptureSession, QualityMode, RateControl, RecordingSettings, VideoEncoder};
        use clipcraft_capture::SyntheticSource;
        let dir = folder();
        let settings = RecordingSettings {
            quality: QualityMode::Custom,
            video_encoder: VideoEncoder::Cpu,
            rate_control: RateControl::Constant,
            height: 360,
            frames_per_second: 30,
            video_kbps: 8_000,
            replay_seconds: 15,
            keyframe_seconds: 1,
            capture_game_audio: false,
            capture_microphone: false,
            ..RecordingSettings::default()
        };
        let mut session = CaptureSession::new();
        session
            .start(
                &settings,
                Box::new(SyntheticSource::new(640, 360, 30)),
                None,
            )
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while session
            .status()
            .is_none_or(|status| status.buffered_seconds < 6.0)
        {
            assert!(std::time::Instant::now() < deadline, "did not fill");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let saved = session
            .prepare_save(&settings, &dir)
            .unwrap()
            .run()
            .unwrap();
        session.stop();
        let clip = PathBuf::from(&saved.clip.path);
        let original = fs::metadata(&clip).unwrap().len();
        assert!(
            original > 2_000_000,
            "the test clip is only {original} bytes"
        );

        let never = AtomicBool::new(false);
        let made = make_share_copy(&clip, 2, &mut |_| {}, &never).unwrap();
        assert!(made.made);
        let copy = PathBuf::from(&made.path);
        assert_eq!(
            copy.parent().unwrap(),
            clip.parent().unwrap().join("Shared")
        );
        let size = fs::metadata(&copy).unwrap().len();
        assert!(size <= 2_000_000, "{size} bytes is over 2 MB");
        assert_eq!(
            fs::metadata(&clip).unwrap().len(),
            original,
            "the clip is untouched"
        );
        // The second time there is nothing to make.
        let scan = crate::scan::scan(&dir, 100, 100, &AtomicBool::new(false)).unwrap();
        assert!(
            scan.entries
                .iter()
                .all(|entry| !entry.path.contains("/Shared/")),
            "share copies are not library clips"
        );
        let again = make_share_copy(&clip, 2, &mut |_| {}, &never).unwrap();
        assert!(!again.made);
        assert_eq!(again.path, made.path);
        let _ = fs::remove_dir_all(&dir);
    }
}
