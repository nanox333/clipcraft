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
use clipcraft_record::{
    EncoderSpeed, HardwareEncoder, HardwarePacket, HardwareRequest, StreamCodec, SurfaceFit,
};
use windows::Win32::Foundation::{FreeLibrary, HMODULE, VARIANT_TRUE};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{
    COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4};
use windows::core::{GUID, HRESULT, Interface, PCSTR, PWSTR, w};

use crate::d3d11::{Converter, Surface};
use crate::h264_stream::{
    annexb_to_length_prefixed, build_avcc, conversion_threads, rgba_to_nv12, split_nal_units,
};
use crate::hevc_stream;

/// How long to wait for an encoder to accept a picture before calling it stuck.
const STALL_LIMIT: Duration = Duration::from_secs(2);
const MAX_OUTPUT_TYPES: u32 = 64;
const MAX_OUTPUT_BUFFER_BYTES: u32 = 1 << 28;

/// The Media Foundation subtype for a codec.
fn subtype_of(codec: StreamCodec) -> GUID {
    match codec {
        StreamCodec::H264 => MFVideoFormat_H264,
        StreamCodec::H265 => MFVideoFormat_HEVC,
    }
}

fn codec_name(codec: StreamCodec) -> &'static str {
    match codec {
        StreamCodec::H264 => "H.264",
        StreamCodec::H265 => "H.265",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OutputContract {
    major: GUID,
    subtype: GUID,
    frame_size: Option<u64>,
    frame_rate: Option<u64>,
    bitrate: Option<u32>,
    profile: Option<u32>,
    level: Option<u32>,
}

fn optional_attribute<T>(value: windows::core::Result<T>) -> Result<Option<T>, String> {
    match value {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.code() == MF_E_ATTRIBUTENOTFOUND => Ok(None),
        Err(error) => Err(format!("reading encoder output attribute failed: {error}")),
    }
}

fn read_output_contract(media: &IMFMediaType) -> Result<OutputContract, String> {
    // SAFETY: read-only typed attributes on a live media type. Absent attributes describe a
    // partial advertised type; other errors remain failures rather than becoming defaults.
    unsafe {
        Ok(OutputContract {
            major: media
                .GetGUID(&MF_MT_MAJOR_TYPE)
                .map_err(|e| e.to_string())?,
            subtype: media.GetGUID(&MF_MT_SUBTYPE).map_err(|e| e.to_string())?,
            frame_size: optional_attribute(media.GetUINT64(&MF_MT_FRAME_SIZE))?,
            frame_rate: optional_attribute(media.GetUINT64(&MF_MT_FRAME_RATE))?,
            bitrate: optional_attribute(media.GetUINT32(&MF_MT_AVG_BITRATE))?,
            profile: optional_attribute(media.GetUINT32(&MF_MT_MPEG2_PROFILE))?,
            level: optional_attribute(media.GetUINT32(&MF_MT_MPEG2_LEVEL))?,
        })
    }
}

fn equal_frame_rates(left: u64, right: u64) -> bool {
    let (left_num, left_den) = (left >> 32, left & u64::from(u32::MAX));
    let (right_num, right_den) = (right >> 32, right & u64::from(u32::MAX));
    // Each factor is at most u32::MAX, so the cross-products fit in u64.
    left_num != 0
        && left_den != 0
        && right_num != 0
        && right_den != 0
        && left_num * right_den == right_num * left_den
}

fn output_codec_matches(expected: OutputContract, major: GUID, subtype: GUID) -> bool {
    expected.major == major && expected.subtype == subtype
}

fn output_contract_matches(expected: OutputContract, actual: OutputContract) -> bool {
    fn matches<T: PartialEq>(expected: Option<T>, actual: Option<T>) -> bool {
        match (expected, actual) {
            (Some(expected), Some(actual)) => expected == actual,
            (Some(_), None) => false,
            (None, _) => true,
        }
    }
    let rate_matches = match (expected.frame_rate, actual.frame_rate) {
        (Some(expected), Some(actual)) => equal_frame_rates(expected, actual),
        _ => false,
    };
    output_codec_matches(expected, actual.major, actual.subtype)
        && matches(expected.frame_size, actual.frame_size)
        && rate_matches
        && matches(expected.bitrate, actual.bitrate)
        && matches(expected.profile, actual.profile)
        && matches(expected.level, actual.level)
}

fn validate_output_contract(
    contract: OutputContract,
    codec: StreamCodec,
    width: u32,
    height: u32,
    fps: u32,
) -> Result<(), String> {
    if contract.major != MFMediaType_Video
        || contract.subtype != subtype_of(codec)
        || width == 0
        || height == 0
        || contract.frame_size != Some(pack(width, height))
        || !contract
            .frame_rate
            .is_some_and(|rate| equal_frame_rates(rate, pack(fps, 1)))
        || contract.bitrate.is_none_or(|bitrate| bitrate == 0)
    {
        return Err(format!(
            "encoder output does not preserve the requested {} dimensions, frame rate and bitrate",
            codec_name(codec)
        ));
    }
    Ok(())
}

fn apply_output_contract(media: &IMFMediaType, contract: OutputContract) -> Result<(), String> {
    // SAFETY: writes known accepted values onto our independent copy, never onto a media type
    // owned by the transform. A partial advertised type may omit these known format attributes.
    unsafe {
        for (key, value) in [
            (&MF_MT_FRAME_SIZE, contract.frame_size),
            (&MF_MT_FRAME_RATE, contract.frame_rate),
        ] {
            if let Some(value) = value {
                check(
                    media.SetUINT64(key, value),
                    "preserving encoder output geometry/rate",
                )?;
            }
        }
        for (key, value) in [
            (&MF_MT_AVG_BITRATE, contract.bitrate),
            (&MF_MT_MPEG2_PROFILE, contract.profile),
            (&MF_MT_MPEG2_LEVEL, contract.level),
        ] {
            if let Some(value) = value {
                check(
                    media.SetUINT32(key, value),
                    "preserving encoder output settings",
                )?;
            }
        }
    }
    Ok(())
}

fn output_buffer_requirements(
    flags: u32,
    size: u32,
    alignment: u32,
) -> Result<(bool, u32), String> {
    let provides = flags
        & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32
        != 0;
    if provides {
        return Ok((true, 0));
    }
    if alignment != 0 {
        return Err(
            "encoder requires aligned caller-owned output buffers, which are unsupported"
                .to_owned(),
        );
    }
    if size > MAX_OUTPUT_BUFFER_BYTES {
        return Err("encoder output buffer exceeds the 256 MiB packet bound".to_owned());
    }
    Ok((false, size.max(1 << 16)))
}

fn update_parameter_sets(
    current_sps: &mut Option<Vec<u8>>,
    current_pps: &mut Option<Vec<u8>>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    published: bool,
) -> Result<(), String> {
    // The recorder publishes avcC with its first picture. Later changes would make existing
    // buffered pictures incompatible with that file header, even if dimensions/rate match.
    if published
        && (sps
            .as_ref()
            .is_some_and(|sps| current_sps.as_ref() != Some(sps))
            || pps
                .as_ref()
                .is_some_and(|pps| current_pps.as_ref() != Some(pps)))
    {
        return Err(
            "encoder changed H.264 parameter sets after output; a new recording stream is required"
                .to_owned(),
        );
    }
    if let Some(sps) = sps {
        *current_sps = Some(sps);
    }
    if let Some(pps) = pps {
        *current_pps = Some(pps);
    }
    Ok(())
}

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
    output_contract: IMFMediaType,
    codec: StreamCodec,
    vps: Option<Vec<u8>>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    parameter_sets_published: bool,
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

fn activates(api: &Api, codec: StreamCodec) -> Result<Vec<IMFActivate>, String> {
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: subtype_of(codec),
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
        return Err(format!(
            "Windows lists no hardware {} encoder (a graphics driver may be missing)",
            codec_name(codec)
        ));
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
                output.SetGUID(&MF_MT_SUBTYPE, &subtype_of(request.codec)),
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
            if request.codec == StreamCodec::H264 {
                // Baseline (66) has no B-frames, so pictures come out in the order they went in.
                check(output.SetUINT32(&MF_MT_MPEG2_PROFILE, 66), "profile")?;
                // Describe the requested stream accurately: 1080p60 needs level 4.2.
                let level = u32::from(crate::h264_stream::level_for(
                    request.width,
                    request.height,
                    request.fps,
                ));
                check(output.SetUINT32(&MF_MT_MPEG2_LEVEL, level), "H.264 level")?;
            }
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
            // On AMD the encoder has a fast tier at 50 and below that uses about half the video
            // engine of the quality tier above it, for a little less quality at the same bitrate
            // (measured: 0.5 dB, 0.014 SSIM on game-like motion).
            codec_value(
                &codec,
                &CODECAPI_AVEncCommonQualityVsSpeed,
                match request.effort {
                    EncoderSpeed::Fast => 0,
                    EncoderSpeed::Balanced => 75,
                    EncoderSpeed::Quality => 100,
                },
            );
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

        // Copy the accepted contract: stream changes may invalidate GetOutputCurrentType, and
        // the transform must never be able to mutate the settings retained for renegotiation.
        let output_contract = media_type(api)?;
        // SAFETY: both media types and the configured transform are live; CopyAllItems writes
        // only into our independently created destination. A just-invalidated current type
        // falls back to the complete output type already accepted above.
        unsafe {
            let accepted = match transform.GetOutputCurrentType(0) {
                Ok(accepted) => accepted,
                Err(error) if error.code() == MF_E_TRANSFORM_TYPE_NOT_SET => output,
                Err(error) => {
                    return Err(format!(
                        "{name}: reading accepted output type failed: {error}"
                    ));
                }
            };
            check(
                accepted.CopyAllItems(&output_contract),
                "retaining encoder output contract",
            )?;
        }
        validate_output_contract(
            read_output_contract(&output_contract)?,
            request.codec,
            request.width,
            request.height,
            request.fps,
        )?;

        // SAFETY: stream-info query on a configured transform.
        let (provides_samples, output_size) = unsafe {
            let info = transform
                .GetOutputStreamInfo(0)
                .map_err(|error| format!("{name}: output stream info failed: {error}"))?;
            output_buffer_requirements(info.dwFlags, info.cbSize, info.cbAlignment)?
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
            output_size,
            output_contract,
            codec: request.codec,
            vps: None,
            sps: None,
            pps: None,
            parameter_sets_published: false,
            last_time: i64::MIN,
            streaming: false,
            gpu: None,
            gpu_declined: None,
            _runtime: runtime,
        };
        encoder.read_sequence_header()?;
        encoder.record_media_types();
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
            self.record_media_types();
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
        let max_samples = (128 * 1024 * 1024 / bytes.max(1)).min(4) as u32;
        if max_samples < 2 {
            return Err("GPU sample pool exceeds its memory budget".to_owned());
        }
        let converter = Converter::new(surface, self.fps, (self.width, self.height))?;
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

    /// Remembers the parameter sets an encoder produced. Once the file header has been written
    /// from them, a change is an error: pictures already buffered would no longer match it.
    fn take_parameter_sets(
        &mut self,
        vps: Option<Vec<u8>>,
        sps: Option<Vec<u8>>,
        pps: Option<Vec<u8>>,
    ) -> Result<(), String> {
        if self.parameter_sets_published
            && vps
                .as_ref()
                .is_some_and(|vps| self.vps.as_ref() != Some(vps))
        {
            return Err(
                "encoder changed H.265 parameter sets after output; a new recording stream is required"
                    .to_owned(),
            );
        }
        if let Some(vps) = vps {
            self.vps = Some(vps);
        }
        update_parameter_sets(
            &mut self.sps,
            &mut self.pps,
            sps,
            pps,
            self.parameter_sets_published,
        )
    }

    /// True once every parameter set the codec needs for its file header is known.
    fn have_parameter_sets(&self) -> bool {
        self.sps.is_some()
            && self.pps.is_some()
            && (self.codec == StreamCodec::H264 || self.vps.is_some())
    }

    /// Takes the parameter sets some encoders publish on the output type.
    fn read_sequence_header(&mut self) -> Result<(), String> {
        // SAFETY: reads a blob attribute from the current output type into a buffer sized by the
        // blob's own reported length.
        let blob = unsafe {
            let current = match self.transform.GetOutputCurrentType(0) {
                Ok(current) => current,
                Err(error) if error.code() == MF_E_TRANSFORM_TYPE_NOT_SET => return Ok(()),
                Err(error) => {
                    return Err(format!(
                        "reading output type for parameter sets failed: {error}"
                    ));
                }
            };
            let length = match current.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER) {
                Ok(length) => length,
                Err(error) if error.code() == MF_E_ATTRIBUTENOTFOUND => return Ok(()),
                Err(error) => return Err(format!("reading parameter-set size failed: {error}")),
            };
            if length == 0 {
                return Ok(());
            }
            if length > 1 << 20 {
                return Err("encoder parameter-set header exceeds its size bound".to_owned());
            }
            let mut buffer = vec![0_u8; length as usize];
            current
                .GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut buffer, None)
                .map_err(|error| format!("reading parameter-set header failed: {error}"))?;
            buffer
        };
        for unit in split_nal_units(&blob) {
            let (vps, sps, pps) = match self.codec {
                StreamCodec::H264 => match unit[0] & 0x1f {
                    7 => (None, Some(unit.to_vec()), None),
                    8 => (None, None, Some(unit.to_vec())),
                    _ => continue,
                },
                StreamCodec::H265 => match hevc_stream::nal_type(unit) {
                    32 => (Some(unit.to_vec()), None, None),
                    33 => (None, Some(unit.to_vec()), None),
                    34 => (None, None, Some(unit.to_vec())),
                    _ => continue,
                },
            };
            self.take_parameter_sets(vps, sps, pps)?;
        }
        Ok(())
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
        let (data, keyframe) = match self.codec {
            StreamCodec::H264 => {
                let picture = annexb_to_length_prefixed(&bytes)?;
                self.take_parameter_sets(None, picture.sps, picture.pps)?;
                (picture.data, picture.keyframe)
            }
            StreamCodec::H265 => {
                let picture = hevc_stream::annexb_to_length_prefixed(&bytes)?;
                self.take_parameter_sets(picture.vps, picture.sps, picture.pps)?;
                (picture.data, picture.keyframe)
            }
        };
        if !self.have_parameter_sets() {
            self.read_sequence_header()?;
        }
        if !self.have_parameter_sets() {
            return Err(format!("{}: the encoder gave no parameter sets", self.name));
        }
        let reordered = time < self.last_time;
        self.last_time = self.last_time.max(time);
        out.push(HardwarePacket {
            data,
            keyframe,
            reordered,
        });
        self.parameter_sets_published = true;
        crate::d3d11::output_picture();
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
        crate::d3d11::stream_changed();
        let expected = read_output_contract(&self.output_contract)?;
        validate_output_contract(expected, self.codec, self.width, self.height, self.fps)?;
        for index in 0..MAX_OUTPUT_TYPES {
            // SAFETY: bounded enumeration of the configured transform's output stream.
            let available = match unsafe { self.transform.GetOutputAvailableType(0, index) } {
                Ok(available) => available,
                Err(error) if error.code() == MF_E_NO_MORE_TYPES => {
                    return Err(format!(
                        "{}: no compatible output type preserves the recording settings after a stream change",
                        self.name
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "{}: listing output types after a change failed: {error}",
                        self.name
                    ));
                }
            };
            // Advertised types are hints and may contain stale/default numeric values. Only
            // filter the codec identity before completing and testing the retained contract.
            // SAFETY: read-only mandatory identity attributes on the advertised media type.
            let (major, subtype) = unsafe {
                (
                    available
                        .GetGUID(&MF_MT_MAJOR_TYPE)
                        .map_err(|error| error.to_string())?,
                    available
                        .GetGUID(&MF_MT_SUBTYPE)
                        .map_err(|error| error.to_string())?,
                )
            };
            if !output_codec_matches(expected, major, subtype) {
                continue;
            }
            let wanted = media_type(self.api)?;
            // SAFETY: copy the advertised attributes into our own type. Do not mutate the
            // driver's advertised object or copy stale codec sequence headers from the old type.
            let copied = unsafe { available.CopyAllItems(&wanted) };
            check(copied, "copying advertised output type")?;
            apply_output_contract(&wanted, expected)?;
            // SAFETY: test-only validation of our complete candidate leaves the transform's
            // active type unchanged; rejected candidates are skipped within the bounded list.
            let tested = unsafe {
                self.transform
                    .SetOutputType(0, &wanted, MFT_SET_TYPE_TEST_ONLY.0 as u32)
            };
            match tested {
                Ok(()) => {}
                Err(error) if error.code() == MF_E_INVALIDMEDIATYPE => continue,
                Err(error) => {
                    return Err(format!(
                        "{}: validating output type after a change failed: {error}",
                        self.name
                    ));
                }
            }
            // SAFETY: the transform accepted this complete candidate in test-only mode.
            let selected = unsafe { self.transform.SetOutputType(0, &wanted, 0) };
            check(selected, "renegotiating the output type")?;
            // SAFETY: query the actual accepted type and updated allocation requirements.
            let (actual, info) = unsafe {
                (
                    self.transform.GetOutputCurrentType(0).map_err(|error| {
                        format!("reading renegotiated output type failed: {error}")
                    })?,
                    self.transform.GetOutputStreamInfo(0).map_err(|error| {
                        format!("reading renegotiated output requirements failed: {error}")
                    })?,
                )
            };
            if !output_contract_matches(expected, read_output_contract(&actual)?) {
                return Err(format!(
                    "{}: renegotiated output changed the recording settings",
                    self.name
                ));
            }
            (self.provides_samples, self.output_size) =
                output_buffer_requirements(info.dwFlags, info.cbSize, info.cbAlignment)?;
            self.read_sequence_header()?;
            self.record_media_types();
            return Ok(());
        }
        Err(format!(
            "{}: output type enumeration exceeded its bound after a stream change",
            self.name
        ))
    }

    fn record_media_types(&self) {
        // SAFETY: optional, read-only attributes on configured live COM interfaces. Missing
        // diagnostics are represented by zero; they do not change negotiation or recording.
        let (input, output, max_mbps) = unsafe {
            (
                self.transform
                    .GetInputCurrentType(0)
                    .and_then(|t| t.GetUINT64(&MF_MT_FRAME_RATE))
                    .unwrap_or(0),
                self.transform
                    .GetOutputCurrentType(0)
                    .and_then(|t| t.GetUINT64(&MF_MT_FRAME_RATE))
                    .unwrap_or(0),
                self.transform
                    .GetAttributes()
                    .and_then(|a| a.GetUINT32(&MF_VIDEO_MAX_MB_PER_SEC))
                    .unwrap_or(0),
            )
        };
        crate::d3d11::media_types(input, output, max_mbps);
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
        rgba_to_nv12(
            rgba,
            self.width,
            self.height,
            &mut self.nv12,
            conversion_threads(),
        )?;
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
        fit: SurfaceFit,
    ) -> Result<Option<Vec<HardwarePacket>>, String> {
        let Some(surface) = frame.as_any_mut().downcast_mut::<Surface>() else {
            return Ok(None);
        };
        if self.gpu.is_none()
            && self.gpu_declined.is_none()
            && let Err(why) = self.enable_gpu(surface)
        {
            self.gpu_declined = Some(why);
        }
        let Some(gpu) = &self.gpu else {
            return Ok(None);
        };
        // A surface of another size than the one this converter was built for takes the CPU path.
        if !gpu.converter.accepts(surface) {
            return Ok(None);
        }
        let conversion_time = crate::d3d11::measure_conversion();
        // SAFETY: a configured bounded allocator; returned sample is exclusively leased until
        // submitted. Empty pools report an error rather than allocating beyond their maximum.
        let sample = unsafe { gpu.allocator.AllocateSample() }
            .map_err(|e| format!("GPU sample unavailable: {e}"))?;
        gpu.converter.convert(surface, &sample, fit)?;
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
        match (self.codec, &self.vps, &self.sps, &self.pps) {
            (StreamCodec::H264, _, Some(sps), Some(pps)) => {
                build_avcc(sps, pps).unwrap_or_default()
            }
            (StreamCodec::H265, Some(vps), Some(sps), Some(pps)) => {
                hevc_stream::build_hvcc(vps, sps, pps).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    fn codec(&self) -> StreamCodec {
        self.codec
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

/// Opens the first hardware encoder for the requested codec that Windows lists and that accepts
/// this request.
pub fn open(request: &HardwareRequest) -> Result<Box<dyn HardwareEncoder>, String> {
    let api = api()?;
    // Listing encoders needs Media Foundation to be running; this is released last.
    let _runtime = Runtime::start(api)?;
    let mut failures = Vec::new();
    for activate in activates(api, request.codec)? {
        match MfEncoder::open(api, activate, request) {
            Ok(encoder) => return Ok(Box::new(encoder)),
            Err(why) => failures.push(why),
        }
    }
    Err(failures.join("; "))
}

#[cfg(test)]
mod output_contract_tests {
    use super::*;

    fn requested() -> OutputContract {
        OutputContract {
            major: MFMediaType_Video,
            subtype: subtype_of(StreamCodec::H264),
            frame_size: Some(pack(1920, 1080)),
            frame_rate: Some(pack(60, 1)),
            bitrate: Some(12_000_000),
            profile: Some(66),
            level: Some(42),
        }
    }

    #[test]
    fn frame_rate_equivalence_is_exact_and_rejects_hostile_ratios() {
        assert!(equal_frame_rates(pack(60, 1), pack(120, 2)));
        assert!(equal_frame_rates(pack(u32::MAX, u32::MAX), pack(1, 1)));
        assert!(!equal_frame_rates(pack(60, 1), pack(60000, 1001)));
        assert!(!equal_frame_rates(pack(0, 1), pack(0, 1)));
        assert!(!equal_frame_rates(pack(60, 0), pack(60, 0)));
        assert!(!equal_frame_rates(
            pack(u32::MAX, u32::MAX - 1),
            pack(u32::MAX - 1, u32::MAX)
        ));
    }

    #[test]
    fn renegotiation_rejects_downgrades_and_selects_an_equivalent_rate() {
        let expected = requested();
        let mut downgrade = expected;
        downgrade.frame_rate = Some(pack(30, 1));
        let mut equivalent = expected;
        equivalent.frame_rate = Some(pack(120, 2));
        let candidates = [downgrade, equivalent];
        assert_eq!(
            candidates
                .into_iter()
                .position(|candidate| output_contract_matches(expected, candidate)),
            Some(1)
        );
        for changed in [
            OutputContract {
                frame_size: Some(pack(1280, 720)),
                ..expected
            },
            OutputContract {
                bitrate: Some(6_000_000),
                ..expected
            },
            OutputContract {
                profile: Some(100),
                ..expected
            },
            OutputContract {
                level: Some(40),
                ..expected
            },
            OutputContract {
                subtype: MFVideoFormat_NV12,
                ..expected
            },
        ] {
            assert!(!output_contract_matches(expected, changed));
        }
    }

    #[test]
    fn partial_advertised_types_cannot_be_accepted_as_complete_negotiated_types() {
        let expected = requested();
        let partial = OutputContract {
            frame_size: None,
            frame_rate: None,
            bitrate: None,
            profile: None,
            level: None,
            ..expected
        };
        assert!(output_codec_matches(
            expected,
            partial.major,
            partial.subtype
        ));
        assert!(!output_contract_matches(expected, partial));
        assert!(validate_output_contract(expected, StreamCodec::H264, 1920, 1080, 60).is_ok());
        assert!(validate_output_contract(partial, StreamCodec::H264, 1920, 1080, 60).is_err());
        assert!(validate_output_contract(expected, StreamCodec::H264, 1920, 1080, 30).is_err());
        assert!(validate_output_contract(expected, StreamCodec::H264, 0, 1080, 60).is_err());
        assert!(validate_output_contract(expected, StreamCodec::H264, 1920, 1080, 0).is_err());
        assert!(
            validate_output_contract(
                OutputContract {
                    bitrate: Some(0),
                    ..expected
                },
                StreamCodec::H264,
                1920,
                1080,
                60
            )
            .is_err()
        );
    }

    #[test]
    fn output_allocation_refresh_respects_sample_ownership_and_packet_bound() {
        assert_eq!(
            output_buffer_requirements(0, 0, 0).unwrap(),
            (false, 1 << 16)
        );
        assert_eq!(
            output_buffer_requirements(0, MAX_OUTPUT_BUFFER_BYTES, 0).unwrap(),
            (false, MAX_OUTPUT_BUFFER_BYTES)
        );
        assert!(output_buffer_requirements(0, MAX_OUTPUT_BUFFER_BYTES + 1, 0).is_err());
        assert!(output_buffer_requirements(0, u32::MAX, 0).is_err());
        for alignment in [1, 16, 32, u32::MAX] {
            assert!(output_buffer_requirements(0, 65_536, alignment).is_err());
        }
        for flag in [
            MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
            MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES,
        ] {
            assert_eq!(
                output_buffer_requirements(flag.0 as u32, u32::MAX, u32::MAX).unwrap(),
                (true, 0)
            );
        }
    }

    #[test]
    fn default_advertised_values_do_not_prevent_testing_the_retained_contract() {
        let expected = requested();
        let default_hint = OutputContract {
            frame_size: Some(pack(640, 480)),
            frame_rate: Some(pack(30, 1)),
            bitrate: Some(1_000_000),
            profile: Some(100),
            level: Some(31),
            ..expected
        };
        assert!(output_codec_matches(
            expected,
            default_hint.major,
            default_hint.subtype
        ));
        assert!(!output_contract_matches(expected, default_hint));
        assert!(!output_codec_matches(
            expected,
            MFMediaType_Video,
            MFVideoFormat_NV12
        ));
    }

    #[test]
    fn first_picture_can_replace_provisional_parameter_sets_but_later_changes_fail() {
        let (mut sps, mut pps) = (Some(vec![0x67, 1]), Some(vec![0x68, 1]));
        update_parameter_sets(
            &mut sps,
            &mut pps,
            Some(vec![0x67, 2]),
            Some(vec![0x68, 2]),
            false,
        )
        .unwrap();
        let accepted = (sps.clone(), pps.clone());
        update_parameter_sets(
            &mut sps,
            &mut pps,
            accepted.0.clone(),
            accepted.1.clone(),
            true,
        )
        .unwrap();
        assert!(
            update_parameter_sets(
                &mut sps,
                &mut pps,
                Some(vec![0x67, 3]),
                Some(vec![0x68, 3]),
                true
            )
            .is_err()
        );
        assert_eq!((sps.clone(), pps.clone()), accepted);
        assert!(
            update_parameter_sets(&mut sps, &mut pps, None, Some(vec![0x68, 3]), true).is_err()
        );
        assert_eq!((sps.clone(), pps.clone()), accepted);
        update_parameter_sets(&mut sps, &mut pps, None, None, true).unwrap();
    }
}
