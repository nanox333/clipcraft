//! The hardware-encoder path, with a stand-in encoder (a real H.264 stream produced by the software
//! encoder from the RGBA pictures it is handed), so the recorder's handling of a GPU can be tested
//! without one. ffprobe and ffmpeg judge the files and are skipped when absent.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use clipcraft_capture::reframe::{Framing, OutputShape};
use clipcraft_capture::{
    CaptureError, CaptureFrame, CaptureSource, CaptureTarget, NativeCaptureFrame, SyntheticSource,
    TargetKind,
};
use clipcraft_record::{
    ClipFormat, EncoderSpeed, HardwareEncoder, HardwarePacket, HardwareProvider, HardwareRequest,
    RecordConfig, RecordState, Recorder,
};
use clipcraft_time::Tick;
use filmcraft_h264enc::{Encoder, EncoderConfig, PacketFormat, Profile, RateControl, YuvFrame};

/// How the stand-in misbehaves.
#[derive(Clone, Copy, Default)]
struct Behaviour {
    /// Return an error from `encode` after this many pictures.
    fail_after: Option<u64>,
    /// Mark pictures as reordered after this many.
    reorder_after: Option<u64>,
    /// Accept an opaque capture surface without asking it for a CPU readback.
    direct_surface: bool,
    /// Accept input indefinitely without returning packets.
    withhold_output: bool,
    fail_flush: bool,
}

struct Fake {
    encoder: Encoder,
    width: usize,
    height: usize,
    seen: u64,
    behaviour: Behaviour,
    native_calls: Arc<AtomicUsize>,
    /// The RGBA of the first picture, so tests can check what the recorder handed over.
    first_pixel: Option<[u8; 4]>,
}

impl HardwareEncoder for Fake {
    fn encode(&mut self, rgba: &[u8], index: u64) -> Result<Vec<HardwarePacket>, String> {
        if rgba.len() != self.width * self.height * 4 {
            return Err(format!(
                "expected {} bytes, got {}",
                self.width * self.height * 4,
                rgba.len()
            ));
        }
        self.first_pixel
            .get_or_insert([rgba[0], rgba[1], rgba[2], rgba[3]]);
        if self
            .behaviour
            .fail_after
            .is_some_and(|limit| self.seen >= limit)
        {
            return Err("boom".to_owned());
        }
        let reordered = self
            .behaviour
            .reorder_after
            .is_some_and(|limit| self.seen >= limit);
        self.seen += 1;
        if self.behaviour.withhold_output {
            return Ok(Vec::new());
        }
        let luma: Vec<u8> = rgba.chunks_exact(4).map(|pixel| pixel[1].max(16)).collect();
        let chroma = vec![128_u8; self.width * self.height / 4];
        let frame = YuvFrame {
            y: &luma,
            u: &chroma,
            v: &chroma,
            y_stride: self.width,
            uv_stride: self.width / 2,
        };
        let packets = self
            .encoder
            .try_encode(&frame, i64::try_from(index).unwrap_or(0))
            .map_err(|e| e.to_string())?;
        Ok(packets
            .into_iter()
            .map(|p| HardwarePacket {
                data: p.data,
                keyframe: p.keyframe,
                reordered,
            })
            .collect())
    }

    fn encode_surface(
        &mut self,
        frame: &mut dyn NativeCaptureFrame,
        index: u64,
        _fit: clipcraft_record::SurfaceFit,
    ) -> Result<Option<Vec<HardwarePacket>>, String> {
        if !self.behaviour.direct_surface {
            return Ok(None);
        }
        let Some(surface) = frame.as_any_mut().downcast_mut::<TestSurface>() else {
            return Ok(None);
        };
        self.native_calls.fetch_add(1, Ordering::Relaxed);
        let pixel = surface.pixel;
        let rgba = pixel.repeat(self.width * self.height);
        self.encode(&rgba, index).map(Some)
    }

    fn flush(&mut self) -> Result<Vec<HardwarePacket>, String> {
        if self.behaviour.fail_flush {
            return Err("device lost during drain".to_owned());
        }
        Ok(self
            .encoder
            .flush()
            .into_iter()
            .map(|p| HardwarePacket {
                data: p.data,
                keyframe: p.keyframe,
                reordered: false,
            })
            .collect())
    }

    fn avcc(&self) -> Vec<u8> {
        self.encoder.avcc()
    }

    fn name(&self) -> String {
        "Fake GPU".to_owned()
    }
}

fn provider(behaviour: Behaviour) -> HardwareProvider {
    provider_counted(behaviour, Arc::new(AtomicUsize::new(0)))
}

fn provider_counted(behaviour: Behaviour, native_calls: Arc<AtomicUsize>) -> HardwareProvider {
    HardwareProvider::new(move |request: &HardwareRequest| {
        let mut config = EncoderConfig::new(request.width, request.height, request.fps, 1);
        config.profile = Profile::Baseline;
        config.bframes = 0;
        config.keyint = request.keyframe_frames;
        config.rate = RateControl::Cbr {
            kbps: request.video_kbps,
        };
        config.format = PacketFormat::LengthPrefixed;
        config.threads = 1;
        let encoder = Encoder::new(config).map_err(|e| e.to_string())?;
        Ok(Box::new(Fake {
            encoder,
            width: request.width as usize,
            height: request.height as usize,
            seen: 0,
            behaviour,
            native_calls: native_calls.clone(),
            first_pixel: None,
        }) as Box<dyn HardwareEncoder>)
    })
}

struct TestSurface {
    width: u32,
    height: u32,
    pixel: [u8; 4],
    readbacks: Arc<AtomicUsize>,
}

impl NativeCaptureFrame for TestSurface {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn readback(&mut self) -> Result<CaptureFrame, CaptureError> {
        self.readbacks.fetch_add(1, Ordering::Relaxed);
        let stride = self.width as usize * 4;
        Ok(CaptureFrame {
            width: self.width,
            height: self.height,
            stride,
            bgra: self
                .pixel
                .repeat(self.width as usize * self.height as usize),
            captured_at: Tick::from_units(0, 1),
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

struct NativeSource {
    readbacks: Arc<AtomicUsize>,
    cpu_captures: Arc<AtomicUsize>,
}

impl CaptureSource for NativeSource {
    fn target(&self) -> &CaptureTarget {
        static TARGET: std::sync::OnceLock<CaptureTarget> = std::sync::OnceLock::new();
        TARGET.get_or_init(|| CaptureTarget {
            id: "test-native".to_owned(),
            name: "Native surface test".to_owned(),
            kind: TargetKind::Display,
            width: 320,
            height: 180,
        })
    }

    fn capture_surface(&mut self) -> Result<Option<Box<dyn NativeCaptureFrame>>, CaptureError> {
        Ok(Some(Box::new(TestSurface {
            width: 320,
            height: 180,
            pixel: [8, 40, 120, 255],
            readbacks: self.readbacks.clone(),
        })))
    }

    fn capture(&mut self) -> Result<CaptureFrame, CaptureError> {
        self.cpu_captures.fetch_add(1, Ordering::Relaxed);
        Err(CaptureError::Failed)
    }
}

fn config(hardware: Option<HardwareProvider>) -> RecordConfig {
    RecordConfig {
        output_height: 180,
        shape: OutputShape::Source,
        framing: Framing::Original,
        fps: 30,
        video_kbps: 2_000,
        constant_bitrate: true,
        speed: EncoderSpeed::Fast,
        keyframe_seconds: 1,
        replay_seconds: 10,
        max_buffer_bytes: 64 * 1024 * 1024,
        threads: 2,
        codec: clipcraft_record::StreamCodec::H264,
        hardware,
        gap_restart_seconds: 10,
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("clipcraft-hw-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
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

fn judged(path: &Path, width: u32, height: u32) {
    if Command::new("ffprobe").arg("-version").output().is_err() {
        return;
    }
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height",
            "-of",
            "default=nw=1",
        ])
        .arg(path)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&probe.stdout);
    assert!(text.contains("codec_name=h264"), "{text}");
    assert!(
        text.contains(&format!("width={width}")) && text.contains(&format!("height={height}")),
        "{text}"
    );
    let decode = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(
        decode.status.success() && decode.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&decode.stderr)
    );
}

#[test]
fn a_hardware_encoder_fills_the_buffer_and_saves_a_valid_clip() {
    let scratch = Scratch::new("ok");
    let recorder = Recorder::start(
        Box::new(SyntheticSource::new(320, 180, 30)),
        config(Some(provider(Behaviour::default()))),
    )
    .unwrap();
    wait_for(&recorder, "video", |s| s.buffered_seconds >= 2.0);
    assert_eq!(recorder.status().encoder, "GPU (Fake GPU)");
    for (format, name) in [(ClipFormat::Mp4, "a.mp4"), (ClipFormat::Mkv, "a.mkv")] {
        let path = scratch.0.join(name);
        let saved = recorder.save_clip(5, format, &path).unwrap();
        assert!(saved.frames >= 40);
        judged(&path, 320, 180);
    }
    assert_eq!(recorder.stop().state, RecordState::Stopped);
}

#[test]
fn a_compatible_encoder_receives_native_surfaces_without_cpu_readback() {
    let readbacks = Arc::new(AtomicUsize::new(0));
    let cpu_captures = Arc::new(AtomicUsize::new(0));
    let native_calls = Arc::new(AtomicUsize::new(0));
    let hardware = provider_counted(
        Behaviour {
            direct_surface: true,
            ..Behaviour::default()
        },
        native_calls.clone(),
    );
    let source = NativeSource {
        readbacks: readbacks.clone(),
        cpu_captures: cpu_captures.clone(),
    };
    let recorder = Recorder::start(Box::new(source), config(Some(hardware))).unwrap();
    wait_for(&recorder, "native-surface video", |status| {
        status.buffered_seconds >= 1.2
    });
    assert!(native_calls.load(Ordering::Relaxed) > 0);
    assert_eq!(readbacks.load(Ordering::Relaxed), 0);
    assert_eq!(cpu_captures.load(Ordering::Relaxed), 0);
    assert_eq!(recorder.stop().state, RecordState::Stopped);
}

#[test]
fn a_surface_smaller_than_the_source_is_scaled_by_the_encoder_without_cpu_readback() {
    let readbacks = Arc::new(AtomicUsize::new(0));
    let cpu_captures = Arc::new(AtomicUsize::new(0));
    let native_calls = Arc::new(AtomicUsize::new(0));
    let hardware = provider_counted(
        Behaviour {
            direct_surface: true,
            ..Behaviour::default()
        },
        native_calls.clone(),
    );
    let source = NativeSource {
        readbacks: readbacks.clone(),
        cpu_captures: cpu_captures.clone(),
    };
    // The 320x180 surface is encoded at 144 high: the card does the scaling.
    let mut scaled = config(Some(hardware));
    scaled.output_height = 144;
    let recorder = Recorder::start(Box::new(source), scaled).unwrap();
    wait_for(&recorder, "scaled native-surface video", |status| {
        status.buffered_seconds >= 1.2
    });
    assert_eq!(recorder.status().output, Some((256, 144)));
    assert!(native_calls.load(Ordering::Relaxed) > 0);
    assert_eq!(readbacks.load(Ordering::Relaxed), 0);
    assert_eq!(cpu_captures.load(Ordering::Relaxed), 0);
    assert_eq!(recorder.stop().state, RecordState::Stopped);
}

#[test]
fn an_encoder_without_native_surface_support_uses_the_cpu_readback_fallback() {
    let readbacks = Arc::new(AtomicUsize::new(0));
    let cpu_captures = Arc::new(AtomicUsize::new(0));
    let source = NativeSource {
        readbacks: readbacks.clone(),
        cpu_captures: cpu_captures.clone(),
    };
    let recorder = Recorder::start(
        Box::new(source),
        config(Some(provider(Behaviour::default()))),
    )
    .unwrap();
    wait_for(&recorder, "fallback video", |status| {
        status.buffered_seconds >= 1.2
    });
    assert!(readbacks.load(Ordering::Relaxed) > 0);
    assert_eq!(cpu_captures.load(Ordering::Relaxed), 0);
    assert_eq!(recorder.stop().state, RecordState::Stopped);
}

#[test]
fn the_hardware_encoder_is_handed_the_fitted_picture() {
    let scratch = Scratch::new("fit");
    let mut wide = config(Some(provider(Behaviour::default())));
    wide.shape = OutputShape::Wide16x9;
    wide.framing = Framing::Letterbox;
    wide.output_height = 144;
    let recorder = Recorder::start(Box::new(SyntheticSource::new(160, 120, 30)), wide).unwrap();
    wait_for(&recorder, "video", |s| s.buffered_seconds >= 1.2);
    assert_eq!(recorder.status().output, Some((256, 144)));
    let path = scratch.0.join("wide.mp4");
    recorder.save_clip(5, ClipFormat::Mp4, &path).unwrap();
    judged(&path, 256, 144);
}

#[test]
fn a_missing_gpu_means_software_with_the_reason_shown() {
    let scratch = Scratch::new("missing");
    let none = HardwareProvider::new(|_| Err("no NVIDIA graphics card was found".to_owned()));
    let recorder = Recorder::start(
        Box::new(SyntheticSource::new(320, 180, 30)),
        config(Some(none)),
    )
    .unwrap();
    wait_for(&recorder, "video", |s| s.buffered_seconds >= 1.5);
    assert_eq!(
        recorder.status().encoder,
        "CPU (GPU unavailable: no NVIDIA graphics card was found)"
    );
    let path = scratch.0.join("cpu.mp4");
    recorder.save_clip(5, ClipFormat::Mp4, &path).unwrap();
    judged(&path, 320, 180);
}

#[test]
fn a_gpu_that_comes_back_takes_the_encoding_over_again() {
    let scratch = Scratch::new("return");
    // The first card fails after 20 pictures; the second try finds a healthy one.
    let opened = Arc::new(AtomicUsize::new(0));
    let counter = opened.clone();
    let healthy = provider(Behaviour::default());
    let flaky = provider(Behaviour {
        fail_after: Some(20),
        ..Behaviour::default()
    });
    let recorder_provider = HardwareProvider::new(move |request: &HardwareRequest| {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            flaky.open(request)
        } else {
            healthy.open(request)
        }
    })
    .with_retry_after(Duration::from_millis(300));
    let recorder = Recorder::start(
        Box::new(SyntheticSource::new(320, 180, 30)),
        config(Some(recorder_provider)),
    )
    .unwrap();
    wait_for(&recorder, "the card to be used again", |s| {
        s.restarts >= 2 && s.encoder == "GPU (Fake GPU)" && s.buffered_seconds >= 1.5
    });
    assert!(opened.load(Ordering::SeqCst) >= 2);
    let path = scratch.0.join("back.mp4");
    recorder.save_clip(5, ClipFormat::Mp4, &path).unwrap();
    judged(&path, 320, 180);
}

#[test]
fn a_gpu_that_keeps_failing_is_asked_less_and_less_often() {
    let opened = Arc::new(AtomicUsize::new(0));
    let counter = opened.clone();
    let flaky = provider(Behaviour {
        fail_after: Some(5),
        ..Behaviour::default()
    });
    let hardware = HardwareProvider::new(move |request: &HardwareRequest| {
        counter.fetch_add(1, Ordering::SeqCst);
        flaky.open(request)
    })
    .with_retry_after(Duration::from_millis(100));
    let recorder = Recorder::start(
        Box::new(SyntheticSource::new(320, 180, 30)),
        config(Some(hardware)),
    )
    .unwrap();
    std::thread::sleep(Duration::from_secs(4));
    let status = recorder.status();
    // Waits of 200, 400, 800... ms after the failures: far fewer tries than one per 100 ms.
    assert!(
        opened.load(Ordering::SeqCst) <= 8,
        "{}",
        opened.load(Ordering::SeqCst)
    );
    assert!(matches!(status.state, RecordState::Running), "{status:?}");
    assert!(status.encoder.starts_with("CPU"), "{status:?}");
}

#[test]
fn a_gpu_that_fails_midway_hands_over_to_software() {
    let scratch = Scratch::new("fail");
    let flaky = provider(Behaviour {
        fail_after: Some(20),
        ..Behaviour::default()
    });
    let recorder = Recorder::start(
        Box::new(SyntheticSource::new(320, 180, 30)),
        config(Some(flaky)),
    )
    .unwrap();
    wait_for(&recorder, "the handover and a second of video", |s| {
        s.restarts >= 1 && s.buffered_seconds >= 1.5
    });
    let status = recorder.status();
    assert_eq!(status.encoder, "CPU (GPU stopped: boom)");
    assert!(matches!(status.state, RecordState::Running), "{status:?}");
    let path = scratch.0.join("after.mp4");
    recorder.save_clip(5, ClipFormat::Mp4, &path).unwrap();
    judged(&path, 320, 180);
}

#[test]
fn a_gpu_that_accepts_input_without_output_cannot_grow_pending_forever() {
    let recorder = Recorder::start(
        Box::new(SyntheticSource::new(320, 180, 30)),
        config(Some(provider(Behaviour {
            withhold_output: true,
            ..Behaviour::default()
        }))),
    )
    .unwrap();
    wait_for(&recorder, "bounded handover from a silent encoder", |s| {
        s.restarts >= 1 && s.buffered_seconds >= 1.0
    });
    let status = recorder.status();
    assert!(
        status.encoder.contains("pending-picture bound"),
        "{status:?}"
    );
    assert!(matches!(status.state, RecordState::Running));
    let scratch = Scratch::new("silent-gpu");
    let path = scratch.0.join("after.mp4");
    recorder.save_clip(5, ClipFormat::Mp4, &path).unwrap();
    judged(&path, 320, 180);
}

#[test]
fn a_failed_drain_is_reported_even_after_halt_and_preserves_saved_clips() {
    let scratch = Scratch::new("drain-failure");
    let recorder = Recorder::start(
        Box::new(SyntheticSource::new(320, 180, 30)),
        config(Some(provider(Behaviour {
            fail_flush: true,
            ..Behaviour::default()
        }))),
    )
    .unwrap();
    wait_for(&recorder, "video before drain failure", |s| {
        s.buffered_seconds >= 1.0
    });
    let path = scratch.0.join("preserved.mp4");
    recorder.save_clip(5, ClipFormat::Mp4, &path).unwrap();
    let before = std::fs::read(&path).unwrap();
    recorder.halt();
    let status = recorder.stop();
    assert_eq!(
        status.state,
        RecordState::Failed(clipcraft_record::RecordError::EncoderFailed)
    );
    assert!(status.encoder.contains("device lost during drain"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    judged(&path, 320, 180);
}

#[test]
fn a_gpu_that_reorders_pictures_is_not_used() {
    let scratch = Scratch::new("reorder");
    let reorders = provider(Behaviour {
        reorder_after: Some(10),
        ..Behaviour::default()
    });
    let recorder = Recorder::start(
        Box::new(SyntheticSource::new(320, 180, 30)),
        config(Some(reorders)),
    )
    .unwrap();
    wait_for(&recorder, "the handover and video", |s| {
        s.restarts >= 1 && s.buffered_seconds >= 1.5
    });
    assert_eq!(
        recorder.status().encoder,
        "CPU (GPU stopped: it reorders pictures)"
    );
    let path = scratch.0.join("r.mkv");
    recorder.save_clip(5, ClipFormat::Mkv, &path).unwrap();
    judged(&path, 320, 180);
}
