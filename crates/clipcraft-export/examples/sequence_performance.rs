//! Generated 1080p60 software export measurement; no display, GPU or external tools required.
//! `cargo run --release -p clipcraft-export --example sequence_performance -- NEW_DIRECTORY [SECONDS]`
use std::{error::Error, path::Path, sync::atomic::AtomicBool, time::Instant};

use clipcraft_export::{
    SequenceQuality, SequenceRequest, SequenceResolution, SequenceSegment, SequenceText,
    sequence_to_mp4,
};
use clipcraft_record::{ClipFormat, VideoInfo, replay::EncodedSample, write_clip};
use clipcraft_time::Tick;
use filmcraft_h264enc::{Encoder, EncoderConfig, PacketFormat, Preset, Profile, YuvFrame};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = args.next().ok_or("provide a new output directory")?;
    let seconds: u32 = args
        .next()
        .map(|v| {
            v.to_str()
                .ok_or("invalid duration")?
                .parse()
                .map_err(|_| "invalid duration")
        })
        .transpose()?
        .unwrap_or(2);
    if args.next().is_some() || !(2..=10).contains(&seconds) || !seconds.is_multiple_of(2) {
        return Err("duration must be an even number from 2 to 10 seconds".into());
    }
    let root = Path::new(&root);
    // Exclusive directory creation: never delete or overwrite a previous run or user media.
    std::fs::create_dir(root)?;
    let input = root.join("generated.mp4");
    generate(&input)?;
    let request = SequenceRequest {
        segments: (0..seconds / 2)
            .map(|_| SequenceSegment {
                input: input.clone(),
                start: Tick::ZERO,
                end: Tick::from_units(2, 1),
                volume_percent: 100,
                look: Default::default(),
            })
            .collect(),
        texts: vec![SequenceText::new(
            Tick::ZERO,
            Tick::from_units(i64::from(seconds), 1),
            "Generated highlight",
        )],
        quality: SequenceQuality {
            resolution: SequenceResolution::FullHd1080,
            fps: 60,
        },
        preset: None,
        output: root.join("warmup.mp4"),
    };
    println!("# 1920x1080, 60 FPS, {seconds}s, H.264 software Speed/2 threads, text, silent AAC");
    println!("run,elapsed_ms,frames,bytes,progress_calls,max_progress_gap_ms,linux_peak_rss_kib");
    for run in 0..4 {
        let request = SequenceRequest {
            output: root.join(format!("export-{run}.mp4")),
            ..request.clone()
        };
        let start = Instant::now();
        let mut last = start;
        let mut gap = std::time::Duration::ZERO;
        let mut calls = 0;
        let report = sequence_to_mp4(
            &request,
            &mut |_| {
                let now = Instant::now();
                gap = gap.max(now.duration_since(last));
                last = now;
                calls += 1;
            },
            &AtomicBool::new(false),
        )?;
        let elapsed = start.elapsed();
        println!(
            "{run},{:.3},{},{},{calls},{:.3},{}",
            elapsed.as_secs_f64() * 1000.0,
            report.pictures,
            report.bytes,
            gap.as_secs_f64() * 1000.0,
            peak_rss()
        );
    }
    // Each process has one fixture, one warm-up, three measured runs. Setup is outside timing.
    // VmHWM includes fixture generation and warm-up: it is not an export-only allocation count.
    Ok(())
}

fn peak_rss() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .map(str::to_owned)
        })
        .and_then(|s| s.split_whitespace().nth(1).map(str::to_owned))
        .unwrap_or_else(|| "unavailable".into())
}

fn generate(path: &Path) -> Result<(), Box<dyn Error>> {
    const W: usize = 1920;
    const H: usize = 1080;
    let mut config = EncoderConfig::new(W as u32, H as u32, 60, 1);
    config.profile = Profile::Baseline;
    config.preset = Preset::Speed;
    config.bframes = 0;
    config.threads = 2;
    config.keyint = 120;
    config.format = PacketFormat::LengthPrefixed;
    let mut encoder = Encoder::new(config)?;
    let info = VideoInfo {
        width: W as u32,
        height: H as u32,
        fps: 60,
        avcc: encoder.avcc(),
    };
    let mut y = vec![0; W * H];
    let u = vec![100; W * H / 4];
    let v = vec![150; W * H / 4];
    let mut packets = Vec::new();
    for index in 0..120 {
        for (pos, pixel) in y.iter_mut().enumerate() {
            let (x, row) = (pos % W, pos / W);
            *pixel = 32 + ((x / 8 + row / 8 + index * 3) % 192) as u8;
        }
        packets.extend(encoder.encode(
            &YuvFrame {
                y: &y,
                u: &u,
                v: &v,
                y_stride: W,
                uv_stride: W / 2,
            },
            index as i64,
        )?);
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
    write_clip(&samples, &info, &[], ClipFormat::Mp4, path)?;
    Ok(())
}
