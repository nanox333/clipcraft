//! Size estimates and hardware failure recovery for assembled editor sequences.
mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use clipcraft_export::{
    ExportError, ExportPreset, FastEncoder, FastOpener, FastPacket, FastRequest, SequenceRequest,
    SequenceSegment, sequence_canvas, sequence_to_mp4_with,
};
use clipcraft_record::ClipFormat;
use clipcraft_time::Tick;
use filmcraft_h264enc::{Encoder, EncoderConfig, PacketFormat, Preset, Profile, YuvFrame};
use filmcraft_media::FrameRequest;
use support::{Scratch, fixture, source};

#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    Refuses,
    DeviceLost,
    Missing,
    NoDescription,
    BadPacket,
}
struct Stand {
    encoder: Encoder,
    request: FastRequest,
    fault: Fault,
    seen: u64,
    dropped: bool,
}
impl FastEncoder for Stand {
    fn encode(&mut self, rgba: &[u8], index: u64) -> Result<Vec<FastPacket>, String> {
        if self.fault == Fault::DeviceLost && index == 3 {
            return Err("device lost".into());
        }
        assert_eq!(
            rgba.len(),
            self.request.width as usize * self.request.height as usize * 4
        );
        self.seen += 1;
        let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
        filmcraft_export::rgba_to_yuv420_8(
            rgba,
            self.request.width as usize,
            self.request.height as usize,
            &mut y,
            &mut u,
            &mut v,
        );
        let packets = self
            .encoder
            .encode(
                &YuvFrame {
                    y: &y,
                    u: &u,
                    v: &v,
                    y_stride: self.request.width as usize,
                    uv_stride: self.request.width as usize / 2,
                },
                index as i64,
            )
            .map_err(|error| error.to_string())?;
        Ok(self.packets(packets))
    }
    fn flush(&mut self) -> Result<Vec<FastPacket>, String> {
        let packets = self.encoder.flush();
        Ok(self.packets(packets))
    }
    fn avcc(&self) -> Vec<u8> {
        if self.fault == Fault::NoDescription || self.seen == 0 {
            Vec::new()
        } else {
            self.encoder.avcc()
        }
    }
    fn name(&self) -> String {
        "Test GPU".into()
    }
}
impl Stand {
    fn packets(&mut self, packets: Vec<filmcraft_h264enc::Packet>) -> Vec<FastPacket> {
        packets
            .into_iter()
            .filter_map(|packet| {
                if self.fault == Fault::Missing && !self.dropped {
                    self.dropped = true;
                    return None;
                }
                Some(FastPacket {
                    data: if self.fault == Fault::BadPacket {
                        vec![0, 0, 0, 20, 0]
                    } else {
                        packet.data
                    },
                    keyframe: packet.keyframe,
                })
            })
            .collect()
    }
}
fn opener(fault: Fault, attempts: Arc<AtomicUsize>) -> FastOpener {
    Arc::new(move |request| {
        attempts.fetch_add(1, Ordering::Relaxed);
        if fault == Fault::Refuses {
            return Err("unavailable".into());
        }
        let mut config = EncoderConfig::new(request.width, request.height, request.fps, 1);
        config.profile = Profile::Baseline;
        config.preset = Preset::Speed;
        config.bframes = 0;
        config.threads = 2;
        config.format = PacketFormat::LengthPrefixed;
        let encoder = Encoder::new(config).map_err(|error| error.to_string())?;
        Ok(Box::new(Stand {
            encoder,
            request: *request,
            fault,
            seen: 0,
            dropped: false,
        }) as Box<dyn FastEncoder>)
    })
}
fn request(
    input: &std::path::Path,
    output: std::path::PathBuf,
    preset: Option<ExportPreset>,
) -> SequenceRequest {
    SequenceRequest {
        quality: Default::default(),
        segments: vec![SequenceSegment {
            input: input.to_path_buf(),
            start: Tick::ZERO,
            end: Tick::from_units(1, 1),
            volume_percent: 100,
            look: Default::default(),
        }],
        texts: Vec::new(),
        preset,
        output,
    }
}
fn decode(request: &SequenceRequest) {
    let decoded = source(&request.output);
    assert_eq!(decoded.info().duration, Tick::from_units(1, 1));
    let picture = decoded
        .video_frame(FrameRequest::full(Tick::from_units(1, 2)))
        .unwrap();
    assert_eq!((picture.width, picture.height), (160, 90));
    assert!(decoded.audio_stream(0, 0, 48_000, 48_000).is_ok());
    if let Ok(result) = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&request.output)
        .args(["-f", "null", "-"])
        .output()
    {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn sequence_estimates_have_bounded_canvas_and_honest_reports() {
    assert_eq!(
        sequence_canvas(1920, 1080, None, Tick::from_units(60, 1)).unwrap(),
        (1280, 720)
    );
    assert_eq!(
        sequence_canvas(
            1920,
            1080,
            Some(ExportPreset::Small),
            Tick::from_units(60, 1)
        )
        .unwrap(),
        (960, 540)
    );
    assert_eq!(
        sequence_canvas(
            1920,
            1080,
            Some(ExportPreset::Small),
            Tick::from_units(120, 1)
        )
        .unwrap(),
        (852, 480)
    );
    assert_eq!(
        sequence_canvas(
            1920,
            1080,
            Some(ExportPreset::Small),
            Tick::from_units(180, 1)
        )
        .unwrap(),
        (640, 360)
    );
    assert!(matches!(
        sequence_canvas(
            1920,
            1080,
            Some(ExportPreset::Small),
            Tick::from_units(1800, 1)
        ),
        Err(ExportError::TargetTooSmall)
    ));
    assert!(
        sequence_canvas(
            160,
            90,
            Some(ExportPreset::Custom { megabytes: 0 }),
            Tick::from_units(1, 1)
        )
        .is_err()
    );
    assert!(
        sequence_canvas(
            160,
            90,
            Some(ExportPreset::Custom { megabytes: 501 }),
            Tick::from_units(1, 1)
        )
        .is_err()
    );
    let dir = Scratch::new("sequence-presets");
    let input = dir.file("in.mp4");
    fixture(&input, ClipFormat::Mp4, true);
    let before = std::fs::read(&input).unwrap();
    for (index, preset) in [
        None,
        Some(ExportPreset::Small),
        Some(ExportPreset::Medium),
        Some(ExportPreset::Custom { megabytes: 1 }),
    ]
    .into_iter()
    .enumerate()
    {
        let req = request(&input, dir.file(&format!("{index}.mp4")), preset);
        let report =
            sequence_to_mp4_with(&req, None, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            report.target_bytes,
            preset.map(|preset| preset.target_bytes().unwrap())
        );
        assert_eq!(
            report.within_target,
            report
                .target_bytes
                .is_none_or(|target| report.clip.bytes <= target)
        );
        assert_eq!((report.width, report.height), (160, 90));
        assert_eq!(report.encoder, "Software");
        assert!(!report.hardware_decode);
        assert_eq!(report.clip.pictures, 30);
        decode(&req);
    }
    assert_eq!(std::fs::read(&input).unwrap(), before);
}

#[test]
fn sequence_gpu_success_and_failures_keep_valid_outputs_and_sources() {
    let dir = Scratch::new("sequence-gpu");
    let input = dir.file("in.mp4");
    fixture(&input, ClipFormat::Mp4, true);
    let before = std::fs::read(&input).unwrap();
    for (index, fault) in [
        Fault::None,
        Fault::Refuses,
        Fault::DeviceLost,
        Fault::Missing,
        Fault::NoDescription,
        Fault::BadPacket,
    ]
    .into_iter()
    .enumerate()
    {
        let attempts = Arc::new(AtomicUsize::new(0));
        let gpu = opener(fault, attempts.clone());
        let req = request(
            &input,
            dir.file(&format!("gpu{index}.mp4")),
            Some(ExportPreset::Small),
        );
        let mut progress = Vec::new();
        let report = sequence_to_mp4_with(
            &req,
            Some(&gpu),
            &mut |value| progress.push(value),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        assert_eq!(
            report.encoder,
            if fault == Fault::None {
                "Test GPU"
            } else {
                "Software"
            }
        );
        assert_eq!(report.clip.pictures, 30);
        assert!(progress.windows(2).all(|values| values[0] <= values[1]));
        assert_eq!(progress.last(), Some(&1.0));
        decode(&req);
        assert!(!req.output.with_extension("mp4.partial").exists());
    }
    assert_eq!(std::fs::read(&input).unwrap(), before);
}

#[test]
fn sequence_presets_cancel_and_existing_destinations_never_retry_or_replace() {
    let dir = Scratch::new("sequence-presets-cancel");
    let input = dir.file("in.mp4");
    fixture(&input, ClipFormat::Mp4, true);
    let attempts = Arc::new(AtomicUsize::new(0));
    let gpu = opener(Fault::None, attempts.clone());
    let req = request(&input, dir.file("out.mp4"), Some(ExportPreset::Small));
    let cancel = AtomicBool::new(false);
    assert_eq!(
        sequence_to_mp4_with(
            &req,
            Some(&gpu),
            &mut |value| {
                if value > 0.1 {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
            &cancel
        )
        .unwrap_err(),
        ExportError::Cancelled
    );
    assert!(!req.output.exists());
    assert!(!req.output.with_extension("mp4.partial").exists());
    assert_eq!(attempts.load(Ordering::Relaxed), 1);
    std::fs::write(&req.output, b"keep").unwrap();
    assert_eq!(
        sequence_to_mp4_with(&req, Some(&gpu), &mut |_| {}, &AtomicBool::new(false)).unwrap_err(),
        ExportError::AlreadyExists
    );
    assert_eq!(std::fs::read(&req.output).unwrap(), b"keep");
    assert_eq!(attempts.load(Ordering::Relaxed), 1);
}

#[test]
fn a_small_sequence_budget_reduces_the_actual_encoded_canvas() {
    let dir = Scratch::new("sequence-preset-resize");
    let input = dir.file("large.mp4");
    // One generated source picture can be repeated in seven cuts without creating a large corpus.
    let mut config = EncoderConfig::new(1280, 720, 1, 1);
    config.profile = Profile::Baseline;
    config.preset = Preset::Speed;
    config.bframes = 0;
    config.threads = 2;
    config.format = PacketFormat::LengthPrefixed;
    let mut encoder = Encoder::new(config).unwrap();
    let info = clipcraft_record::VideoInfo {
        width: 1280,
        height: 720,
        fps: 1,
        avcc: encoder.avcc(),
    };
    let y = vec![100; 1280 * 720];
    let uv = vec![128; 640 * 360];
    let mut packets = encoder
        .encode(
            &YuvFrame {
                y: &y,
                u: &uv,
                v: &uv,
                y_stride: 1280,
                uv_stride: 640,
            },
            0,
        )
        .unwrap();
    packets.extend(encoder.flush());
    let samples: Vec<_> = packets
        .into_iter()
        .map(|packet| clipcraft_record::replay::EncodedSample {
            pts: packet.pts,
            data: packet.data.into(),
            keyframe: packet.keyframe,
        })
        .collect();
    clipcraft_record::write_clip(&samples, &info, &[], ClipFormat::Mp4, &input).unwrap();
    let mut req = request(
        &input,
        dir.file("small.mp4"),
        Some(ExportPreset::Custom { megabytes: 1 }),
    );
    req.segments = vec![req.segments[0].clone(); 7];
    let report = sequence_to_mp4_with(&req, None, &mut |_| {}, &AtomicBool::new(false)).unwrap();
    assert_eq!((report.width, report.height), (852, 480));
    assert_eq!(report.clip.duration, Tick::from_units(7, 1));
    assert_eq!(report.clip.pictures, 210);
    assert_eq!(report.target_bytes, Some(1_000_000));
    assert_eq!(report.within_target, report.clip.bytes <= 1_000_000);
    let decoded = source(&req.output);
    let picture = decoded
        .video_frame(FrameRequest::full(Tick::from_units(13, 2)))
        .unwrap();
    assert_eq!((picture.width, picture.height), (852, 480));
    assert_eq!(decoded.info().duration, Tick::from_units(7, 1));
}

#[test]
fn sixty_fps_reaches_hardware_and_survives_device_failure_without_changing_timing() {
    use clipcraft_export::{SequenceQuality, SequenceResolution};
    let dir = Scratch::new("sequence-quality-hardware");
    let input = dir.file("in.mp4");
    fixture(&input, ClipFormat::Mp4, true);
    let before = std::fs::read(&input).unwrap();
    for (i, fault) in [Fault::None, Fault::DeviceLost, Fault::Missing]
        .into_iter()
        .enumerate()
    {
        let attempts = Arc::new(AtomicUsize::new(0));
        let base = opener(fault, attempts.clone());
        let gpu: FastOpener = Arc::new(move |request| {
            assert_eq!(request.fps, 60);
            assert_eq!(request.keyframe_frames, 120);
            base(request)
        });
        let mut req = request(&input, dir.file(&format!("out-{i}.mp4")), None);
        req.quality = SequenceQuality {
            resolution: SequenceResolution::Source,
            fps: 60,
        };
        let report =
            sequence_to_mp4_with(&req, Some(&gpu), &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!((report.fps, report.clip.pictures), (60, 60));
        assert_eq!(
            report.encoder,
            if fault == Fault::None {
                "Test GPU"
            } else {
                "Software"
            }
        );
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        assert_eq!(source(&req.output).info().duration, Tick::from_units(1, 1));
        decode(&req);
    }
    assert_eq!(std::fs::read(&input).unwrap(), before);
}
