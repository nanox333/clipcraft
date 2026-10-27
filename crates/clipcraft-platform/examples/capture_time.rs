//! Times screen capture on this computer: `capture_time [TARGET_ID] [FRAMES]`. Prints the average
//! and worst time one picture takes to arrive, and the frame rate that allows.
use std::time::Instant;

use clipcraft_platform::{CaptureOptions, list_targets, open};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let targets = list_targets()?;
    let id = match arguments.next() {
        Some(id) => id,
        None => targets.first().ok_or("no capture target")?.id.clone(),
    };
    let frames: usize = arguments.next().and_then(|n| n.parse().ok()).unwrap_or(120);
    let mut source = open(&id, &CaptureOptions::default())?;
    let target = source.target().clone();
    println!("{} ({}x{})", target.name, target.width, target.height);
    let mut times = Vec::new();
    for _ in 0..frames + 5 {
        let started = Instant::now();
        let frame = source.capture()?;
        let _ = frame.bgra.len();
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let times = &times[5..];
    let average = times.iter().sum::<f64>() / times.len() as f64;
    let worst = times.iter().cloned().fold(0.0, f64::max);
    println!(
        "{average:.2} ms average, {worst:.2} ms worst -> up to {:.0} fps",
        1000.0 / average
    );
    Ok(())
}
