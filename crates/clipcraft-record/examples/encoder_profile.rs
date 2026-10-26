//! Measures the software H.264 encoder on raw I420 pictures: `encoder_profile WIDTH HEIGHT FILE`.
//! It tries the settings the recorder can vary (threads, profile, preset, variance AQ, scene-cut
//! detection, slices) and prints the milliseconds per picture, so the recorder's defaults can be
//! chosen from numbers. The pictures come from a real capture (`ffmpeg -pix_fmt yuv420p -f rawvideo`).
use std::time::Instant;

use filmcraft_h264enc::{
    Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl, YuvFrame,
};

struct Case {
    name: &'static str,
    profile: Profile,
    preset: Preset,
    threads: usize,
    aq: f32,
    scenecut: bool,
    slices: usize,
}

fn run(
    width: u32,
    height: u32,
    data: &[u8],
    case: &Case,
) -> Result<(f64, usize), Box<dyn std::error::Error>> {
    let (w, h) = (width as usize, height as usize);
    let frame_bytes = w * h * 3 / 2;
    let count = data.len() / frame_bytes;
    let mut config = EncoderConfig::new(width, height, 60, 1);
    config.profile = case.profile;
    config.preset = case.preset;
    config.rate = RateControl::Vbr {
        target_kbps: 12_000,
        max_kbps: 24_000,
    };
    config.keyint = 60;
    config.bframes = 0;
    config.format = PacketFormat::LengthPrefixed;
    config.threads = case.threads;
    config.aq_strength = case.aq;
    config.scenecut = case.scenecut;
    config.slices = case.slices;
    let mut encoder = Encoder::new(config)?;
    let mut bytes = 0;
    let started = Instant::now();
    for index in 0..count {
        let base = index * frame_bytes;
        let picture = YuvFrame {
            y: &data[base..base + w * h],
            u: &data[base + w * h..base + w * h * 5 / 4],
            v: &data[base + w * h * 5 / 4..base + frame_bytes],
            y_stride: w,
            uv_stride: w / 2,
        };
        for packet in encoder.encode(&picture, index as i64)? {
            bytes += packet.data.len();
        }
    }
    for packet in encoder.flush() {
        bytes += packet.data.len();
    }
    Ok((
        started.elapsed().as_secs_f64() * 1000.0 / count as f64,
        bytes / count,
    ))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let width: u32 = arguments.next().ok_or("width")?.parse()?;
    let height: u32 = arguments.next().ok_or("height")?.parse()?;
    let data = std::fs::read(arguments.next().ok_or("file")?)?;
    let base = Case {
        name: "",
        profile: Profile::High,
        preset: Preset::Speed,
        threads: 2,
        aq: 0.6,
        scenecut: true,
        slices: 0,
    };
    let cases = [
        Case {
            name: "current (High, Speed, 2 threads, AQ 0.6, scenecut)",
            ..base
        },
        Case {
            name: "1 thread",
            threads: 1,
            ..base
        },
        Case {
            name: "4 threads",
            threads: 4,
            ..base
        },
        Case {
            name: "AQ off",
            aq: 0.0,
            ..base
        },
        Case {
            name: "scenecut off",
            scenecut: false,
            ..base
        },
        Case {
            name: "AQ off + scenecut off",
            aq: 0.0,
            scenecut: false,
            ..base
        },
        Case {
            name: "Main profile",
            profile: Profile::Main,
            aq: 0.0,
            scenecut: false,
            ..base
        },
        Case {
            name: "Baseline (CAVLC)",
            profile: Profile::Baseline,
            aq: 0.0,
            scenecut: false,
            ..base
        },
        Case {
            name: "AQ+scenecut off, 4 threads",
            threads: 4,
            aq: 0.0,
            scenecut: false,
            ..base
        },
        Case {
            name: "AQ+scenecut off, 4 threads, 8 slices",
            threads: 4,
            aq: 0.0,
            scenecut: false,
            slices: 8,
            ..base
        },
        Case {
            name: "Baseline, 4 threads, AQ+scenecut off",
            profile: Profile::Baseline,
            threads: 4,
            aq: 0.0,
            scenecut: false,
            ..base
        },
    ];
    println!("{width}x{height}");
    for case in &cases {
        let (ms, size) = run(width, height, &data, case)?;
        println!(
            "{:>7.1} ms/picture  {:>7.1} fps  {:>7} B/picture  {}",
            ms,
            1000.0 / ms,
            size,
            case.name
        );
    }
    Ok(())
}
