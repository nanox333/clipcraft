//! Cleaning up after a crash or a power cut: a clip being saved or trimmed is written to a
//! `.partial` file and renamed only when complete, so an interruption leaves such a file behind.
//! On launch, ClipCraft's own leftovers are dealt with: a Matroska file that still reads is
//! saved as a clip, anything else is unusable and is removed. Files ClipCraft did not make are
//! never touched, and a leftover young enough to belong to a save still running is left alone.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::session::CLIP_PREFIX;

/// Leftovers newer than this might still be being written.
const SETTLE: Duration = Duration::from_secs(30);
/// Most directory entries looked at.
const MAX_ENTRIES: usize = 20_000;

/// What clean-up did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Recovery {
    /// Unfinished files that could not be used and were removed.
    pub removed: Vec<PathBuf>,
    /// Unfinished clips that still played and were kept; the new names.
    pub salvaged: Vec<PathBuf>,
}

impl Recovery {
    /// True when there was nothing to do.
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.salvaged.is_empty()
    }

    /// A sentence for the person, or `None` when nothing happened.
    pub fn message(&self) -> Option<String> {
        let (removed, salvaged) = (self.removed.len(), self.salvaged.len());
        match (removed, salvaged) {
            (0, 0) => None,
            (removed, 0) => Some(format!(
                "ClipCraft closed unexpectedly while saving. {removed} unfinished {} removed; your other clips are safe.",
                if removed == 1 {
                    "file was"
                } else {
                    "files were"
                }
            )),
            (0, salvaged) => Some(format!(
                "ClipCraft closed unexpectedly while saving. {salvaged} {} recovered.",
                if salvaged == 1 {
                    "clip was"
                } else {
                    "clips were"
                }
            )),
            (removed, salvaged) => Some(format!(
                "ClipCraft closed unexpectedly while saving. {salvaged} recovered, {removed} unfinished removed."
            )),
        }
    }
}

/// Whether ClipCraft made a file with this name (without the `.partial`).
fn is_ours(base: &str) -> bool {
    let playable = base.ends_with(".mp4") || base.ends_with(".mkv");
    playable && (base.starts_with(CLIP_PREFIX) || base.contains(" (trimmed"))
}

/// Deals with unfinished saves in `folder`. Never fails: what cannot be read or removed is left.
pub fn recover_interrupted(folder: &Path) -> Recovery {
    recover_interrupted_after(folder, SETTLE)
}

fn recover_interrupted_after(folder: &Path, settle: Duration) -> Recovery {
    let mut recovery = Recovery::default();
    let Ok(listing) = fs::read_dir(folder) else {
        return recovery;
    };
    for entry in listing.filter_map(Result::ok).take(MAX_ENTRIES) {
        let path = entry.path();
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        let Some(base) = name.strip_suffix(".partial") else {
            continue;
        };
        if !is_ours(base) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let age = metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok());
        if age.is_none_or(|age| age < settle) {
            continue;
        }
        if base.ends_with(".mkv")
            && clipcraft_media::probe_file(&path)
                .is_ok_and(|probe| probe.width.is_some() && probe.height.is_some())
            && let Some(target) = free_name(folder, base)
            && fs::rename(&path, &target).is_ok()
        {
            recovery.salvaged.push(target);
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            recovery.removed.push(path);
        }
    }
    recovery.removed.sort();
    recovery.salvaged.sort();
    recovery
}

/// `base`, or `base (recovered)`, whichever does not exist yet.
fn free_name(folder: &Path, base: &str) -> Option<PathBuf> {
    let plain = folder.join(base);
    if !plain.exists() {
        return Some(plain);
    }
    let (stem, extension) = base.rsplit_once('.')?;
    (1..100).find_map(|n| {
        let tag = if n == 1 {
            "recovered".to_owned()
        } else {
            format!("recovered {n}")
        };
        let candidate = folder.join(format!("{stem} ({tag}).{extension}"));
        (!candidate.exists()).then_some(candidate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("clipcraft-recovery-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("scratch");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn only_our_unfinished_files_are_removed_and_never_other_programs_files() {
        let scratch = Scratch::new("ours");
        let dir = &scratch.0;
        for name in [
            "ClipCraft 2026-10-20 07-15-36.mp4.partial",
            "Trip (trimmed).mp4.partial",
            "movie.mp4.partial",
            "ClipCraft notes.txt.partial",
            "ClipCraft 2026-10-20 07-15-36.mp4",
            "notes.txt",
        ] {
            fs::write(dir.join(name), b"half a file").expect("fixture");
        }
        let recovery = recover_interrupted_after(dir, Duration::ZERO);
        assert_eq!(recovery.removed.len(), 2, "{recovery:?}");
        assert!(recovery.salvaged.is_empty());
        for kept in [
            "movie.mp4.partial",
            "ClipCraft notes.txt.partial",
            "ClipCraft 2026-10-20 07-15-36.mp4",
            "notes.txt",
        ] {
            assert!(dir.join(kept).exists(), "{kept} must stay");
        }
        assert!(
            recovery
                .message()
                .expect("message")
                .contains("2 unfinished files were removed")
        );
        // Nothing left to do the second time.
        assert!(recover_interrupted_after(dir, Duration::ZERO).is_empty());
    }

    #[test]
    fn a_leftover_that_is_still_young_might_be_a_save_in_progress_and_is_left_alone() {
        let scratch = Scratch::new("young");
        let file = scratch.0.join("ClipCraft 2026-10-20 07-15-36.mp4.partial");
        fs::write(&file, b"being written").expect("fixture");
        assert!(recover_interrupted(&scratch.0).is_empty());
        assert!(file.exists());
    }

    #[test]
    fn a_missing_or_unreadable_folder_is_not_an_error() {
        assert!(recover_interrupted(Path::new("/definitely/not/here")).is_empty());
    }

    #[test]
    fn messages_are_worded_for_each_outcome() {
        let one = Recovery {
            removed: vec!["a".into()],
            salvaged: Vec::new(),
        };
        assert!(
            one.message()
                .expect("message")
                .contains("1 unfinished file was removed")
        );
        let saved = Recovery {
            removed: Vec::new(),
            salvaged: vec!["a".into(), "b".into()],
        };
        assert!(
            saved
                .message()
                .expect("message")
                .contains("2 clips were recovered")
        );
        let both = Recovery {
            removed: vec!["a".into()],
            salvaged: vec!["b".into()],
        };
        assert!(
            both.message()
                .expect("message")
                .contains("1 recovered, 1 unfinished removed")
        );
        assert_eq!(Recovery::default().message(), None);
    }

    /// A real MKV and MP4 clip cut off partway, as a crash leaves them.
    fn truncated_clips(dir: &Path) {
        use clipcraft_capture::SyntheticSource;
        use clipcraft_capture::reframe::{Framing, OutputShape};
        use clipcraft_record::{ClipFormat, EncoderSpeed, RecordConfig, Recorder};
        let recorder = Recorder::start(
            Box::new(SyntheticSource::new(160, 90, 30)),
            RecordConfig {
                output_height: 90,
                shape: OutputShape::Source,
                framing: Framing::Original,
                fps: 30,
                video_kbps: 800,
                constant_bitrate: false,
                speed: EncoderSpeed::Fast,
                keyframe_seconds: 1,
                replay_seconds: 10,
                max_buffer_bytes: 64 * 1024 * 1024,
                threads: 2,
                hardware: None,
            },
        )
        .expect("recorder");
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while recorder.status().buffered_seconds < 3.0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(50));
        }
        for (format, name) in [
            (ClipFormat::Mkv, "ClipCraft 2026-10-20 07-15-36.mkv"),
            (ClipFormat::Mp4, "ClipCraft 2026-10-20 07-15-37.mp4"),
        ] {
            let whole = dir.join(format!("whole.{}", format.extension()));
            recorder.save_clip(10, format, &whole).expect("saved");
            let bytes = fs::read(&whole).expect("read");
            fs::remove_file(&whole).expect("remove");
            let partial = dir.join(format!("{name}.partial"));
            fs::write(&partial, &bytes[..bytes.len() * 6 / 10]).expect("truncate");
        }
    }

    #[test]
    fn a_cut_off_mkv_that_still_plays_is_kept_and_a_cut_off_mp4_is_removed() {
        let scratch = Scratch::new("salvage");
        truncated_clips(&scratch.0);
        let recovery = recover_interrupted_after(&scratch.0, Duration::ZERO);
        println!("{recovery:?}");
        assert_eq!(recovery.removed.len(), 1, "{recovery:?}");
        assert!(
            recovery.removed[0]
                .to_string_lossy()
                .ends_with(".mp4.partial")
        );
        assert_eq!(recovery.salvaged.len(), 1, "{recovery:?}");
        let kept = &recovery.salvaged[0];
        assert_eq!(
            kept.file_name().and_then(|n| n.to_str()),
            Some("ClipCraft 2026-10-20 07-15-36.mkv")
        );
        let probe = clipcraft_media::probe_file(kept).expect("the recovered clip probes");
        assert_eq!((probe.width, probe.height), (Some(160), Some(90)));
        assert!(
            !scratch
                .0
                .join("ClipCraft 2026-10-20 07-15-36.mkv.partial")
                .exists()
        );
    }
}
