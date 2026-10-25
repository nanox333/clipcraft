use super::*;
fn patterned(w: u32, h: u32, alpha: bool) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        Rgba([
            (x * 17 + y * 3) as u8,
            (x * 5 + y * 29) as u8,
            (x * 11 + y * 7) as u8,
            if alpha { ((x + y) * 13) as u8 } else { 255 },
        ])
    })
}
#[test]
fn aspect_fit_keeps_portrait_and_ultrawide_geometry_and_never_upscales() {
    assert_eq!(fit_size((720, 1280), [1280, 720]), [405, 720]);
    assert_eq!(fit_size((3440, 1440), [1280, 720]), [1280, 535]);
    assert_eq!(fit_size((64, 48), [1280, 720]), [64, 48]);
    let image = RgbaImage::from_pixel(9, 16, Rgba([10, 200, 50, 255]));
    let result = Compositor::cpu()
        .render(image, &FramePlan::plain([16, 8], Effect::None))
        .unwrap();
    assert_eq!(result.get_pixel(0, 4).0, [0, 0, 0, 255]);
    assert_eq!(result.get_pixel(7, 4).0, [10, 200, 50, 255]);
}
#[test]
fn no_op_cpu_retains_ownership_and_reference_text_matches_exactly() {
    let image = patterned(64, 48, false);
    let pointer = image.as_ptr();
    let image = Compositor::cpu()
        .render(image, &FramePlan::plain([64, 48], Effect::None))
        .unwrap();
    assert_eq!(pointer, image.as_ptr());
    let text = crate::SequenceText::new(
        clipcraft_time::Tick::ZERO,
        clipcraft_time::Tick::from_units(1, 1),
        "Inter",
    );
    let raster = crate::TextRaster::render(&text, 128, 72).unwrap().unwrap();
    let size = raster.size();
    let mut expected = RgbaImage::from_pixel(128, 72, Rgba([30, 40, 50, 255]));
    raster.draw(&mut expected, &text, 0.37);
    let mut plan = FramePlan::plain([128, 72], Effect::None);
    plan.overlays.push(Overlay {
        size: [size[0] as u32, size[1] as u32],
        rgba: raster.rgba(),
        position: raster.position(text.x, text.y),
        opacity: 0.37,
    });
    let actual = Compositor::cpu()
        .render(
            RgbaImage::from_pixel(128, 72, Rgba([30, 40, 50, 255])),
            &plan,
        )
        .unwrap();
    assert_eq!(actual, expected);
}
#[test]
fn hostile_plans_are_rejected_before_allocations_or_device_access() {
    let image = patterned(4, 4, false);
    let oversized = RgbaImage::from_raw(4, 4, vec![0; 128]).unwrap();
    assert!(
        Compositor::cpu()
            .render(oversized, &FramePlan::plain([4, 4], Effect::None))
            .is_err()
    );
    for canvas in [[0, 2], [2, 0], [u32::MAX, u32::MAX], [3840, 3840]] {
        assert!(
            Compositor::cpu()
                .render(image.clone(), &FramePlan::plain(canvas, Effect::None))
                .is_err()
        );
    }
    for amount in [f32::NAN, f32::INFINITY, -0.1, 1.01] {
        let mut plan = FramePlan::plain([4, 4], Effect::None);
        plan.fade = Some((Fade::White, amount));
        assert!(Compositor::cpu().render(image.clone(), &plan).is_err());
    }
    let pixels = vec![0; 64];
    for (size, position, opacity) in [
        ([u32::MAX, 2], [0, 0], 1.0),
        ([4, 4], [i64::MIN, 0], 1.0),
        ([4, 4], [0, 0], f32::NAN),
        ([4, 3], [0, 0], 1.0),
    ] {
        let mut plan = FramePlan::plain([4, 4], Effect::None);
        plan.overlays.push(Overlay {
            size,
            position,
            opacity,
            rgba: &pixels,
        });
        assert!(Compositor::cpu().render(image.clone(), &plan).is_err());
    }
    let mut plan = FramePlan::plain([4, 4], Effect::None);
    plan.overlays = vec![
        Overlay {
            size: [4, 4],
            position: [0, 0],
            opacity: 1.0,
            rgba: &pixels
        };
        MAX_LAYERS + 1
    ];
    assert!(Compositor::cpu().render(image, &plan).is_err());
    let ink = vec![0; 2 * 1024 * 1024 * 4];
    let mut plan = FramePlan::plain([4, 4], Effect::None);
    plan.overlays = vec![
        Overlay {
            size: [2048, 1024],
            position: [0, 0],
            opacity: 1.0,
            rgba: &ink,
        };
        3
    ];
    assert!(
        Compositor::cpu()
            .render(patterned(4, 4, false), &plan)
            .is_err()
    );
}
#[test]
fn preview_and_export_use_identical_canvas_look_fade_and_text_at_the_same_tick() {
    use clipcraft_time::Tick;
    let text = crate::SequenceText::new(Tick::ZERO, Tick::from_units(2, 1), "same frame");
    let recipe = PreviewRecipe {
        canvas: Some([128, 72]),
        source_start: Tick::from_units(1, 1),
        source_end: Tick::from_units(3, 1),
        timeline_start: Tick::ZERO,
        look: crate::SegmentLook {
            effect: Effect::Sepia,
            fade_in: Fade::White,
            fade_out: Fade::Black,
        },
        texts: vec![text],
    };
    let source_time = Tick::from_units(5, 4);
    let time = source_time - recipe.source_start;
    let image = patterned(160, 90, true);
    let preview = PreviewRenderer::default()
        .frame(image.clone(), source_time, &recipe)
        .unwrap();
    let export = crate::sequence::compose_frame(
        image,
        [128, 72],
        recipe.look.effect,
        recipe
            .look
            .fade_at(time, recipe.source_end - recipe.source_start),
        &recipe.texts,
        time,
        &mut Default::default(),
        &mut Compositor::cpu(),
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(preview.dimensions(), export.dimensions());
    assert!(
        preview
            .as_raw()
            .iter()
            .zip(export.as_raw())
            .all(|(&a, &b)| a.abs_diff(b) <= 1)
    );
    let invalid = PreviewRecipe {
        source_end: Tick::ZERO,
        ..Default::default()
    };
    assert!(
        PreviewRenderer::default()
            .frame(patterned(4, 4, false), Tick::ZERO, &invalid)
            .is_err()
    );
}
#[test]
fn automatic_backend_and_explicit_cpu_agree_without_requiring_a_gpu() {
    let image = patterned(37, 29, true);
    let mut plan = FramePlan::plain([24, 18], Effect::Vivid);
    plan.fade = Some((Fade::White, 0.37));
    let expected = Compositor::cpu().render(image.clone(), &plan).unwrap();
    let actual = Compositor::default().render(image, &plan).unwrap();
    assert!(
        expected
            .as_raw()
            .iter()
            .zip(actual.as_raw())
            .all(|(&a, &b)| a.abs_diff(b) <= 1)
    );
}

#[test]
fn a_failed_context_is_retired_and_the_cpu_reference_can_complete_the_frame() {
    let image = patterned(64, 48, false);
    let plan = FramePlan::plain([32, 24], Effect::Contrast);
    let Ok(gpu) = gpu::Gpu::open(true) else {
        assert!(Compositor::cpu().render(image, &plan).is_ok());
        eprintln!(
            "no software adapter: CPU fallback verified; device-loss branch requires compositor_verify setup"
        );
        return;
    };
    gpu.destroy();
    let mut context = Some(gpu);
    assert!(
        try_context(
            &mut context,
            &image,
            &plan,
            fit_size(image.dimensions(), plan.canvas)
        )
        .is_none()
    );
    assert!(context.is_none());
    assert!(Compositor::cpu().render(image, &plan).is_ok());
    eprintln!("actual software GPU context destroyed: retirement and CPU completion verified");
}
