//! Records what a real PulseAudio server plays. Skipped when no server answers `pactl info`; to
//! run it, start a server with a null sink and make it the default.
#![cfg(target_os = "linux")]

use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Both tests play into the one sink, so they take turns.
static TURN: Mutex<()> = Mutex::new(());

fn server_running() -> bool {
    Command::new("pactl")
        .arg("info")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn the_monitor_hears_what_is_played_and_a_missing_server_is_an_error() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    if !server_running()
        || Command::new("ffmpeg").arg("-version").output().is_err()
        || Command::new("paplay").arg("--version").output().is_err()
    {
        // Without a server the open must fail with words, not hang or panic.
        if let Err(why) = clipcraft_platform::open_system_monitor("") {
            assert!(!why.is_empty());
        }
        return;
    }
    let mut source = clipcraft_platform::open_system_monitor("").expect("monitor opens");
    assert_eq!(source.format().sample_rate, 48_000);
    // Play a 1.5 s 1 kHz tone into the default sink.
    let wav = std::env::temp_dir().join(format!("clipcraft-pulse-{}.wav", std::process::id()));
    let made = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:duration=1.5",
        ])
        .args(["-ar", "48000", "-ac", "2"])
        .arg(&wav)
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(made);
    let path = wav.clone();
    let player = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        Command::new("paplay")
            .arg(&path)
            .output()
            .is_ok_and(|o| o.status.success())
    });
    let mut heard = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        source.read(&mut heard).expect("source stays alive");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        player.join().unwrap(),
        "paplay could not play into the sink"
    );
    let _ = std::fs::remove_file(&wav);
    let peak = heard.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.05, "the monitor heard nothing (peak {peak})");
    // Zero crossings of the left channel while loud: about 1000 per second of tone.
    let left: Vec<f32> = heard.iter().step_by(2).copied().collect();
    let loud: Vec<f32> = left.iter().copied().filter(|s| s.abs() > 0.02).collect();
    assert!(loud.len() > 48_000 / 2, "{} loud samples", loud.len());
    let crossings = left
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count();
    assert!(
        (1_200..=1_800).contains(&crossings),
        "{crossings} crossings"
    );
}

/// The whole path: a real sound server, the monitor, the recorder, a saved clip, and ffmpeg
/// decoding the clip to find the tone that was played.
#[test]
fn a_clip_of_what_the_computer_played_contains_it() {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    use clipcraft_capture::SyntheticSource;
    use clipcraft_capture::reframe::{Framing, OutputShape};
    use clipcraft_record::{
        AudioConfig, AudioControl, AudioInput, AudioSetup, ClipFormat, EncoderSpeed, RecordConfig,
        Recorder,
    };
    if !server_running()
        || Command::new("ffmpeg").arg("-version").output().is_err()
        || Command::new("paplay").arg("--version").output().is_err()
    {
        return;
    }
    let dir = std::env::temp_dir().join(format!("clipcraft-pulse-clip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("tone.wav");
    assert!(
        Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:duration=2"
            ])
            .args(["-ar", "48000", "-ac", "2"])
            .arg(&wav)
            .output()
            .unwrap()
            .status
            .success()
    );
    let source = clipcraft_platform::open_system_monitor("").unwrap();
    let control = AudioControl::default();
    control.set_gain_percent(200);
    let config = RecordConfig {
        output_height: 144,
        shape: OutputShape::Source,
        framing: Framing::Original,
        fps: 30,
        video_kbps: 1_000,
        constant_bitrate: false,
        speed: EncoderSpeed::Fast,
        keyframe_seconds: 1,
        replay_seconds: 10,
        max_buffer_bytes: 64 * 1024 * 1024,
        threads: 2,
        hardware: None,
    };
    let recorder = Recorder::start_with_audio(
        Box::new(SyntheticSource::new(320, 180, 30)),
        config,
        Some(AudioSetup {
            config: AudioConfig {
                sample_rate: 48_000,
                channels: 2,
                kbps: 128,
                tracks: vec!["Game and system".to_owned()],
            },
            inputs: vec![AudioInput::new(source, control, 0)],
        }),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(1_000));
    assert!(
        Command::new("paplay")
            .arg(&wav)
            .output()
            .unwrap()
            .status
            .success()
    );
    std::thread::sleep(Duration::from_millis(1_500));
    let clip = dir.join("clip.mp4");
    recorder.save_clip(10, ClipFormat::Mp4, &clip).unwrap();
    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&clip)
        .args(["-f", "f32le", "-ac", "1", "-ar", "48000", "-"])
        .output()
        .unwrap();
    assert!(decoded.status.success() && decoded.stderr.is_empty());
    let pcm: Vec<f32> = decoded
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    // Goertzel at 1 kHz over one-second windows: quiet before the tone, loud during it.
    let power = |window: &[f32]| -> f32 {
        let w = std::f32::consts::TAU * 1_000.0 / 48_000.0;
        let c = 2.0 * w.cos();
        let (mut a, mut b) = (0.0_f32, 0.0_f32);
        for s in window {
            let n = s + c * a - b;
            b = a;
            a = n;
        }
        2.0 * (a * a + b * b - c * a * b).max(0.0).sqrt() / window.len().max(1) as f32
    };
    let loudest = pcm
        .chunks(24_000)
        .filter(|w| w.len() == 24_000)
        .map(power)
        .fold(0.0_f32, f32::max);
    let quietest = pcm
        .chunks(24_000)
        .filter(|w| w.len() == 24_000)
        .map(power)
        .fold(f32::MAX, f32::min);
    assert!(
        loudest > 0.1,
        "the tone is missing from the clip ({loudest})"
    );
    assert!(quietest < 0.02, "the clip is never quiet ({quietest})");
    let _ = std::fs::remove_dir_all(&dir);
}
