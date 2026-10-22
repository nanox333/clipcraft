//! Generate H.264/AAC in Rust, export locally, then decode the results. No capture or GPU needed.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clipcraft_export::{ExportError, ExportPreset, TrimRequest, compress_to_mp4};
use clipcraft_record::audio::AudioEngine;
use clipcraft_record::replay::EncodedSample;
use clipcraft_record::{
    AudioConfig, AudioControl, AudioError, AudioFormat, AudioInput, AudioSetup, AudioSource,
    ClipAudio, ClipFormat, VideoInfo, write_clip,
};
use clipcraft_time::Tick;
use filmcraft_h264enc::{
    Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl, YuvFrame,
};
use filmcraft_media::reader::{FileReader, open_reader_within};
use filmcraft_media::{FrameRequest, SharedReader, SharedSource};

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("clipcraft-compress-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
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

fn source(path: &Path) -> SharedSource {
    let reader: SharedReader = Arc::new(FileReader::open(path).unwrap());
    open_reader_within(
        path.file_name().unwrap().to_str().unwrap(),
        reader,
        &filmcraft_codecs::reader_openers(),
        &filmcraft_codecs::openers(),
        64 * 1024 * 1024,
    )
    .unwrap()
}

struct TestSound {
    emitted: u64,
    click: bool,
}
impl AudioSource for TestSound {
    fn name(&self) -> String {
        "Generated audio".into()
    }
    fn format(&self) -> AudioFormat {
        AudioFormat {
            sample_rate: 48_000,
            channels: 2,
        }
    }
    fn read(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
        for i in self.emitted..self.emitted + 800 {
            let clicking = (96_000..96_480).contains(&i);
            let level = if self.click {
                if clicking { 0.9 } else { 0.0 }
            } else {
                0.2
            };
            let value =
                (std::f64::consts::TAU * 1_000.0 * i as f64 / 48_000.0).sin() as f32 * level;
            out.extend([value, value]);
        }
        self.emitted += 800;
        Ok(())
    }
}

fn fixture(path: &Path, format: ClipFormat, audio: bool) {
    fixture_size(path, format, audio, 160, 90);
}
fn fixture_size(path: &Path, format: ClipFormat, audio: bool, width: u32, height: u32) {
    let fps = 60_u32;
    let mut config = EncoderConfig::new(width, height, fps, 1);
    config.profile = Profile::Baseline;
    config.bframes = 0;
    config.threads = 2;
    config.preset = Preset::Speed;
    config.keyint = fps;
    config.format = PacketFormat::LengthPrefixed;
    config.rate = RateControl::Qp(6);
    let mut encoder = Encoder::new(config).unwrap();
    let video = VideoInfo {
        width,
        height,
        fps,
        avcc: encoder.avcc(),
    };
    let mut sound = AudioEngine::new(AudioSetup {
        config: AudioConfig {
            sample_rate: 48_000,
            channels: 2,
            kbps: 128,
            tracks: vec!["Click".into(), "Tone".into()],
        },
        inputs: vec![
            AudioInput::new(
                Box::new(TestSound {
                    emitted: 0,
                    click: true,
                }),
                AudioControl::default(),
                0,
            ),
            AudioInput::new(
                Box::new(TestSound {
                    emitted: 0,
                    click: false,
                }),
                AudioControl::default(),
                1,
            ),
        ],
    })
    .unwrap();
    let mut tracks: Vec<ClipAudio> = sound
        .tracks()
        .into_iter()
        .map(|info| ClipAudio {
            info,
            packets: Vec::new(),
        })
        .collect();
    let mut samples = Vec::new();
    let mut noise = 17_u32;
    let chroma = vec![128; width as usize * height as usize / 4];
    for i in 0..fps * 4 {
        let mut luma = vec![0; width as usize * height as usize];
        for (pixel, y) in luma.iter_mut().enumerate() {
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *y = if pixel % (width as usize) < width as usize / 2 {
                40 + (i / fps) as u8 * 45
            } else {
                (noise >> 24) as u8
            };
        }
        for p in encoder
            .encode(
                &YuvFrame {
                    y: &luma,
                    u: &chroma,
                    v: &chroma,
                    y_stride: width as usize,
                    uv_stride: width as usize / 2,
                },
                i64::from(i),
            )
            .unwrap()
        {
            samples.push(EncodedSample {
                pts: p.pts,
                data: p.data.into(),
                keyframe: p.keyframe,
            });
        }
        if audio {
            for (track, packet) in sound.tick(Duration::from_nanos(
                u64::from(i + 1) * 1_000_000_000 / u64::from(fps),
            )) {
                tracks[track].packets.push(packet);
            }
        }
    }
    for p in encoder.flush() {
        samples.push(EncodedSample {
            pts: p.pts,
            data: p.data.into(),
            keyframe: p.keyframe,
        });
    }
    write_clip(
        &samples,
        &video,
        if audio { &tracks } else { &[] },
        format,
        path,
    )
    .unwrap();
}

fn request(input: PathBuf, output: PathBuf) -> TrimRequest {
    TrimRequest {
        input,
        output,
        start: Tick::from_units(550, 1_000),
        end: Tick::from_units(2_550, 1_000),
    }
}

#[test]
fn full_hd_frames_are_resized_before_encoding() {
    let dir = Scratch::new("resize");
    let input = dir.file("hd.mp4");
    let output = dir.file("small.mp4");
    let mut config = EncoderConfig::new(1920, 1080, 30, 1);
    config.profile = Profile::Baseline;
    config.bframes = 0;
    config.threads = 2;
    config.preset = Preset::Speed;
    config.format = PacketFormat::LengthPrefixed;
    let mut encoder = Encoder::new(config).unwrap();
    let video = VideoInfo {
        width: 1920,
        height: 1080,
        fps: 30,
        avcc: encoder.avcc(),
    };
    let luma = vec![100; 1920 * 1080];
    let chroma = vec![128; 960 * 540];
    let mut frames = Vec::new();
    for i in 0..4 {
        for p in encoder
            .encode(
                &YuvFrame {
                    y: &luma,
                    u: &chroma,
                    v: &chroma,
                    y_stride: 1920,
                    uv_stride: 960,
                },
                i,
            )
            .unwrap()
        {
            frames.push(EncodedSample {
                pts: p.pts,
                data: p.data.into(),
                keyframe: p.keyframe,
            });
        }
    }
    for p in encoder.flush() {
        frames.push(EncodedSample {
            pts: p.pts,
            data: p.data.into(),
            keyframe: p.keyframe,
        });
    }
    write_clip(&frames, &video, &[], ClipFormat::Mp4, &input).unwrap();
    let report = compress_to_mp4(
        &TrimRequest {
            input,
            output: output.clone(),
            start: Tick::ZERO,
            end: Tick::from_units(4, 30),
        },
        ExportPreset::Small,
        &mut |_| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!((report.width, report.height), (1280, 720));
    let decoded = source(&output)
        .video_frame(FrameRequest::full(Tick::ZERO))
        .unwrap();
    assert_eq!((decoded.width, decoded.height), (1280, 720));
    assert!((90..=110).contains(&decoded.luma8()[0]));
    assert_eq!(report.clip.audio_tracks, 0);
}

#[test]
fn compressed_mp4_and_mkv_ranges_decode_with_correct_audio_and_frame_rate() {
    let dir = Scratch::new("av");
    for format in [ClipFormat::Mp4, ClipFormat::Mkv] {
        let input = dir.file(&format!("input.{}", format.extension()));
        fixture(&input, format, true);
        let original = std::fs::read(&input).unwrap();
        let output = dir.file(&format!("output-{}.mp4", format.extension()));
        let mut progress = Vec::new();
        let report = compress_to_mp4(
            &request(input.clone(), output.clone()),
            ExportPreset::Custom { megabytes: 1 },
            &mut |p| progress.push(p),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(report.clip.duration, Tick::from_units(2, 1));
        assert_eq!(report.clip.pictures, 60);
        assert_eq!(report.clip.audio_tracks, 2);
        assert_eq!(report.clip.bytes, std::fs::metadata(&output).unwrap().len());
        assert!(report.clip.bytes <= 1_100_000, "{report:?}");
        assert_eq!(report.within_target, report.clip.bytes <= 1_000_000);
        assert!(progress.windows(2).all(|p| p[0] <= p[1]));
        assert_eq!(progress.last(), Some(&1.0));
        assert_eq!(std::fs::read(&input).unwrap(), original);
        let result = source(&output);
        let info = result.info().video.as_ref().unwrap();
        assert_eq!(info.frame_rate.as_f64(), 30.0);
        assert_eq!((info.width, info.height), (160, 90));
        assert_eq!(result.info().audio_streams.len(), 2);
        let before = source(&input)
            .video_frame(FrameRequest::full(Tick::from_units(550, 1_000)))
            .unwrap()
            .to_rgba8();
        let after = result
            .video_frame(FrameRequest::full(Tick::ZERO))
            .unwrap()
            .to_rgba8();
        assert!(
            (i32::from(before[0]) - i32::from(after[0])).abs() < 12,
            "wrong first picture"
        );
        let click = result.audio_stream(0, 0, 96_000, 48_000).unwrap();
        let pcm = click.interleaved();
        // Loudest 10 ms window, in stereo. The original click at 2 s should now be at 1.45 s.
        let peak = pcm
            .chunks_exact(960)
            .enumerate()
            .map(|(i, c)| (i, c.iter().map(|v| v * v).sum::<f32>()))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        assert!(
            (peak.0 as f64 * 0.01 - 1.45).abs() < 0.05,
            "click at {}",
            peak.0 as f64 * 0.01
        );
        assert!(!output.with_extension("mp4.partial").exists());
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
                decode.status.success(),
                "{}",
                String::from_utf8_lossy(&decode.stderr)
            );
        }
    }
}

#[test]
fn cancellation_collision_and_bad_inputs_preserve_files_and_clean_up() {
    let dir = Scratch::new("fail");
    let input = dir.file("input.mp4");
    fixture(&input, ClipFormat::Mp4, false);
    let original = std::fs::read(&input).unwrap();
    let output = dir.file("output.mp4");
    let range = request(input.clone(), output.clone());
    let cancel = AtomicBool::new(false);
    let result = compress_to_mp4(
        &range,
        ExportPreset::Small,
        &mut |p| {
            if p > 0.1 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
        &cancel,
    );
    assert_eq!(result, Err(ExportError::Cancelled));
    assert!(!output.exists());
    let partial = dir.file("output.mp4.partial");
    assert!(!partial.exists());
    let never = AtomicBool::new(false);
    std::fs::write(&partial, b"someone else's job").unwrap();
    assert_eq!(
        compress_to_mp4(&range, ExportPreset::Small, &mut |_| {}, &never),
        Err(ExportError::AlreadyExists)
    );
    assert_eq!(std::fs::read(&partial).unwrap(), b"someone else's job");
    std::fs::remove_file(&partial).unwrap();
    // Create the destination during encoding: publication must not overwrite it.
    assert_eq!(
        compress_to_mp4(
            &range,
            ExportPreset::Small,
            &mut |p| {
                if p > 0.1 && !output.exists() {
                    std::fs::write(&output, b"keep me").unwrap();
                }
            },
            &never
        ),
        Err(ExportError::AlreadyExists)
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"keep me");
    assert!(!partial.exists());
    std::fs::remove_file(&output).unwrap();
    assert_eq!(
        compress_to_mp4(
            &range,
            ExportPreset::Custom { megabytes: 0 },
            &mut |_| {},
            &never
        ),
        Err(ExportError::BadPreset)
    );
    let invalid = request(dir.file("bad.mp4"), output.clone());
    std::fs::write(&invalid.input, b"garbage").unwrap();
    assert!(compress_to_mp4(&invalid, ExportPreset::Small, &mut |_| {}, &never).is_err());
    assert!(!output.exists());
    assert_eq!(std::fs::read(&input).unwrap(), original);
}

#[test]
fn sequence_cuts_reorder_video_and_reencode_continuous_audio_with_mute_and_gain() {
    use clipcraft_export::{SequenceRequest, SequenceSegment, sequence_to_mp4};
    let dir = Scratch::new("sequence-av");
    let a = dir.file("a.mp4");
    let b = dir.file("b.mkv");
    fixture(&a, ClipFormat::Mp4, true);
    fixture(&b, ClipFormat::Mkv, true);
    let originals = [std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap()];
    let segment = |input: PathBuf, start: i64, end: i64, volume_percent| SequenceSegment {
        input,
        start: Tick::from_units(start, 1000),
        end: Tick::from_units(end, 1000),
        volume_percent,
    };
    let output = dir.file("edit.mp4");
    let request = SequenceRequest {
        output: output.clone(),
        segments: vec![
            segment(a.clone(), 1500, 2500, 100),
            segment(b.clone(), 500, 1500, 0),
            segment(a.clone(), 1500, 2500, 50),
        ],
    };
    let mut progress = Vec::new();
    let report =
        sequence_to_mp4(&request, &mut |p| progress.push(p), &AtomicBool::new(false)).unwrap();
    assert_eq!(report.duration, Tick::from_units(3, 1));
    assert_eq!(report.pictures, 90);
    assert_eq!(report.audio_tracks, 1);
    assert!(progress.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(progress.last(), Some(&1.0));
    let decoded = source(&output);
    assert_eq!(decoded.info().duration, Tick::from_units(3, 1));
    for (at, input, start) in [(0, &a, 1500), (1, &b, 500), (2, &a, 1500)] {
        let reference = source(input)
            .video_frame(FrameRequest::full(Tick::from_units(start, 1000)))
            .unwrap()
            .to_rgba8();
        let result = decoded
            .video_frame(FrameRequest::full(Tick::from_units(at, 1)))
            .unwrap()
            .to_rgba8();
        assert!(
            (i32::from(reference[0]) - i32::from(result[0])).abs() < 15,
            "incorrect cut at {at}"
        );
    }
    let audio = decoded.audio_stream(0, 0, 144_000, 48_000).unwrap();
    let left = &audio.channels[0];
    let peak = |from: usize, to: usize| {
        left[from..to]
            .iter()
            .map(|v| v.abs())
            .fold(0.0_f32, f32::max)
    };
    let loud = peak(21_600, 31_200);
    let quiet = peak(117_600, 127_200);
    assert!(loud > 0.2, "click missing: {loud}");
    assert!(
        quiet > loud * 0.2 && quiet < loud * 0.75,
        "gain not applied: {loud}, {quiet}"
    );
    assert!(peak(57_600, 76_800) < 0.01, "muted segment is audible");
    assert_eq!(std::fs::read(&a).unwrap(), originals[0]);
    assert_eq!(std::fs::read(&b).unwrap(), originals[1]);
    let result = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&output)
        .args(["-map", "0", "-f", "null", "-"])
        .output();
    if let Ok(result) = result {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn sequence_fractional_cuts_do_not_accumulate_audio_delay() {
    use clipcraft_export::{SequenceRequest, SequenceSegment, sequence_to_mp4};
    let dir = Scratch::new("sequence-fractional");
    let input = dir.file("source.mp4");
    fixture(&input, ClipFormat::Mp4, true);
    let request = SequenceRequest {
        output: dir.file("edit.mp4"),
        segments: vec![
            SequenceSegment {
                input,
                start: Tick::from_units(1900, 1000),
                end: Tick::from_units(2101, 1000),
                volume_percent: 100
            };
            10
        ],
    };
    let report = sequence_to_mp4(&request, &mut |_| {}, &AtomicBool::new(false)).unwrap();
    assert_eq!(report.duration, Tick::from_units(2010, 1000));
    assert_eq!(report.pictures, 61);
    let decoded = source(&request.output);
    let audio = decoded.audio_stream(0, 0, 96_480, 48_000).unwrap();
    let pcm = &audio.channels[0];
    for cut in 0..10 {
        let expected_ms = cut * 201 + 100;
        let from = (expected_ms - 25) * 48;
        let to = (expected_ms + 35) * 48;
        let peak = pcm[from..to]
            .chunks_exact(240)
            .enumerate()
            .map(|(i, c)| (i, c.iter().map(|v| v * v).sum::<f32>()))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        let actual_ms = (from + peak.0 * 240) / 48;
        assert!(
            actual_ms.abs_diff(expected_ms) < 30,
            "cut {cut}: click at {actual_ms}, expected {expected_ms}"
        );
        assert!(peak.1 > 0.01, "click missing at cut {cut}");
    }
}

#[test]
fn sequence_failures_cancellation_and_publication_collisions_preserve_files() {
    use clipcraft_export::{SequenceRequest, SequenceSegment, sequence_to_mp4};
    let dir = Scratch::new("sequence-errors");
    let input = dir.file("source.mp4");
    fixture(&input, ClipFormat::Mp4, false);
    let original = std::fs::read(&input).unwrap();
    let output = dir.file("edit.mp4");
    let mut request = SequenceRequest {
        output: output.clone(),
        segments: vec![SequenceSegment {
            input: input.clone(),
            start: Tick::ZERO,
            end: Tick::from_units(1, 1),
            volume_percent: 100,
        }],
    };
    let cancel = AtomicBool::new(false);
    assert_eq!(
        sequence_to_mp4(
            &request,
            &mut |_| cancel.store(true, Ordering::Relaxed),
            &cancel
        ),
        Err(ExportError::Cancelled)
    );
    assert!(!output.exists());
    assert!(!output.with_extension("mp4.partial").exists());
    let raced = AtomicBool::new(false);
    assert_eq!(
        sequence_to_mp4(
            &request,
            &mut |_| {
                if !raced.swap(true, Ordering::Relaxed) {
                    std::fs::write(&output, b"external winner").unwrap();
                }
            },
            &AtomicBool::new(false)
        ),
        Err(ExportError::AlreadyExists)
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"external winner");
    std::fs::remove_file(&output).unwrap();
    let partial = output.with_extension("mp4.partial");
    std::fs::write(&partial, b"existing temporary").unwrap();
    assert_eq!(
        sequence_to_mp4(&request, &mut |_| {}, &AtomicBool::new(false)),
        Err(ExportError::AlreadyExists)
    );
    assert_eq!(std::fs::read(&partial).unwrap(), b"existing temporary");
    std::fs::remove_file(&partial).unwrap();
    for (start, end) in [
        (Tick(i64::MIN), Tick(i64::MAX)),
        (Tick::ZERO, Tick(i64::MAX)),
        (Tick::ZERO, Tick(1)),
        (Tick::from_units(4, 1), Tick::from_units(5, 1)),
    ] {
        request.segments[0].start = start;
        request.segments[0].end = end;
        assert!(sequence_to_mp4(&request, &mut |_| {}, &AtomicBool::new(false)).is_err());
        assert!(!output.exists());
    }
    request.segments[0].start = Tick::ZERO;
    request.segments[0].end = Tick::from_units(1, 1);
    request.segments = vec![request.segments[0].clone(); 54_001];
    assert_eq!(
        sequence_to_mp4(&request, &mut |_| {}, &AtomicBool::new(false)),
        Err(ExportError::InvalidSequence)
    );
    request.segments.truncate(1);
    request.segments[0].input = dir.file("missing.mp4");
    assert!(sequence_to_mp4(&request, &mut |_| {}, &AtomicBool::new(false)).is_err());
    assert_eq!(std::fs::read(&input).unwrap(), original);
}

#[test]
fn sequence_normalizes_portrait_and_silent_sources_without_stretching() {
    use clipcraft_export::{SequenceRequest, SequenceSegment, sequence_to_mp4};
    let dir = Scratch::new("sequence-size");
    let a = dir.file("wide.mp4");
    let b = dir.file("portrait.mp4");
    fixture_size(&a, ClipFormat::Mp4, false, 160, 90);
    fixture_size(&b, ClipFormat::Mp4, false, 90, 160);
    let request = SequenceRequest {
        output: dir.file("edit.mp4"),
        segments: vec![
            SequenceSegment {
                input: a,
                start: Tick::ZERO,
                end: Tick::from_units(100, 1000),
                volume_percent: 100,
            },
            SequenceSegment {
                input: b,
                start: Tick::ZERO,
                end: Tick::from_units(100, 1000),
                volume_percent: 100,
            },
        ],
    };
    sequence_to_mp4(&request, &mut |_| {}, &AtomicBool::new(false)).unwrap();
    let decoded = source(&request.output);
    let frame = decoded
        .video_frame(FrameRequest::full(Tick::from_units(100, 1000)))
        .unwrap();
    assert_eq!((frame.width, frame.height), (160, 90));
    let pixels = frame.to_rgba8();
    assert!(
        pixels[0] < 8,
        "portrait picture was stretched into the black bars"
    );
    assert!(
        pixels[(45 * 160 + 75) * 4] > 10,
        "portrait content is missing"
    );
    let sound = decoded.audio_stream(0, 0, 9600, 48_000).unwrap();
    assert!(sound.channels.iter().flatten().all(|v| v.abs() < 0.001));
}

#[test]
fn sequence_exports_more_than_thirty_two_segments() {
    use clipcraft_export::{SequenceRequest, SequenceSegment, sequence_to_mp4};
    let dir = Scratch::new("sequence-many");
    let input = dir.file("source.mp4");
    fixture(&input, ClipFormat::Mp4, false);
    let request = SequenceRequest {
        output: dir.file("edit.mp4"),
        segments: vec![
            SequenceSegment {
                input,
                start: Tick::ZERO,
                end: Tick::from_units(1, 30),
                volume_percent: 0
            };
            40
        ],
    };
    let report = sequence_to_mp4(&request, &mut |_| {}, &AtomicBool::new(false)).unwrap();
    assert_eq!(report.pictures, 40);
    assert_eq!(report.duration, Tick::from_units(40, 30));
    assert_eq!(
        source(&request.output)
            .info()
            .video
            .as_ref()
            .unwrap()
            .frame_rate
            .as_f64(),
        30.0
    );
}
