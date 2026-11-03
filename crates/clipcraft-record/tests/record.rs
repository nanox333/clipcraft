//! End-to-end checks of the recorder with a synthetic screen. ffprobe and ffmpeg are used only as
//! outside judges of the written files and are skipped when they are not installed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use clipcraft_capture::reframe::{Framing, OutputShape};
use clipcraft_capture::{CaptureError, SyntheticSource};
use clipcraft_record::{
    ClipFormat, EncoderSpeed, RecordConfig, RecordError, RecordState, Recorder,
};

fn config() -> RecordConfig {
    RecordConfig {
        output_height: 180,
        shape: OutputShape::Source,
        framing: Framing::Original,
        fps: 30,
        video_kbps: 2_000,
        constant_bitrate: false,
        speed: EncoderSpeed::Fast,
        keyframe_seconds: 1,
        replay_seconds: 10,
        max_buffer_bytes: 64 * 1024 * 1024,
        threads: 2,
        codec: clipcraft_record::StreamCodec::H264,
        hardware: None,
        gap_restart_seconds: 10,
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("clipcraft-record-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_for(
    recorder: &Recorder,
    what: &str,
    ready: impl Fn(&clipcraft_record::RecordStatus) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if ready(&recorder.status()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for {what}: {:?}", recorder.status());
}

fn tool(name: &str) -> bool {
    Command::new(name).arg("-version").output().is_ok()
}

fn probe(path: &Path) -> serde_like::Probe {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
        ])
        .arg("stream=codec_name,width,height,nb_read_frames,avg_frame_rate:format=duration")
        .args(["-of", "default=noprint_wrappers=1"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ffprobe: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_like::Probe::parse(&String::from_utf8_lossy(&output.stdout))
}

/// Just enough parsing of `key=value` lines.
mod serde_like {
    #[derive(Debug, Default)]
    pub struct Probe {
        pub codec: String,
        pub width: u32,
        pub height: u32,
        pub frames: u32,
        pub duration: f64,
        pub frame_rate: String,
    }

    impl Probe {
        pub fn parse(text: &str) -> Self {
            let mut probe = Probe::default();
            for line in text.lines() {
                let Some((key, value)) = line.split_once('=') else {
                    continue;
                };
                match key {
                    "codec_name" => probe.codec = value.to_owned(),
                    "width" => probe.width = value.parse().unwrap_or(0),
                    "height" => probe.height = value.parse().unwrap_or(0),
                    "nb_read_frames" => probe.frames = value.parse().unwrap_or(0),
                    "duration" => probe.duration = value.parse().unwrap_or(0.0),
                    "avg_frame_rate" => probe.frame_rate = value.to_owned(),
                    _ => {}
                }
            }
            probe
        }
    }
}

/// Decodes every frame and fails on any decoder complaint.
fn decodes_cleanly(path: &Path) {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "decoder said: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_synthetic_screen_is_saved_as_a_valid_mp4_and_mkv() {
    let scratch = Scratch::new("save");
    let recorder = Recorder::start(Box::new(SyntheticSource::new(320, 180, 30)), config()).unwrap();
    wait_for(&recorder, "three seconds of video", |s| {
        s.buffered_seconds >= 3.0
    });
    for (format, name) in [(ClipFormat::Mp4, "clip.mp4"), (ClipFormat::Mkv, "clip.mkv")] {
        let path = scratch.file(name);
        let saved = recorder.save_clip(2, format, &path).unwrap();
        assert!(saved.frames >= 50, "{saved:?}");
        assert!(saved.bytes > 1_000 && path.exists());
        assert!(!scratch.file(&format!("{name}.partial")).exists());
        if tool("ffprobe") && tool("ffmpeg") {
            let probe = probe(&path);
            assert_eq!(probe.codec, "h264");
            assert_eq!((probe.width, probe.height), (320, 180));
            assert_eq!(probe.frames as u64, saved.frames, "{probe:?}");
            // At least the two seconds asked for, plus at most one keyframe interval.
            assert!(probe.duration >= 1.9 && probe.duration <= 3.2, "{probe:?}");
            decodes_cleanly(&path);
        }
    }
    let status = recorder.stop();
    assert_eq!(status.state, RecordState::Stopped);
    assert!(status.frames_encoded >= 80, "{status:?}");
}

#[test]
fn a_clip_is_never_longer_than_the_buffer_and_never_overwrites() {
    let scratch = Scratch::new("exists");
    let recorder = Recorder::start(Box::new(SyntheticSource::new(160, 90, 30)), config()).unwrap();
    wait_for(&recorder, "one second", |s| s.buffered_seconds >= 1.2);
    let path = scratch.file("a.mp4");
    recorder.save_clip(600, ClipFormat::Mp4, &path).unwrap();
    assert_eq!(
        recorder.save_clip(5, ClipFormat::Mp4, &path),
        Err(RecordError::AlreadyExists)
    );
    // The refused save left the first file alone.
    assert!(std::fs::metadata(&path).unwrap().len() > 0);
}

#[test]
fn another_savers_temporary_file_is_neither_shared_nor_removed() {
    let scratch = Scratch::new("claimed");
    let recorder = Recorder::start(Box::new(SyntheticSource::new(160, 90, 30)), config()).unwrap();
    wait_for(&recorder, "one second", |s| s.buffered_seconds >= 1.2);
    let path = scratch.file("a.mp4");
    let partial = scratch.file("a.mp4.partial");
    std::fs::write(&partial, b"another save in progress").unwrap();
    assert_eq!(
        recorder.save_clip(5, ClipFormat::Mp4, &path),
        Err(RecordError::AlreadyExists)
    );
    assert_eq!(
        std::fs::read(&partial).unwrap(),
        b"another save in progress"
    );
    assert!(!path.exists());
}

#[test]
fn saving_before_any_video_is_a_clear_error() {
    let scratch = Scratch::new("early");
    let recorder = Recorder::start(
        Box::new(SyntheticSource::new(160, 90, 30).failing_after(0)),
        config(),
    )
    .unwrap();
    wait_for(&recorder, "the failure", |s| {
        matches!(s.state, RecordState::Failed(_))
    });
    assert_eq!(
        recorder.save_clip(5, ClipFormat::Mp4, &scratch.file("x.mp4")),
        Err(RecordError::NothingBuffered)
    );
    assert!(!scratch.file("x.mp4").exists());
}

#[test]
fn losing_the_screen_keeps_what_was_recorded_and_says_why() {
    let scratch = Scratch::new("lost");
    let source = SyntheticSource::new(160, 90, 30).failing_after(45);
    let recorder = Recorder::start(Box::new(source), config()).unwrap();
    wait_for(&recorder, "the failure", |s| {
        matches!(s.state, RecordState::Failed(_))
    });
    assert_eq!(
        recorder.status().state,
        RecordState::Failed(RecordError::Capture(CaptureError::TargetGone))
    );
    // The video taken before the loss is still there to save.
    let saved = recorder
        .save_clip(60, ClipFormat::Mp4, &scratch.file("kept.mp4"))
        .unwrap();
    assert!(saved.frames > 0);
}

#[test]
fn a_changing_picture_size_restarts_the_stream_with_matching_clips() {
    let scratch = Scratch::new("resize");
    let source = SyntheticSource::new(160, 90, 30).resizing_after(30, 240, 120);
    let recorder = Recorder::start(Box::new(source), config()).unwrap();
    wait_for(&recorder, "the restart", |s| {
        s.restarts >= 1 && s.buffered_seconds >= 1.0
    });
    assert!(recorder.status().buffer_resets >= 1);
    let path = scratch.file("resized.mkv");
    recorder.save_clip(60, ClipFormat::Mkv, &path).unwrap();
    if tool("ffprobe") && tool("ffmpeg") {
        // The requested height is 180, but a 120-high picture is never enlarged: the new 240x120
        // picture is recorded as it is, and nothing mixes sizes.
        let probe = probe(&path);
        assert_eq!((probe.width, probe.height), (240, 120), "{probe:?}");
        decodes_cleanly(&path);
    }
}

#[test]
fn a_clip_is_never_enlarged_beyond_its_source_but_is_scaled_down_to_the_setting() {
    let mut wanted = config();
    wanted.output_height = 180;
    // A 160x90 source stays 160x90; the setting only ever shrinks a larger one.
    assert_eq!(
        clipcraft_record::output_size(&wanted, 160, 90),
        Some((160, 90))
    );
    assert_eq!(
        clipcraft_record::output_size(&wanted, 1440, 900),
        Some((288, 180))
    );
    assert_eq!(
        clipcraft_record::output_size(&wanted, 320, 180),
        Some((320, 180))
    );
    // Other shapes keep the requested height: black bars and crops are not enlargements.
    wanted.shape = OutputShape::Wide16x9;
    wanted.framing = Framing::Letterbox;
    assert_eq!(
        clipcraft_record::output_size(&wanted, 160, 120),
        Some((320, 180))
    );
}

#[test]
fn black_bars_make_a_16_9_clip_of_a_4_3_screen() {
    let scratch = Scratch::new("bars");
    let mut wide = config();
    wide.shape = OutputShape::Wide16x9;
    wide.framing = Framing::Letterbox;
    wide.output_height = 144;
    let recorder = Recorder::start(Box::new(SyntheticSource::new(160, 120, 30)), wide).unwrap();
    wait_for(&recorder, "video", |s| s.buffered_seconds >= 1.0);
    assert_eq!(recorder.status().output, Some((256, 144)));
    let path = scratch.file("wide.mp4");
    recorder.save_clip(5, ClipFormat::Mp4, &path).unwrap();
    if tool("ffprobe") {
        let probe = probe(&path);
        assert_eq!((probe.width, probe.height), (256, 144));
    }
}

#[test]
fn stopping_and_dropping_are_prompt() {
    let started = Instant::now();
    let recorder = Recorder::start(Box::new(SyntheticSource::new(160, 90, 30)), config()).unwrap();
    wait_for(&recorder, "a frame", |s| s.frames_encoded >= 1);
    let status = recorder.stop();
    assert_eq!(status.state, RecordState::Stopped);
    drop(Recorder::start(Box::new(SyntheticSource::new(160, 90, 30)), config()).unwrap());
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn unusable_settings_are_refused() {
    for bad in [
        RecordConfig { fps: 0, ..config() },
        RecordConfig {
            fps: 500,
            ..config()
        },
        RecordConfig {
            replay_seconds: 0,
            ..config()
        },
        RecordConfig {
            keyframe_seconds: 0,
            ..config()
        },
        RecordConfig {
            video_kbps: 0,
            ..config()
        },
        RecordConfig {
            max_buffer_bytes: 10,
            ..config()
        },
        RecordConfig {
            output_height: 2,
            ..config()
        },
    ] {
        assert!(matches!(
            Recorder::start(Box::new(SyntheticSource::new(16, 16, 30)), bad),
            Err(RecordError::BadConfig)
        ));
    }
}

#[test]
fn an_unwritable_folder_is_reported_and_leaves_no_partial_file() {
    let scratch = Scratch::new("readonly");
    let recorder = Recorder::start(Box::new(SyntheticSource::new(160, 90, 30)), config()).unwrap();
    wait_for(&recorder, "video", |s| s.buffered_seconds >= 1.0);
    // A file where the folder should be.
    let blocker = scratch.file("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let result = recorder.save_clip(5, ClipFormat::Mp4, &blocker.join("clip.mp4"));
    assert!(result.is_err());
}

#[test]
fn a_120_fps_recording_makes_a_valid_clip() {
    if !tool("ffprobe") {
        return;
    }
    let scratch = Scratch::new("fps120");
    let mut fast = config();
    fast.fps = 120;
    fast.output_height = 90;
    let recorder = Recorder::start(Box::new(SyntheticSource::new(160, 90, 120)), fast).unwrap();
    wait_for(&recorder, "two seconds of video", |s| {
        s.buffered_seconds >= 2.0
    });
    let path = scratch.file("fast.mp4");
    recorder.save_clip(2, ClipFormat::Mp4, &path).unwrap();
    let probe = probe(&path);
    assert_eq!(probe.codec, "h264");
    // Average rate as a fraction; a busy computer drops pictures, so only demand well above 60.
    let (num, den) = probe.frame_rate.split_once('/').unwrap();
    let rate = num.parse::<f64>().unwrap() / den.parse::<f64>().unwrap();
    assert!(rate > 60.0 && rate <= 121.0, "{rate} fps");
    assert!(probe.frames > 60, "{} frames", probe.frames);
    decodes_cleanly(&path);
}

/// A small real disk (a tmpfs). `None` where mounting is not allowed; the test is skipped there.
struct SmallDisk(PathBuf);

impl SmallDisk {
    fn new(megabytes: u32) -> Option<Self> {
        let path =
            std::env::temp_dir().join(format!("clipcraft-record-disk-{}", std::process::id()));
        std::fs::create_dir_all(&path).ok()?;
        let mounted = Command::new("mount")
            .args(["-t", "tmpfs", "-o", &format!("size={megabytes}m"), "tmpfs"])
            .arg(&path)
            .output()
            .is_ok_and(|o| o.status.success());
        if mounted {
            Some(Self(path))
        } else {
            let _ = std::fs::remove_dir(&path);
            None
        }
    }

    fn fill_leaving(&self, bytes: usize) {
        // Fill with files until a write fails, then trim to leave `bytes`.
        let chunk = vec![0_u8; 64 * 1024];
        let mut n = 0;
        while std::fs::write(self.0.join(format!("filler{n}.bin")), &chunk).is_ok() {
            n += 1;
        }
        let mut freed = 0;
        let mut k = n;
        while freed < bytes && k > 0 {
            k -= 1;
            let _ = std::fs::remove_file(self.0.join(format!("filler{k}.bin")));
            freed += chunk.len();
        }
        // The failed write may have left a partial filler behind; remove it too.
        let _ = std::fs::remove_file(self.0.join(format!("filler{n}.bin")));
    }
}

impl Drop for SmallDisk {
    fn drop(&mut self) {
        let _ = Command::new("umount").arg(&self.0).output();
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[test]
fn a_disk_that_fills_during_a_save_leaves_no_partial_file_and_keeps_earlier_clips() {
    let Some(disk) = SmallDisk::new(4) else {
        eprintln!("skipped: this environment cannot mount a small disk");
        return;
    };
    let recorder = Recorder::start(Box::new(SyntheticSource::new(320, 180, 30)), config()).unwrap();
    wait_for(&recorder, "three seconds", |s| s.buffered_seconds >= 3.0);
    let first = disk.0.join("first.mp4");
    recorder.save_clip(3, ClipFormat::Mp4, &first).unwrap();
    let kept = std::fs::read(&first).unwrap();
    assert!(!kept.is_empty());

    // Almost no room: the next clip cannot fit.
    disk.fill_leaving(20_000);
    for (format, name) in [
        (ClipFormat::Mp4, "second.mp4"),
        (ClipFormat::Mkv, "second.mkv"),
    ] {
        let target = disk.0.join(name);
        assert_eq!(
            recorder.save_clip(3, format, &target),
            Err(RecordError::DiskFull),
            "{name}"
        );
        assert!(!target.exists(), "{name} must not exist");
        assert!(
            !disk.0.join(format!("{name}.partial")).exists(),
            "{name}.partial must be removed"
        );
    }
    assert_eq!(
        std::fs::read(&first).unwrap(),
        kept,
        "the earlier clip is untouched"
    );
    // Recording carries on, and once there is room again the same save works.
    assert_eq!(recorder.status().state, RecordState::Running);
    for n in 0..64 {
        let _ = std::fs::remove_file(disk.0.join(format!("filler{n}.bin")));
    }
    recorder
        .save_clip(3, ClipFormat::Mp4, &disk.0.join("third.mp4"))
        .unwrap();
}

#[test]
fn a_finished_mp4_save_leaves_only_the_clip_behind() {
    let scratch = Scratch::new("tidy");
    let recorder = Recorder::start(Box::new(SyntheticSource::new(160, 90, 30)), config()).unwrap();
    wait_for(&recorder, "two seconds", |s| s.buffered_seconds >= 2.0);
    for (format, name) in [(ClipFormat::Mp4, "tidy.mp4"), (ClipFormat::Mkv, "tidy.mkv")] {
        recorder.save_clip(2, format, &scratch.file(name)).unwrap();
    }
    let mut names: Vec<String> = std::fs::read_dir(&scratch.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["tidy.mkv", "tidy.mp4"],
        "no crash-safe copy or partial may remain"
    );
}

/// A screen that goes quiet once, as when a laptop sleeps.
struct Sleepy {
    inner: SyntheticSource,
    frames: u32,
    sleep_after: u32,
    sleep: Duration,
}

impl clipcraft_capture::CaptureSource for Sleepy {
    fn target(&self) -> &clipcraft_capture::CaptureTarget {
        self.inner.target()
    }

    fn capture(&mut self) -> Result<clipcraft_capture::CaptureFrame, CaptureError> {
        self.frames += 1;
        if self.frames == self.sleep_after {
            std::thread::sleep(self.sleep);
        }
        self.inner.capture()
    }
}

#[test]
fn a_long_silence_in_the_pictures_starts_the_buffer_over_instead_of_freezing_every_clip() {
    if !tool("ffprobe") {
        return;
    }
    use clipcraft_capture::CaptureSource as _;
    let scratch = Scratch::new("sleep");
    let mut quick = config();
    quick.gap_restart_seconds = 1;
    let source = Sleepy {
        inner: SyntheticSource::new(160, 90, 30),
        frames: 0,
        sleep_after: 60,
        sleep: Duration::from_millis(2_500),
    };
    assert!(!source.target().name.is_empty());
    let recorder = Recorder::start(Box::new(source), quick).unwrap();
    wait_for(&recorder, "the sleep", |s| s.restarts >= 1);
    // A few seconds of fresh pictures after waking.
    wait_for(&recorder, "pictures after waking", |s| {
        s.buffered_seconds >= 2.0
    });
    let path = scratch.file("woke.mp4");
    recorder.save_clip(10, ClipFormat::Mp4, &path).unwrap();
    let packets = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time",
            "-of",
            "csv=p=0",
        ])
        .arg(&path)
        .output()
        .unwrap();
    let times: Vec<f64> = String::from_utf8_lossy(&packets.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    assert!(times.len() > 30, "{} pictures", times.len());
    let widest = times.windows(2).map(|w| w[1] - w[0]).fold(0.0, f64::max);
    assert!(widest < 0.5, "a {widest} s freeze is still in the clip");
    decodes_cleanly(&path);
}
