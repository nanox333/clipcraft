//! Deterministic, offline CPU/memory-path measurements; no display or encoder needed.
use std::hint::black_box;
use std::time::{Duration, Instant};

use clipcraft_capture::CaptureFrame;
use clipcraft_capture::reframe::{Framing, OutputShape, reframe};
use clipcraft_record::convert::{I420, convert, convert_rgba};
use clipcraft_record::replay::{EncodedSample, ReplayBuffer};
use clipcraft_time::Tick;

fn measure(
    name: &str,
    iterations: u32,
    mut run: impl FnMut() -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..3 {
        run()?;
    }
    let mut times = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        for _ in 0..iterations {
            run()?;
        }
        times.push(started.elapsed() / iterations);
    }
    times.sort();
    println!(
        "{name}: median={:.3} ms min={:.3} ms max={:.3} ms",
        ms(times[2]),
        ms(times[0]),
        ms(times[4])
    );
    Ok(())
}

fn ms(time: Duration) -> f64 {
    time.as_secs_f64() * 1_000.0
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frame = CaptureFrame {
        width: 1920,
        height: 1080,
        stride: 1920 * 4,
        bgra: (0..1920 * 1080 * 4).map(|i| (i * 13 % 251) as u8).collect(),
        captured_at: Tick::from_units(0, 60),
    };
    for height in [1080, 720] {
        let plan = reframe(1920, 1080, OutputShape::Source, Framing::Original, height)
            .ok_or("invalid benchmark geometry")?;
        for threads in [1, 4] {
            let mut rgba = Vec::new();
            let mut yuv = I420::black(plan.output_width, plan.output_height)?;
            // Validate outside timing; timed failures are still checked afterward.
            convert_rgba(&frame, &plan, &mut rgba, threads)?;
            convert(&frame, &plan, &mut yuv, threads)?;
            measure(
                &format!("rgba 1080p -> {height}p threads={threads}"),
                30,
                || {
                    black_box(convert_rgba(
                        black_box(&frame),
                        &plan,
                        black_box(&mut rgba),
                        threads,
                    ))?;
                    Ok(())
                },
            )?;
            measure(
                &format!("i420 1080p -> {height}p threads={threads}"),
                30,
                || {
                    black_box(convert(
                        black_box(&frame),
                        &plan,
                        black_box(&mut yuv),
                        threads,
                    ))?;
                    Ok(())
                },
            )?;
        }
    }
    // 60 seconds at 60 fps, approximately 20 Mbit/s; payloads allocated before timing.
    let mut replay = ReplayBuffer::new(3600, 160_000_000);
    for pts in 0..3600 {
        replay
            .push(EncodedSample {
                pts,
                data: vec![pts as u8; 41_666].into(),
                keyframe: pts % 120 == 0,
            })
            .map_err(|_| "invalid benchmark stream")?;
    }
    measure("snapshot 60s 20Mbit/s", 10, || {
        black_box(replay.snapshot(3600));
        Ok(())
    })?;
    // Worst supported GOP at 60 fps: avoid scanning thousands of samples on every insertion.
    measure("replay push 14400 frames GOP=3600", 5, || {
        let mut buffer = ReplayBuffer::new(3600, 1_000_000);
        for pts in 0..14400 {
            buffer
                .push(EncodedSample {
                    pts,
                    data: [].into(),
                    keyframe: pts % 3600 == 0,
                })
                .map_err(|_| "invalid benchmark stream")?;
        }
        black_box(buffer);
        Ok(())
    })?;
    Ok(())
}
