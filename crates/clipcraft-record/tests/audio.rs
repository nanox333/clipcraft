//! End-to-end checks of recorded sound: tones go in through the recorder, clips come out as MP4
//! and MKV, and ffprobe and ffmpeg (outside judges, skipped when absent) decode them to measure
//! what is really in the file.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use clipcraft_capture::SyntheticSource;
use clipcraft_capture::reframe::{Framing, OutputShape};
use clipcraft_record::{
    AudioConfig, AudioControl, AudioFormat, AudioInput, AudioSetup, ClipFormat, EncoderSpeed,
    RecordConfig, RecordError, Recorder, ToneSource,
};

fn config() -> RecordConfig {
    RecordConfig {
        output_height: 144,
        shape: OutputShape::Source,
        framing: Framing::Original,
        fps: 30,
        video_kbps: 1_000,
        constant_bitrate: false,
        speed: EncoderSpeed::Fast,
        keyframe_seconds: 1,
        replay_seconds: 20,
        max_buffer_bytes: 64 * 1024 * 1024,
        threads: 2,
        hardware: None,
    }
}

fn layout(tracks: &[&str]) -> AudioConfig {
    AudioConfig {
        sample_rate: 48_000,
        channels: 2,
        kbps: 128,
        tracks: tracks.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn tone(rate: u32, channels: u16, hz: f32, amplitude: f32) -> ToneSource {
    ToneSource::new(
        AudioFormat {
            sample_rate: rate,
            channels,
        },
        hz,
        amplitude,
    )
}

fn input(source: ToneSource, track: usize) -> (AudioInput, AudioControl) {
    let control = AudioControl::default();
    (
        AudioInput::new(Box::new(source), control.clone(), track),
        control,
    )
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("clipcraft-audio-{}-{name}", std::process::id()));
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

fn record_for(setup: AudioSetup, seconds: f32) -> Recorder {
    let recorder = Recorder::start_with_audio(
        Box::new(SyntheticSource::new(320, 180, 30)),
        config(),
        Some(setup),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while recorder.status().buffered_seconds < seconds {
        assert!(Instant::now() < deadline, "{:?}", recorder.status());
        std::thread::sleep(Duration::from_millis(50));
    }
    recorder
}

/// `(codec, sample_rate, channels, start_time, duration)` of every audio stream.
fn audio_streams(path: &Path) -> Vec<(String, u32, u32, f64, f64)> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "a", "-show_entries"])
        .arg("stream=codec_name,sample_rate,channels,start_time,duration")
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split(',').collect();
            (
                f[0].to_owned(),
                f[1].parse().unwrap(),
                f[2].parse().unwrap(),
                f[3].parse().unwrap_or(0.0),
                f[4].parse().unwrap_or(0.0),
            )
        })
        .collect()
}

fn video_duration(path: &Path) -> f64 {
    // Pictures are 30 per second; Matroska has no stream duration, so count them.
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_packets",
            "-show_entries",
        ])
        .arg("stream=nb_read_packets")
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    let frames: f64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap_or(0.0);
    frames / 30.0
}

/// Decodes audio stream `index` to 48 kHz stereo floats.
fn decode(path: &Path, index: usize) -> Vec<f32> {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map",
            &format!("0:a:{index}"),
            "-f",
            "f32le",
            "-ac",
            "2",
            "-ar",
            "48000",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "decoder said: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// Power of `hz` in the left channel of interleaved stereo (Goertzel), as an amplitude.
fn amplitude_at(stereo: &[f32], hz: f32) -> f32 {
    let left: Vec<f32> = stereo.iter().step_by(2).copied().collect();
    let w = std::f32::consts::TAU * hz / 48_000.0;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0_f32, 0.0_f32);
    for sample in &left {
        let s = sample + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    2.0 * power.max(0.0).sqrt() / left.len().max(1) as f32
}

fn rms(stereo: &[f32]) -> f32 {
    (stereo.iter().map(|s| s * s).sum::<f32>() / stereo.len().max(1) as f32).sqrt()
}

#[test]
fn two_sources_on_separate_tracks_reach_both_file_formats_intact() {
    if !tools() {
        return;
    }
    let scratch = Scratch::new("separate");
    let (a, _) = input(tone(48_000, 2, 440.0, 0.5), 0);
    let (b, _) = input(tone(44_100, 1, 1_000.0, 0.25), 1);
    let recorder = record_for(
        AudioSetup {
            config: layout(&["Game", "Microphone"]),
            inputs: vec![a, b],
        },
        4.0,
    );
    for format in [ClipFormat::Mp4, ClipFormat::Mkv] {
        let path = scratch.file(&format!("clip.{}", format.extension()));
        let saved = recorder.save_clip(3, format, &path).unwrap();
        assert_eq!(saved.audio_tracks, 2);
        let streams = audio_streams(&path);
        assert_eq!(streams.len(), 2, "{format:?}");
        let video = video_duration(&path);
        for (codec, rate, channels, _, _) in &streams {
            assert_eq!(
                (codec.as_str(), *rate, *channels),
                ("aac", 48_000, 2),
                "{format:?}"
            );
        }
        let game = decode(&path, 0);
        let mic = decode(&path, 1);
        // Each track carries its own tone, at its own level (AAC loses a little).
        assert!(
            (amplitude_at(&game, 440.0) - 0.5).abs() < 0.06,
            "{format:?}"
        );
        assert!(
            (amplitude_at(&mic, 1_000.0) - 0.25).abs() < 0.04,
            "{format:?}"
        );
        assert!(amplitude_at(&game, 1_000.0) < 0.01);
        assert!(amplitude_at(&mic, 440.0) < 0.01);
        // The sound is as long as the picture, to within a frame or two.
        let seconds = game.len() as f64 / 2.0 / 48_000.0;
        assert!(
            (seconds - video).abs() < 0.08,
            "{format:?}: audio {seconds} video {video}"
        );
    }
}

#[test]
fn sources_on_one_track_are_mixed_and_mute_and_level_apply() {
    if !tools() {
        return;
    }
    let scratch = Scratch::new("mixed");
    let (a, _) = input(tone(48_000, 2, 440.0, 0.4), 0);
    let (b, mic) = input(tone(48_000, 2, 880.0, 0.4), 0);
    let recorder = record_for(
        AudioSetup {
            config: layout(&["Mix"]),
            inputs: vec![a, b],
        },
        3.0,
    );
    let path = scratch.file("mixed.mp4");
    recorder.save_clip(2, ClipFormat::Mp4, &path).unwrap();
    assert_eq!(audio_streams(&path).len(), 1);
    let pcm = decode(&path, 0);
    assert!((amplitude_at(&pcm, 440.0) - 0.4).abs() < 0.06);
    assert!((amplitude_at(&pcm, 880.0) - 0.4).abs() < 0.06);

    // Mute one source and halve nothing else: later sound has only the first tone. The buffer
    // still holds earlier mixed sound, so judge the end of the clip.
    mic.set_muted(true);
    std::thread::sleep(Duration::from_millis(1_500));
    let path = scratch.file("muted.mkv");
    recorder.save_clip(1, ClipFormat::Mkv, &path).unwrap();
    let pcm = decode(&path, 0);
    let tail = &pcm[pcm.len().saturating_sub(48_000 * 2 / 2)..];
    assert!(
        amplitude_at(tail, 880.0) < 0.02,
        "{}",
        amplitude_at(tail, 880.0)
    );
    assert!((amplitude_at(tail, 440.0) - 0.4).abs() < 0.08);

    mic.set_muted(false);
    mic.set_gain_percent(50);
    std::thread::sleep(Duration::from_millis(1_500));
    let path = scratch.file("quiet.mkv");
    recorder.save_clip(1, ClipFormat::Mkv, &path).unwrap();
    let pcm = decode(&path, 0);
    let tail = &pcm[pcm.len().saturating_sub(48_000 * 2 / 2)..];
    assert!(
        (amplitude_at(tail, 880.0) - 0.2).abs() < 0.05,
        "{}",
        amplitude_at(tail, 880.0)
    );
    let status = recorder.status();
    assert_eq!(status.audio.len(), 2);
    assert!(status.audio.iter().all(|s| s.level > 0.3 && !s.lost));
}

#[test]
fn a_device_clock_that_runs_fast_cannot_pull_the_sound_out_of_step() {
    if !tools() {
        return;
    }
    let scratch = Scratch::new("drift");
    // 1.5 % fast: over 6 s the device delivers 90 ms more than the clock allows.
    let mut fast = tone(48_000, 2, 440.0, 0.5);
    fast.clock_error_ppm = 15_000;
    let mut slow = tone(48_000, 2, 880.0, 0.5);
    slow.clock_error_ppm = -15_000;
    let (a, _) = input(fast, 0);
    let (b, _) = input(slow, 1);
    let recorder = record_for(
        AudioSetup {
            config: layout(&["Fast", "Slow"]),
            inputs: vec![a, b],
        },
        6.0,
    );
    let status = recorder.status();
    let path = scratch.file("drift.mp4");
    recorder.save_clip(5, ClipFormat::Mp4, &path).unwrap();
    let video = video_duration(&path);
    for index in 0..2 {
        let seconds = decode(&path, index).len() as f64 / 2.0 / 48_000.0;
        assert!(
            (seconds - video).abs() < 0.08,
            "track {index}: audio {seconds} video {video}"
        );
    }
    assert!(
        status.audio[0].dropped_ms > 0 || status.audio[1].silence_ms > 0,
        "{status:?}"
    );
}

#[test]
fn a_lost_microphone_keeps_the_clip_and_says_so() {
    if !tools() {
        return;
    }
    let scratch = Scratch::new("lost");
    let mut dying = tone(48_000, 2, 440.0, 0.5);
    dying.fail_after = Some(Duration::from_millis(1_500));
    let (a, _) = input(dying, 0);
    let (b, _) = input(tone(48_000, 2, 880.0, 0.5), 1);
    let recorder = record_for(
        AudioSetup {
            config: layout(&["Mic", "Game"]),
            inputs: vec![a, b],
        },
        4.0,
    );
    let status = recorder.status();
    assert!(status.audio[0].lost && !status.audio[1].lost, "{status:?}");
    let path = scratch.file("lost.mp4");
    recorder.save_clip(3, ClipFormat::Mp4, &path).unwrap();
    let mic = decode(&path, 0);
    let game = decode(&path, 1);
    assert_eq!(mic.len(), game.len());
    // The lost track is silent at the end; the other is unaffected.
    assert!(rms(&mic[mic.len() - 48_000..]) < 0.001);
    assert!(amplitude_at(&game, 880.0) > 0.4);
}

#[test]
fn a_recovering_device_resumes_after_a_pause() {
    if !tools() {
        return;
    }
    let scratch = Scratch::new("recover");
    let mut flaky = tone(48_000, 2, 440.0, 0.5);
    flaky.fail_after = Some(Duration::from_millis(500));
    flaky.recoverable = true;
    let (a, _) = input(flaky, 0);
    let recorder = record_for(
        AudioSetup {
            config: layout(&["Mic"]),
            inputs: vec![a],
        },
        5.0,
    );
    let path = scratch.file("recovered.mkv");
    recorder.save_clip(2, ClipFormat::Mkv, &path).unwrap();
    let pcm = decode(&path, 0);
    assert!(amplitude_at(&pcm[pcm.len() - 48_000..], 440.0) > 0.4);
    assert!(!recorder.status().audio[0].lost);
}

#[test]
fn clips_without_sound_still_work_and_bad_layouts_are_refused() {
    let scratch = Scratch::new("none");
    let recorder = Recorder::start(Box::new(SyntheticSource::new(320, 180, 30)), config()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while recorder.status().buffered_seconds < 1.5 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    let path = scratch.file("silent.mp4");
    let saved = recorder.save_clip(1, ClipFormat::Mp4, &path).unwrap();
    assert_eq!(saved.audio_tracks, 0);
    assert!(recorder.status().audio.is_empty());
    if tools() {
        assert!(audio_streams(&path).is_empty());
    }
    drop(recorder);

    let mut bad = layout(&["x"]);
    bad.sample_rate = 12_345;
    let result = Recorder::start_with_audio(
        Box::new(SyntheticSource::new(320, 180, 30)),
        config(),
        Some(AudioSetup {
            config: bad,
            inputs: Vec::new(),
        }),
    );
    assert!(matches!(result, Err(RecordError::AudioFailed)));
}

#[test]
fn sound_alone_is_enough_to_start_and_a_silent_source_makes_a_silent_track() {
    if !tools() {
        return;
    }
    let scratch = Scratch::new("quiet");
    let (a, _) = input(tone(48_000, 2, 440.0, 0.0), 0);
    let recorder = record_for(
        AudioSetup {
            config: layout(&["Quiet"]),
            inputs: vec![a],
        },
        2.5,
    );
    let path = scratch.file("quiet.mp4");
    recorder.save_clip(2, ClipFormat::Mp4, &path).unwrap();
    assert!(rms(&decode(&path, 0)) < 0.001);
}

/// Where the loudest 10 ms of a decoded stereo stream begins, in seconds.
fn click_time(stereo: &[f32]) -> f64 {
    let window = 480;
    let left: Vec<f32> = stereo.iter().step_by(2).copied().collect();
    let mut best = (0.0_f32, 0);
    for start in (0..left.len().saturating_sub(window)).step_by(48) {
        let energy: f32 = left[start..start + window].iter().map(|s| s * s).sum();
        if energy > best.0 {
            best = (energy, start);
        }
    }
    best.1 as f64 / 48_000.0
}

#[test]
fn sound_lines_up_with_the_pictures_in_both_formats() {
    if !tools() {
        return;
    }
    let scratch = Scratch::new("sync");
    for format in [ClipFormat::Mp4, ClipFormat::Mkv] {
        let mut clicker = tone(48_000, 2, 1_000.0, 0.0);
        clicker.click_at = Some(Duration::from_millis(2_500));
        let (a, _) = input(clicker, 0);
        let recorder = record_for(
            AudioSetup {
                config: layout(&["Click"]),
                inputs: vec![a],
            },
            4.5,
        );
        let path = scratch.file(&format!("sync.{}", format.extension()));
        let saved = recorder.save_clip(3, format, &path).unwrap();
        let expected = 2.5 - saved.start_frame as f64 / 30.0;
        assert!(expected > 0.0, "the click must be inside the clip");
        let found = click_time(&decode(&path, 0));
        assert!(
            (found - expected).abs() < 0.04,
            "{format:?}: click at {found:.3} s, expected {expected:.3} s"
        );
    }
}
