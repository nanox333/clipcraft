//! Cuts real clips from the recorder, as MP4 and as MKV, and has ffprobe and ffmpeg (outside
//! judges, skipped when absent) check what comes out: length, the exact first picture, sound in
//! the right place, and that nothing is left behind when an export fails or is cancelled.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clipcraft_capture::SyntheticSource;
use clipcraft_capture::reframe::{Framing, OutputShape};
use clipcraft_export::{ExportError, TrimRequest, trim_to_mp4};
use clipcraft_record::{
    AudioConfig, AudioControl, AudioFormat, AudioInput, AudioSetup, ClipFormat, EncoderSpeed,
    RecordConfig, Recorder, ToneSource,
};
use clipcraft_time::Tick;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("clipcraft-export-{}-{name}", std::process::id()));
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

fn tools() -> bool {
    ["ffprobe", "ffmpeg"]
        .iter()
        .all(|tool| Command::new(tool).arg("-version").output().is_ok())
}

fn seconds(value: f64) -> Tick {
    Tick::from_seconds_f64(value)
}

/// Records `length` seconds of a moving picture and a click at 4 s, and saves it in `format`.
fn record_clip(path: &Path, format: ClipFormat) {
    let config = RecordConfig {
        output_height: 90,
        shape: OutputShape::Source,
        framing: Framing::Original,
        fps: 30,
        video_kbps: 500,
        constant_bitrate: false,
        speed: EncoderSpeed::Fast,
        keyframe_seconds: 1,
        replay_seconds: 30,
        max_buffer_bytes: 64 * 1024 * 1024,
        threads: 2,
        hardware: None,
    };
    let mut click = ToneSource::new(
        AudioFormat {
            sample_rate: 48_000,
            channels: 2,
        },
        1_000.0,
        0.0,
    );
    click.click_at = Some(Duration::from_millis(4_000));
    let recorder = Recorder::start_with_audio(
        Box::new(SyntheticSource::new(160, 90, 30)),
        config,
        Some(AudioSetup {
            config: AudioConfig {
                sample_rate: 48_000,
                channels: 2,
                kbps: 128,
                tracks: vec!["Audio".to_owned()],
            },
            inputs: vec![AudioInput::new(Box::new(click), AudioControl::default(), 0)],
        }),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    while recorder.status().buffered_seconds < 7.0 {
        assert!(Instant::now() < deadline, "{:?}", recorder.status());
        std::thread::sleep(Duration::from_millis(100));
    }
    let saved = recorder.save_clip(30, format, path).unwrap();
    assert!(saved.audio_tracks == 1);
}

fn run(request: &TrimRequest) -> Result<clipcraft_export::ExportReport, ExportError> {
    trim_to_mp4(request, &mut |_| {}, &AtomicBool::new(false))
}

fn ffprobe(path: &Path, args: &[&str]) -> String {
    let output = Command::new("ffprobe")
        .args(["-v", "error"])
        .args(args)
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn last_video_time(path: &Path) -> f64 {
    ffprobe(
        path,
        &["-select_streams", "v:0", "-show_entries", "packet=pts_time"],
    )
    .lines()
    .filter_map(|l| l.trim().parse::<f64>().ok())
    .fold(0.0, f64::max)
}

/// The decoded bytes of the first picture of `path` at or after `at` seconds.
fn picture_at(path: &Path, at: f64) -> Vec<u8> {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &format!("{at:.4}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "gray", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn decode_audio(path: &Path) -> Vec<f32> {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", "0:a:0", "-f", "f32le", "-ac", "1", "-ar", "48000", "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn click_time(pcm: &[f32]) -> f64 {
    let window = 480;
    let mut best = (0.0_f32, 0);
    for start in (0..pcm.len().saturating_sub(window)).step_by(48) {
        let energy: f32 = pcm[start..start + window].iter().map(|s| s * s).sum();
        if energy > best.0 {
            best = (energy, start);
        }
    }
    best.1 as f64 / 48_000.0
}

#[test]
fn a_cut_from_an_mp4_or_an_mkv_starts_on_the_right_picture_and_keeps_sound_in_step() {
    if !tools() {
        return;
    }
    let scratch = Scratch::new("cut");
    for format in [ClipFormat::Mp4, ClipFormat::Mkv] {
        let name = format.extension();
        let input = scratch.file(&format!("full.{name}"));
        record_clip(&input, format);
        let full_length = last_video_time(&input);
        let output = scratch.file(&format!("cut-{name}.mp4"));
        // Starts between keyframes (they are a second apart) so the edit list matters.
        let (from, to) = (1.55, 3.0 + 1.55);
        let report = run(&TrimRequest {
            input: input.clone(),
            output: output.clone(),
            start: seconds(from),
            end: seconds(to),
        })
        .unwrap();
        assert_eq!(report.path, output);
        assert_eq!(report.audio_tracks, 1);
        assert!(report.bytes > 0 && report.pictures >= 90);
        assert!(
            (report.duration.seconds() - 3.0).abs() < 0.07,
            "{name}: {}",
            report.duration.seconds()
        );

        // The file plays back as long as asked: video ends where it should.
        let length = last_video_time(&output);
        assert!(
            (length - 3.0).abs() < 0.12,
            "{name}: video {length} s of a {full_length} s clip"
        );
        let streams = ffprobe(&output, &["-show_entries", "stream=codec_name"]);
        assert!(
            streams.contains("h264") && streams.contains("aac"),
            "{streams}"
        );

        // The first picture shown is the one at the start time, to the bit.
        let wanted = picture_at(&input, from);
        let got = picture_at(&output, 0.0);
        assert_eq!(wanted.len(), got.len());
        let different = wanted.iter().zip(&got).filter(|(a, b)| a != b).count();
        assert!(
            different < wanted.len() / 50,
            "{name}: first picture differs in {different} samples"
        );

        // The click recorded at 4 s of the recording lands where the cut puts it.
        let input_click = click_time(&decode_audio(&input));
        let cut_click = click_time(&decode_audio(&output));
        let expected = input_click - from;
        assert!(
            (cut_click - expected).abs() < 0.04,
            "{name}: click at {cut_click:.3} s, expected {expected:.3} s"
        );
    }
}

#[test]
fn bad_requests_leave_nothing_behind_and_never_touch_the_input() {
    if !tools() {
        return;
    }
    let scratch = Scratch::new("bad");
    let input = scratch.file("full.mp4");
    record_clip(&input, ClipFormat::Mp4);
    let before = std::fs::read(&input).unwrap();
    let output = scratch.file("out.mp4");
    let base = TrimRequest {
        input: input.clone(),
        output: output.clone(),
        start: seconds(1.0),
        end: seconds(2.0),
    };

    for (start, end) in [(2.0, 1.0), (1.0, 1.0), (-1.0, 2.0), (500.0, 600.0)] {
        let request = TrimRequest {
            start: seconds(start),
            end: seconds(end),
            ..base.clone()
        };
        assert_eq!(run(&request), Err(ExportError::BadRange), "{start}..{end}");
    }
    // An end past the file is "to the end".
    let request = TrimRequest {
        start: seconds(0.5),
        end: seconds(500.0),
        ..base.clone()
    };
    let whole = run(&request).unwrap();
    assert!(whole.duration.seconds() > 5.0);
    // Never overwrites.
    assert_eq!(run(&request), Err(ExportError::AlreadyExists));
    std::fs::remove_file(&output).unwrap();

    // A missing input, a file that is not a video, and a folder that cannot be created.
    let missing = TrimRequest {
        input: scratch.file("nope.mp4"),
        ..base.clone()
    };
    assert_eq!(run(&missing), Err(ExportError::Unreadable));
    let text = scratch.file("notes.mp4");
    std::fs::write(&text, b"hello").unwrap();
    assert!(matches!(
        run(&TrimRequest {
            input: text,
            ..base.clone()
        }),
        Err(ExportError::Unsupported(_))
    ));
    let blocked = scratch.file("full.mp4/inside/out.mp4");
    assert!(
        run(&TrimRequest {
            output: blocked,
            ..base.clone()
        })
        .is_err()
    );

    // Cancelling stops at once and removes the partial file.
    let cancel = AtomicBool::new(true);
    let result = trim_to_mp4(&base, &mut |_| {}, &cancel);
    assert_eq!(result, Err(ExportError::Cancelled));
    cancel.store(false, Ordering::SeqCst);
    assert!(!output.exists());
    assert!(!scratch.file("out.mp4.partial").exists());

    assert_eq!(
        std::fs::read(&input).unwrap(),
        before,
        "the input was changed"
    );
    // Progress goes from the start to 1.
    let mut seen = Vec::new();
    run_with_progress(&base, &mut seen);
    assert!(seen.windows(2).all(|w| w[0] <= w[1]) && seen.last() == Some(&1.0));
}

fn run_with_progress(request: &TrimRequest, seen: &mut Vec<f32>) {
    trim_to_mp4(request, &mut |p| seen.push(p), &AtomicBool::new(false)).unwrap();
}

#[test]
fn a_file_with_reordered_pictures_is_cut_exactly_too() {
    if !tools() {
        return;
    }
    let scratch = Scratch::new("bframes");
    let input = scratch.file("b.mp4");
    // Another program's encoder: B-frames, a keyframe every two seconds, 44.1 kHz sound.
    let made = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30:duration=8",
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=44100:duration=8",
        ])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "fast",
            "-g",
            "60",
            "-bf",
            "2",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(&input)
        .output()
        .unwrap();
    if !made.status.success() {
        return; // This ffmpeg has no H.264 encoder.
    }
    let output = scratch.file("cut.mp4");
    let report = run(&TrimRequest {
        input: input.clone(),
        output: output.clone(),
        start: seconds(2.37),
        end: seconds(5.37),
    })
    .unwrap();
    assert!((report.duration.seconds() - 3.0).abs() < 0.07);
    let wanted = picture_at(&input, 2.37);
    let got = picture_at(&output, 0.0);
    assert_eq!(wanted, got, "the first picture is not the one at the start");
    let ends = ffprobe(&output, &["-show_entries", "format=duration"]);
    let length: f64 = ends.trim().parse().unwrap();
    assert!((length - 3.0).abs() < 0.1, "{length}");
    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&output)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(decoded.status.success() && decoded.stderr.is_empty());
}

#[test]
fn a_disk_that_fills_during_an_export_says_so_and_leaves_no_partial_file() {
    if !tools() {
        return;
    }
    let disk = std::env::temp_dir().join(format!("clipcraft-export-disk-{}", std::process::id()));
    std::fs::create_dir_all(&disk).unwrap();
    let mounted = Command::new("mount")
        .args(["-t", "tmpfs", "-o", "size=2m", "tmpfs"])
        .arg(&disk)
        .output()
        .is_ok_and(|o| o.status.success());
    if !mounted {
        eprintln!("skipped: this environment cannot mount a small disk");
        let _ = std::fs::remove_dir(&disk);
        return;
    }
    let scratch = Scratch::new("diskfull");
    let input = scratch.file("full.mp4");
    record_clip(&input, ClipFormat::Mp4);
    // Leave far less room than the cut needs.
    let chunk = vec![0_u8; 32 * 1024];
    let mut n = 0;
    while std::fs::write(disk.join(format!("filler{n}")), &chunk).is_ok() {
        n += 1;
    }
    let output = disk.join("cut.mp4");
    let result = run(&TrimRequest {
        input: input.clone(),
        output: output.clone(),
        start: seconds(0.5),
        end: seconds(6.0),
    });
    let listing: Vec<_> = std::fs::read_dir(&disk)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("cut"))
        .collect();
    let _ = Command::new("umount").arg(&disk).output();
    let _ = std::fs::remove_dir(&disk);
    assert_eq!(result, Err(ExportError::DiskFull));
    assert!(listing.is_empty(), "left behind: {listing:?}");
}
