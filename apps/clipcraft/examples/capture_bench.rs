//! Generated-window and headless recorder diagnostics. Saves only generated synthetic media.
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use clipcraft_capture::reframe::{Framing, OutputShape};
use clipcraft_record::{
    AudioConfig, AudioControl, AudioFormat, AudioInput, AudioSetup, ClipFormat, EncoderSpeed,
    RecordConfig, RecordState, Recorder, ToneSource,
};
use eframe::egui;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Idle,
    Native,
    NativeSilent,
    Cpu,
    Synthetic,
    SyntheticSilent,
    Headless,
    HeadlessSilent,
}

impl Mode {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "idle" => Ok(Self::Idle),
            "record" | "native" => Ok(Self::Native),
            "native-silent" => Ok(Self::NativeSilent),
            "cpu" => Ok(Self::Cpu),
            "synthetic" => Ok(Self::Synthetic),
            "synthetic-silent" => Ok(Self::SyntheticSilent),
            "headless" => Ok(Self::Headless),
            "headless-silent" => Ok(Self::HeadlessSilent),
            _ => Err("unknown capture benchmark mode".to_owned()),
        }
    }

    fn synthetic(self) -> bool {
        matches!(
            self,
            Self::Synthetic | Self::SyntheticSilent | Self::Headless | Self::HeadlessSilent
        )
    }

    fn headless(self) -> bool {
        matches!(self, Self::Headless | Self::HeadlessSilent)
    }

    fn audio(self) -> bool {
        matches!(
            self,
            Self::Native | Self::Cpu | Self::Synthetic | Self::Headless
        )
    }
}

struct Workload {
    started: Instant,
    previous: Instant,
    times: Vec<f64>,
    seconds: u64,
    worker: Option<mpsc::Receiver<Result<String, String>>>,
    launched: bool,
    mode: Mode,
    output: PathBuf,
    done: bool,
    failed: Arc<AtomicBool>,
    fps: u32,
    system: sysinfo::System,
    sampled: Instant,
}

struct CpuInput(Box<dyn clipcraft_capture::CaptureSource>);

impl Drop for Workload {
    fn drop(&mut self) {
        if !self.done {
            self.failed.store(true, Ordering::Relaxed);
            eprintln!("FAILED benchmark closed before the requested run completed");
        }
    }
}
impl clipcraft_capture::CaptureSource for CpuInput {
    fn target(&self) -> &clipcraft_capture::CaptureTarget {
        self.0.target()
    }
    fn capture(
        &mut self,
    ) -> Result<clipcraft_capture::CaptureFrame, clipcraft_capture::CaptureError> {
        self.0.capture()
    }
}

fn record(seconds: u64, output: PathBuf, mode: Mode, fps: u32) -> Result<String, String> {
    let source: Box<dyn clipcraft_capture::CaptureSource> = if mode.synthetic() {
        Box::new(clipcraft_capture::SyntheticSource::new(1920, 1080, fps))
    } else {
        let target = clipcraft_platform::window_for_process(std::process::id())
            .map_err(|e| e.to_string())?
            .ok_or("generated window unavailable")?;
        clipcraft_platform::open(
            &target.id,
            &clipcraft_platform::CaptureOptions {
                show_cursor: false,
                hdr_to_sdr: false,
                requested_fps: fps,
            },
        )
        .map_err(|e| e.to_string())?
    };
    let source = if mode == Mode::Cpu {
        Box::new(CpuInput(source)) as Box<dyn clipcraft_capture::CaptureSource>
    } else {
        source
    };
    let initial_pipeline = clipcraft_platform::capture_pipeline_stats();
    let recorder = Recorder::start_with_audio(
        source,
        RecordConfig {
            output_height: 1080,
            shape: OutputShape::Source,
            framing: Framing::Original,
            fps,
            video_kbps: 12000,
            constant_bitrate: true,
            speed: EncoderSpeed::Fast,
            keyframe_seconds: 2,
            replay_seconds: 10,
            max_buffer_bytes: 32 * 1024 * 1024,
            threads: 2,
            hardware: clipcraft_platform::hardware_provider(),
            gap_restart_seconds: 10,
        },
        mode.audio().then(|| AudioSetup {
            config: AudioConfig {
                sample_rate: 48000,
                channels: 2,
                kbps: 128,
                tracks: vec!["Generated tone".to_owned()],
            },
            inputs: vec![AudioInput::new(
                Box::new(ToneSource::new(
                    AudioFormat {
                        sample_rate: 48000,
                        channels: 2,
                    },
                    440.0,
                    0.15,
                )),
                AudioControl::default(),
                0,
            )],
        }),
    )
    .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut next_save = Duration::from_secs(300);
    let mut previous_encoded = 0;
    let mut next_health = Duration::from_secs(10);
    let mut system = mode.headless().then(sysinfo::System::new);
    let mut sampled = Instant::now();
    while started.elapsed() < Duration::from_secs(seconds) {
        if sampled.elapsed() >= Duration::from_secs(1)
            && let Some(system) = &mut system
        {
            report_resources(system, started)?;
            sampled = Instant::now();
        }
        let status = recorder.status();
        if let RecordState::Failed(error) = status.state {
            return Err(error.to_string());
        }
        if status.buffered_bytes > 32 * 1024 * 1024 {
            return Err("replay exceeded 32 MiB budget".to_owned());
        }
        if started.elapsed() >= next_health {
            if status.frames_encoded == previous_encoded {
                return Err("encoder made no progress for ten seconds".to_owned());
            }
            previous_encoded = status.frames_encoded;
            println!(
                "health seconds={} encoded={} dropped={} replay_bytes={} restarts={}",
                started.elapsed().as_secs(),
                status.frames_encoded,
                status.frames_dropped,
                status.buffered_bytes,
                status.restarts
            );
            next_health += Duration::from_secs(10);
        }
        if started.elapsed() >= next_save {
            let checkpoint =
                output.with_extension(format!("checkpoint-{}.mp4", next_save.as_secs()));
            recorder
                .save_clip(5, ClipFormat::Mp4, &checkpoint)
                .map_err(|e| e.to_string())?;
            println!("checkpoint seconds={} saved=true", next_save.as_secs());
            next_save += Duration::from_secs(300);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Stop admitting work before the final disk write. Include in-flight output/drain below,
    // and compare against measured feed time rather than requested time.
    recorder.halt();
    let feed_elapsed = started.elapsed();
    let status = recorder.status();
    if status.frames_encoded == 0 {
        return Err("no frames encoded".to_owned());
    }
    recorder
        .save_clip(5, ClipFormat::Mp4, &output)
        .map_err(|e| e.to_string())?;
    let stopped = recorder.stop();
    if let RecordState::Failed(error) = stopped.state {
        return Err(error.to_string());
    }
    let pipeline = clipcraft_platform::capture_pipeline_stats();
    let expected = u64::try_from(feed_elapsed.as_nanos() * u128::from(fps) / 1_000_000_000)
        .map_err(|_| "measured frame count overflow")?;
    let returned = pipeline
        .mf_output_pictures
        .saturating_sub(initial_pipeline.mf_output_pictures);
    let fresh = pipeline
        .fresh_capture_reads
        .saturating_sub(initial_pipeline.fresh_capture_reads);
    let deficit = expected.saturating_sub(returned);
    let fresh_deficit = expected.saturating_sub(fresh);
    let mf = if stopped.encoder.contains("Media Foundation:") {
        format!(
            "mf_returned={returned} mf_missing={deficit} mf_output_fps={:.3}",
            returned as f64 / feed_elapsed.as_secs_f64()
        )
    } else {
        "mf_returned=n/a mf_missing=n/a mf_output_fps=n/a".to_owned()
    };
    let wgc = if !mode.synthetic() {
        format!(
            "wgc_fresh={fresh} wgc_fresh_missing={fresh_deficit} wgc_fresh_fps={:.3}",
            fresh as f64 / feed_elapsed.as_secs_f64()
        )
    } else {
        "wgc_fresh=n/a wgc_fresh_missing=n/a wgc_fresh_fps=n/a".to_owned()
    };
    println!(
        "cadence requested_seconds={seconds} feed_ms={:.3} expected={expected} {mf} {wgc} counters=post-stop",
        feed_elapsed.as_secs_f64() * 1000.0
    );
    Ok(format!(
        "encoder={} mode={mode:?} output={:?} captured={} encoded={} dropped={} encode_ms={:.3} pipeline={:?}",
        stopped.encoder,
        stopped.output,
        stopped.frames_captured,
        stopped.frames_encoded,
        stopped.frames_dropped,
        stopped.average_encode_ms,
        pipeline
    ))
}

fn report_resources(system: &mut sysinfo::System, started: Instant) -> Result<(), String> {
    let pid = sysinfo::Pid::from_u32(std::process::id());
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory(),
    );
    if let Some(process) = system.process(pid) {
        if process.memory() > 512 * 1024 * 1024 {
            return Err("process exceeded 512 MiB RSS budget".to_owned());
        }
        println!(
            "resource seconds={:.1} cpu_core_percent={:.2} rss_mib={:.2}",
            started.elapsed().as_secs_f64(),
            process.cpu_usage(),
            process.memory() as f64 / 1048576.0
        );
    }
    Ok(())
}

impl eframe::App for Workload {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        if self.done {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let now = Instant::now();
        if self.started.elapsed() > Duration::from_secs(3) && self.times.len() < 4_000_000 {
            self.times
                .push(now.duration_since(self.previous).as_secs_f64() * 1000.0);
        }
        self.previous = now;
        if self.sampled.elapsed() >= Duration::from_secs(1) {
            self.sampled = now;
            if let Err(error) = report_resources(&mut self.system, self.started) {
                self.failed.store(true, Ordering::Relaxed);
                self.done = true;
                eprintln!("FAILED {error}");
                context.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        let phase = self.started.elapsed().as_secs_f32();
        egui::CentralPanel::default().show(context, |ui| {
            let rect = ui.max_rect();
            ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
            for index in 0..2048 {
                let x = ((index % 64) as f32 / 64.0 + phase * 0.05).fract();
                let y = (index / 64) as f32 / 32.0;
                let color = egui::Color32::from_rgb(
                    (index % 251) as u8,
                    ((index * 7) % 251) as u8,
                    ((index * 13) % 251) as u8,
                );
                let position = rect.min + egui::vec2(x * rect.width(), y * rect.height());
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        position,
                        egui::vec2(rect.width() / 80.0, rect.height() / 40.0),
                    ),
                    0.0,
                    color,
                );
            }
            ui.label("ClipCraft generated capture benchmark — original synthetic media");
        });
        if !self.launched && self.started.elapsed() > Duration::from_secs(3) {
            self.launched = true;
            if self.mode != Mode::Idle {
                let (send, receive) = mpsc::channel();
                let seconds = self.seconds;
                let output = self.output.clone();
                let mode = self.mode;
                let fps = self.fps;
                match std::thread::Builder::new()
                    .name("capture-benchmark".to_owned())
                    .spawn(move || {
                        let _ = send.send(record(seconds, output, mode, fps));
                    }) {
                    Ok(_) => self.worker = Some(receive),
                    Err(error) => {
                        self.failed.store(true, Ordering::Relaxed);
                        self.done = true;
                        eprintln!("benchmark worker: {error}");
                        context.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        let done = if let Some(receive) = &self.worker {
            match receive.try_recv() {
                Ok(result) => {
                    if result.is_err() {
                        self.failed.store(true, Ordering::Relaxed);
                    }
                    println!(
                        "{}",
                        result.unwrap_or_else(|error| format!("FAILED {error}"))
                    );
                    true
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.failed.store(true, Ordering::Relaxed);
                    eprintln!("benchmark worker ended without a result");
                    true
                }
                Err(mpsc::TryRecvError::Empty) => false,
            }
        } else {
            self.started.elapsed() > Duration::from_secs(self.seconds + 3)
        };
        if done {
            self.done = true;
            self.times.sort_by(f64::total_cmp);
            let percentile = |n: usize| {
                self.times
                    .get(self.times.len().saturating_sub(1) * n / 100)
                    .copied()
                    .unwrap_or(0.0)
            };
            println!(
                "workload_frames={} frame_ms_p50={:.3} p95={:.3} p99={:.3}",
                self.times.len(),
                percentile(50),
                percentile(95),
                percentile(99)
            );
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        context.request_repaint();
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() > 4 {
        return Err(
            "usage: capture_bench [idle|native|native-silent|cpu|synthetic|synthetic-silent|headless|headless-silent] [seconds:5..7200] [output.mp4] [fps:30|60]"
                .into(),
        );
    }
    let mode = Mode::parse(args.first().map(String::as_str).unwrap_or("idle"))?;
    let seconds = args
        .get(1)
        .map(|s| s.parse::<u64>())
        .transpose()?
        .unwrap_or(15);
    if !(5..=7200).contains(&seconds) {
        return Err("duration must be 5..7200 seconds".into());
    }
    let fps = args
        .get(3)
        .map(|s| s.parse::<u32>())
        .transpose()?
        .unwrap_or(60);
    if !matches!(fps, 30 | 60) {
        return Err("fps must be 30 or 60".into());
    }
    let output = PathBuf::from(
        args.get(2)
            .map(String::as_str)
            .unwrap_or("target/capture-bench.mp4"),
    );
    if mode.headless() {
        println!("{}", record(seconds, output, mode, fps)?);
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1920.0, 1080.0])
            .with_decorations(false)
            .with_resizable(false),
        ..Default::default()
    };
    let failed = Arc::new(AtomicBool::new(false));
    let worker_failed = Arc::clone(&failed);
    eframe::run_native(
        "ClipCraft Generated Benchmark",
        options,
        Box::new(move |_creation| {
            Ok(Box::new(Workload {
                started: Instant::now(),
                previous: Instant::now(),
                times: Vec::new(),
                seconds,
                worker: None,
                launched: false,
                mode,
                output,
                done: false,
                failed: worker_failed,
                fps,
                system: sysinfo::System::new(),
                sampled: Instant::now(),
            }))
        }),
    )
    .map_err(|error| error.to_string())?;
    if failed.load(Ordering::Relaxed) {
        return Err("capture benchmark failed".into());
    }
    Ok(())
}
