//! Bounded owned D3D11 capture textures and GPU color conversion (ADR 0014).
#![allow(unsafe_code)]
#![warn(clippy::undocumented_unsafe_blocks)]

use clipcraft_capture::{CaptureError, CaptureFrame, MAX_CAPTURE_EDGE, NativeCaptureFrame};
use clipcraft_record::SurfaceFit;
use clipcraft_time::Tick;
use std::any::Any;
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Media::MediaFoundation::{IMFDXGIBuffer, IMFSample};
use windows::core::Interface;
use windows_capture::frame::Frame;

const MAX_SURFACES: usize = 4;
const MAX_POOL_BYTES: usize = 128 * 1024 * 1024;

fn surface_limit(desc: &D3D11_TEXTURE2D_DESC) -> Result<usize, String> {
    if desc.Width == 0
        || desc.Height == 0
        || desc.Width > MAX_CAPTURE_EDGE
        || desc.Height > MAX_CAPTURE_EDGE
        || desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
        || desc.MipLevels != 1
        || desc.ArraySize != 1
        || desc.SampleDesc.Count != 1
    {
        return Err("native capture requires bounded single-plane SDR BGRA textures".to_owned());
    }
    let bytes = (desc.Width as usize)
        .checked_mul(desc.Height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("native surface is too large")?;
    let limit = (MAX_POOL_BYTES / bytes).min(MAX_SURFACES);
    // Latest always retains a lease. With only one slot the callback could never replace
    // that image, so decline native capture instead of silently repeating the first frame.
    if limit < 2 {
        return Err("native capture needs two surfaces within the GPU budget".to_owned());
    }
    Ok(limit)
}

static GPU_COPIES: AtomicU64 = AtomicU64::new(0);
static READBACKS: AtomicU64 = AtomicU64::new(0);
static NATIVE_INPUTS: AtomicU64 = AtomicU64::new(0);
static CPU_INPUTS: AtomicU64 = AtomicU64::new(0);
static POOL_DROPS: AtomicU64 = AtomicU64::new(0);
static FRESH_READS: AtomicU64 = AtomicU64::new(0);
static REPEATED_READS: AtomicU64 = AtomicU64::new(0);
static FEED_US: AtomicU64 = AtomicU64::new(0);
static CONVERT_US: AtomicU64 = AtomicU64::new(0);
static OUTPUT_US: AtomicU64 = AtomicU64::new(0);
static WAIT_US: AtomicU64 = AtomicU64::new(0);
static INPUT_US: AtomicU64 = AtomicU64::new(0);
static EVENT_US: AtomicU64 = AtomicU64::new(0);
static MF_INPUT_RATE: AtomicU64 = AtomicU64::new(0);
static MF_OUTPUT_RATE: AtomicU64 = AtomicU64::new(0);
static MF_MAX_MBPS: AtomicU64 = AtomicU64::new(0);
static MF_STREAM_CHANGES: AtomicU64 = AtomicU64::new(0);
static MF_OUTPUT_PICTURES: AtomicU64 = AtomicU64::new(0);

pub(crate) fn media_types(input: u64, output: u64, max_mbps: u32) {
    MF_INPUT_RATE.store(input, Ordering::Relaxed);
    MF_OUTPUT_RATE.store(output, Ordering::Relaxed);
    MF_MAX_MBPS.store(u64::from(max_mbps & 0x0fff_ffff), Ordering::Relaxed);
}
pub(crate) fn stream_changed() {
    MF_STREAM_CHANGES.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn output_picture() {
    MF_OUTPUT_PICTURES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) struct Timer(&'static AtomicU64, std::time::Instant);
impl Drop for Timer {
    fn drop(&mut self) {
        self.0.fetch_add(
            u64::try_from(self.1.elapsed().as_micros()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }
}
pub(crate) fn measure_feed() -> Timer {
    Timer(&FEED_US, std::time::Instant::now())
}
pub(crate) fn measure_conversion() -> Timer {
    Timer(&CONVERT_US, std::time::Instant::now())
}
pub(crate) fn measure_output() -> Timer {
    Timer(&OUTPUT_US, std::time::Instant::now())
}
pub(crate) fn measure_wait() -> Timer {
    Timer(&WAIT_US, std::time::Instant::now())
}
pub(crate) fn measure_input() -> Timer {
    Timer(&INPUT_US, std::time::Instant::now())
}
pub(crate) fn measure_event() -> Timer {
    Timer(&EVENT_US, std::time::Instant::now())
}

pub(crate) fn readback_count() {
    READBACKS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn native_input_count() {
    NATIVE_INPUTS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn cpu_input_count() {
    CPU_INPUTS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn capture_read(fresh: bool) {
    if fresh { &FRESH_READS } else { &REPEATED_READS }.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn stats() -> crate::CapturePipelineStats {
    let rate = |packed: u64| ((packed >> 32) as u32, packed as u32);
    crate::CapturePipelineStats {
        gpu_copies: GPU_COPIES.load(Ordering::Relaxed),
        cpu_readbacks: READBACKS.load(Ordering::Relaxed),
        native_encoder_inputs: NATIVE_INPUTS.load(Ordering::Relaxed),
        cpu_encoder_inputs: CPU_INPUTS.load(Ordering::Relaxed),
        pool_drops: POOL_DROPS.load(Ordering::Relaxed),
        fresh_capture_reads: FRESH_READS.load(Ordering::Relaxed),
        repeated_capture_reads: REPEATED_READS.load(Ordering::Relaxed),
        feed_us: FEED_US.load(Ordering::Relaxed),
        conversion_us: CONVERT_US.load(Ordering::Relaxed),
        output_us: OUTPUT_US.load(Ordering::Relaxed),
        input_wait_us: WAIT_US.load(Ordering::Relaxed),
        input_submit_us: INPUT_US.load(Ordering::Relaxed),
        event_us: EVENT_US.load(Ordering::Relaxed),
        mf_input_rate: rate(MF_INPUT_RATE.load(Ordering::Relaxed)),
        mf_output_rate: rate(MF_OUTPUT_RATE.load(Ordering::Relaxed)),
        mf_max_macroblocks_per_second: MF_MAX_MBPS.load(Ordering::Relaxed) as u32,
        mf_stream_changes: MF_STREAM_CHANGES.load(Ordering::Relaxed),
        mf_output_pictures: MF_OUTPUT_PICTURES.load(Ordering::Relaxed),
    }
}

struct Slot {
    texture: ID3D11Texture2D,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    width: u32,
    height: u32,
    /// A CPU-readable copy target kept between readbacks: creating one per picture is slow.
    staging: std::sync::Mutex<Option<ID3D11Texture2D>>,
}

/// An immutable lease: the callback cannot overwrite this slot while any consumer owns it.
#[derive(Clone)]
pub(crate) struct Surface {
    slot: Arc<Slot>,
    captured_at: Tick,
}

impl Surface {
    pub(crate) fn device(&self) -> &ID3D11Device {
        &self.slot.device
    }
    pub(crate) fn context(&self) -> &ID3D11DeviceContext {
        &self.slot.context
    }
    pub(crate) fn texture(&self) -> &ID3D11Texture2D {
        &self.slot.texture
    }
}

#[derive(Default)]
pub(crate) struct Pool {
    slots: Vec<Arc<Slot>>,
}

impl Pool {
    pub(crate) fn copy(
        &mut self,
        frame: &Frame<'_>,
        captured_at: Tick,
    ) -> Result<Option<Surface>, String> {
        let desc = frame.desc();
        if frame.width() != desc.Width || frame.height() != desc.Height {
            return Err("capture content and texture dimensions differ".to_owned());
        }
        self.copy_texture(
            frame.device(),
            frame.device_context(),
            frame.as_raw_texture(),
            desc,
            captured_at,
        )
    }

    fn copy_texture(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        texture: &ID3D11Texture2D,
        desc: &D3D11_TEXTURE2D_DESC,
        captured_at: Tick,
    ) -> Result<Option<Surface>, String> {
        let limit = surface_limit(desc)?;
        if self.slots.first().is_some_and(|s| {
            s.width != desc.Width || s.height != desc.Height || &s.device != device
        }) {
            self.slots.clear();
        }
        let free = self
            .slots
            .iter()
            .position(|slot| Arc::strong_count(slot) == 1);
        let index = match free {
            Some(index) => index,
            None if self.slots.len() < limit => {
                let multithread: ID3D11Multithread = context.cast().map_err(|e| e.to_string())?;
                // SAFETY: the live context is shared by capture, readback and encode threads;
                // D3D's internal serialization protects every immediate-context call.
                unsafe {
                    let _ = multithread.SetMultithreadProtected(true);
                }
                let texture_desc = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: desc.Format,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                    ..Default::default()
                };
                let mut texture = None;
                // SAFETY: validated bounded dimensions and a valid writable out-parameter.
                unsafe { device.CreateTexture2D(&texture_desc, None, Some(&mut texture)) }
                    .map_err(|e| e.to_string())?;
                self.slots.push(Arc::new(Slot {
                    texture: texture.ok_or("no capture texture")?,
                    device: device.clone(),
                    context: context.clone(),
                    width: desc.Width,
                    height: desc.Height,
                    staging: std::sync::Mutex::new(None),
                }));
                self.slots.len() - 1
            }
            None => {
                POOL_DROPS.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
        };
        let slot = self.slots.get(index).ok_or("invalid capture slot")?;
        // SAFETY: descriptions match and the slot has no outstanding lease. The source is valid
        // through this callback. A GPU copy owns the pixels beyond capture frame-pool reuse.
        unsafe {
            slot.context.CopyResource(&slot.texture, texture);
        }
        GPU_COPIES.fetch_add(1, Ordering::Relaxed);
        Ok(Some(Surface {
            slot: Arc::clone(slot),
            captured_at,
        }))
    }
}

impl NativeCaptureFrame for Surface {
    fn width(&self) -> u32 {
        self.slot.width
    }
    fn height(&self) -> u32 {
        self.slot.height
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn readback(&mut self) -> Result<CaptureFrame, CaptureError> {
        let s = &self.slot;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: s.width,
            Height: s.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        };
        let cached = s
            .staging
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let texture = match cached {
            Some(texture) => texture,
            None => {
                let mut texture = None;
                // SAFETY: a bounded CPU-readable staging texture, owned until after Unmap.
                unsafe { s.device.CreateTexture2D(&desc, None, Some(&mut texture)) }
                    .map_err(|_| CaptureError::Failed)?;
                texture.ok_or(CaptureError::Failed)?
            }
        };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: identical texture dimensions; the live context is internally serialized.
        unsafe {
            s.context.CopyResource(&texture, &s.texture);
            s.context
                .Map(&texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }
        .map_err(|_| CaptureError::Failed)?;
        readback_count();
        let stride = s.width as usize * 4;
        let result = if mapped.pData.is_null() || (mapped.RowPitch as usize) < stride {
            Err(CaptureError::BadFrame)
        } else {
            // Filled row by row without zeroing the buffer first, which would touch all of it twice.
            let mut bgra = Vec::with_capacity(stride * s.height as usize);
            for row in 0..s.height as usize {
                // SAFETY: Map provides Height rows of RowPitch bytes, checked above. Only the
                // visible bytes are read and copied into the output.
                let input = unsafe {
                    std::slice::from_raw_parts(
                        mapped
                            .pData
                            .cast::<u8>()
                            .add(row * mapped.RowPitch as usize),
                        stride,
                    )
                };
                bgra.extend_from_slice(input);
            }
            Ok(CaptureFrame {
                width: s.width,
                height: s.height,
                stride,
                bgra,
                captured_at: self.captured_at,
            })
        };
        // SAFETY: balances this successful Map on every path, before releasing the texture.
        unsafe {
            s.context.Unmap(&texture, 0);
        }
        *s.staging
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(texture);
        result
    }
}

pub(crate) struct Converter {
    pub(crate) device: ID3D11Device,
    video: ID3D11VideoDevice,
    context: ID3D11VideoContext,
    processor: ID3D11VideoProcessor,
    enumerator: ID3D11VideoProcessorEnumerator,
    /// The surface size this converter was built for.
    input: (u32, u32),
    /// The encoded picture size.
    output: (u32, u32),
}

impl Converter {
    /// Builds a converter from `surface`-sized BGRA to an NV12 picture of `output` size; the card
    /// scales when the sizes differ.
    pub(crate) fn new(surface: &Surface, fps: u32, output: (u32, u32)) -> Result<Self, String> {
        if output.0 == 0
            || output.1 == 0
            || output.0 > MAX_CAPTURE_EDGE
            || output.1 > MAX_CAPTURE_EDGE
        {
            return Err("encoded picture size is out of range".to_owned());
        }
        let video: ID3D11VideoDevice = surface.device().cast().map_err(|e| e.to_string())?;
        let context: ID3D11VideoContext = surface.context().cast().map_err(|e| e.to_string())?;
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: DXGI_RATIONAL {
                Numerator: fps,
                Denominator: 1,
            },
            InputWidth: surface.width(),
            InputHeight: surface.height(),
            OutputFrameRate: DXGI_RATIONAL {
                Numerator: fps,
                Denominator: 1,
            },
            OutputWidth: output.0,
            OutputHeight: output.1,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        };
        // SAFETY: descriptors use the validated surface dimensions and non-zero encoder FPS.
        let enumerator =
            unsafe { video.CreateVideoProcessorEnumerator(&desc) }.map_err(|e| e.to_string())?;
        // SAFETY: index zero selects the first processor advertised by this enumerator.
        let processor =
            unsafe { video.CreateVideoProcessor(&enumerator, 0) }.map_err(|e| e.to_string())?;
        // SAFETY: live processor and context. Input is full-range RGB; output is limited-range
        // BT.709 YUV (YCbCr_Matrix bit 2, Nominal_Range bits 4..5 = 1).
        unsafe {
            context.VideoProcessorSetStreamColorSpace(
                &processor,
                0,
                &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 },
            );
            context.VideoProcessorSetOutputColorSpace(
                &processor,
                &D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
                    _bitfield: 4 | (1 << 4),
                },
            );
        }
        Ok(Self {
            device: surface.device().clone(),
            video,
            context,
            processor,
            enumerator,
            input: (surface.width(), surface.height()),
            output,
        })
    }

    /// True when `surface` has the size this converter was built for.
    pub(crate) fn accepts(&self, surface: &Surface) -> bool {
        self.input == (surface.width(), surface.height())
    }

    pub(crate) fn convert(
        &self,
        surface: &Surface,
        sample: &IMFSample,
        fit: SurfaceFit,
    ) -> Result<(), String> {
        if surface.device() != &self.device {
            return Err("capture GPU device changed".to_owned());
        }
        // SAFETY: allocator sample contains one DXGI buffer; GetResource transfers one owned COM
        // reference to a texture; the sample is retained through the blit and encoder submission.
        let (texture, subresource) = unsafe {
            let buffer: IMFDXGIBuffer = sample
                .GetBufferByIndex(0)
                .map_err(|e| e.to_string())?
                .cast()
                .map_err(|e| e.to_string())?;
            let mut raw = std::ptr::null_mut();
            buffer
                .GetResource(&ID3D11Texture2D::IID, &mut raw)
                .map_err(|e| e.to_string())?;
            if raw.is_null() {
                return Err("allocator returned no GPU texture".to_owned());
            }
            (
                ID3D11Texture2D::from_raw(raw),
                buffer.GetSubresourceIndex().map_err(|e| e.to_string())?,
            )
        };
        if subresource != 0 {
            return Err("texture-array encoder samples are unsupported".to_owned());
        }
        self.convert_texture(surface, &texture, fit)
    }

    fn convert_texture(
        &self,
        surface: &Surface,
        texture: &ID3D11Texture2D,
        fit: SurfaceFit,
    ) -> Result<(), String> {
        if surface.device() != &self.device {
            return Err("capture GPU device changed".to_owned());
        }
        if !self.accepts(surface) {
            return Err("capture surface size changed".to_owned());
        }
        let rect = |r: clipcraft_capture::reframe::PixelRect| -> Result<RECT, String> {
            let edge = |start: u32, length: u32| {
                start
                    .checked_add(length)
                    .and_then(|end| Some((i32::try_from(start).ok()?, i32::try_from(end).ok()?)))
                    .ok_or_else(|| "picture placement is out of range".to_owned())
            };
            let (left, right) = edge(r.x, r.width)?;
            let (top, bottom) = edge(r.y, r.height)?;
            Ok(RECT {
                left,
                top,
                right,
                bottom,
            })
        };
        let (source, destination) = (rect(fit.source)?, rect(fit.destination)?);
        let target = RECT {
            left: 0,
            top: 0,
            right: i32::try_from(self.output.0).map_err(|e| e.to_string())?,
            bottom: i32::try_from(self.output.1).map_err(|e| e.to_string())?,
        };
        let black = D3D11_VIDEO_COLOR {
            Anonymous: D3D11_VIDEO_COLOR_0 {
                RGBA: D3D11_VIDEO_COLOR_RGBA {
                    R: 0.0,
                    G: 0.0,
                    B: 0.0,
                    A: 1.0,
                },
            },
        };
        // SAFETY: live processor and context; the rectangles are valid for the call and describe
        // the validated surface (source) and encoded picture (destination, target). Anything the
        // destination does not cover is filled with the black background, so a reused encoder
        // texture never shows an older picture in a letterbox bar.
        unsafe {
            self.context
                .VideoProcessorSetStreamSourceRect(&self.processor, 0, true, Some(&source));
            self.context.VideoProcessorSetStreamDestRect(
                &self.processor,
                0,
                true,
                Some(&destination),
            );
            self.context
                .VideoProcessorSetOutputTargetRect(&self.processor, true, Some(&target));
            self.context
                .VideoProcessorSetOutputBackgroundColor(&self.processor, false, &black);
        }
        let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        let (mut input, mut output) = (None, None);
        // SAFETY: live device-owned BGRA input and allocator-owned NV12 output, matching the
        // enumerator; descriptors select mip zero of non-array textures.
        unsafe {
            self.video
                .CreateVideoProcessorInputView(
                    surface.texture(),
                    &self.enumerator,
                    &input_desc,
                    Some(&mut input),
                )
                .map_err(|e| e.to_string())?;
            self.video
                .CreateVideoProcessorOutputView(
                    texture,
                    &self.enumerator,
                    &output_desc,
                    Some(&mut output),
                )
                .map_err(|e| e.to_string())?;
        }
        let output = output.ok_or("no GPU output view")?;
        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: ManuallyDrop::new(Some(input.ok_or("no GPU input view")?)),
            ..Default::default()
        };
        // SAFETY: one enabled stream, valid input/output views, internally serialized context.
        let result = unsafe {
            self.context.VideoProcessorBlt(
                &self.processor,
                &output,
                0,
                std::slice::from_ref(&stream),
            )
        }
        .map_err(|e| e.to_string());
        // SAFETY: release the single owned input-view reference held by this ManuallyDrop field.
        unsafe {
            ManuallyDrop::drop(&mut stream.pInputSurface);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn description(width: u32, height: u32) -> D3D11_TEXTURE2D_DESC {
        D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            ..Default::default()
        }
    }

    #[test]
    fn native_pool_rejects_hostile_or_incompatible_descriptions() {
        for (width, height) in [(0, 1), (1, 0), (u32::MAX, 1), (1, u32::MAX), (16384, 16384)] {
            assert!(surface_limit(&description(width, height)).is_err());
        }
        let mut desc = description(1920, 1080);
        assert_eq!(surface_limit(&desc).unwrap(), 4);
        desc.Format = DXGI_FORMAT_NV12;
        assert!(surface_limit(&desc).is_err());
        desc = description(1920, 1080);
        desc.ArraySize = 2;
        assert!(surface_limit(&desc).is_err());
        desc.ArraySize = 1;
        desc.SampleDesc.Count = 4;
        assert!(surface_limit(&desc).is_err());
        assert_eq!(surface_limit(&description(4096, 4096)).unwrap(), 2);
        assert!(surface_limit(&description(7680, 4096)).is_err());
        assert!(surface_limit(&description(7680, 7680)).is_err());
    }

    #[test]
    #[ignore = "requires a Windows D3D11 device; run explicitly for platform evidence"]
    fn gpu_leases_preserve_pixels_exhaust_reuse_and_survive_resize() {
        use windows::Win32::Foundation::HMODULE;
        use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
        let (mut device, mut context) = (None, None);
        // SAFETY: documented device creation with valid writable out-parameters.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .unwrap();
        }
        let device = device.unwrap();
        let context = context.unwrap();
        let desc = description(4, 4);
        let create = |desc: &D3D11_TEXTURE2D_DESC, pixel: [u8; 4]| {
            let pixels = pixel.repeat((desc.Width * desc.Height) as usize);
            let data = D3D11_SUBRESOURCE_DATA {
                pSysMem: pixels.as_ptr().cast(),
                SysMemPitch: desc.Width * 4,
                SysMemSlicePitch: 0,
            };
            let mut texture = None;
            // SAFETY: one subresource, complete tightly packed pixel buffer valid for this call.
            unsafe {
                device
                    .CreateTexture2D(desc, Some(&data), Some(&mut texture))
                    .unwrap();
            }
            texture.unwrap()
        };
        let red = create(&desc, [0, 0, 255, 255]);
        let blue = create(&desc, [255, 0, 0, 255]);
        let mut pool = Pool::default();
        let mut first = pool
            .copy_texture(&device, &context, &red, &desc, Tick(7))
            .unwrap()
            .unwrap();
        let mut leases = Vec::new();
        for _ in 1..MAX_SURFACES {
            leases.push(
                pool.copy_texture(&device, &context, &blue, &desc, Tick(8))
                    .unwrap()
                    .unwrap(),
            );
        }
        assert!(
            pool.copy_texture(&device, &context, &blue, &desc, Tick(9))
                .unwrap()
                .is_none()
        );
        assert_eq!(first.readback().unwrap().bgra, [0, 0, 255, 255].repeat(16));
        // Exercise the same GPU video-processor conversion used by allocator samples, then
        // compare an explicit test-only readback with the portable limited-range converter.
        let color_desc = description(64, 64);
        let mut nv12_desc = color_desc;
        nv12_desc.Format = DXGI_FORMAT_NV12;
        let mut nv12 = None;
        // SAFETY: bounded NV12 render target with valid writable out-parameter.
        unsafe {
            device
                .CreateTexture2D(&nv12_desc, None, Some(&mut nv12))
                .unwrap();
        }
        let nv12 = nv12.unwrap();
        let mut staging_desc = nv12_desc;
        staging_desc.Usage = D3D11_USAGE_STAGING;
        staging_desc.BindFlags = 0;
        staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        let mut staging = None;
        // SAFETY: matching bounded NV12 CPU-readable staging texture.
        unsafe {
            device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .unwrap();
        }
        let staging = staging.unwrap();
        for rgba in [
            [0, 0, 0, 255],
            [255, 255, 255, 255],
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [128, 128, 128, 255],
        ] {
            let input = create(&color_desc, [rgba[2], rgba[1], rgba[0], rgba[3]]);
            let surface = pool
                .copy_texture(&device, &context, &input, &color_desc, Tick(12))
                .unwrap()
                .unwrap();
            let converter = Converter::new(&surface, 30, (64, 64)).unwrap();
            let full = clipcraft_capture::reframe::PixelRect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            };
            converter
                .convert_texture(
                    &surface,
                    &nv12,
                    SurfaceFit {
                        source: full,
                        destination: full,
                    },
                )
                .unwrap();
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            // SAFETY: identical NV12 descriptors; Map synchronizes the copy before CPU access.
            unsafe {
                context.CopyResource(&staging, &nv12);
                context
                    .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                    .unwrap();
            }
            let mut actual = Vec::new();
            if !mapped.pData.is_null() && mapped.RowPitch >= 64 {
                for row in 0..96 {
                    // SAFETY: Map exposes 64 Y rows plus 32 interleaved UV rows of RowPitch bytes.
                    actual.extend_from_slice(unsafe {
                        std::slice::from_raw_parts(
                            mapped
                                .pData
                                .cast::<u8>()
                                .add(row * mapped.RowPitch as usize),
                            64,
                        )
                    });
                }
            }
            // SAFETY: balances the successful Map before assertions can panic.
            unsafe {
                context.Unmap(&staging, 0);
            }
            let mut expected = Vec::new();
            crate::h264_stream::rgba_to_nv12(&rgba.repeat(64 * 64), 64, 64, &mut expected, 1)
                .unwrap();
            assert_eq!(actual.len(), expected.len());
            assert!(
                actual
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| a.abs_diff(*b) <= 3),
                "GPU NV12 differs from BT.709 CPU conversion for {rgba:?}: YUV={:?}, expected={:?}",
                [actual[0], actual[4096], actual[4097]],
                [expected[0], expected[4096], expected[4097]]
            );
        }
        // Scaling and letterboxing on the card: a red 32x32 picture is drawn 64 wide and 32 tall in
        // the middle of a 64x64 output, so the bars above and below are black and the middle red.
        let small_desc = description(32, 32);
        let small = create(&small_desc, [0, 0, 255, 255]);
        let surface = pool
            .copy_texture(&device, &context, &small, &small_desc, Tick(13))
            .unwrap()
            .unwrap();
        let converter = Converter::new(&surface, 30, (64, 64)).unwrap();
        assert!(converter.accepts(&surface));
        let rect = |x, y, width, height| clipcraft_capture::reframe::PixelRect {
            x,
            y,
            width,
            height,
        };
        converter
            .convert_texture(
                &surface,
                &nv12,
                SurfaceFit {
                    source: rect(0, 0, 32, 32),
                    destination: rect(0, 16, 64, 32),
                },
            )
            .unwrap();
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: identical NV12 descriptors; Map synchronizes the copy before CPU access.
        unsafe {
            context.CopyResource(&staging, &nv12);
            context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .unwrap();
        }
        let luma = |row: usize| {
            // SAFETY: Map exposes 64 luma rows of RowPitch bytes; the row and column are inside.
            unsafe {
                *mapped
                    .pData
                    .cast::<u8>()
                    .add(row * mapped.RowPitch as usize + 32)
            }
        };
        let (bar, picture) = (luma(4), luma(32));
        // SAFETY: balances the successful Map.
        unsafe {
            context.Unmap(&staging, 0);
        }
        assert!(bar <= 24, "the letterbox bar is black, not {bar}");
        assert!(
            (50..=80).contains(&picture),
            "the scaled red picture has the luma of red, not {picture}"
        );
        drop(leases.pop());
        let mut reused = pool
            .copy_texture(&device, &context, &red, &desc, Tick(10))
            .unwrap()
            .unwrap();
        assert_eq!(reused.readback().unwrap().captured_at, Tick(10));
        let resized = description(8, 4);
        let texture = create(&resized, [255, 0, 0, 255]);
        let mut next = pool
            .copy_texture(&device, &context, &texture, &resized, Tick(11))
            .unwrap()
            .unwrap();
        assert_eq!(next.readback().unwrap().bgra, [255, 0, 0, 255].repeat(32));
        assert_eq!(first.readback().unwrap().bgra, [0, 0, 255, 255].repeat(16));
    }
}
