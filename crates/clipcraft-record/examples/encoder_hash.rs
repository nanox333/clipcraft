//! Checks that an encoder change did not alter the bitstream, and times it:
//! `encoder_hash WIDTH HEIGHT FILE`. For each setting it prints a hash of every byte the encoder
//! produces, the size, and the milliseconds per picture. Run it before and after a speed-only change:
//! the hashes must be identical. The pictures are raw I420 (`ffmpeg -pix_fmt yuv420p -f rawvideo`).
use std::time::Instant;

use filmcraft_h264enc::{
    Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl, YuvFrame,
};

struct Case {
    name: &'static str,
    profile: Profile,
    preset: Preset,
    rate: RateControl,
    threads: usize,
    bframes: u8,
    aq: f32,
    keyint: u32,
}

/// FNV-1a over bytes: small, deterministic, and good enough to notice any change.
fn fnv(hash: &mut u64, bytes: &[u8]) {
    for &byte in bytes {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn run(
    width: u32,
    height: u32,
    data: &[u8],
    case: &Case,
) -> Result<(u64, usize, f64), Box<dyn std::error::Error>> {
    let (w, h) = (width as usize, height as usize);
    let frame_bytes = w * h * 3 / 2;
    let count = data.len() / frame_bytes;
    let mut config = EncoderConfig::new(width, height, 60, 1);
    config.profile = case.profile;
    config.preset = case.preset;
    config.rate = case.rate;
    config.keyint = case.keyint;
    config.bframes = case.bframes;
    config.format = PacketFormat::AnnexB;
    config.threads = case.threads;
    config.aq_strength = case.aq;
    let mut encoder = Encoder::new(config)?;
    let (mut hash, mut bytes) = (0xcbf2_9ce4_8422_2325_u64, 0);
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
            fnv(&mut hash, &packet.data);
        }
    }
    for packet in encoder.flush() {
        bytes += packet.data.len();
        fnv(&mut hash, &packet.data);
    }
    Ok((
        hash,
        bytes,
        started.elapsed().as_secs_f64() * 1000.0 / count as f64,
    ))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let width: u32 = arguments.next().ok_or("width")?.parse()?;
    let height: u32 = arguments.next().ok_or("height")?.parse()?;
    let data = std::fs::read(arguments.next().ok_or("file")?)?;
    let vbr = RateControl::Vbr {
        target_kbps: 12_000,
        max_kbps: 24_000,
    };
    let base = Case {
        name: "",
        profile: Profile::High,
        preset: Preset::Speed,
        rate: RateControl::Qp(28),
        threads: 1,
        bframes: 0,
        aq: 0.0,
        keyint: 60,
    };
    let cases = [
        Case {
            name: "Speed  QP28  1 thread",
            ..base
        },
        Case {
            name: "Speed  QP20  1 thread",
            rate: RateControl::Qp(20),
            ..base
        },
        Case {
            name: "Speed  VBR12M AQ 2 threads (recorder default)",
            rate: vbr,
            threads: 2,
            aq: 0.6,
            ..base
        },
        Case {
            name: "Balanced QP28 1 thread",
            preset: Preset::Balanced,
            ..base
        },
        Case {
            name: "Quality QP28 1 thread",
            preset: Preset::Quality,
            ..base
        },
        Case {
            name: "Speed  QP28  Main profile",
            profile: Profile::Main,
            ..base
        },
        Case {
            name: "Speed  QP28  Baseline",
            profile: Profile::Baseline,
            ..base
        },
        Case {
            name: "Balanced VBR12M 2 B-frames 2 threads",
            preset: Preset::Balanced,
            rate: vbr,
            threads: 2,
            bframes: 2,
            aq: 0.6,
            ..base
        },
    ];
    println!(
        "{}x{}, {} pictures",
        width,
        height,
        data.len() / (width as usize * height as usize * 3 / 2)
    );
    for case in &cases {
        // The fastest of three runs, so other programs on the machine disturb the numbers less.
        let (hash, bytes, mut ms) = run(width, height, &data, case)?;
        for _ in 0..2 {
            ms = ms.min(run(width, height, &data, case)?.2);
        }
        println!("{hash:016x} {bytes:>9} B {ms:>7.1} ms/pic  {}", case.name);
    }
    Ok(())
}
