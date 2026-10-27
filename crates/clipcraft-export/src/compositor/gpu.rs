use super::{Effect, Fade, FramePlan};
use crate::ExportError;
use image::RgbaImage;
use std::num::NonZeroU64;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;
const WAIT: Duration = Duration::from_millis(500);
fn unavailable() -> ExportError {
    ExportError::Unsupported("GPU compositor unavailable; CPU composition remains supported")
}
struct Buffer {
    buffer: wgpu::Buffer,
    bytes: u64,
}
impl Buffer {
    fn new(device: &wgpu::Device, bytes: u64, usage: wgpu::BufferUsages) -> Self {
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("clipcraft-frame"),
                size: bytes.max(4),
                usage,
                mapped_at_creation: false,
            }),
            bytes: bytes.max(4),
        }
    }
    fn ensure(&mut self, device: &wgpu::Device, bytes: u64, usage: wgpu::BufferUsages) {
        if self.bytes < bytes {
            *self = Self::new(device, bytes, usage);
        }
    }
}
pub(super) struct Gpu {
    pub description: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    base: wgpu::ComputePipeline,
    overlay: wgpu::ComputePipeline,
    source: Buffer,
    ink: Buffer,
    output: Buffer,
    readback: Buffer,
    uniform: Buffer,
    failed: Arc<AtomicBool>,
}
/// Held while a device is being opened.
static OPEN: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Gpu {
    pub fn open(software: bool) -> Result<Self, ExportError> {
        // Graphics drivers are not safe to start up from several threads at once: opening two
        // devices together crashes the process now and then. Opening is rare, so one at a time.
        let _one_at_a_time = OPEN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: software,
            compatible_surface: None,
        }))
        .map_err(|_| unavailable())?;
        if !software
            && matches!(
                adapter.get_info().device_type,
                wgpu::DeviceType::Cpu | wgpu::DeviceType::Other
            )
        {
            return Err(unavailable());
        }
        let description = format!(
            "{} ({:?}, {:?})",
            adapter.get_info().name,
            adapter.get_info().device_type,
            adapter.get_info().backend
        );
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ClipCraft compositor"),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        }))
        .map_err(|_| unavailable())?;
        let failed = Arc::new(AtomicBool::new(false));
        let flag = failed.clone();
        device.on_uncaptured_error(Arc::new(move |_error| flag.store(true, Ordering::Relaxed)));
        let flag = failed.clone();
        device
            .set_device_lost_callback(move |_reason, _message| flag.store(true, Ordering::Relaxed));
        let entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(64),
                },
                count: None,
            },
        ];
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ClipCraft original compositor"),
            source: wgpu::ShaderSource::Wgsl(include_str!("kernel.wgsl").into()),
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let base = pipeline("base");
        let overlay = pipeline("overlay");
        if failed.load(Ordering::Relaxed) {
            return Err(unavailable());
        }
        let source = Buffer::new(
            &device,
            4,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let ink = Buffer::new(
            &device,
            4,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let output = Buffer::new(
            &device,
            4,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let readback = Buffer::new(
            &device,
            4,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let uniform = Buffer::new(
            &device,
            256,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let mut gpu = Self {
            description,
            device,
            queue,
            layout,
            base,
            overlay,
            source,
            ink,
            output,
            readback,
            uniform,
            failed,
        };
        // Validate the fixed kernel against the CPU reference before selecting a hardware driver.
        if !software {
            let image = RgbaImage::from_fn(17, 13, |x, y| {
                image::Rgba([(x * 13) as u8, (y * 17) as u8, 99, ((x + y) * 7) as u8])
            });
            for effect in Effect::ALL {
                let mut plan = FramePlan::plain([12, 8], effect);
                plan.fade = Some((Fade::White, 0.37));
                let ink = [
                    255, 30, 90, 128, 20, 220, 10, 64, 0, 0, 0, 0, 90, 40, 230, 255,
                ];
                plan.overlays.push(super::Overlay {
                    size: [2, 2],
                    rgba: &ink,
                    position: [3, 1],
                    opacity: 0.63,
                });
                let fit = super::fit_size(image.dimensions(), plan.canvas);
                let expected = super::cpu_render(image.clone(), &plan, fit);
                let actual = gpu.render(&image, &plan, fit)?;
                if expected
                    .as_raw()
                    .iter()
                    .zip(actual.as_raw())
                    .any(|(&a, &b)| a.abs_diff(b) > 1)
                {
                    return Err(unavailable());
                }
            }
        }
        Ok(gpu)
    }
    #[cfg(test)]
    pub(super) fn destroy(&self) {
        self.device.destroy();
        self.failed.store(true, Ordering::Relaxed);
    }
    pub fn render(
        &mut self,
        image: &RgbaImage,
        plan: &FramePlan<'_>,
        fit: [u32; 2],
    ) -> Result<RgbaImage, ExportError> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(unavailable());
        }
        // Bound per-invocation work, not just total frame pixels, to avoid pathological downscales.
        if image.width().div_ceil(fit[0]) as u64 * image.height().div_ceil(fit[1]) as u64 > 4096 {
            return Err(unavailable());
        }
        let bytes = u64::from(plan.canvas[0]) * u64::from(plan.canvas[1]) * 4;
        self.source.ensure(
            &self.device,
            (image.as_raw().len() + (fit[0] as usize + fit[1] as usize) * 8) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        self.output.ensure(
            &self.device,
            bytes,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        self.readback.ensure(
            &self.device,
            bytes,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let mut ink = Vec::new();
        let mut params = Vec::with_capacity((plan.overlays.len() + 1) * 256);
        let code = match plan.effect {
            Effect::None => 0,
            Effect::BlackAndWhite => 1,
            Effect::Sepia => 2,
            Effect::Brighten => 3,
            Effect::Darken => 4,
            Effect::Contrast => 5,
            Effect::Vivid => 6,
            Effect::Invert => 7,
        };
        let (fade, amount) = plan.fade.map_or((0, 0.0), |(f, a)| {
            (
                match f {
                    Fade::None => 0,
                    Fade::Black => 1,
                    Fade::White => 2,
                },
                a,
            )
        });
        append(
            &mut params,
            &[
                image.width(),
                image.height(),
                plan.canvas[0],
                plan.canvas[1],
                fit[0],
                fit[1],
                (plan.canvas[0] - fit[0]) / 2,
                (plan.canvas[1] - fit[1]) / 2,
                code,
                fade,
                amount.to_bits(),
                (image.as_raw().len() / 4) as u32,
            ],
        );
        for layer in &plan.overlays {
            append(
                &mut params,
                &[
                    plan.canvas[0],
                    plan.canvas[1],
                    layer.size[0],
                    layer.size[1],
                    layer.position[0] as i32 as u32,
                    layer.position[1] as i32 as u32,
                    (ink.len() / 4) as u32,
                    layer.opacity.to_bits(),
                ],
            );
            ink.extend_from_slice(layer.rgba);
        }
        self.ink.ensure(
            &self.device,
            ink.len().max(4) as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        self.uniform.ensure(
            &self.device,
            params.len() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        self.queue
            .write_buffer(&self.source.buffer, 0, image.as_raw());
        let mut bounds = Vec::with_capacity((fit[0] as usize + fit[1] as usize) * 8);
        for (source, count) in [(image.width(), fit[0]), (image.height(), fit[1])] {
            let ratio = source as f32 / count as f32;
            for index in 0..count {
                let lowf = index as f32 * ratio;
                let highf = lowf + ratio;
                let low = (lowf.ceil() as u32).min(source - 1);
                let high = (highf.ceil() as u32).clamp(low + 1, source);
                bounds.extend_from_slice(&low.to_le_bytes());
                bounds.extend_from_slice(&high.to_le_bytes());
            }
        }
        self.queue
            .write_buffer(&self.source.buffer, image.as_raw().len() as u64, &bounds);
        if !ink.is_empty() {
            self.queue.write_buffer(&self.ink.buffer, 0, &ink);
        }
        self.queue.write_buffer(&self.uniform.buffer, 0, &params);
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.source.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.ink.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.output.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniform.buffer,
                        offset: 0,
                        size: NonZeroU64::new(64),
                    }),
                },
            ],
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.base);
            pass.set_bind_group(0, &group, &[0]);
            pass.dispatch_workgroups(plan.canvas[0].div_ceil(8), plan.canvas[1].div_ceil(8), 1);
            pass.set_pipeline(&self.overlay);
            for (index, layer) in plan.overlays.iter().enumerate() {
                pass.set_bind_group(0, &group, &[((index + 1) * 256) as u32]);
                pass.dispatch_workgroups(layer.size[0].div_ceil(8), layer.size[1].div_ceil(8), 1);
            }
        }
        encoder.copy_buffer_to_buffer(&self.output.buffer, 0, &self.readback.buffer, 0, bytes);
        let submission = self.queue.submit([encoder.finish()]);
        let slice = self.readback.buffer.slice(..bytes);
        let (sender, receiver) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.try_send(result.is_ok());
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(WAIT),
            })
            .map_err(|_| unavailable())?;
        if receiver.recv_timeout(WAIT) != Ok(true) || self.failed.load(Ordering::Relaxed) {
            return Err(unavailable());
        }
        let pixels = slice.get_mapped_range().to_vec();
        self.readback.buffer.unmap();
        RgbaImage::from_raw(plan.canvas[0], plan.canvas[1], pixels).ok_or_else(unavailable)
    }
}
fn append(out: &mut Vec<u8>, words: &[u32]) {
    let start = out.len();
    for word in words {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.resize(start + 256, 0);
}
