//! Required real software-adapter parity + transfer-inclusive microbenchmark; no media paths.
use clipcraft_export::compositor::{Compositor, FramePlan, Overlay};
use clipcraft_export::{Effect, Fade, SequenceText, TextRaster};
use clipcraft_time::Tick;
use image::{Rgba, RgbaImage};
use std::error::Error;
use std::time::Instant;
fn main() -> Result<(), Box<dyn Error>> {
    let mut gpu = Compositor::software_gpu()?;
    println!(
        "adapter: {}",
        gpu.adapter_description().unwrap_or("unknown")
    );
    let mut cpu = Compositor::cpu();
    let mut cases = 0;
    for (source, canvas) in [
        ([64, 48], [64, 48]),
        ([97, 61], [64, 48]),
        ([45, 80], [96, 54]),
        ([101, 37], [64, 48]),
    ] {
        let image = RgbaImage::from_fn(source[0], source[1], |x, y| {
            Rgba([
                (x * 17 + y * 3) as u8,
                (x * 5 + y * 29) as u8,
                (x * 11 + y * 7) as u8,
                ((x + y) * 13) as u8,
            ])
        });
        let mut text = SequenceText::new(Tick::ZERO, Tick::from_units(1, 1), "GPU");
        text.x = 750;
        let raster =
            TextRaster::render(&text, canvas[0], canvas[1])?.ok_or("text raster missing")?;
        for effect in Effect::ALL {
            for fade in [None, Some((Fade::White, 0.37)), Some((Fade::Black, 0.71))] {
                let mut plan = FramePlan::plain(canvas, effect);
                plan.fade = fade;
                let size = raster.size();
                plan.overlays.push(Overlay {
                    size: [size[0] as u32, size[1] as u32],
                    rgba: raster.rgba(),
                    position: raster.position(text.x, text.y),
                    opacity: 0.63,
                });
                plan.overlays.push(Overlay {
                    size: [size[0] as u32, size[1] as u32],
                    rgba: raster.rgba(),
                    position: [-2, 1],
                    opacity: 0.37,
                });
                let expected = cpu.render(image.clone(), &plan)?;
                let actual = gpu.render(image.clone(), &plan)?;
                let difference = expected
                    .as_raw()
                    .iter()
                    .zip(actual.as_raw())
                    .map(|(&a, &b)| a.abs_diff(b))
                    .max()
                    .unwrap_or(0);
                if difference > 1 {
                    return Err(format!("GPU parity failed: {source:?}->{canvas:?}, {effect:?}/{fade:?}, max difference {difference}").into());
                }
                cases += 1;
            }
        }
    }
    println!(
        "{cases} actual software-adapter parity cases passed (<=1 LSB); backend={}",
        gpu.backend().label()
    );
    for size in [[1280, 720], [1920, 1080], [3840, 2160]] {
        let image = RgbaImage::from_fn(size[0], size[1], |x, y| {
            Rgba([
                (x * 17 + y * 3) as u8,
                (x * 5 + y * 29) as u8,
                (x * 11 + y * 7) as u8,
                255,
            ])
        });
        let mut plan = FramePlan::plain(size, Effect::Sepia);
        plan.fade = Some((Fade::White, 0.37));
        for (label, renderer) in [("cpu", &mut cpu), ("software-gpu", &mut gpu)] {
            let _ = renderer.render(image.clone(), &plan)?;
            let mut times = Vec::new();
            for _ in 0..5 {
                let frame = image.clone();
                let now = Instant::now();
                let result = renderer.render(frame, &plan)?;
                times.push(now.elapsed().as_secs_f64() * 1000.0);
                std::hint::black_box(result);
            }
            times.sort_by(f64::total_cmp);
            println!(
                "{size:?} {label}: median {:.3} ms (upload/submit/readback included; source cloning outside timer)",
                times[2]
            );
        }
    }
    Ok(())
}
