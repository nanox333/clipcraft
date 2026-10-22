//! The hardware-encoder path, with a stand-in encoder (the software one behind the hardware
//! seam), so it runs anywhere: the result decodes with the right length, size and sound, and
//! every kind of hardware trouble falls back to the software encoder.
mod support;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use clipcraft_export::{
    ExportError, ExportPreset, FastEncoder, FastOpener, FastPacket, FastRequest, TrimRequest,
    compress_to_mp4_with,
};
use clipcraft_record::ClipFormat;
use clipcraft_time::Tick;
use filmcraft_h264enc::{
    Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl, YuvFrame,
};
use filmcraft_media::FrameRequest;
use support::{Scratch, fixture, source};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Trouble {
    None,
    WillNotOpen,
    FailsAfter(u64),
    DropsAPicture,
    NoDescription,
}

struct Stand {
    encoder: Encoder,
    request: FastRequest,
    trouble: Trouble,
    dropped: bool,
    seen: u64,
}

impl FastEncoder for Stand {
    fn encode(&mut self, rgba: &[u8], index: u64) -> Result<Vec<FastPacket>, String> {
        let (w, h) = (self.request.width as usize, self.request.height as usize);
        assert_eq!(
            rgba.len(),
            w * h * 4,
            "pictures arrive at the requested size"
        );
        if let Trouble::FailsAfter(n) = self.trouble
            && self.seen >= n
        {
            return Err("the device was lost".into());
        }
        self.seen += 1;
        let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
        filmcraft_export::rgba_to_yuv420_8(rgba, w, h, &mut y, &mut u, &mut v);
        let packets = self
            .encoder
            .encode(
                &YuvFrame {
                    y: &y,
                    u: &u,
                    v: &v,
                    y_stride: w,
                    uv_stride: w / 2,
                },
                i64::try_from(index).unwrap(),
            )
            .map_err(|e| e.to_string())?;
        Ok(self.convert(packets))
    }

    fn flush(&mut self) -> Result<Vec<FastPacket>, String> {
        let packets = self.encoder.flush();
        Ok(self.convert(packets))
    }

    fn avcc(&self) -> Vec<u8> {
        if self.trouble == Trouble::NoDescription || self.seen == 0 {
            Vec::new()
        } else {
            self.encoder.avcc()
        }
    }

    fn name(&self) -> String {
        "Stand-in GPU".into()
    }
}

impl Stand {
    fn convert(&mut self, packets: Vec<filmcraft_h264enc::Packet>) -> Vec<FastPacket> {
        packets
            .into_iter()
            .filter(|_| {
                // Drop exactly one picture, the first one asked about.
                if self.trouble == Trouble::DropsAPicture && !self.dropped {
                    self.dropped = true;
                    return false;
                }
                true
            })
            .map(|p| FastPacket {
                data: p.data,
                keyframe: p.keyframe,
            })
            .collect()
    }
}

fn opener(trouble: Trouble, opened: Arc<AtomicUsize>) -> FastOpener {
    Arc::new(move |request: &FastRequest| {
        opened.fetch_add(1, Ordering::Relaxed);
        if trouble == Trouble::WillNotOpen {
            return Err("no hardware encoder here".to_owned());
        }
        let mut config = EncoderConfig::new(request.width, request.height, request.fps, 1);
        config.profile = Profile::Baseline;
        config.preset = Preset::Speed;
        config.bframes = 0;
        config.threads = 2;
        config.keyint = request.keyframe_frames;
        config.format = PacketFormat::LengthPrefixed;
        config.rate = RateControl::Vbr {
            target_kbps: request.video_kbps,
            max_kbps: request.video_kbps * 2,
        };
        let encoder = Encoder::new(config).map_err(|e| e.to_string())?;
        Ok(Box::new(Stand {
            encoder,
            request: *request,
            trouble,
            dropped: false,
            seen: 0,
        }) as Box<dyn FastEncoder>)
    })
}

fn request(input: &Path, output: &Path) -> TrimRequest {
    TrimRequest {
        input: input.to_path_buf(),
        output: output.to_path_buf(),
        start: Tick::from_units(550, 1_000),
        end: Tick::from_units(2_550, 1_000),
    }
}

#[test]
fn a_hardware_export_decodes_with_the_right_length_picture_and_sound() {
    let dir = Scratch::new("hw-good");
    let input = dir.file("in.mp4");
    fixture(&input, ClipFormat::Mp4, true);
    let before = std::fs::read(&input).unwrap();
    let output = dir.file("out.mp4");
    let opened = Arc::new(AtomicUsize::new(0));
    let hardware = opener(Trouble::None, opened.clone());
    let mut seen = Vec::new();
    let report = compress_to_mp4_with(
        &request(&input, &output),
        ExportPreset::Custom { megabytes: 1 },
        Some(&hardware),
        &mut |p| seen.push(p),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(report.encoder, "Stand-in GPU");
    assert_eq!(opened.load(Ordering::Relaxed), 1);
    assert_eq!(report.clip.duration, Tick::from_units(2, 1));
    assert_eq!(report.clip.pictures, 60);
    assert_eq!(report.clip.audio_tracks, 2);
    assert!(seen.windows(2).all(|p| p[0] <= p[1]));
    assert_eq!(seen.last(), Some(&1.0));
    assert_eq!(std::fs::read(&input).unwrap(), before);
    assert!(!dir.file("out.mp4.partial").exists());

    let result = source(&output);
    let info = result.info().video.as_ref().unwrap();
    assert_eq!(info.frame_rate.as_f64(), 30.0);
    assert_eq!((info.width, info.height), (160, 90));
    // The left half's brightness steps up each second; the first picture is the cut's start.
    let first = |clip: &Path, at: i64| {
        source(clip)
            .video_frame(FrameRequest::full(Tick::from_units(at, 1_000)))
            .unwrap()
            .to_rgba8()[0]
    };
    let (a, b) = (first(&input, 550), first(&output, 0));
    assert!((i32::from(a) - i32::from(b)).abs() < 16, "{a} vs {b}");
    let (a, b) = (first(&input, 1_750), first(&output, 1_200));
    assert!((i32::from(a) - i32::from(b)).abs() < 16, "{a} vs {b}");
    // The click that was at 2.0 s is at 1.45 s.
    let pcm = result
        .audio_stream(0, 0, 96_000, 48_000)
        .unwrap()
        .interleaved();
    let peak = pcm
        .chunks_exact(960)
        .enumerate()
        .map(|(i, c)| (i, c.iter().map(|v| v * v).sum::<f32>()))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    assert!((peak.0 as f64 * 0.01 - 1.45).abs() < 0.05, "{}", peak.0);
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok()
    {
        let decode = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-map", "0", "-f", "null", "-"])
            .output()
            .unwrap();
        assert!(
            decode.status.success() && decode.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&decode.stderr)
        );
    }
}

#[test]
fn every_kind_of_hardware_trouble_falls_back_to_the_software_encoder() {
    let dir = Scratch::new("hw-bad");
    let input = dir.file("in.mp4");
    fixture(&input, ClipFormat::Mp4, true);
    for trouble in [
        Trouble::WillNotOpen,
        Trouble::FailsAfter(20),
        Trouble::DropsAPicture,
        Trouble::NoDescription,
    ] {
        let output = dir.file(&format!("out-{trouble:?}.mp4"));
        let hardware = opener(trouble, Arc::new(AtomicUsize::new(0)));
        let report = compress_to_mp4_with(
            &request(&input, &output),
            ExportPreset::Custom { megabytes: 1 },
            Some(&hardware),
            &mut |_| {},
            &AtomicBool::new(false),
        )
        .unwrap_or_else(|e| panic!("{trouble:?}: {e}"));
        assert_eq!(report.encoder, "Software", "{trouble:?}");
        assert_eq!(report.clip.pictures, 60, "{trouble:?}");
        let info = source(&output).info().video.clone().unwrap();
        assert_eq!(info.frame_rate.as_f64(), 30.0, "{trouble:?}");
        assert!(!dir.file(&format!("out-{trouble:?}.mp4.partial")).exists());
    }
}

#[test]
fn cancelling_a_hardware_export_stops_it_and_leaves_nothing() {
    let dir = Scratch::new("hw-cancel");
    let input = dir.file("in.mp4");
    fixture(&input, ClipFormat::Mp4, true);
    let output = dir.file("out.mp4");
    let hardware = opener(Trouble::None, Arc::new(AtomicUsize::new(0)));
    let cancel = AtomicBool::new(false);
    let result = compress_to_mp4_with(
        &request(&input, &output),
        ExportPreset::Custom { megabytes: 1 },
        Some(&hardware),
        &mut |p| {
            if p > 0.2 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    );
    assert_eq!(result.unwrap_err(), ExportError::Cancelled);
    assert!(!output.exists());
    assert!(!dir.file("out.mp4.partial").exists());
}
