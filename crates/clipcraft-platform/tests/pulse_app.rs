//! Records one application's stream from a real PulseAudio server. Skipped when no server answers
//! `pactl info`; to run it, start a server with a null sink and make it the default, as for
//! `pulse.rs`.
#![cfg(target_os = "linux")]

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::{Duration, Instant};

use clipcraft_record::AudioError;

fn tools_and_server() -> bool {
    let works = |program: &str, arguments: &[&str]| {
        Command::new(program)
            .args(arguments)
            .output()
            .is_ok_and(|output| output.status.success())
    };
    works("pactl", &["info"])
        && works("parec", &["--version"])
        && works("paplay", &["--version"])
        && works("ffmpeg", &["-version"])
}

#[test]
fn one_program_is_recorded_while_other_sound_is_left_out() {
    if !tools_and_server() {
        return;
    }
    let wav = std::env::temp_dir().join(format!("clipcraft-pulse-app-{}.wav", std::process::id()));
    let made = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1000:duration=2.5",
        ])
        .args(["-ar", "48000", "-ac", "2"])
        .arg(&wav)
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(made, "the tone was not made");

    // Opened before the program plays: it waits, and says so.
    let mut source = clipcraft_platform::open_app_audio("exe:paplay", Arc::new(AtomicU32::new(0)))
        .expect("the source opens");
    let mut other =
        clipcraft_platform::open_app_audio("exe:nothing-plays-this", Arc::new(AtomicU32::new(0)))
            .expect("a program that is not running still opens");
    let mut samples = Vec::new();
    assert!(
        matches!(source.read(&mut samples), Err(AudioError::DeviceLost(_))),
        "nothing is playing yet"
    );

    let path = wav.clone();
    let player = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        Command::new("paplay")
            .arg(&path)
            .output()
            .is_ok_and(|output| output.status.success())
    });
    let mut heard = Vec::new();
    let mut elsewhere = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline {
        if source.read(&mut samples).is_ok() {
            heard.append(&mut samples);
        } else {
            samples.clear();
            source.recover();
        }
        if other.read(&mut samples).is_ok() {
            elsewhere.append(&mut samples);
        } else {
            samples.clear();
            other.recover();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(player.join().unwrap_or(false), "paplay did not play");
    let _ = std::fs::remove_file(&wav);
    let peak = heard
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        peak > 0.05,
        "the program's sound was not recorded (peak {peak})"
    );
    assert!(elsewhere.is_empty(), "another program's row recorded sound");
    drop(source);
    drop(other);
}
