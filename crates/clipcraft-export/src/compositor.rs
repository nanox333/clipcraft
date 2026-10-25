//! One bounded pixel contract for editor preview and export. GPU use is optional.
use crate::{Effect, ExportError, Fade};
use image::{Rgba, RgbaImage};
use std::sync::{Mutex, OnceLock};

mod gpu;
mod preview;
pub use preview::{PreviewRecipe, PreviewRenderer};
pub const MAX_LAYERS: usize = 256;
pub const MAX_OVERLAY_BYTES: usize = 16 * 1024 * 1024;
const MAX_INPUT_PIXELS: u64 = 7680 * 4320;
const MAX_OUTPUT_PIXELS: u64 = 3840 * 2160;

#[derive(Clone, Copy)]
pub struct Overlay<'a> {
    pub size: [u32; 2],
    pub rgba: &'a [u8],
    pub position: [i64; 2],
    pub opacity: f32,
}
pub struct FramePlan<'a> {
    pub canvas: [u32; 2],
    pub effect: Effect,
    pub fade: Option<(Fade, f32)>,
    pub overlays: Vec<Overlay<'a>>,
}
impl FramePlan<'_> {
    pub fn plain(canvas: [u32; 2], effect: Effect) -> Self {
        Self {
            canvas,
            effect,
            fade: None,
            overlays: Vec::new(),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Backend {
    Cpu,
    Gpu,
    SoftwareGpu,
}
impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "wgpu",
            Self::SoftwareGpu => "wgpu-software",
        }
    }
}
#[derive(Default)]
pub struct Compositor {
    forced: Option<gpu::Gpu>,
    last: Option<Backend>,
    cpu_only: bool,
}
impl Compositor {
    pub fn cpu() -> Self {
        Self {
            cpu_only: true,
            ..Self::default()
        }
    }
    /// Explicit real software-adapter path for reproducible parity/benchmarks, never automatic.
    pub fn software_gpu() -> Result<Self, ExportError> {
        Ok(Self {
            forced: Some(
                std::panic::catch_unwind(|| gpu::Gpu::open(true)).unwrap_or(Err(
                    ExportError::Unsupported("software GPU initialization failed"),
                ))?,
            ),
            ..Self::default()
        })
    }
    pub fn adapter_description(&self) -> Option<&str> {
        self.forced.as_ref().map(|gpu| gpu.description.as_str())
    }
    pub fn backend(&self) -> Backend {
        self.last.unwrap_or(Backend::Cpu)
    }
    pub fn render(
        &mut self,
        image: RgbaImage,
        plan: &FramePlan<'_>,
    ) -> Result<RgbaImage, ExportError> {
        validate(&image, plan)?;
        let fit = fit_size(image.dimensions(), plan.canvas);
        let no_op = fit == [image.width(), image.height()]
            && plan.canvas == fit
            && plan.effect == Effect::None
            && plan.fade.is_none()
            && plan.overlays.is_empty()
            && image.pixels().all(|p| p[3] == 255);
        if self.forced.is_none() && no_op {
            self.last = Some(Backend::Cpu);
            return Ok(image);
        }
        if let Some(gpu) = &mut self.forced {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                gpu.render(&image, plan, fit)
            }))
            .unwrap_or(Err(ExportError::Unsupported(
                "explicit GPU composition failed",
            )))?;
            self.last = Some(Backend::SoftwareGpu);
            return Ok(result);
        }
        // No frame/driver wait behind another preview/export. The existing worker can use CPU.
        if !self.cpu_only
            && let Ok(mut shared) = shared().try_lock()
            && let Some(frame) = try_context(&mut shared, &image, plan, fit)
        {
            self.last = Some(Backend::Gpu);
            return Ok(frame);
        }
        self.last = Some(Backend::Cpu);
        Ok(cpu_render(image, plan, fit))
    }
}
fn try_context(
    context: &mut Option<gpu::Gpu>,
    image: &RgbaImage,
    plan: &FramePlan<'_>,
    fit: [u32; 2],
) -> Option<RgbaImage> {
    let gpu = context.as_mut()?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gpu.render(image, plan, fit)
    }));
    if let Ok(Ok(frame)) = result {
        return Some(frame);
    }
    *context = None;
    None
}
fn shared() -> &'static Mutex<Option<gpu::Gpu>> {
    static SHARED: OnceLock<Mutex<Option<gpu::Gpu>>> = OnceLock::new();
    SHARED.get_or_init(|| {
        Mutex::new(
            std::panic::catch_unwind(|| gpu::Gpu::open(false))
                .ok()
                .and_then(Result::ok),
        )
    })
}
fn invalid() -> ExportError {
    ExportError::Unsupported("invalid or oversized compositor frame/layers")
}
fn validate(image: &RgbaImage, plan: &FramePlan<'_>) -> Result<(), ExportError> {
    let [w, h] = plan.canvas;
    if image.as_raw().len() as u64 != u64::from(image.width()) * u64::from(image.height()) * 4
        || image.width() == 0
        || image.height() == 0
        || image.width() > 8192
        || image.height() > 8192
        || u64::from(image.width()) * u64::from(image.height()) > MAX_INPUT_PIXELS
        || !(2..=3840).contains(&w)
        || !(2..=3840).contains(&h)
        || u64::from(w) * u64::from(h) > MAX_OUTPUT_PIXELS
        || plan.overlays.len() > MAX_LAYERS
    {
        return Err(invalid());
    }
    if plan
        .fade
        .is_some_and(|(_, a)| !a.is_finite() || !(0.0..=1.0).contains(&a))
    {
        return Err(invalid());
    }
    let mut total = 0usize;
    for layer in &plan.overlays {
        let pixels = u64::from(layer.size[0]) * u64::from(layer.size[1]);
        if layer.size.contains(&0)
            || layer.size.iter().any(|&n| n > 8192)
            || pixels > 2 * 1024 * 1024
            || pixels * 4 != layer.rgba.len() as u64
            || !layer.opacity.is_finite()
            || !(0.0..=1.0).contains(&layer.opacity)
            || layer.position.iter().any(|&n| !(-8192..=8192).contains(&n))
        {
            return Err(invalid());
        }
        total = total.checked_add(layer.rgba.len()).ok_or_else(invalid)?;
        if total > MAX_OVERLAY_BYTES {
            return Err(invalid());
        }
    }
    Ok(())
}
/// Aspect-preserving fit, no upscale; all division is bounded integer arithmetic.
pub(crate) fn fit_size(source: (u32, u32), canvas: [u32; 2]) -> [u32; 2] {
    let (w, h) = source;
    let [cw, ch] = canvas;
    if w <= cw && h <= ch {
        return [w, h];
    }
    if u64::from(w) * u64::from(ch) > u64::from(h) * u64::from(cw) {
        [
            w.min(cw),
            ((u64::from(h) * u64::from(w.min(cw))) / u64::from(w)).max(1) as u32,
        ]
    } else {
        [
            ((u64::from(w) * u64::from(h.min(ch))) / u64::from(h)).max(1) as u32,
            h.min(ch),
        ]
    }
}
fn cpu_render(mut image: RgbaImage, plan: &FramePlan<'_>, fit: [u32; 2]) -> RgbaImage {
    if image.dimensions() != (fit[0], fit[1]) {
        image = image::imageops::thumbnail(&image, fit[0], fit[1]);
    }
    plan.effect.apply_rgba(&mut image);
    let mut canvas = if image.dimensions() == (plan.canvas[0], plan.canvas[1])
        && image.pixels().all(|p| p[3] == 255)
    {
        image
    } else {
        let mut canvas =
            RgbaImage::from_pixel(plan.canvas[0], plan.canvas[1], Rgba([0, 0, 0, 255]));
        image::imageops::overlay(
            &mut canvas,
            &image,
            i64::from((plan.canvas[0] - fit[0]) / 2),
            i64::from((plan.canvas[1] - fit[1]) / 2),
        );
        canvas
    };
    if let Some((fade, amount)) = plan.fade {
        crate::sequence::blend_fade(&mut canvas, fade, amount);
    }
    for layer in &plan.overlays {
        draw_overlay(&mut canvas, *layer);
    }
    canvas
}
pub(crate) fn draw_overlay(canvas: &mut RgbaImage, layer: Overlay<'_>) {
    for row in 0..layer.size[1] {
        let py = layer.position[1] + i64::from(row);
        if py < 0 || py >= i64::from(canvas.height()) {
            continue;
        }
        for col in 0..layer.size[0] {
            let px = layer.position[0] + i64::from(col);
            if px < 0 || px >= i64::from(canvas.width()) {
                continue;
            }
            let index = (row as usize * layer.size[0] as usize + col as usize) * 4;
            let Some(source) = layer.rgba.get(index..index + 4) else {
                continue;
            };
            let amount = f32::from(source[3]) / 255.0 * layer.opacity;
            let pixel = canvas.get_pixel_mut(px as u32, py as u32);
            for (channel, &ink) in pixel.0.iter_mut().take(3).zip(source) {
                *channel = (f32::from(*channel) * (1.0 - amount) + f32::from(ink) * amount)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests;
