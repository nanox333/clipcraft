//! Export speed on generated clips. Ignored by default; run with
//! `cargo test -p clipcraft-export --release --test timing -- --ignored --nocapture`
//! and set `CLIPCRAFT_TIMING_SECONDS` (default 30) and `CLIPCRAFT_TIMING_SIZE` (`WxH@FPS`, default
//! `1280x720@30`). The numbers describe the machine they ran on, not a general expectation.
mod support;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use clipcraft_export::{
    ExportPreset, FastEncoder, FastOpener, FastPacket, FastRequest, GifOptions, TrimRequest,
    compress_to_mp4, compress_to_mp4_with, export_gif, normalize_to_mp4, trim_to_mp4,
};
use clipcraft_record::ClipFormat;
use clipcraft_time::Tick;
use support::{Scratch, fixture_sized};

fn size_from_env() -> (u32, u32, u32) {
    let text = std::env::var("CLIPCRAFT_TIMING_SIZE").unwrap_or_else(|_| "1280x720@30".into());
    let parse = || -> Option<(u32, u32, u32)> {
        let (size, fps) = text.split_once('@')?;
        let (w, h) = size.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?, fps.parse().ok()?))
    };
    parse().expect("CLIPCRAFT_TIMING_SIZE looks like 1280x720@30")
}

/// An encoder that costs nothing: it shows how fast the rest of the export can go when a graphics
/// card does the encoding. Its output is not a real picture.
struct Free(Vec<u8>, bool);

impl FastEncoder for Free {
    fn encode(&mut self, _: &[u8], _: u64) -> Result<Vec<FastPacket>, String> {
        self.1 = true;
        Ok(vec![FastPacket {
            data: vec![0, 0, 0, 1, 0x65],
            keyframe: true,
        }])
    }
    fn flush(&mut self) -> Result<Vec<FastPacket>, String> {
        Ok(Vec::new())
    }
    fn avcc(&self) -> Vec<u8> {
        if self.1 { self.0.clone() } else { Vec::new() }
    }
    fn name(&self) -> String {
        "free".into()
    }
}

#[test]
#[ignore = "a measurement, not a check"]
fn export_speed() {
    let seconds: u32 = std::env::var("CLIPCRAFT_TIMING_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    let size = size_from_env();
    let dir = Scratch::new("timing");
    let mut input = dir.file("in.mp4");
    let made = Instant::now();
    if let Ok(existing) = std::env::var("CLIPCRAFT_TIMING_INPUT") {
        input = existing.into();
    } else {
        fixture_sized(&input, ClipFormat::Mp4, true, size, seconds);
    }
    let source_mb = std::fs::metadata(&input).unwrap().len() as f64 / 1e6;
    println!(
        "clip: {}x{}@{} {seconds} s, {source_mb:.1} MB (made in {:.1} s)",
        size.0,
        size.1,
        size.2,
        made.elapsed().as_secs_f64()
    );
    println!(
        "{:<28}{:>10}{:>12}{:>10}",
        "export", "seconds", "x realtime", "MB"
    );
    let off = AtomicBool::new(false);
    let request = |name: &str| TrimRequest {
        input: input.clone(),
        output: dir.file(name),
        start: Tick::ZERO,
        end: Tick::from_units(i64::from(seconds), 1),
    };
    let row = |label: &str, started: Instant, bytes: u64| {
        let took = started.elapsed().as_secs_f64();
        println!(
            "{label:<28}{took:>10.2}{:>12.1}{:>10.1}",
            f64::from(seconds) / took.max(1e-9),
            bytes as f64 / 1e6
        );
    };
    let t = Instant::now();
    let r = trim_to_mp4(&request("trim.mp4"), &mut |_| {}, &off).unwrap();
    row("trim (copy)", t, r.bytes);
    for (label, preset) in [
        ("small (~10 MB)", ExportPreset::Small),
        ("medium (~25 MB)", ExportPreset::Medium),
    ] {
        let t = Instant::now();
        let r =
            compress_to_mp4(&request(&format!("{label}.mp4")), preset, &mut |_| {}, &off).unwrap();
        row(label, t, r.clip.bytes);
    }
    // The same export with a free encoder: the most a hardware encoder could save.
    let avcc = {
        let mut config = filmcraft_h264enc::EncoderConfig::new(640, 360, 30, 1);
        config.format = filmcraft_h264enc::PacketFormat::LengthPrefixed;
        filmcraft_h264enc::Encoder::new(config).unwrap().avcc()
    };
    let opener: FastOpener = Arc::new(move |_: &FastRequest| {
        Ok(Box::new(Free(avcc.clone(), false)) as Box<dyn FastEncoder>)
    });
    let t = Instant::now();
    let r = compress_to_mp4_with(
        &request("free.mp4"),
        ExportPreset::Small,
        Some(&opener),
        &mut |_| {},
        &off,
    )
    .unwrap();
    row("small, free encoder (bound)", t, r.clip.bytes);
    let gif_seconds = seconds.min(10);
    let t = Instant::now();
    let mut gif_request = request("clip.gif");
    gif_request.end = Tick::from_units(i64::from(gif_seconds), 1);
    let r = export_gif(&gif_request, GifOptions::default(), &mut |_| {}, &off).unwrap();
    row(&format!("gif ({gif_seconds} s, 480 wide)"), t, r.bytes);
    let t = Instant::now();
    let r = normalize_to_mp4(&request("norm.mp4"), -16.0, &mut |_| {}, &off).unwrap();
    row("even out loudness", t, r.clip.bytes);
}
