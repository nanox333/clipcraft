//! A hardware H.264 encoder through Windows Media Foundation, for graphics cards FilmCraft has no
//! encoder for: AMD, Intel, and NVIDIA all install a hardware H.264 encoder transform that Windows
//! lists as a Media Foundation transform (MFT).
//!
//! Platform `unsafe` boundaries are recorded in ADRs 0007 and 0014. Every block documents its
//! invariants, and nothing in it can reach the
//! rest of the app except through the safe [`clipcraft_record::HardwareEncoder`] trait.
//!
//! Media Foundation's entry points are loaded from `mfplat.dll` when an encoder is first asked
//! for, not linked, so ClipCraft still starts on Windows editions that ship without the Media
//! Feature Pack. The interface objects themselves are called through their virtual tables.
//!
//! AMD generated-window hardware evidence is recorded in `docs/windows-performance.md`.

#![allow(unsafe_code)]
#![warn(clippy::undocumented_unsafe_blocks)]

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use clipcraft_capture::NativeCaptureFrame;
use clipcraft_record::{HardwareEncoder, HardwarePacket, HardwareRequest};
use windows::Win32::Foundation::{FreeLibrary, HMODULE, VARIANT_TRUE};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{
    COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4};
use windows::core::{GUID, HRESULT, Interface, PCSTR, PWSTR, w};

use crate::d3d11::{Converter, Surface};
use crate::h264_stream::{annexb_to_length_prefixed, build_avcc, rgba_to_nv12, split_nal_units};

/// How long to wait for an encoder to accept a picture before calling it stuck.
const STALL_LIMIT: Duration = Duration::from_secs(2);

type StartupFn = unsafe extern "system" fn(u32, u32) -> HRESULT;
type ShutdownFn = unsafe extern "system" fn() -> HRESULT;
type CreateFn = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type CreateBufferFn = unsafe extern "system" fn(u32, *mut *mut c_void) -> HRESULT;
type CreateManagerFn = unsafe extern "system" fn(*mut u32, *mut *mut c_void) -> HRESULT;
type CreateAllocatorFn = unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
type EnumFn = unsafe extern "system" fn(
    GUID,
    u32,
    *const MFT_REGISTER_TYPE_INFO,
    *const MFT_REGISTER_TYPE_INFO,
    *mut *mut *mut c_void,
    *mut u32,
) -> HRESULT;

/// The few `mfplat.dll` functions used, loaded once.
struct Api {
    _module: HMODULE,
    startup: StartupFn,
    shutdown: ShutdownFn,
    create_media_type: CreateFn,
    create_sample: CreateFn,
    create_memory_buffer: CreateBufferFn,
    create_dxgi_manager: CreateManagerFn,
    create_video_allocator: CreateAllocatorFn,
    enum_transforms: EnumFn,
}

// SAFETY: the module handle and function pointers are plain values; `mfplat.dll` stays loaded for
// the life of the process (it is never freed) and its functions are documented as thread-safe.
unsafe impl Send for Api {}
// SAFETY: see above; nothing here is mutated after loading.
unsafe impl Sync for Api {}

fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(load_api).as_ref().map_err(Clone::clone)
}

// Each function pointer is cast to the type of the `Api` field it fills, which is how the
// signature is checked; spelling the type again at every `transmute` adds nothing.
#[allow(clippy::missing_transmute_annotations)]
fn load_api() -> Result<Api, String> {
    // SAFETY: `LoadLibraryW` is given a valid, NUL-terminated wide string literal.
    let module = unsafe { LoadLibraryW(w!("mfplat.dll")) }
        .map_err(|_| "Windows Media Foundation is not installed (mfplat.dll)".to_owned())?;
    macro_rules! symbol {
        ($name:literal) => {{
            // SAFETY: `module` is a valid loaded module and the name is a NUL-terminated string.
            let address = unsafe { GetProcAddress(module, PCSTR(concat!($name, "\0").as_ptr())) };
            match address {
                // SAFETY: the symbol has the documented signature of the type it is cast to.
                Some(function) => unsafe { std::mem::transmute(function) },
                None => {
                    // SAFETY: `module` was returned by `LoadLibraryW` above and is not used again.
                    let _ = unsafe { FreeLibrary(module) };
                    return Err(format!("mfplat.dll has no {}", $name));
                }
            }
        }};
    }
    Ok(Api {
        _module: module,
        startup: symbol!("MFStartup"),
        shutdown: symbol!("MFShutdown"),
        create_media_type: symbol!("MFCreateMediaType"),
        create_sample: symbol!("MFCreateSample"),
        create_memory_buffer: symbol!("MFCreateMemoryBuffer"),
        create_dxgi_manager: symbol!("MFCreateDXGIDeviceManager"),
        create_video_allocator: symbol!("MFCreateVideoSampleAllocatorEx"),
        enum_transforms: symbol!("MFTEnumEx"),
    })
}

fn check(result: windows::core::Result<()>, what: &str) -> Result<(), String> {
    result.map_err(|error| format!("{what} failed: {error}"))
}

fn hr(code: HRESULT, what: &str) -> Result<(), String> {
    code.ok().map_err(|error| format!("{what} failed: {error}"))
}

/// Brings COM and Media Foundation up for this thread's encoder and takes them down again.
struct Runtime {
    shutdown: ShutdownFn,
    uninitialize_com: bool,
}

impl Runtime {
    fn start(api: &Api) -> Result<Self, String> {
        // SAFETY: plain COM initialization for the calling thread. `S_FALSE` (already initialized
        // the same way) is balanced by `CoUninitialize`; `RPC_E_CHANGED_MODE` (the thread is
        // already initialized differently) is fine for free-threaded Media Foundation objects and
        // must not be balanced.
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let uninitialize_com = com.is_ok();
        // SAFETY: `startup` is `MFStartup`, called with the documented version and flags.
        let started = unsafe { (api.startup)(MF_VERSION, MFSTARTUP_FULL) };
        if let Err(error) = started.ok() {
            if uninitialize_com {
                // SAFETY: balances the successful `CoInitializeEx` above.
                unsafe { CoUninitialize() };
            }
            return Err(format!("MFStartup failed: {error}"));
        }
        Ok(Self {
            shutdown: api.shutdown,
            uninitialize_com,
        })
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: balances the successful `MFStartup` in `start`.
        let _ = unsafe { (self.shutdown)() };
        if self.uninitialize_com {
            // SAFETY: balances the successful `CoInitializeEx` in `start`.
            unsafe { CoUninitialize() };
        }
    }
}

/// An encoder transform, ready for pictures.
pub struct MfEncoder {
    // Declared first so the transform is released before Media Foundation shuts down.
    transform: IMFTransform,
    activate: IMFActivate,
    events: Option<crate::mf_events::Events>,
    api: &'static Api,
    name: String,
    width: u32,
    height: u32,
    fps: u32,
    nv12: Vec<u8>,
    input_credit: u32,
    provides_samples: bool,
    output_size: u32,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    last_time: i64,
    streaming: bool,
    gpu: Option<GpuInput>,
    gpu_declined: Option<String>,
    _runtime: Runtime,
}

struct GpuInput {
    allocator: IMFVideoSampleAllocatorEx,
    converter: Converter,
    _manager: IMFDXGIDeviceManager,
}

// SAFETY: an encoder is created on the recorder's encode thread and used only there; Media
// Foundation transforms are free-threaded (the thread joins the multithreaded apartment), and the
// trait requires `Send` only so the box can be returned from the opening function.
unsafe impl Send for MfEncoder {}

fn activates(api: &Api) -> Result<Vec<IMFActivate>, String> {
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let mut list: *mut *mut c_void = std::ptr::null_mut();
    let mut count = 0_u32;
    let flags = (MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0) as u32;
    // SAFETY: `enum_transforms` is `MFTEnumEx`; the type-info pointers and out-parameters are valid
    // for the call.
    let result = unsafe {
        (api.enum_transforms)(
            MFT_CATEGORY_VIDEO_ENCODER,
            flags,
            std::ptr::null(),
            &output,
            &mut list,
            &mut count,
        )
    };
    hr(result, "listing hardware encoders")?;
    let mut found = Vec::new();
    if !list.is_null() {
        for index in 0..count as usize {
            // SAFETY: `list` holds `count` interface pointers allocated by `MFTEnumEx`.
            let raw = unsafe { *list.add(index) };
            if !raw.is_null() {
                // SAFETY: `raw` is an owned `IMFActivate` pointer; each is taken over once.
                found.push(unsafe { IMFActivate::from_raw(raw) });
            }
        }
        // SAFETY: the array itself was allocated by `MFTEnumEx` with `CoTaskMemAlloc`.
        unsafe { CoTaskMemFree(Some(list.cast())) };
    }
    if found.is_empty() {
        return Err(
            "Windows lists no hardware H.264 encoder (a graphics driver may be missing)".to_owned(),
        );
    }
    Ok(found)
}

fn friendly_name(activate: &IMFActivate) -> String {
    let mut text = PWSTR::null();
    let mut length = 0_u32;
    // SAFETY: valid out-parameters; on success the string is allocated with `CoTaskMemAlloc`.
    let read = unsafe {
        activate.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut text, &mut length)
    };
    if read.is_err() || text.is_null() {
        return "Media Foundation encoder".to_owned();
    }
    // SAFETY: `text` points to `length` UTF-16 units that stay valid until freed just below.
    let name =
        unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(text.0, length as usize)) };
    // SAFETY: frees the string `GetAllocatedString` allocated; not used afterwards.
    unsafe { CoTaskMemFree(Some(text.0.cast())) };
    name
}

fn media_type(api: &Api) -> Result<IMFMediaType, String> {
    let mut raw: *mut c_void = std::ptr::null_mut();
    hr(
        // SAFETY: `create_media_type` is `MFCreateMediaType` with a valid out-pointer.
        unsafe { (api.create_media_type)(&mut raw) },
        "creating a media type",
    )?;
    if raw.is_null() {
        return Err("creating a media type returned nothing".to_owned());
    }
    // SAFETY: on success `raw` is an owned `IMFMediaType` pointer.
    Ok(unsafe { IMFMediaType::from_raw(raw) })
}

fn pack(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

fn codec_value(codec: &ICodecAPI, key: &GUID, value: u32) {
    let mut variant = VARIANT::default();
    // SAFETY: the variant is zeroed; this sets its type tag and the matching union member.
    unsafe {
        let inner = &mut *variant.Anonymous.Anonymous;
        inner.vt = VT_UI4;
        inner.Anonymous.ulVal = value;
    }
    // SAFETY: `key` and `variant` are valid for the call. Encoders may refuse a setting; that is
    // not an error here, the encoder simply keeps its default.
    let _ = unsafe { codec.SetValue(key, &variant) };
}

fn codec_bool(codec: &ICodecAPI, key: &GUID) {
    let mut variant = VARIANT::default();
    // SAFETY: zeroed variant with the matching VT_BOOL tag/union member; no owning payload.
    unsafe {
        let inner = &mut *variant.Anonymous.Anonymous;
        inner.vt = VT_BOOL;
        inner.Anonymous.boolVal = VARIANT_TRUE;
    }
    // SAFETY: live codec, valid GUID/variant. Unsupported optional latency hints keep defaults.
    let _ = unsafe { codec.SetValue(key, &variant) };
}

impl MfEncoder {
    fn open(
        api: &'static Api,
        activate: IMFActivate,
        request: &HardwareRequest,
    ) -> Result<Self, String> {
        let name = friendly_name(&activate);
        let runtime = Runtime::start(api)?;
        // SAFETY: activating an encoder from its own activation object.
        let transform: IMFTransform = unsafe { activate.ActivateObject() }
            .map_err(|error| format!("{name}: could not be started: {error}"))?;

        // Hardware encoders are usually asynchronous: they must be unlocked, and report their
        // needs as events.
        let mut events = None;
        // SAFETY: attribute reads and writes on a live transform.
        unsafe {
            if let Ok(attributes) = transform.GetAttributes() {
                let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
            }
            if let Ok(attributes) = transform.GetAttributes()
                && attributes.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) == 1
            {
                check(
                    attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1),
                    "unlocking the encoder",
                )?;
                let generator = transform
                    .cast::<IMFMediaEventGenerator>()
                    .map_err(|error| format!("asynchronous encoder has no events: {error}"))?;
                events = Some(crate::mf_events::Events::new(generator)?);
            }
        }

        // The output type comes first on an encoder.
        let output = media_type(api)?;
        // SAFETY: attribute writes on a media type we own.
        unsafe {
            check(
                output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video),
                "output type",
            )?;
            check(
                output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264),
                "output type",
            )?;
            check(
                output.SetUINT32(&MF_MT_AVG_BITRATE, request.video_kbps.saturating_mul(1000)),
                "bitrate",
            )?;
            check(
                output.SetUINT64(&MF_MT_FRAME_SIZE, pack(request.width, request.height)),
                "frame size",
            )?;
            check(
                output.SetUINT64(&MF_MT_FRAME_RATE, pack(request.fps, 1)),
                "frame rate",
            )?;
            check(
                output.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1)),
                "pixel aspect",
            )?;
            check(output.SetUINT32(&MF_MT_INTERLACE_MODE, 2), "interlace mode")?;
            // Baseline (66) has no B-frames, so pictures come out in the order they went in.
            check(output.SetUINT32(&MF_MT_MPEG2_PROFILE, 66), "profile")?;
            // Describe the requested stream accurately: 1080p60 needs level 4.2.
            let level = u32::from(crate::h264_stream::level_for(
                request.width,
                request.height,
                request.fps,
            ));
            check(output.SetUINT32(&MF_MT_MPEG2_LEVEL, level), "H.264 level")?;
            // Some encoders refuse the Baseline tag. Without it they still make in-order pictures,
            // because B-frames are switched off below; reordering is checked on every picture.
            if transform.SetOutputType(0, &output, 0).is_err() {
                let _ = output.DeleteItem(&MF_MT_MPEG2_PROFILE);
                check(
                    transform.SetOutputType(0, &output, 0),
                    "setting the output type",
                )
                .map_err(|why| format!("{name}: {why}"))?;
            }
        }

        // Rate control and low latency, after the output type. Each is best effort.
        if let Ok(codec) = transform.cast::<ICodecAPI>() {
            let mode = if request.constant_bitrate { 0 } else { 1 };
            codec_value(&codec, &CODECAPI_AVEncCommonRateControlMode, mode);
            codec_value(
                &codec,
                &CODECAPI_AVEncCommonMeanBitRate,
                request.video_kbps.saturating_mul(1000),
            );
            codec_value(
                &codec,
                &CODECAPI_AVEncMPVGOPSize,
                request.keyframe_frames.max(1),
            );
            codec_value(&codec, &CODECAPI_AVEncMPVDefaultBPictureCount, 0);
            codec_bool(&codec, &CODECAPI_AVLowLatencyMode);
            codec_bool(&codec, &CODECAPI_AVEncCommonRealTime);
        }

        let input = media_type(api)?;
        // SAFETY: attribute writes on a media type we own, then handing it to the transform.
        unsafe {
            check(
                input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video),
                "input type",
            )?;
            check(
                input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12),
                "input type",
            )?;
            check(
                input.SetUINT64(&MF_MT_FRAME_SIZE, pack(request.width, request.height)),
                "frame size",
            )?;
            check(
                input.SetUINT64(&MF_MT_FRAME_RATE, pack(request.fps, 1)),
                "frame rate",
            )?;
            check(
                input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1)),
                "pixel aspect",
            )?;
            check(input.SetUINT32(&MF_MT_INTERLACE_MODE, 2), "interlace mode")?;
            check(
                transform.SetInputType(0, &input, 0),
                "setting the input type",
            )
            .map_err(|why| format!("{name}: {why}"))?;
        }

        // SAFETY: stream-info query and messages on a configured transform.
        let (provides_samples, output_size) = unsafe {
            let info = transform
                .GetOutputStreamInfo(0)
                .map_err(|error| format!("{name}: output stream info failed: {error}"))?;
            let provides = info.dwFlags
                & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0)
                    as u32
                != 0;
            (provides, info.cbSize)
        };

        let mut encoder = Self {
            transform,
            activate,
            events,
            api,
            name,
            width: request.width,
            height: request.height,
            fps: request.fps.max(1),
            nv12: Vec::new(),
            input_credit: 0,
            provides_samples,
            output_size: output_size.max(1 << 16),
            sps: None,
            pps: None,
            last_time: i64::MIN,
            streaming: false,
            gpu: None,
            gpu_declined: None,
            _runtime: runtime,
        };
        encoder.read_sequence_header();
        Ok(encoder)
    }

    fn begin(&mut self) -> Result<(), String> {
        if !self.streaming {
            // SAFETY: configured live transform; GPU manager negotiation, if any, is complete.
            unsafe {
                check(
                    self.transform
                        .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0),
                    "begin streaming",
                )?;
                check(
                    self.transform
                        .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0),
                    "start stream",
                )?;
            }
            self.streaming = true;
        }
        Ok(())
    }

    fn enable_gpu(&mut self, surface: &Surface) -> Result<(), String> {
        if self.streaming {
            return Err("encoder already accepted CPU frames".to_owned());
        }
        let bytes = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|n| n.checked_mul(3))
            .map(|n| n / 2)
            .ok_or("GPU input size overflow")?;
        let max_samples = (128 * 1024 * 1024 / bytes.max(1)).min(8) as u32;
        if max_samples < 2 {
            return Err("GPU sample pool exceeds its memory budget".to_owned());
        }
        let converter = Converter::new(surface, self.fps)?;
        // SAFETY: live transform; the attribute is optional and absence declines native input.
        let aware = unsafe {
            self.transform
                .GetAttributes()
                .and_then(|a| a.GetUINT32(&MF_SA_D3D11_AWARE))
        }
        .unwrap_or(0);
        if aware != 1 {
            return Err("encoder does not advertise D3D11 input".to_owned());
        }
        let mut raw = std::ptr::null_mut();
        let mut token = 0;
        hr(
            // SAFETY: dynamically loaded documented function, valid owned out-parameters.
            unsafe { (self.api.create_dxgi_manager)(&mut token, &mut raw) },
            "creating GPU device manager",
        )?;
        if raw.is_null() {
            return Err("no GPU device manager".to_owned());
        }
        // SAFETY: successful MFCreateDXGIDeviceManager transfers one owned COM reference.
        let manager = unsafe { IMFDXGIDeviceManager::from_raw(raw) };
        // SAFETY: capture device is live, multithread-protected, and retained by converter/surface.
        unsafe {
            check(
                manager.ResetDevice(surface.device(), token),
                "binding capture GPU",
            )?;
        }
        raw = std::ptr::null_mut();
        hr(
            // SAFETY: a valid requested interface IID and out-pointer for an owned COM reference.
            unsafe { (self.api.create_video_allocator)(&IMFVideoSampleAllocatorEx::IID, &mut raw) },
            "creating GPU sample pool",
        )?;
        if raw.is_null() {
            return Err("no GPU sample allocator".to_owned());
        }
        // SAFETY: the allocator function returned the requested owned interface on success.
        let allocator = unsafe { IMFVideoSampleAllocatorEx::from_raw(raw) };
        let attributes = media_type(self.api)?;
        // SAFETY: attributes are ours; allocator retains device/sample references. Media Foundation
        // tracks leases and returns textures only after all consumers release their sample.
        unsafe {
            check(attributes.SetUINT32(&MF_SA_D3D11_USAGE, 0), "GPU usage")?;
            check(
                attributes.SetUINT32(
                    &MF_SA_D3D11_BINDFLAGS,
                    windows::Win32::Graphics::Direct3D11::D3D11_BIND_RENDER_TARGET.0 as u32,
                ),
                "GPU bind flags",
            )?;
            check(allocator.SetDirectXManager(&manager), "sample pool GPU")?;
            let input = self
                .transform
                .GetInputCurrentType(0)
                .map_err(|e| e.to_string())?;
            check(
                allocator.InitializeSampleAllocatorEx(2, max_samples, &attributes, &input),
                "allocating GPU samples",
            )?;
            // Attach only after every fallible pool/converter setup step succeeds, so a
            // declined native path leaves the CPU transform configuration untouched.
            check(
                self.transform
                    .ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize),
                "setting capture GPU",
            )?;
        }
        self.gpu = Some(GpuInput {
            allocator,
            converter,
            _manager: manager,
        });
        Ok(())
    }

    fn sample_time(&self, sample: &IMFSample, index: u64) -> Result<(), String> {
        let time = i64::try_from(index)
            .unwrap_or(i64::MAX)
            .saturating_mul(10_000_000)
            / i64::from(self.fps);
        // SAFETY: caller owns this sample; integer timestamp and positive duration are bounded.
        unsafe {
            check(sample.SetSampleTime(time), "GPU sample time")?;
            check(
                sample.SetSampleDuration(10_000_000 / i64::from(self.fps)),
                "GPU sample duration",
            )?;
        }
        Ok(())
    }

    /// Takes the parameter sets some encoders publish on the output type.
    fn read_sequence_header(&mut self) {
        // SAFETY: reads a blob attribute from the current output type into a buffer sized by the
        // blob's own reported length.
        let blob = unsafe {
            let Ok(current) = self.transform.GetOutputCurrentType(0) else {
                return;
            };
            let Ok(length) = current.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER) else {
                return;
            };
            if length == 0 || length > 1 << 20 {
                return;
            }
            let mut buffer = vec![0_u8; length as usize];
            if current
                .GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut buffer, None)
                .is_err()
            {
                return;
            }
            buffer
        };
        for unit in split_nal_units(&blob) {
            match unit[0] & 0x1f {
                7 => self.sps = Some(unit.to_vec()),
                8 => self.pps = Some(unit.to_vec()),
                _ => {}
            }
        }
    }

    fn make_sample(&self, index: u64) -> Result<IMFSample, String> {
        let mut raw: *mut c_void = std::ptr::null_mut();
        hr(
            // SAFETY: `create_sample` is `MFCreateSample` with a valid out-pointer.
            unsafe { (self.api.create_sample)(&mut raw) },
            "creating a sample",
        )?;
        if raw.is_null() {
            return Err("creating a sample returned nothing".to_owned());
        }
        // SAFETY: on success `raw` is an owned `IMFSample` pointer.
        let sample = unsafe { IMFSample::from_raw(raw) };
        let length =
            u32::try_from(self.nv12.len()).map_err(|_| "the picture is too large".to_owned())?;
        let mut buffer_raw: *mut c_void = std::ptr::null_mut();
        hr(
            // SAFETY: `create_memory_buffer` is `MFCreateMemoryBuffer` with a valid out-pointer.
            unsafe { (self.api.create_memory_buffer)(length, &mut buffer_raw) },
            "creating a buffer",
        )?;
        if buffer_raw.is_null() {
            return Err("creating a buffer returned nothing".to_owned());
        }
        // SAFETY: on success `buffer_raw` is an owned `IMFMediaBuffer` pointer.
        let buffer = unsafe { IMFMediaBuffer::from_raw(buffer_raw) };
        // SAFETY: the buffer was created with room for `length` bytes; the copy stays inside it
        // while it is locked, and it is unlocked before use.
        unsafe {
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut maximum = 0_u32;
            check(
                buffer.Lock(&mut data, Some(&mut maximum), None),
                "locking a buffer",
            )?;
            if data.is_null() || maximum < length {
                let _ = buffer.Unlock();
                return Err("the encoder's buffer is too small".to_owned());
            }
            std::ptr::copy_nonoverlapping(self.nv12.as_ptr(), data, self.nv12.len());
            check(buffer.Unlock(), "unlocking a buffer")?;
            check(buffer.SetCurrentLength(length), "setting a buffer length")?;
            check(sample.AddBuffer(&buffer), "adding a buffer")?;
            let ticks = 10_000_000_i64;
            let time = i64::try_from(index)
                .unwrap_or(i64::MAX)
                .saturating_mul(ticks)
                / i64::from(self.fps);
            check(sample.SetSampleTime(time), "setting the sample time")?;
            check(
                sample.SetSampleDuration(ticks / i64::from(self.fps)),
                "setting the sample duration",
            )?;
        }
        Ok(sample)
    }

    /// Reads one encoded picture, if the encoder has one. Returns whether it produced one.
    fn process_output(&mut self, out: &mut Vec<HardwarePacket>) -> Result<bool, String> {
        let _time = crate::d3d11::measure_output();
        let mut buffers = [MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: 0,
            pSample: ManuallyDrop::new(None),
            dwStatus: 0,
            pEvents: ManuallyDrop::new(None),
        }];
        if !self.provides_samples {
            buffers[0].pSample = ManuallyDrop::new(Some(self.empty_sample()?));
        }
        let mut status = 0_u32;
        // SAFETY: `buffers` is a valid one-element array for the call.
        let result = unsafe { self.transform.ProcessOutput(0, &mut buffers, &mut status) };
        // SAFETY: both fields are owned by us after the call (the transform handed over, or left
        // untouched, any sample or event collection); each is taken exactly once.
        let (sample, _events) = unsafe {
            (
                ManuallyDrop::take(&mut buffers[0].pSample),
                ManuallyDrop::take(&mut buffers[0].pEvents),
            )
        };
        match result {
            Ok(()) => {}
            Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(false),
            Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                self.renegotiate_output()?;
                return Ok(true);
            }
            Err(error) => return Err(format!("{}: encoding failed: {error}", self.name)),
        }
        let Some(sample) = sample else {
            return Ok(false);
        };
        let bytes = copy_sample(&sample)?;
        // SAFETY: a plain attribute read.
        let time = unsafe { sample.GetSampleTime() }.unwrap_or(0);
        let picture = annexb_to_length_prefixed(&bytes)?;
        if let Some(sps) = picture.sps {
            self.sps = Some(sps);
        }
        if let Some(pps) = picture.pps {
            self.pps = Some(pps);
        }
        if self.sps.is_none() || self.pps.is_none() {
            self.read_sequence_header();
        }
        if self.sps.is_none() || self.pps.is_none() {
            return Err(format!("{}: the encoder gave no parameter sets", self.name));
        }
        let reordered = time < self.last_time;
        self.last_time = self.last_time.max(time);
        out.push(HardwarePacket {
            data: picture.data,
            keyframe: picture.keyframe,
            reordered,
        });
        Ok(true)
    }

    fn empty_sample(&self) -> Result<IMFSample, String> {
        let mut raw: *mut c_void = std::ptr::null_mut();
        hr(
            // SAFETY: valid out-pointer for `MFCreateSample`.
            unsafe { (self.api.create_sample)(&mut raw) },
            "creating an output sample",
        )?;
        if raw.is_null() {
            return Err("creating an output sample returned nothing".to_owned());
        }
        // SAFETY: owned `IMFSample` pointer on success.
        let sample = unsafe { IMFSample::from_raw(raw) };
        let mut buffer_raw: *mut c_void = std::ptr::null_mut();
        hr(
            // SAFETY: valid out-pointer for `MFCreateMemoryBuffer`.
            unsafe { (self.api.create_memory_buffer)(self.output_size, &mut buffer_raw) },
            "creating an output buffer",
        )?;
        if buffer_raw.is_null() {
            return Err("creating an output buffer returned nothing".to_owned());
        }
        // SAFETY: owned `IMFMediaBuffer` pointer on success.
        let buffer = unsafe { IMFMediaBuffer::from_raw(buffer_raw) };
        check(
            // SAFETY: adding a buffer to a sample we own.
            unsafe { sample.AddBuffer(&buffer) },
            "adding an output buffer",
        )?;
        Ok(sample)
    }

    fn renegotiate_output(&mut self) -> Result<(), String> {
        // SAFETY: asks the transform for its first available output type and sets it, both on a
        // live transform.
        unsafe {
            let wanted = self
                .transform
                .GetOutputAvailableType(0, 0)
                .map_err(|error| {
                    format!("{}: no output type after a change: {error}", self.name)
                })?;
            check(
                self.transform.SetOutputType(0, &wanted, 0),
                "renegotiating the output type",
            )?;
        }
        self.read_sequence_header();
        Ok(())
    }

    /// Hands one picture to the encoder and collects what is ready.
    fn feed(&mut self, sample: IMFSample, out: &mut Vec<HardwarePacket>) -> Result<(), String> {
        let _time = crate::d3d11::measure_feed();
        let started = Instant::now();
        match self.events.clone() {
            Some(events) => {
                let mut waiting = Some(sample);
                loop {
                    self.drain_events(&events, out)?;
                    if self.input_credit > 0
                        && let Some(sample) = waiting.take()
                    {
                        check(
                            // SAFETY: the encoder asked for input; the sample is a complete picture.
                            unsafe {
                                let _time = crate::d3d11::measure_input();
                                self.transform.ProcessInput(0, &sample, 0)
                            },
                            "sending a picture",
                        )?;
                        self.input_credit -= 1;
                    }
                    if waiting.is_none() {
                        // Give output that is already waiting a chance to come out now.
                        self.drain_events(&events, out)?;
                        return Ok(());
                    }
                    if started.elapsed() > STALL_LIMIT {
                        return Err(format!("{} stopped accepting pictures", self.name));
                    }
                    events.wait(STALL_LIMIT.saturating_sub(started.elapsed()));
                }
            }
            None => {
                // A synchronous encoder: input, then output until it wants more.
                // SAFETY: a complete picture handed to a configured transform.
                let sent = unsafe { self.transform.ProcessInput(0, &sample, 0) };
                if let Err(error) = sent {
                    if error.code() != MF_E_NOTACCEPTING {
                        return Err(format!("{}: sending a picture failed: {error}", self.name));
                    }
                    while self.process_output(out)? {}
                    check(
                        // SAFETY: as above, after making room.
                        unsafe { self.transform.ProcessInput(0, &sample, 0) },
                        "sending a picture",
                    )?;
                }
                while self.process_output(out)? {}
                Ok(())
            }
        }
    }

    fn drain_events(
        &mut self,
        events: &crate::mf_events::Events,
        out: &mut Vec<HardwarePacket>,
    ) -> Result<(), String> {
        for _ in 0..256 {
            let Some(kind) = events.try_next()? else {
                return Ok(());
            };
            if kind == METransformNeedInput.0 as u32 {
                if self.input_credit >= 32 {
                    return Err("encoder requested too many outstanding inputs".to_owned());
                }
                self.input_credit += 1;
            } else if kind == METransformHaveOutput.0 as u32 {
                self.process_output(out)?;
            } else if kind == MEError.0 as u32 {
                return Err(format!("{} reported an error", self.name));
            }
        }
        Err("encoder event burst exceeded its bound".to_owned())
    }
}

fn copy_sample(sample: &IMFSample) -> Result<Vec<u8>, String> {
    // SAFETY: joins the sample's buffers into one, locks it, copies exactly its current length,
    // and unlocks it before returning.
    unsafe {
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|error| format!("reading an encoded picture failed: {error}"))?;
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut length = 0_u32;
        check(
            buffer.Lock(&mut data, None, Some(&mut length)),
            "locking an encoded picture",
        )?;
        if data.is_null() || length == 0 || length > 1 << 28 {
            let _ = buffer.Unlock();
            return Err("the encoder returned an empty or implausible picture".to_owned());
        }
        let copy = std::slice::from_raw_parts(data, length as usize).to_vec();
        check(buffer.Unlock(), "unlocking an encoded picture")?;
        Ok(copy)
    }
}

impl HardwareEncoder for MfEncoder {
    fn encode(&mut self, rgba: &[u8], index: u64) -> Result<Vec<HardwarePacket>, String> {
        self.begin()?;
        rgba_to_nv12(rgba, self.width, self.height, &mut self.nv12, 2)?;
        let sample = self.make_sample(index)?;
        let mut out = Vec::new();
        self.feed(sample, &mut out)?;
        crate::d3d11::cpu_input_count();
        Ok(out)
    }

    fn encode_surface(
        &mut self,
        frame: &mut dyn NativeCaptureFrame,
        index: u64,
    ) -> Result<Option<Vec<HardwarePacket>>, String> {
        let Some(surface) = frame.as_any_mut().downcast_mut::<Surface>() else {
            return Ok(None);
        };
        if surface.width() != self.width || surface.height() != self.height {
            return Ok(None);
        }
        if self.gpu.is_none()
            && self.gpu_declined.is_none()
            && let Err(why) = self.enable_gpu(surface)
        {
            self.gpu_declined = Some(why);
        }
        let Some(gpu) = &self.gpu else {
            return Ok(None);
        };
        let conversion_time = crate::d3d11::measure_conversion();
        // SAFETY: a configured bounded allocator; returned sample is exclusively leased until
        // submitted. Empty pools report an error rather than allocating beyond their maximum.
        let sample = unsafe { gpu.allocator.AllocateSample() }
            .map_err(|e| format!("GPU sample unavailable: {e}"))?;
        gpu.converter.convert(surface, &sample)?;
        self.sample_time(&sample, index)?;
        self.begin()?;
        drop(conversion_time);
        let mut out = Vec::new();
        self.feed(sample, &mut out)?;
        crate::d3d11::native_input_count();
        Ok(Some(out))
    }

    fn flush(&mut self) -> Result<Vec<HardwarePacket>, String> {
        let mut out = Vec::new();
        check(
            // SAFETY: a drain request to a live transform.
            unsafe { self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0) },
            "draining the encoder",
        )?;
        let started = Instant::now();
        match self.events.clone() {
            Some(events) => loop {
                if started.elapsed() > STALL_LIMIT {
                    return Err(format!("{} timed out while draining", self.name));
                }
                match events.try_next()? {
                    Some(kind) => {
                        if kind == METransformHaveOutput.0 as u32 {
                            self.process_output(&mut out)?;
                        } else if kind == MEError.0 as u32 {
                            return Err(format!("{} failed while draining", self.name));
                        } else if kind == METransformDrainComplete.0 as u32 {
                            break;
                        }
                    }
                    None => {
                        events.wait(STALL_LIMIT.saturating_sub(started.elapsed()));
                    }
                }
            },
            None => while self.process_output(&mut out)? {},
        }
        Ok(out)
    }

    fn avcc(&self) -> Vec<u8> {
        match (&self.sps, &self.pps) {
            (Some(sps), Some(pps)) => build_avcc(sps, pps).unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn name(&self) -> String {
        if self.gpu.is_some() {
            format!("Media Foundation: {}; D3D11 GPU surfaces", self.name)
        } else if let Some(why) = &self.gpu_declined {
            format!("Media Foundation: {}; CPU input ({why})", self.name)
        } else {
            format!("Media Foundation: {}", self.name)
        }
    }
}

impl Drop for MfEncoder {
    fn drop(&mut self) {
        if let Some(events) = &self.events {
            events.close();
        }
        // SAFETY: orderly shutdown of a live transform; errors cannot be acted on while dropping.
        unsafe {
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            let _ = self.activate.ShutdownObject();
        }
    }
}

/// Opens the first hardware H.264 encoder Windows lists that accepts this request.
pub fn open(request: &HardwareRequest) -> Result<Box<dyn HardwareEncoder>, String> {
    let api = api()?;
    // Listing encoders needs Media Foundation to be running; this is released last.
    let _runtime = Runtime::start(api)?;
    let mut failures = Vec::new();
    for activate in activates(api)? {
        match MfEncoder::open(api, activate, request) {
            Ok(encoder) => return Ok(Box::new(encoder)),
            Err(why) => failures.push(why),
        }
    }
    Err(failures.join("; "))
}
