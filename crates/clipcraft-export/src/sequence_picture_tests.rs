use super::*;
use std::{hint::black_box, time::Instant};

// Deliberately retain the pre-M11.1e algorithm as an independent pixel oracle and benchmark.
fn reference(image: image::RgbaImage, size: (u32, u32), effect: Effect) -> image::RgbaImage {
    let (width, height) = size;
    let mut fitted =
        image::imageops::thumbnail(&image, width.min(image.width()), height.min(image.height()));
    effect.apply_rgba(&mut fitted);
    let mut canvas = image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    image::imageops::overlay(
        &mut canvas,
        &fitted,
        i64::from((width - fitted.width()) / 2),
        i64::from((height - fitted.height()) / 2),
    );
    canvas
}

fn patterned(size: (u32, u32), alpha: bool) -> image::RgbaImage {
    image::RgbaImage::from_fn(size.0, size.1, |x, y| {
        image::Rgba([
            ((x * 17 + y * 11) % 256) as u8,
            ((x * 7 + y * 19) % 256) as u8,
            ((x * 31 + y * 3) % 256) as u8,
            if alpha {
                ((x * 13 + y * 23) % 256) as u8
            } else {
                255
            },
        ])
    })
}

#[test]
fn fitting_preserves_pixels_for_sizes_alpha_effects_and_fades() {
    for size in [(32, 18), (16, 10), (18, 32), (64, 36), (63, 35)] {
        for alpha in [false, true] {
            for effect in Effect::ALL {
                let image = patterned(size, alpha);
                let expected = reference(image.clone(), (32, 18), effect);
                let actual = fit_picture(image, (32, 18), effect);
                assert_eq!(
                    actual, expected,
                    "size={size:?} alpha={alpha} effect={effect:?}"
                );
                for fade in [Fade::Black, Fade::White] {
                    for amount in [0.0, 0.37, 1.0] {
                        let (mut a, mut b) = (actual.clone(), expected.clone());
                        blend_fade(&mut a, fade, amount);
                        blend_fade(&mut b, fade, amount);
                        assert_eq!(a, b);
                    }
                }
            }
        }
    }
}

#[test]
fn matching_opaque_frames_reuse_the_owned_rgba_allocation() {
    for effect in Effect::ALL {
        let image = patterned((32, 18), false);
        let pointer = image.as_ptr();
        let expected = reference(image.clone(), (32, 18), effect);
        let actual = fit_picture(image, (32, 18), effect);
        assert_eq!(
            actual.as_ptr(),
            pointer,
            "a full canvas needs no extra allocation"
        );
        assert_eq!(actual, expected);
    }
}

#[test]
#[ignore = "manual release-profile measurement; no timing assertions in CI"]
fn measure_sequence_composition() {
    for (source, canvas) in [
        ((1920, 1080), (1920, 1080)),
        ((3840, 2160), (3840, 2160)),
        ((1920, 1080), (1280, 720)),
        ((720, 1280), (1920, 1080)),
    ] {
        let image = patterned(source, false);
        let mut before = Vec::new();
        let mut after = Vec::new();
        for batch in 0..7 {
            for old in if batch % 2 == 0 {
                [true, false]
            } else {
                [false, true]
            } {
                let start = Instant::now();
                for _ in 0..4 {
                    let frame = black_box(image.clone());
                    black_box(if old {
                        reference(frame, canvas, Effect::None)
                    } else {
                        fit_picture(frame, canvas, Effect::None)
                    });
                }
                if batch >= 2 {
                    let ms = start.elapsed().as_secs_f64() * 1000.0 / 4.0;
                    if old {
                        before.push(ms);
                    } else {
                        after.push(ms);
                    }
                }
            }
        }
        before.sort_by(f64::total_cmp);
        after.sort_by(f64::total_cmp);
        println!(
            "{source:?}->{canvas:?}: before median={:.3} range={:.3}..{:.3} ms; after median={:.3} range={:.3}..{:.3} ms",
            before[2], before[0], before[4], after[2], after[0], after[4]
        );
    }
}

fn split_picture(width: u32, height: u32) -> image::RgbaImage {
    // Red on the left half, green in the middle fifth of the height band, blue on the right half.
    image::RgbaImage::from_fn(width, height, |x, y| {
        let band = (height / 3..height * 2 / 3).contains(&y);
        match (x < width / 2, band) {
            (_, true) => image::Rgba([0, 255, 0, 255]),
            (true, false) => image::Rgba([255, 0, 0, 255]),
            (false, false) => image::Rgba([0, 0, 255, 255]),
        }
    })
}

#[test]
fn each_framing_makes_the_picture_suit_a_tall_canvas() {
    let canvas = [90, 160];
    let source = split_picture(160, 90);
    // Fit leaves the picture for the compositor to centre.
    assert_eq!(
        shape_for_canvas(source.clone(), canvas, SequenceFraming::Fit).dimensions(),
        (160, 90)
    );
    // Stretch, Fill and Blur all give back exactly the canvas.
    for framing in [
        SequenceFraming::Stretch,
        SequenceFraming::Fill,
        SequenceFraming::Blur,
    ] {
        let shaped = shape_for_canvas(source.clone(), canvas, framing);
        assert_eq!(shaped.dimensions(), (90, 160), "{framing:?}");
        assert!(shaped.pixels().all(|p| p[3] == 255), "{framing:?}");
    }
    // Fill crops the sides: only the middle of the width is left, so the left and right edges
    // are the colours that were near the middle (red just left of it, blue just right).
    let filled = shape_for_canvas(source.clone(), canvas, SequenceFraming::Fill);
    assert_eq!(filled.get_pixel(1, 5).0, [255, 0, 0, 255]);
    assert_eq!(filled.get_pixel(88, 5).0, [0, 0, 255, 255]);
    assert_eq!(filled.get_pixel(45, 80).0, [0, 255, 0, 255]);
    // Stretch keeps every part of the picture: red on the far left, blue on the far right.
    let stretched = shape_for_canvas(source.clone(), canvas, SequenceFraming::Stretch);
    assert_eq!(stretched.get_pixel(0, 5).0, [255, 0, 0, 255]);
    assert_eq!(stretched.get_pixel(89, 5).0, [0, 0, 255, 255]);
    // Blur: the clip is in the middle at its own shape, and above and below it is a soft,
    // darker copy rather than black bars.
    let blurred = shape_for_canvas(source.clone(), canvas, SequenceFraming::Blur);
    let front = crate::compositor::fit_size((160, 90), canvas);
    assert_eq!(front, [90, 50]);
    let middle = blurred.get_pixel(45, 80).0;
    assert_eq!(
        middle,
        [0, 255, 0, 255],
        "the clip itself is untouched in front"
    );
    let bar = blurred.get_pixel(10, 10).0;
    assert!(
        bar[3] == 255 && (bar[0] > 0 || bar[1] > 0 || bar[2] > 0),
        "{bar:?} is a bar"
    );
    assert!(
        u16::from(bar[0]) + u16::from(bar[1]) + u16::from(bar[2]) < 255 + 40,
        "{bar:?} is darkened"
    );
    // A clip that already is the shape of the canvas is not touched by any of them.
    let exact = split_picture(90, 160);
    for framing in SequenceFraming::ALL {
        assert_eq!(
            shape_for_canvas(exact.clone(), canvas, framing),
            exact,
            "{framing:?}"
        );
    }
    // Hostile shapes: an empty canvas leaves the picture alone, and a very wide canvas is bounded.
    assert_eq!(
        shape_for_canvas(source.clone(), [0, 0], SequenceFraming::Fill).dimensions(),
        (160, 90)
    );
    let ultra = shape_for_canvas(source, [3840, 960], SequenceFraming::Fill);
    assert_eq!(ultra.dimensions(), (3840, 960));
}
