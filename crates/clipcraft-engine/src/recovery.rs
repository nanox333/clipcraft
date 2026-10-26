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
    // Unfinished files first, crash-safe copies after: a finished-but-unrenamed clip is kept as
    // it is, and its copy is then just left over.
    let mut entries: Vec<fs::DirEntry> = listing.filter_map(Result::ok).take(MAX_ENTRIES).collect();
    entries.sort_by_key(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        (name.ends_with(".recovery"), name)
    });
    for entry in entries {
        let path = entry.path();
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        let recovery_copy = name.ends_with(".recovery");
        let Some(base) = name
            .strip_suffix(".partial")
            .or_else(|| name.strip_suffix(".recovery"))
        else {
            continue;
        };
        if !is_ours(base) || (recovery_copy && !base.ends_with(".mp4")) {
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
        if recovery_copy {
            remake_from_recovery_copy(folder, base, &path, &mut recovery);
            continue;
        }
        // A cut-off Matroska file, or an MP4 that was finished but not yet renamed, may still play.
        if clipcraft_media::probe_file(&path)
            .is_ok_and(|probe| probe.width.is_some() && probe.height.is_some())
            && let Some(target) = free_name(folder, base)
            && fs::rename(&path, &target).is_ok()
        {
            recovery.salvaged.push(target);
            continue;
        }
        // An MP4 with no index is unusable, but its crash-safe copy may be there; the copy is
        // handled when its own turn comes, and an unusable half-written file is removed.
        if fs::remove_file(&path).is_ok() {
            recovery.removed.push(path);
        }
    }
    recovery.removed.sort();
    recovery.salvaged.sort();
    recovery
}

/// Turns the crash-safe copy of a clip (a fragmented MP4, playable up to its last complete
/// fragment) back into an ordinary clip.
fn remake_from_recovery_copy(folder: &Path, base: &str, copy: &Path, recovery: &mut Recovery) {
    let target = folder.join(base);
    // The clip was finished (or already recovered from its other file); the copy is just left over.
    if target.exists() {
        let _ = fs::remove_file(copy);
        return;
    }
    let remade = complete_prefix(copy)
        .and_then(|keep| {
            let file = fs::OpenOptions::new().write(true).open(copy).ok()?;
            file.set_len(keep).ok()
        })
        .and_then(|()| {
            let never = std::sync::atomic::AtomicBool::new(false);
            clipcraft_export::trim_to_mp4(
                &clipcraft_export::TrimRequest {
                    input: copy.to_path_buf(),
                    output: target.clone(),
                    start: clipcraft_time::Tick::ZERO,
                    end: clipcraft_time::Tick::from_seconds_f64(7.0 * 24.0 * 3600.0),
                },
                &mut |_| {},
                &never,
            )
            .ok()
        })
        .is_some();
    if remade {
        recovery.salvaged.push(target);
    } else {
        recovery.removed.push(copy.to_path_buf());
    }
    let _ = fs::remove_file(copy);
}

/// The length of the longest prefix of a fragmented MP4 that holds its header and only whole
/// fragments, or `None` when even the header is incomplete.
fn complete_prefix(path: &Path) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let (mut offset, mut good, mut moov_seen, mut fragment_header_end) =
        (0_u64, None, false, false);
    for _ in 0..1_000_000 {
        if offset.checked_add(8)? > length {
            break;
        }
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut header = [0_u8; 16];
        let wanted = usize::try_from((length - offset).min(16)).ok()?;
        file.read_exact(header.get_mut(..wanted)?).ok()?;
        let declared = u64::from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]));
        let size = match declared {
            0 => length - offset,
            1 if wanted >= 16 => u64::from_be_bytes([
                header[8], header[9], header[10], header[11], header[12], header[13], header[14],
                header[15],
            ]),
            1 => break,
            other => other,
        };
        let end = offset.checked_add(size)?;
        if size < 8 || end > length {
            break;
        }
        match &header[4..8] {
            b"moov" => {
                moov_seen = true;
                good = Some(end);
            }
            b"moof" => fragment_header_end = true,
            b"mdat" if moov_seen && fragment_header_end => {
                good = Some(end);
                fragment_header_end = false;
            }
            _ => {}
        }
        offset = end;
    }
    good
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
                codec: clipcraft_record::StreamCodec::H264,
                hardware: None,
                gap_restart_seconds: 10,
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

    /// A recorder with a tone on its sound, five seconds in, for the crash-safe copy tests.
    fn recorder_with_sound() -> clipcraft_record::Recorder {
        use clipcraft_capture::SyntheticSource;
        use clipcraft_capture::reframe::{Framing, OutputShape};
        use clipcraft_record::{
            AudioConfig, AudioControl, AudioFormat, AudioInput, AudioSetup, EncoderSpeed,
            RecordConfig, Recorder, ToneSource,
        };
        let recorder = Recorder::start_with_audio(
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
                replay_seconds: 20,
                max_buffer_bytes: 64 * 1024 * 1024,
                threads: 2,
                codec: clipcraft_record::StreamCodec::H264,
                hardware: None,
                gap_restart_seconds: 10,
            },
            Some(AudioSetup {
                config: AudioConfig {
                    sample_rate: 48_000,
                    channels: 2,
                    kbps: 128,
                    tracks: vec!["Audio".to_owned()],
                },
                inputs: vec![AudioInput::new(
                    Box::new(ToneSource::new(
                        AudioFormat {
                            sample_rate: 48_000,
                            channels: 2,
                        },
                        440.0,
                        0.4,
                    )),
                    AudioControl::default(),
                    0,
                )],
            }),
        )
        .expect("recorder");
        let deadline = std::time::Instant::now() + Duration::from_secs(40);
        while recorder.status().buffered_seconds < 5.0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(50));
        }
        recorder
    }

    /// Duration, whether sound is present, and whether it decodes without a complaint.
    fn judge(path: &Path) -> Option<(f64, bool, bool)> {
        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration:stream=codec_type",
                "-of",
                "csv=p=0",
            ])
            .arg(path)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&probe.stdout).into_owned();
        let duration = text
            .lines()
            .filter_map(|l| l.trim().parse::<f64>().ok())
            .fold(0.0, f64::max);
        let decoded = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args(["-f", "null", "-"])
            .output()
            .ok()?;
        Some((
            duration,
            text.contains("audio"),
            decoded.status.success() && decoded.stderr.is_empty(),
        ))
    }

    #[test]
    fn a_cut_off_mp4_is_recovered_from_its_crash_safe_copy_with_picture_and_sound() {
        let scratch = Scratch::new("mp4-copy");
        let recorder = recorder_with_sound();
        let name = "ClipCraft 2026-10-20 07-15-36.mp4";
        // The program died after two fragments: the copy has them, the real file is half written.
        recorder
            .save_clip_interrupted(20, &scratch.0.join(format!("{name}.recovery")), 2)
            .expect("copy");
        fs::write(
            scratch.0.join(format!("{name}.partial")),
            b"half an mp4 with no index",
        )
        .expect("partial");
        let recovery = recover_interrupted_after(&scratch.0, Duration::ZERO);
        assert_eq!(
            recovery.salvaged,
            vec![scratch.0.join(name)],
            "{recovery:?}"
        );
        assert_eq!(
            recovery.removed.len(),
            1,
            "the unusable half file goes: {recovery:?}"
        );
        assert!(!scratch.0.join(format!("{name}.recovery")).exists());
        assert!(!scratch.0.join(format!("{name}.partial")).exists());
        let probe = clipcraft_media::probe_file(&scratch.0.join(name)).expect("probes");
        assert_eq!(
            (probe.width, probe.height, probe.audio_tracks),
            (Some(160), Some(90), 1)
        );
        if let Some((seconds, has_sound, clean)) = judge(&scratch.0.join(name)) {
            assert!((1.5..=4.0).contains(&seconds), "{seconds} s");
            assert!(has_sound && clean, "sound {has_sound}, clean {clean}");
        }
    }

    #[test]
    fn a_crash_safe_copy_cut_off_mid_fragment_keeps_everything_before_the_cut() {
        let scratch = Scratch::new("mid-fragment");
        let recorder = recorder_with_sound();
        let name = "ClipCraft 2026-10-20 07-15-37.mp4";
        let copy = scratch.0.join(format!("{name}.recovery"));
        // The whole copy, then cut at points inside different fragments.
        recorder
            .save_clip_interrupted(20, &copy, 10_000)
            .expect("copy");
        let whole = fs::read(&copy).expect("read");
        let mut longest = 0.0_f64;
        for percent in [35_usize, 55, 80] {
            fs::write(&copy, &whole[..whole.len() * percent / 100]).expect("cut");
            let recovery = recover_interrupted_after(&scratch.0, Duration::ZERO);
            assert_eq!(recovery.salvaged.len(), 1, "{percent}%: {recovery:?}");
            let kept = scratch.0.join(name);
            if let Some((seconds, _, clean)) = judge(&kept) {
                assert!(clean, "{percent}% does not decode cleanly");
                assert!(
                    seconds > longest,
                    "{percent}%: {seconds} s should beat {longest} s"
                );
                longest = seconds;
            }
            fs::remove_file(&kept).expect("clear");
        }
        // A copy cut inside the header, before any fragment, is not a clip.
        fs::write(&copy, &whole[..40]).expect("cut");
        let nothing = recover_interrupted_after(&scratch.0, Duration::ZERO);
        assert!(
            nothing.salvaged.is_empty() && nothing.removed.len() == 1,
            "{nothing:?}"
        );
    }

    #[test]
    fn a_copy_left_after_a_finished_clip_is_just_removed() {
        let scratch = Scratch::new("leftover");
        let name = "ClipCraft 2026-10-20 07-15-38.mp4";
        fs::write(scratch.0.join(name), b"the real clip").expect("clip");
        fs::write(scratch.0.join(format!("{name}.recovery")), b"copy").expect("copy");
        let recovery = recover_interrupted_after(&scratch.0, Duration::ZERO);
        assert!(
            recovery.is_empty(),
            "nothing to tell the person: {recovery:?}"
        );
        assert!(!scratch.0.join(format!("{name}.recovery")).exists());
        assert_eq!(
            fs::read(scratch.0.join(name)).expect("clip"),
            b"the real clip"
        );
    }
}
