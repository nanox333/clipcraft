//! Measures the software H.264 encoder the way the recorder uses it, on raw I420 pictures:
//! `encoder_profile WIDTH HEIGHT RAW.yuv [THREADS] [OUT.h264]`.
//!
//! Prints milliseconds per picture and bytes per picture. With `OUT.h264` the stream is written so
//! its quality can be scored against the source (see `docs/testing.md`), which keeps a speed-up from
//! quietly costing quality. The pictures come from a real capture:
//! `ffmpeg -i clip.mp4 -pix_fmt yuv420p -f rawvideo raw.yuv`.
use std::io::Write;
use std::time::Instant;

use filmcraft_h264enc::{
    Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl, YuvFrame,
};

fn picture(
    encoder: &mut Encoder,
    data: &[u8],
    w: usize,
    h: usize,
    index: usize,
) -> Result<Vec<filmcraft_h264enc::Packet>, Box<dyn std::error::Error>> {
    let frame_bytes = w * h * 3 / 2;
    let base = index * frame_bytes;
    let frame = YuvFrame {
        y: &data[base..base + w * h],
        u: &data[base + w * h..base + w * h * 5 / 4],
        v: &data[base + w * h * 5 / 4..base + frame_bytes],
        y_stride: w,
        uv_stride: w / 2,
    };
    Ok(encoder.encode(&frame, index as i64)?)
}

#[inline(never)]
fn predicted_frame(
    encoder: &mut Encoder,
    data: &[u8],
    w: usize,
    h: usize,
    index: usize,
) -> Result<Vec<filmcraft_h264enc::Packet>, Box<dyn std::error::Error>> {
    picture(encoder, data, w, h, index)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let width: u32 = arguments.next().ok_or("width")?.parse()?;
    let height: u32 = arguments.next().ok_or("height")?.parse()?;
    let data = std::fs::read(arguments.next().ok_or("file")?)?;
    let threads: usize = arguments.next().map_or(Ok(2), |t| t.parse())?;
    let mut output = match arguments.next() {
        Some(path) => Some(std::fs::File::create(path)?),
        None => None,
    };
    let (w, h) = (width as usize, height as usize);
    let frame_bytes = w * h * 3 / 2;
    let count = data.len() / frame_bytes;
    // The settings `clipcraft-record` uses for a 60 fps recording on the processor.
    let mut config = EncoderConfig::new(width, height, 60, 1);
    config.profile = Profile::High;
    config.preset = Preset::Speed;
    // `QP=26` in the environment measures at a fixed quantiser instead: the same quality target
    // before and after a change, so size and PSNR show the efficiency cost of a speed-up.
    config.rate = match std::env::var("QP").ok().and_then(|q| q.parse::<u8>().ok()) {
        Some(qp) => RateControl::Qp(qp),
        None => RateControl::Vbr {
            target_kbps: 12_000,
            max_kbps: 24_000,
        },
    };
    config.keyint = 60;
    config.bframes = 0;
    config.format = if output.is_some() {
        PacketFormat::AnnexB
    } else {
        PacketFormat::LengthPrefixed
    };
    config.threads = threads;
    let mut encoder = Encoder::new(config)?;
    let mut bytes = 0;
    let mut slowest = 0.0_f64;
    let started = Instant::now();
    for index in 0..count {
        let one = Instant::now();
        // The first picture (a keyframe) is kept out of `predicted_frame`, so a profiler can be
        // pointed at the predicted frames alone (`valgrind --toggle-collect=*predicted_frame*`).
        let packets = if index == 0 {
            picture(&mut encoder, &data, w, h, 0)?
        } else {
            predicted_frame(&mut encoder, &data, w, h, index)?
        };
        for packet in packets {
            bytes += packet.data.len();
            if let Some(file) = output.as_mut() {
                file.write_all(&packet.data)?;
            }
        }
        slowest = slowest.max(one.elapsed().as_secs_f64() * 1000.0);
    }
    for packet in encoder.flush() {
        bytes += packet.data.len();
        if let Some(file) = output.as_mut() {
            file.write_all(&packet.data)?;
        }
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0 / count as f64;
    println!(
        "{width}x{height} {threads} thread(s): {ms:6.1} ms/picture ({:5.1} fps), slowest {slowest:5.1} ms, {:7} B/picture",
        1000.0 / ms,
        bytes / count
    );
    Ok(())
}
