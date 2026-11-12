//! Isolate hardware encoding from capture, the UI, audio and recorder scheduling.
//!
//! `cargo run --release -p clipcraft-platform --example encoder_bench -- 60 240 target/encoder-60.mp4`
//! Inputs default to unpaced submission. An optional final `paced` argument schedules them at
//! integer frame deadlines, isolating input pacing from capture. Independently decode the clip.

use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clipcraft_record::replay::EncodedSample;
use clipcraft_record::{ClipFormat, HardwarePacket, HardwareRequest, VideoInfo, write_clip};

const WIDTH: usize = 1920;
const HEIGHT: usize = 1080;
const MAX_FRAMES: u32 = 600;
const MAX_PENDING: usize = 32;
const MAX_ENCODED_BYTES: usize = 128 * 1024 * 1024;
const FEED_LIMIT: Duration = Duration::from_secs(60);

struct Options {
    fps: u32,
    frames: u32,
    output: PathBuf,
    paced: bool,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let fps = args
            .next()
            .map(|value| value.parse::<u32>().map_err(|_| "fps must be 30 or 60"))
            .transpose()?
            .unwrap_or(60);
        if !matches!(fps, 30 | 60) {
            return Err("fps must be 30 or 60".to_owned());
        }
        let frames = args
            .next()
            .map(|value| value.parse::<u32>().map_err(|_| "invalid frame count"))
            .transpose()?
            .unwrap_or(240);
        if !(2..=MAX_FRAMES).contains(&frames) {
            return Err(format!("frame count must be 2..={MAX_FRAMES}"));
        }
        let output = args.next().map_or_else(
            || PathBuf::from(format!("target/encoder-bench-{fps}fps.mp4")),
            PathBuf::from,
        );
        let paced = match args.next().as_deref() {
            None | Some("unpaced") => false,
            Some("paced") => true,
            Some(_) => return Err("mode must be paced or unpaced".to_owned()),
        };
        if args.next().is_some() {
            return Err(
                "usage: encoder_bench [fps:30|60] [frames:2..600] [output.mp4] [paced|unpaced]"
                    .to_owned(),
            );
        }
        Ok(Self {
            fps,
            frames,
            output,
            paced,
        })
    }
}

#[derive(Default)]
struct Pictures {
    pending: VecDeque<i64>,
    samples: Vec<EncodedSample>,
    bytes: usize,
    submitted: u32,
}

impl Pictures {
    fn submit(&mut self, index: u32) -> Result<(), String> {
        if self.pending.len() >= MAX_PENDING || index != self.submitted {
            return Err(
                "encoder exceeded the pending-picture bound or input order changed".to_owned(),
            );
        }
        self.pending.push_back(i64::from(index));
        self.submitted = self
            .submitted
            .checked_add(1)
            .ok_or("submission counter overflow")?;
        Ok(())
    }

    fn receive(&mut self, packets: Vec<HardwarePacket>) -> Result<(), String> {
        if packets.len() > self.pending.len() {
            return Err("encoder returned more pictures than were submitted".to_owned());
        }
        for packet in packets {
            if packet.reordered || packet.data.is_empty() {
                return Err("encoder returned a reordered or empty picture".to_owned());
            }
            self.bytes = self
                .bytes
                .checked_add(packet.data.len())
                .filter(|bytes| *bytes <= MAX_ENCODED_BYTES)
                .ok_or("encoded pictures exceeded the 128 MiB budget")?;
            let pts = self
                .pending
                .pop_front()
                .ok_or("unexpected output picture")?;
            self.samples.push(EncodedSample {
                pts,
                keyframe: packet.keyframe,
                data: packet.data.into(),
            });
        }
        Ok(())
    }

    fn complete(&self, requested: u32) -> Result<(), String> {
        if self.submitted != requested
            || !self.pending.is_empty()
            || self.samples.len() != requested as usize
        {
            return Err(format!(
                "frame accounting failed: requested={requested} submitted={} returned={} pending={}",
                self.submitted,
                self.samples.len(),
                self.pending.len()
            ));
        }
        if !self.samples.first().is_some_and(|sample| sample.keyframe) {
            return Err("encoder did not start with a keyframe".to_owned());
        }
        Ok(())
    }
}

/// Real pictures instead of the test pattern, so an encoder is judged on footage. Everything is
/// optional and read from the environment: `BENCH_RGBA` (a raw 1920x1080 RGBA file, for example
/// `ffmpeg -i clip.mp4 -pix_fmt rgba -f rawvideo raw.rgba`), `BENCH_CODEC` (`h264` or `h265`),
/// `BENCH_KBPS`, `BENCH_CBR` (`0` for variable bitrate), `BENCH_KEYFRAMES` (frames between
/// keyframes) and `BENCH_EFFORT` (`fast`, `balanced` or `quality`).
struct RealFrames {
    path: Option<PathBuf>,
    codec: clipcraft_record::StreamCodec,
    kbps: u32,
    constant_bitrate: bool,
    keyframes: Option<u32>,
    effort: clipcraft_record::EncoderSpeed,
}

impl RealFrames {
    fn from_environment() -> Result<Self, String> {
        let text = |name: &str| std::env::var(name).ok();
        let number = |name: &str| -> Result<Option<u32>, String> {
            text(name)
                .map(|value| {
                    value
                        .parse::<u32>()
                        .map_err(|_| format!("{name} must be a whole number"))
                })
                .transpose()
        };
        let codec = match text("BENCH_CODEC").as_deref() {
            None | Some("h264") => clipcraft_record::StreamCodec::H264,
            Some("h265") => clipcraft_record::StreamCodec::H265,
            Some(_) => return Err("BENCH_CODEC must be h264 or h265".to_owned()),
        };
        let effort = match text("BENCH_EFFORT").as_deref() {
            None | Some("balanced") => clipcraft_record::EncoderSpeed::Balanced,
            Some("fast") => clipcraft_record::EncoderSpeed::Fast,
            Some("quality") => clipcraft_record::EncoderSpeed::Quality,
            Some(_) => return Err("BENCH_EFFORT must be fast, balanced or quality".to_owned()),
        };
        let kbps = number("BENCH_KBPS")?.unwrap_or(12_000);
        if !(100..=200_000).contains(&kbps) {
            return Err("BENCH_KBPS must be 100..=200000".to_owned());
        }
        Ok(Self {
            path: text("BENCH_RGBA").map(PathBuf::from),
            codec,
            kbps,
            constant_bitrate: text("BENCH_CBR").as_deref() != Some("0"),
            keyframes: number("BENCH_KEYFRAMES")?.filter(|frames| *frames > 0),
            effort,
        })
    }

    /// The opened file, when real pictures were asked for.
    fn open(&self) -> Result<Option<std::fs::File>, String> {
        let Some(path) = &self.path else {
            return Ok(None);
        };
        let file = std::fs::File::open(path).map_err(|why| format!("BENCH_RGBA: {why}"))?;
        let picture = (WIDTH * HEIGHT * 4) as u64;
        let length = file
            .metadata()
            .map_err(|why| format!("BENCH_RGBA: {why}"))?
            .len();
        if length < picture {
            return Err("BENCH_RGBA holds less than one 1920x1080 RGBA picture".to_owned());
        }
        Ok(Some(file))
    }
}

fn pattern() -> Vec<u8> {
    let mut rgba = vec![0; WIDTH * HEIGHT * 4];
    for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
        let x = index % WIDTH;
        let y = index / WIDTH;
        pixel.copy_from_slice(&[
            (x * 251 / WIDTH) as u8,
            (y * 251 / HEIGHT) as u8,
            ((x / 32 + y / 32) % 2 * 200) as u8,
            255,
        ]);
    }
    rgba
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse(std::env::args().skip(1))?;
    if options.output.try_exists()? {
        return Err("output already exists; choose a new file".into());
    }
    let real = RealFrames::from_environment()?;
    let codec = real.codec;
    let provider = clipcraft_platform::hardware_provider().ok_or("no hardware encoder provider")?;
    let opened = Instant::now();
    let mut encoder = provider.open(&HardwareRequest {
        codec,
        width: WIDTH as u32,
        height: HEIGHT as u32,
        fps: options.fps,
        video_kbps: real.kbps,
        constant_bitrate: real.constant_bitrate,
        keyframe_frames: real.keyframes.unwrap_or(options.fps * 2),
        effort: real.effort,
    })?;
    let open_ms = opened.elapsed().as_secs_f64() * 1000.0;
    let mut real_file = real.open()?;
    let mut read_time = Duration::ZERO;
    let mut rgba = pattern();
    let mut pictures = Pictures::default();
    let mut times = Vec::with_capacity(options.frames as usize);
    let started = Instant::now();
    for index in 0..options.frames {
        if options.paced {
            // Parser bounds the positive frame rate and frame count. Integer nanosecond
            // deadlines avoid accumulating a rounded per-frame sleep or floating-point drift.
            let due_ns = u64::from(index)
                .checked_mul(1_000_000_000)
                .ok_or("frame deadline overflow")?
                / u64::from(options.fps);
            let remaining = Duration::from_nanos(due_ns).saturating_sub(started.elapsed());
            if !remaining.is_zero() {
                std::thread::sleep(remaining);
            }
        }
        if started.elapsed() >= FEED_LIMIT {
            return Err("encoder benchmark exceeded its 60-second feed budget".into());
        }
        if let Some(file) = real_file.as_mut() {
            // Reading the picture is not encoding: it is timed apart and left out of the throughput.
            let read = Instant::now();
            file.read_exact(&mut rgba)
                .map_err(|why| format!("picture {index} of the BENCH_RGBA file: {why}"))?;
            read_time += read.elapsed();
        } else {
            // Keep the full image allocation, changing only a small marker between inputs.
            let marker = (index % 251) as u8;
            for pixel in rgba.chunks_exact_mut(4).take(64) {
                pixel.copy_from_slice(&[marker, 255 - marker, marker, 255]);
            }
        }
        pictures.submit(index)?;
        let call = Instant::now();
        let packets = encoder.encode(&rgba, u64::from(index))?;
        times.push(call.elapsed().as_secs_f64() * 1000.0);
        pictures.receive(packets)?;
    }
    let flush = Instant::now();
    pictures.receive(encoder.flush()?)?;
    let flush_ms = flush.elapsed().as_secs_f64() * 1000.0;
    let elapsed = started.elapsed();
    if elapsed >= FEED_LIMIT {
        return Err("encoder benchmark exceeded its 60-second feed/drain budget".into());
    }
    pictures.complete(options.frames)?;
    let video = VideoInfo {
        width: WIDTH as u32,
        height: HEIGHT as u32,
        fps: options.fps,
        avcc: encoder.avcc(),
        codec,
    };
    let saved = write_clip(
        &pictures.samples,
        &video,
        &[],
        ClipFormat::Mp4,
        &options.output,
    )?;
    times.sort_by(f64::total_cmp);
    let percentile = |percent: usize| {
        times
            .get(times.len().saturating_sub(1) * percent / 100)
            .copied()
            .unwrap_or(0.0)
    };
    let mean_ms = times.iter().sum::<f64>() / f64::from(options.frames);
    let encoding = elapsed.saturating_sub(read_time);
    let throughput = f64::from(options.frames) / encoding.as_secs_f64().max(f64::EPSILON);
    let mode = if options.paced { "paced" } else { "unpaced" };
    println!(
        "encoder={} mode={mode} requested_fps={} submitted={} returned={} saved={} bytes={} open_ms={open_ms:.3} wall_ms={:.3} throughput_fps={throughput:.3} encode_ms_mean={mean_ms:.3} p50={:.3} p95={:.3} flush_ms={flush_ms:.3} pipeline={:?}",
        encoder.name(),
        options.fps,
        pictures.submitted,
        pictures.samples.len(),
        saved.frames,
        saved.bytes,
        elapsed.as_secs_f64() * 1000.0,
        percentile(50),
        percentile(95),
        clipcraft_platform::capture_pipeline_stats()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(keyframe: bool) -> HardwarePacket {
        HardwarePacket {
            data: vec![1, 2, 3],
            keyframe,
            reordered: false,
        }
    }

    #[test]
    fn delayed_packets_keep_submission_times_and_require_a_complete_drain() {
        let mut pictures = Pictures::default();
        pictures.submit(0).unwrap();
        pictures.receive(Vec::new()).unwrap();
        pictures.submit(1).unwrap();
        pictures.receive(vec![packet(true)]).unwrap();
        assert!(pictures.complete(2).is_err());
        pictures.receive(vec![packet(false)]).unwrap();
        pictures.complete(2).unwrap();
        assert_eq!(pictures.samples[0].pts, 0);
        assert_eq!(pictures.samples[1].pts, 1);
        assert!(pictures.receive(vec![packet(false)]).is_err());
    }

    #[test]
    fn invalid_or_unbounded_encoder_output_is_rejected() {
        let mut pictures = Pictures::default();
        for index in 0..MAX_PENDING as u32 {
            pictures.submit(index).unwrap();
        }
        assert!(pictures.submit(MAX_PENDING as u32).is_err());
        let mut reordered = packet(true);
        reordered.reordered = true;
        assert!(pictures.receive(vec![reordered]).is_err());
        pictures.bytes = MAX_ENCODED_BYTES;
        assert!(pictures.receive(vec![packet(true)]).is_err());
    }

    #[test]
    fn arguments_reject_hostile_counts_rates_and_trailing_values() {
        for args in [
            vec!["0"],
            vec!["59"],
            vec!["60", "0"],
            vec!["60", "601"],
            vec!["60", "4294967296"],
            vec!["60", "2", "clip.mp4", "extra"],
            vec!["60", "2", "clip.mp4", "paced", "extra"],
        ] {
            assert!(Options::parse(args.into_iter().map(str::to_owned)).is_err());
        }
        let options =
            Options::parse(["30", "180", "clip.mp4"].into_iter().map(str::to_owned)).unwrap();
        assert_eq!((options.fps, options.frames), (30, 180));
        assert!(!options.paced);
        let paced = Options::parse(
            ["60", "240", "clip.mp4", "paced"]
                .into_iter()
                .map(str::to_owned),
        )
        .unwrap();
        assert!(paced.paced);
    }
}
