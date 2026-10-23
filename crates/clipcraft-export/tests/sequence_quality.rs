//! Generated motion, fractional cuts and real high-resolution encoder output.
mod support;

use clipcraft_export::{
    SequenceQuality, SequenceRequest, SequenceResolution, SequenceSegment, sequence_to_mp4_with,
};
use clipcraft_record::{ClipFormat, VideoInfo, replay::EncodedSample, write_clip};
use clipcraft_time::{FrameRate, Tick};
use filmcraft_h264enc::{Encoder, EncoderConfig, PacketFormat, Preset, Profile, YuvFrame};
use filmcraft_media::FrameRequest;
use std::sync::atomic::AtomicBool;
use support::{Scratch, fixture, source};

fn segment(input: &std::path::Path, start: Tick, end: Tick) -> SequenceSegment {
    SequenceSegment {
        input: input.into(),
        start,
        end,
        volume_percent: 100,
        look: Default::default(),
    }
}

#[test]
fn sixty_fps_fractional_cuts_keep_real_motion_and_audio_on_the_integer_clock() {
    let dir = Scratch::new("sequence-quality-clock");
    let input = dir.file("motion.mp4");
    fixture(&input, ClipFormat::Mp4, true);
    let original = std::fs::read(&input).unwrap();
    let request = SequenceRequest {
        segments: vec![
            segment(&input, Tick::ZERO, Tick::from_units(1, 7)),
            segment(&input, Tick::ZERO, Tick::from_units(2, 7)),
            segment(&input, Tick::from_units(37, 20), Tick::from_units(9, 4)),
        ],
        texts: vec![],
        preset: None,
        quality: SequenceQuality {
            resolution: SequenceResolution::Source,
            fps: 60,
        },
        output: dir.file("out.mp4"),
    };
    let report =
        sequence_to_mp4_with(&request, None, &mut |_| {}, &AtomicBool::new(false)).unwrap();
    assert_eq!((report.fps, report.clip.pictures), (60, 50));
    let decoded = source(&request.output);
    assert!(
        (decoded.info().duration - Tick::from_units(29, 35)).0.abs()
            <= Tick::from_units(1, 90_000).0
    );
    let a = decoded.video_frame(FrameRequest::full(Tick::ZERO)).unwrap();
    let b = decoded
        .video_frame(FrameRequest::full(Tick::from_units(1, 60)))
        .unwrap();
    assert_ne!(
        a.to_rgba8(),
        b.to_rgba8(),
        "adjacent 60 FPS source motion must not become repeated 30 FPS frames"
    );
    let audio = decoded.audio_stream(0, 0, 39_772, 48_000).unwrap();
    let first_peak = audio.channels[0]
        .iter()
        .position(|v| v.abs() > 0.1)
        .unwrap();
    let expected = Tick::from_units(81, 140).to_units_floor(48_000);
    assert!(
        (first_peak as i64 - expected).abs() < 256,
        "audio click drifted: {first_peak} vs {expected}"
    );
    let parsed = filmcraft_isobmff::open(std::fs::read(&request.output).unwrap()).unwrap();
    let track = parsed
        .tracks
        .iter()
        .find(|t| t.kind == filmcraft_isobmff::TrackKind::Video)
        .unwrap();
    assert_eq!(track.samples.len(), 50);
    assert!(track.samples.iter().take(49).all(|s| s.duration == 1500));
    assert!(
        track.samples[49].duration < 1500,
        "the last partial frame must not lengthen the edit"
    );
    assert_eq!(std::fs::read(&input).unwrap(), original);
}

fn flat_source(path: &std::path::Path, width: u32, height: u32) {
    let mut config = EncoderConfig::new(width, height, 60, 1);
    config.profile = Profile::Baseline;
    config.preset = Preset::Speed;
    config.bframes = 0;
    config.threads = 2;
    config.format = PacketFormat::LengthPrefixed;
    let mut encoder = Encoder::new(config).unwrap();
    let info = VideoInfo {
        width,
        height,
        fps: 60,
        avcc: encoder.avcc(),
    };
    let chroma = vec![128; width as usize * height as usize / 4];
    let mut packets = vec![];
    for i in 0..2 {
        let y = vec![40 + i as u8 * 140; width as usize * height as usize];
        packets.extend(
            encoder
                .encode(
                    &YuvFrame {
                        y: &y,
                        u: &chroma,
                        v: &chroma,
                        y_stride: width as usize,
                        uv_stride: width as usize / 2,
                    },
                    i,
                )
                .unwrap(),
        );
    }
    packets.extend(encoder.flush());
    let samples: Vec<_> = packets
        .into_iter()
        .map(|p| EncodedSample {
            pts: p.pts,
            data: p.data.into(),
            keyframe: p.keyframe,
        })
        .collect();
    write_clip(&samples, &info, &[], ClipFormat::Mp4, path).unwrap();
}

#[test]
fn real_1080p_4k_portrait_and_ultrawide_outputs_have_the_requested_canvas() {
    let dir = Scratch::new("sequence-quality-canvas");
    for (i, (w, h, resolution, fps)) in [
        (1920, 1080, SequenceResolution::FullHd1080, 60),
        (3840, 2160, SequenceResolution::Source, 30),
        (720, 1280, SequenceResolution::Source, 60),
        (2560, 1080, SequenceResolution::Source, 60),
    ]
    .into_iter()
    .enumerate()
    {
        let input = dir.file(&format!("in-{i}.mp4"));
        flat_source(&input, w, h);
        let original = std::fs::read(&input).unwrap();
        let request = SequenceRequest {
            segments: vec![segment(&input, Tick::ZERO, Tick::from_units(1, 30))],
            texts: vec![clipcraft_export::SequenceText::new(
                Tick::ZERO,
                Tick::from_units(1, 30),
                "Highlight",
            )],
            preset: None,
            quality: SequenceQuality { resolution, fps },
            output: dir.file(&format!("out-{i}.mp4")),
        };
        let report =
            sequence_to_mp4_with(&request, None, &mut |_| {}, &AtomicBool::new(false)).unwrap();
        assert_eq!((report.width, report.height, report.fps), (w, h, fps));
        assert_eq!(report.clip.pictures, u64::from(fps / 30));
        let decoded = source(&request.output);
        let a = decoded.video_frame(FrameRequest::full(Tick::ZERO)).unwrap();
        assert_eq!((a.width, a.height), (w, h));
        if fps == 60 {
            let b = decoded
                .video_frame(FrameRequest::full(FrameRate { num: 60, den: 1 }.tick_of(1)))
                .unwrap();
            assert!(b.to_rgba8()[0] > a.to_rgba8()[0] + 70);
        }
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
        assert_eq!(std::fs::read(&input).unwrap(), original);
    }
}
