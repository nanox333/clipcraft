//! GIF export and loudness evening, checked by decoding what they write.
mod support;

use std::sync::atomic::AtomicBool;

use clipcraft_export::{ExportError, GifOptions, TrimRequest, export_gif, trim_to_mp4};
use clipcraft_record::ClipFormat;
use clipcraft_time::Tick;
use filmcraft_media::FrameRequest;
use support::{Scratch, fixture, source};

fn secs(millis: i64) -> Tick {
    Tick::from_units(millis, 1_000)
}

#[test]
fn a_gif_has_the_right_size_frame_count_and_loops() {
    let dir = Scratch::new("gif");
    let input = dir.file("in.mp4");
    fixture(&input, ClipFormat::Mp4, false);
    let before = std::fs::read(&input).unwrap();
    let output = dir.file("out.gif");
    let mut seen = Vec::new();
    let report = export_gif(
        &TrimRequest {
            input: input.clone(),
            output: output.clone(),
            start: secs(500),
            end: secs(1_500),
        },
        GifOptions {
            max_width: 160,
            fps: 10,
        },
        &mut |p| seen.push(p),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(report.pictures, 10);
    assert_eq!(report.audio_tracks, 0);
    assert_eq!(report.bytes, std::fs::metadata(&output).unwrap().len());
    assert!(seen.windows(2).all(|p| p[0] <= p[1]));
    assert_eq!(seen.last(), Some(&1.0));
    assert_eq!(std::fs::read(&input).unwrap(), before);

    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options
        .read_info(std::fs::File::open(&output).unwrap())
        .unwrap();
    assert_eq!((decoder.width(), decoder.height()), (160, 90));
    let mut frames = 0;
    while let Some(frame) = decoder.read_next_frame().unwrap() {
        assert_eq!(frame.delay, 10);
        frames += 1;
    }
    assert_eq!(frames, 10);
    assert!(!dir.file("out.gif.partial").exists());
}

#[test]
fn gif_limits_collisions_and_cancel_leave_nothing_behind() {
    let dir = Scratch::new("gif-bad");
    let input = dir.file("in.mp4");
    fixture(&input, ClipFormat::Mp4, false);
    let output = dir.file("out.gif");
    let request = |start, end| TrimRequest {
        input: input.clone(),
        output: output.clone(),
        start: secs(start),
        end: secs(end),
    };
    let run = |request: &TrimRequest, options: GifOptions, cancel: &AtomicBool| {
        export_gif(request, options, &mut |_| {}, cancel)
    };
    let fine = GifOptions::default();
    let off = AtomicBool::new(false);
    assert_eq!(
        run(&request(1_000, 1_000), fine, &off).unwrap_err(),
        ExportError::BadRange
    );
    assert_eq!(
        run(&request(0, 50), fine, &off).unwrap_err(),
        ExportError::BadRange
    );
    assert_eq!(
        run(&request(0, 31_000), fine, &off).unwrap_err(),
        ExportError::GifTooLong
    );
    assert_eq!(
        run(
            &request(0, 1_000),
            GifOptions {
                max_width: 50,
                fps: 12
            },
            &off
        )
        .unwrap_err(),
        ExportError::BadPreset
    );
    assert_eq!(
        run(&request(0, 1_000), fine, &AtomicBool::new(true)).unwrap_err(),
        ExportError::Cancelled
    );
    assert!(!output.exists());
    std::fs::write(&output, b"mine").unwrap();
    assert_eq!(
        run(&request(0, 1_000), fine, &off).unwrap_err(),
        ExportError::AlreadyExists
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"mine");
}

fn mix_loudness(clip: &std::path::Path, tracks: usize, frames: usize) -> f64 {
    let decoded = source(clip);
    let mut mix = vec![0.0_f32; frames * 2];
    for track in 0..tracks {
        let pcm = decoded
            .audio_stream(track, 0, frames, 48_000)
            .unwrap()
            .interleaved();
        for (sum, sample) in mix.iter_mut().zip(pcm) {
            *sum += sample;
        }
    }
    let mut meter = filmcraft_audio_dsp::loudness::LoudnessMeter::new(48_000.0, 2);
    meter.process_interleaved(&mix);
    meter.integrated()
}

#[test]
fn normalizing_reaches_the_target_keeps_timing_and_never_clips() {
    let dir = Scratch::new("normalize");
    let input = dir.file("in.mp4");
    fixture(&input, ClipFormat::Mp4, true);
    let before = std::fs::read(&input).unwrap();
    let range = |output: &std::path::Path| TrimRequest {
        input: input.clone(),
        output: output.to_path_buf(),
        start: secs(550),
        end: secs(2_550),
    };
    let off = AtomicBool::new(false);

    // A quieter target turns the sound down by the difference.
    let quiet = dir.file("quiet.mp4");
    let mut seen = Vec::new();
    let report =
        clipcraft_export::normalize_to_mp4(&range(&quiet), -23.0, &mut |p| seen.push(p), &off)
            .unwrap();
    assert!(!report.peak_limited);
    assert!(report.gain_db < -5.0, "{report:?}");
    assert!(
        (report.measured_lufs + report.gain_db + 23.0).abs() < 0.01,
        "{report:?}"
    );
    assert_eq!(report.clip.audio_tracks, 2);
    assert!(seen.windows(2).all(|p| p[0] <= p[1]));
    assert_eq!(seen.last(), Some(&1.0));
    assert_eq!(std::fs::read(&input).unwrap(), before);
    // Measured on the written file, the sum of its tracks is at the target.
    let measured = mix_loudness(&quiet, 2, 72_000);
    assert!((measured + 23.0).abs() < 1.0, "{measured}");
    // The click that was at 2.0 s is at 1.45 s in the 0.55 s-based cut.
    let pcm = source(&quiet)
        .audio_stream(0, 0, 96_000, 48_000)
        .unwrap()
        .interleaved();
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
    // The picture is the original's, not re-encoded: same first picture.
    let first = |clip: &std::path::Path| {
        source(clip)
            .video_frame(FrameRequest::full(Tick::ZERO))
            .unwrap()
            .to_rgba8()
    };
    let cut = dir.file("cut.mp4");
    let trimmed = trim_to_mp4(&range(&cut), &mut |_| {}, &off).unwrap();
    assert_eq!(report.clip.pictures, trimmed.pictures);
    assert_eq!(report.clip.duration, trimmed.duration);
    assert_eq!(first(&quiet), first(&cut));

    // A louder target is held back by the click's peak instead of clipping.
    let loud = dir.file("loud.mp4");
    let report =
        clipcraft_export::normalize_to_mp4(&range(&loud), -10.0, &mut |_| {}, &off).unwrap();
    assert!(report.peak_limited, "{report:?}");
    assert!(report.gain_db < 1.0);
    let pcm = source(&loud)
        .audio_stream(0, 0, 96_000, 48_000)
        .unwrap()
        .interleaved();
    assert!(pcm.iter().all(|v| v.abs() <= 1.0));

    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok()
    {
        let decode = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&quiet)
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
fn normalizing_refuses_bad_requests_and_leaves_nothing_behind() {
    let dir = Scratch::new("normalize-bad");
    let silent = dir.file("silent.mp4");
    let sound = dir.file("sound.mp4");
    fixture(&silent, ClipFormat::Mp4, false);
    fixture(&sound, ClipFormat::Mp4, true);
    let output = dir.file("out.mp4");
    let request = |input: &std::path::Path| TrimRequest {
        input: input.to_path_buf(),
        output: output.clone(),
        start: secs(0),
        end: secs(1_000),
    };
    let off = AtomicBool::new(false);
    let run = |input: &std::path::Path, target: f64, cancel: &AtomicBool| {
        clipcraft_export::normalize_to_mp4(&request(input), target, &mut |_| {}, cancel)
    };
    assert_eq!(run(&sound, 0.0, &off).unwrap_err(), ExportError::BadPreset);
    assert_eq!(
        run(&sound, -40.0, &off).unwrap_err(),
        ExportError::BadPreset
    );
    assert!(matches!(
        run(&silent, -16.0, &off).unwrap_err(),
        ExportError::Unsupported(_)
    ));
    assert_eq!(
        run(&sound, -16.0, &AtomicBool::new(true)).unwrap_err(),
        ExportError::Cancelled
    );
    assert!(!output.exists());
    assert!(!dir.file("out.mp4.partial").exists());
    std::fs::write(&output, b"mine").unwrap();
    assert_eq!(
        run(&sound, -16.0, &off).unwrap_err(),
        ExportError::AlreadyExists
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"mine");
}
