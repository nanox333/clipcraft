//! A soak: the operations a long session repeats (save a clip, pause, resume, swap the source,
//! lighten the load, change quality) are cycled for a while, and memory and thread counts must
//! stay flat. Not part of the normal gate because it takes minutes:
//!
//! ```text
//! CLIPCRAFT_SOAK_SECONDS=300 cargo test -p clipcraft-record --test soak -- --ignored --nocapture
//! ```
//!
//! A short run (30 s) is also done by the gate.
#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use clipcraft_capture::SyntheticSource;
use clipcraft_capture::reframe::{Framing, OutputShape};
use clipcraft_record::{ClipFormat, EncoderSpeed, RecordConfig, RecordState, Recorder};

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

/// (resident memory in KiB, threads) of this process.
fn usage() -> (u64, u64) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let number = |key: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
    };
    (number("VmRSS:"), number("Threads:"))
}

fn soak(seconds: u64) {
    let folder = std::env::temp_dir().join(format!("clipcraft-soak-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("scratch folder");
    let recorder = Recorder::start(Box::new(SyntheticSource::new(320, 180, 30)), config())
        .expect("recording starts");
    recorder.adapt_to_load(true);
    recorder.keep_through_source_loss(true);
    let begin = Instant::now();
    let mut samples: Vec<(u64, u64, u64)> = Vec::new();
    let mut step = 0_u64;
    let mut saved = 0_u64;
    while begin.elapsed() < Duration::from_secs(seconds) {
        step += 1;
        std::thread::sleep(Duration::from_millis(500));
        if step.is_multiple_of(4) && recorder.status().buffered_seconds > 1.5 {
            let path = folder.join(format!("clip-{saved}.mp4"));
            if recorder.save_clip(5, ClipFormat::Mp4, &path).is_ok() {
                saved += 1;
            }
            let _ = std::fs::remove_file(path);
        }
        if step % 10 == 3 {
            recorder.set_paused(true);
        }
        if step % 10 == 4 {
            recorder.set_paused(false);
        }
        if step % 14 == 6 {
            recorder.replace_source(Box::new(SyntheticSource::new(320, 180, 30)));
        }
        if step % 22 == 9 {
            recorder.lighten_now();
        }
        if step.is_multiple_of(10) {
            let (memory, threads) = usage();
            samples.push((begin.elapsed().as_secs(), memory, threads));
        }
        let status = recorder.status();
        assert!(
            !matches!(status.state, RecordState::Failed(_)),
            "the recording failed during the soak: {status:?}"
        );
    }
    let status = recorder.stop();
    let _ = std::fs::remove_dir_all(&folder);
    assert!(saved > 0, "no clip was saved during the soak");
    println!("soak samples (seconds, KiB, threads): {samples:?}; {saved} clips saved");
    assert!(samples.len() >= 3, "too short to judge");
    // Compare the first sample after a warm-up with the last: the buffer is full by then.
    let early = samples[samples.len().min(2) - 1];
    let late = *samples.last().expect("samples");
    assert!(
        late.1 <= early.1 + early.1 / 4 + 32 * 1024,
        "memory grew from {} KiB to {} KiB",
        early.1,
        late.1
    );
    assert!(
        late.2 <= early.2 + 2,
        "threads grew from {} to {}",
        early.2,
        late.2
    );
    assert!(status.restarts < 50, "{status:?}");
}

#[test]
fn a_short_soak_keeps_memory_and_threads_flat() {
    soak(30);
}

#[test]
#[ignore = "takes minutes; set CLIPCRAFT_SOAK_SECONDS"]
fn a_long_soak_keeps_memory_and_threads_flat() {
    let seconds = std::env::var("CLIPCRAFT_SOAK_SECONDS")
        .ok()
        .and_then(|text| text.parse::<u64>().ok())
        .unwrap_or(300);
    soak(seconds);
}
