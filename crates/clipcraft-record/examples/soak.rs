//! A long-run check of the recorder: memory, processor time, dropped pictures, saved clips, and
//! what happens when the whole program is frozen for a while (as a suspended laptop is).
//!
//! `cargo run --release -p clipcraft-record --example soak -- --minutes 120`
//!
//! The first process starts a worker (itself, with `--worker`) that records a synthetic screen and
//! a tone, saves a clip every `--save-every` seconds, and prints one line of numbers every few
//! seconds. The first process reads those lines and the worker's memory from `/proc`, can freeze
//! the worker with `SIGSTOP` and wake it with `SIGCONT` (`--freeze-for`), asks ffprobe to judge
//! every clip, and exits non-zero when a budget is broken. Linux only for the numbers; the
//! recording itself runs anywhere.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clipcraft_capture::SyntheticSource;
use clipcraft_capture::reframe::{Framing, OutputShape};
use clipcraft_record::{
    AudioConfig, AudioControl, AudioFormat, AudioInput, AudioSetup, ClipFormat, EncoderSpeed,
    RecordConfig, RecordState, Recorder, ToneSource,
};

struct Options {
    minutes: f64,
    height: u32,
    fps: u32,
    replay: u32,
    save_every: u64,
    freeze_for: u64,
    out: PathBuf,
    worker: bool,
}

fn options() -> Options {
    let mut options = Options {
        minutes: 2.0,
        height: 720,
        fps: 30,
        replay: 60,
        save_every: 60,
        freeze_for: 0,
        out: std::env::temp_dir().join(format!("clipcraft-soak-{}", std::process::id())),
        worker: false,
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = || arguments.next().unwrap_or_default();
        match argument.as_str() {
            "--minutes" => options.minutes = value().parse().unwrap_or(2.0),
            "--height" => options.height = value().parse().unwrap_or(720),
            "--fps" => options.fps = value().parse().unwrap_or(30),
            "--replay" => options.replay = value().parse().unwrap_or(60),
            "--save-every" => options.save_every = value().parse().unwrap_or(60),
            "--freeze-for" => options.freeze_for = value().parse().unwrap_or(0),
            "--out" => options.out = PathBuf::from(value()),
            "--worker" => options.worker = true,
            _ => {}
        }
    }
    options
}

/// Resident memory in megabytes and processor time in seconds, from `/proc`.
fn usage(pid: u32) -> Option<(f64, f64)> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let kilobytes: f64 = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = stat.rsplit_once(')')?.1;
    let fields: Vec<&str> = after_name.split_whitespace().collect();
    // utime and stime are fields 14 and 15 of the line, 12 and 13 after the name.
    let ticks: f64 = fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
    Some((kilobytes / 1024.0, ticks / 100.0))
}

fn worker(options: &Options) {
    let config = RecordConfig {
        output_height: options.height,
        shape: OutputShape::Source,
        framing: Framing::Original,
        fps: options.fps,
        video_kbps: 5_000,
        constant_bitrate: false,
        speed: EncoderSpeed::Fast,
        keyframe_seconds: 2,
        replay_seconds: options.replay,
        max_buffer_bytes: 256 * 1024 * 1024,
        threads: 2,
        codec: clipcraft_record::StreamCodec::H264,
        hardware: None,
        gap_restart_seconds: 10,
    };
    let width = options.height * 16 / 9 / 2 * 2;
    let recorder = Recorder::start_with_audio(
        Box::new(SyntheticSource::new(width, options.height, options.fps)),
        config,
        Some(AudioSetup {
            config: AudioConfig {
                sample_rate: 48_000,
                channels: 2,
                kbps: 160,
                tracks: vec!["Audio".to_owned()],
            },
            inputs: vec![AudioInput::new(
                Box::new(ToneSource::new(
                    AudioFormat {
                        sample_rate: 48_000,
                        channels: 2,
                    },
                    440.0,
                    0.3,
                )),
                AudioControl::default(),
                0,
            )],
        }),
    )
    .expect("recorder starts");
    std::fs::create_dir_all(&options.out).expect("out folder");
    let started = Instant::now();
    let (mut last_line, mut last_save) = (Instant::now(), Instant::now());
    let mut saved = 0_u32;
    loop {
        std::thread::sleep(Duration::from_millis(200));
        let status = recorder.status();
        if last_line.elapsed() >= Duration::from_secs(5) {
            last_line = Instant::now();
            println!(
                "STAT t={:.0} captured={} encoded={} dropped={} buffered_mb={:.1} buffered_s={:.1} encode_ms={:.1} state={:?}",
                started.elapsed().as_secs_f64(),
                status.frames_captured,
                status.frames_encoded,
                status.frames_dropped,
                status.buffered_bytes as f64 / 1e6,
                status.buffered_seconds,
                status.average_encode_ms,
                status.state
            );
        }
        if last_save.elapsed() >= Duration::from_secs(options.save_every) {
            last_save = Instant::now();
            saved += 1;
            let path = options.out.join(format!("clip{saved}.mp4"));
            let began = Instant::now();
            match recorder.save_clip(options.replay, ClipFormat::Mp4, &path) {
                Ok(clip) => println!(
                    "CLIP {} frames={} save_ms={} bytes={}",
                    path.display(),
                    clip.frames,
                    began.elapsed().as_millis(),
                    clip.bytes
                ),
                Err(error) => println!("SAVEFAIL {error}"),
            }
        }
        if status.state != RecordState::Running {
            println!("FAILED {:?}", status.state);
            return;
        }
    }
}

/// Length of the picture and of the sound in a clip, in seconds (last packet's end).
fn clip_lengths(path: &Path) -> Option<(f64, f64)> {
    let end_of = |select: &str| -> Option<f64> {
        let output = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", select])
            .args([
                "-show_entries",
                "packet=pts_time,duration_time",
                "-of",
                "csv=p=0",
            ])
            .arg(path)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let mut end = 0.0_f64;
        let mut previous: Option<f64> = None;
        for line in text.lines() {
            let mut parts = line.split(',');
            let start: f64 = parts.next()?.trim().parse().ok()?;
            if select == "v:0" {
                if let Some(before) = previous.filter(|before| start - before > 1.0) {
                    eprintln!(
                        "  a {:.1} s freeze in the pictures at {before:.1} s",
                        start - before
                    );
                    end = f64::INFINITY;
                }
                previous = Some(start);
            }
            let length: f64 = parts
                .next()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0.0);
            end = end.max(start + length);
        }
        (end > 0.0).then_some(end)
    };
    Some((end_of("v:0")?, end_of("a:0")?))
}

fn main() {
    let options = options();
    if options.worker {
        worker(&options);
        return;
    }
    let total = Duration::from_secs_f64(options.minutes * 60.0);
    let mut command = Command::new(std::env::current_exe().expect("own path"));
    command
        .args([
            "--worker",
            "--height",
            &options.height.to_string(),
            "--fps",
            &options.fps.to_string(),
        ])
        .args([
            "--replay",
            &options.replay.to_string(),
            "--save-every",
            &options.save_every.to_string(),
        ])
        .args(["--out", &options.out.to_string_lossy()])
        .stdout(Stdio::piped());
    let mut child = command.spawn().expect("worker starts");
    let pid = child.id();
    let reader = BufReader::new(child.stdout.take().expect("pipe"));
    let (sender, receiver) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in reader.lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    let started = Instant::now();
    let freeze_at = total.mul_f64(0.4);
    let mut frozen = false;
    let (mut memory, mut problems, mut clips) = (
        Vec::<(f64, f64)>::new(),
        Vec::<String>::new(),
        Vec::<PathBuf>::new(),
    );
    let mut last_stat = String::new();
    println!(
        "soak: {} min at {}p{}, replay {} s, save every {} s, freeze for {} s",
        options.minutes,
        options.height,
        options.fps,
        options.replay,
        options.save_every,
        options.freeze_for
    );
    while started.elapsed() < total {
        if options.freeze_for > 0 && !frozen && started.elapsed() >= freeze_at {
            frozen = true;
            println!(
                "-- freezing the recorder for {} s (like a suspended laptop)",
                options.freeze_for
            );
            let _ = Command::new("kill")
                .args(["-STOP", &pid.to_string()])
                .status();
            std::thread::sleep(Duration::from_secs(options.freeze_for));
            let _ = Command::new("kill")
                .args(["-CONT", &pid.to_string()])
                .status();
            println!("-- awake again");
        }
        while let Ok(line) = receiver.try_recv() {
            if let Some(rest) = line.strip_prefix("STAT ") {
                last_stat = rest.to_owned();
                if let Some((rss, cpu)) = usage(pid) {
                    memory.push((started.elapsed().as_secs_f64(), rss));
                    println!(
                        "{:>6.0}s rss={rss:.0} MB cpu={cpu:.0} s  {rest}",
                        started.elapsed().as_secs_f64()
                    );
                }
            } else if let Some(rest) = line.strip_prefix("CLIP ") {
                if let Some(path) = rest.split(' ').next() {
                    clips.push(PathBuf::from(path));
                }
                println!("        {line}");
            } else if line.starts_with("SAVEFAIL") || line.starts_with("FAILED") {
                problems.push(line.clone());
                println!("        {line}");
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let _ = child.kill();
    let _ = child.wait();

    // Budgets.
    if let (Some(&(_, early)), Some(&(_, late))) = (memory.get(memory.len() / 5), memory.last()) {
        let growth = late - early;
        println!("memory: {early:.0} MB after warm-up, {late:.0} MB at the end ({growth:+.0} MB)");
        if growth > 64.0 {
            problems.push(format!(
                "memory grew by {growth:.0} MB after warm-up (budget 64 MB)"
            ));
        }
    }
    if let Some(dropped) = last_stat
        .split_whitespace()
        .find_map(|p| p.strip_prefix("dropped="))
    {
        let captured: f64 = last_stat
            .split_whitespace()
            .find_map(|p| p.strip_prefix("captured="))
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);
        let dropped: f64 = dropped.parse().unwrap_or(0.0);
        println!(
            "dropped pictures: {:.1} % ({dropped} of {captured})",
            dropped * 100.0 / captured.max(1.0)
        );
    }
    for clip in &clips {
        match clip_lengths(clip) {
            Some((video, audio)) => {
                println!(
                    "{}: picture {video:.2} s, sound {audio:.2} s",
                    clip.display()
                );
                if (video - audio).abs() > 0.15 {
                    problems.push(format!(
                        "{}: picture and sound differ by {:.2} s",
                        clip.display(),
                        (video - audio).abs()
                    ));
                }
            }
            None => problems.push(format!(
                "{}: ffprobe could not read the clip",
                clip.display()
            )),
        }
    }
    if clips.is_empty() {
        problems.push("no clip was saved".to_owned());
    }
    let _ = std::fs::remove_dir_all(&options.out);
    if problems.is_empty() {
        println!("SOAK OK");
    } else {
        for problem in &problems {
            println!("PROBLEM: {problem}");
        }
        std::process::exit(1);
    }
}
