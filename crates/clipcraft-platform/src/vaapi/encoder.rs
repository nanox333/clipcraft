//! The VA-API H.264 encoder: Constrained Baseline, one slice per picture, IPPP with a single
//! reference, and an IDR picture every `keyframe_frames` pictures.
//!
//! Every libva resource is owned by [`VaEncoder`] and released by its `Drop`, whichever step of
//! opening or encoding failed. Sizes the driver reports are bounded before they are used.

use std::ffi::{CStr, c_int, c_uint, c_void};
use std::fs::File;
use std::mem::size_of;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;

use clipcraft_record::{HardwareEncoder, HardwarePacket, HardwareRequest};

use super::ffi::*;
use crate::h264_headers::{self, LOG2_MAX_FRAME_NUM, LOG2_MAX_POC_LSB, PIC_INIT_QP, StreamShape};
use crate::h264_stream::{annexb_to_length_prefixed, build_avcc, conversion_threads, rgba_to_nv12};

/// Largest picture side accepted, in pixels.
const MAX_SIDE: u32 = 8_192;
/// Most render devices tried.
const MAX_DEVICES: usize = 8;
/// Most coded-buffer segments followed, so a damaged list cannot loop forever.
const MAX_SEGMENTS: usize = 64;
/// `NV12` as a FourCC.
const FOURCC_NV12: u32 = 0x3231_564e;
/// The input surface, then the two reconstructed pictures taking turns as reference.
const SURFACES: usize = 3;
/// Most entry points a driver may list.
const MAX_ENTRYPOINTS: usize = 64;

/// All-zero value of a plain-integer FFI structure.
fn zeroed<T: Copy>() -> Box<T> {
    // SAFETY: only used for the structures in `ffi`, which hold integers and arrays of integers
    // (no pointers, references, or enums), for which every bit pattern, zero included, is valid.
    Box::new(unsafe { std::mem::zeroed() })
}

/// The bytes of a plain-integer FFI structure.
fn bytes_of<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live reference, so the pointer is valid for `size_of::<T>()` bytes;
    // the structures passed hold only integers, so reading them as bytes is sound.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn render_nodes() -> Vec<PathBuf> {
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("renderD"))
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default();
    nodes.sort();
    nodes.truncate(MAX_DEVICES);
    nodes
}

fn validate(request: &HardwareRequest) -> Result<(), String> {
    let HardwareRequest {
        width,
        height,
        fps,
        video_kbps,
        ..
    } = *request;
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(format!("{width}x{height} is not an even, non-zero size"));
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("{width}x{height} is larger than the encoder takes"));
    }
    if fps == 0 || fps > 240 || video_kbps == 0 || video_kbps > 2_000_000 {
        return Err("the frame rate or bitrate is out of range".to_owned());
    }
    Ok(())
}

/// Opens the first render device whose VA-API driver can encode this recording.
pub fn open(request: &HardwareRequest) -> Result<Box<dyn HardwareEncoder>, String> {
    validate(request)?;
    let api = Arc::new(VaApi::load()?);
    let nodes = render_nodes();
    if nodes.is_empty() {
        return Err("there is no graphics render device (/dev/dri/renderD*)".to_owned());
    }
    let mut reasons = Vec::new();
    for node in nodes {
        let file = match File::options().read(true).write(true).open(&node) {
            Ok(file) => file,
            Err(error) => {
                reasons.push(format!("{}: {error}", node.display()));
                continue;
            }
        };
        let fd = file.as_raw_fd();
        match VaEncoder::open_on(api.clone(), fd, Some(file), request) {
            Ok(encoder) => return Ok(Box::new(encoder)),
            Err(why) => reasons.push(format!("{}: {why}", node.display())),
        }
    }
    Err(format!("VA-API: {}", reasons.join("; ")))
}

pub struct VaEncoder {
    api: Arc<VaApi>,
    /// Keeps the render device open for as long as the display uses it.
    _device: Option<File>,
    display: VADisplay,
    initialized: bool,
    config: VAConfigID,
    context: VAContextID,
    surfaces: Vec<VASurfaceID>,
    coded: VABufferID,
    coded_size: usize,
    /// Parameter buffers of the picture being encoded, destroyed when it is done.
    pending: Vec<VABufferID>,
    request: HardwareRequest,
    shape: StreamShape,
    aligned: (u32, u32),
    level: u8,
    rate_control: c_uint,
    packed_headers: bool,
    period: u32,
    since_idr: u32,
    idr_pic_id: u16,
    pictures: u64,
    reference: Option<VAPictureH264>,
    nv12: Vec<u8>,
    avcc: Vec<u8>,
    label: String,
}

// SAFETY: the display handle and the libva objects it owns are touched only through `&mut self`
// (or `&self` for read-only queries), never from two threads at once, and libva allows a display
// to be used from whichever thread holds it, one at a time.
unsafe impl Send for VaEncoder {}

impl VaEncoder {
    fn check(&self, status: VAStatus, what: &str) -> Result<(), String> {
        if status == VA_STATUS_SUCCESS {
            return Ok(());
        }
        // SAFETY: `vaErrorStr` returns a pointer to a static NUL-terminated string (or null).
        let text = unsafe {
            let pointer = (self.api.error_str)(status);
            (!pointer.is_null()).then(|| CStr::from_ptr(pointer).to_string_lossy().into_owned())
        };
        Err(format!(
            "{what} failed: {}",
            text.unwrap_or_else(|| format!("status {status}"))
        ))
    }

    /// Opens an encoder on one render device. On any error the partly built value is dropped,
    /// which releases whatever was created.
    pub(super) fn open_on(
        api: Arc<VaApi>,
        fd: c_int,
        device: Option<File>,
        request: &HardwareRequest,
    ) -> Result<Self, String> {
        validate(request)?;
        let shape = StreamShape {
            width: request.width,
            height: request.height,
            fps: request.fps,
        };
        let aligned = (shape.width_in_mbs() * 16, shape.height_in_mbs() * 16);
        let ten_seconds = request.fps.saturating_mul(10);
        let mut encoder = Self {
            api,
            _device: device,
            display: std::ptr::null_mut(),
            initialized: false,
            config: VA_INVALID_ID,
            context: VA_INVALID_ID,
            surfaces: Vec::new(),
            coded: VA_INVALID_ID,
            coded_size: 0,
            pending: Vec::new(),
            request: *request,
            shape,
            aligned,
            level: h264_headers::level_for(shape),
            rate_control: VA_RC_CBR,
            packed_headers: false,
            period: if request.keyframe_frames == 0 {
                ten_seconds
            } else {
                request.keyframe_frames
            }
            .clamp(1, 1 << 20),
            since_idr: 0,
            idr_pic_id: 0,
            pictures: 0,
            reference: None,
            nv12: Vec::new(),
            avcc: Vec::new(),
            label: "VA-API".to_owned(),
        };
        encoder.start(fd)?;
        Ok(encoder)
    }

    fn start(&mut self, fd: c_int) -> Result<(), String> {
        // SAFETY: `fd` is an open render device that outlives the display (`_device`).
        self.display = unsafe { (self.api.get_display_drm)(fd) };
        if self.display.is_null() {
            return Err("libva could not use the render device".to_owned());
        }
        let (mut major, mut minor) = (0, 0);
        // SAFETY: the display is valid and both out-pointers refer to live integers.
        let status = unsafe { (self.api.initialize)(self.display, &mut major, &mut minor) };
        self.check(status, "vaInitialize")?;
        self.initialized = true;
        self.label = self.name_from_vendor();

        let entrypoint = self.find_entrypoint()?;
        self.negotiate(entrypoint)?;
        self.create_resources()
    }

    fn name_from_vendor(&self) -> String {
        // SAFETY: the display is initialized; `vaQueryVendorString` returns a static string.
        let vendor = unsafe {
            let pointer = (self.api.query_vendor_string)(self.display);
            if pointer.is_null() {
                String::new()
            } else {
                CStr::from_ptr(pointer).to_string_lossy().into_owned()
            }
        };
        let lower = vendor.to_lowercase();
        if lower.contains("intel") {
            "Intel VA-API".to_owned()
        } else if lower.contains("mesa") || lower.contains("amd") || lower.contains("radeon") {
            "AMD VA-API".to_owned()
        } else {
            "VA-API".to_owned()
        }
    }

    fn find_entrypoint(&self) -> Result<c_int, String> {
        // SAFETY: the display is initialized.
        let reported = unsafe { (self.api.max_num_entrypoints)(self.display) };
        let capacity = usize::try_from(reported)
            .unwrap_or(0)
            .clamp(1, MAX_ENTRYPOINTS);
        let mut found = vec![0 as c_int; capacity];
        let mut count: c_int = 0;
        // SAFETY: `found` has room for the maximum number of entry points libva reports.
        let status = unsafe {
            (self.api.query_config_entrypoints)(
                self.display,
                VA_PROFILE_H264_CONSTRAINED_BASELINE,
                found.as_mut_ptr(),
                &mut count,
            )
        };
        self.check(status, "vaQueryConfigEntrypoints")?;
        let listed = found
            .get(..usize::try_from(count).unwrap_or(0).min(capacity))
            .unwrap_or(&[]);
        // The full encoder first; the low-power one is what newer Intel parts offer.
        [VA_ENTRYPOINT_ENC_SLICE, VA_ENTRYPOINT_ENC_SLICE_LP]
            .into_iter()
            .find(|entry| listed.contains(entry))
            .ok_or_else(|| "this graphics driver has no H.264 encoder".to_owned())
    }

    fn negotiate(&mut self, entrypoint: c_int) -> Result<(), String> {
        let mut attributes = [
            VAConfigAttrib {
                kind: VA_CONFIG_ATTRIB_RT_FORMAT,
                value: 0,
            },
            VAConfigAttrib {
                kind: VA_CONFIG_ATTRIB_RATE_CONTROL,
                value: 0,
            },
            VAConfigAttrib {
                kind: VA_CONFIG_ATTRIB_ENC_PACKED_HEADERS,
                value: 0,
            },
        ];
        // SAFETY: the array is live and its length is passed.
        let status = unsafe {
            (self.api.get_config_attributes)(
                self.display,
                VA_PROFILE_H264_CONSTRAINED_BASELINE,
                entrypoint,
                attributes.as_mut_ptr(),
                attributes.len() as c_int,
            )
        };
        self.check(status, "vaGetConfigAttributes")?;
        let [format, rate, packed] = attributes.map(|attribute| attribute.value);
        if format == VA_ATTRIB_NOT_SUPPORTED || format & VA_RT_FORMAT_YUV420 == 0 {
            return Err("the encoder does not take 4:2:0 pictures".to_owned());
        }
        let rate = if rate == VA_ATTRIB_NOT_SUPPORTED {
            0
        } else {
            rate
        };
        let wanted: [c_uint; 2] = if self.request.constant_bitrate {
            [VA_RC_CBR, VA_RC_VBR]
        } else {
            [VA_RC_VBR, VA_RC_CBR]
        };
        self.rate_control = wanted
            .into_iter()
            .find(|mode| rate & mode != 0)
            .ok_or_else(|| "the encoder cannot hold a bitrate (no CBR or VBR)".to_owned())?;
        let both = VA_ENC_PACKED_HEADER_SEQUENCE | VA_ENC_PACKED_HEADER_PICTURE;
        self.packed_headers = packed != VA_ATTRIB_NOT_SUPPORTED && packed & both == both;

        let mut chosen = vec![
            VAConfigAttrib {
                kind: VA_CONFIG_ATTRIB_RT_FORMAT,
                value: VA_RT_FORMAT_YUV420,
            },
            VAConfigAttrib {
                kind: VA_CONFIG_ATTRIB_RATE_CONTROL,
                value: self.rate_control,
            },
        ];
        if self.packed_headers {
            chosen.push(VAConfigAttrib {
                kind: VA_CONFIG_ATTRIB_ENC_PACKED_HEADERS,
                value: both,
            });
        }
        let mut config = VA_INVALID_ID;
        // SAFETY: `chosen` is live with its length passed; `config` is a live out-value.
        let status = unsafe {
            (self.api.create_config)(
                self.display,
                VA_PROFILE_H264_CONSTRAINED_BASELINE,
                entrypoint,
                chosen.as_mut_ptr(),
                chosen.len() as c_int,
                &mut config,
            )
        };
        self.check(status, "vaCreateConfig")?;
        self.config = config;
        Ok(())
    }

    fn create_resources(&mut self) -> Result<(), String> {
        let (width, height) = self.aligned;
        let mut surfaces = vec![VA_INVALID_SURFACE; SURFACES];
        // SAFETY: `surfaces` has room for the number of ids requested; no attributes are passed.
        let status = unsafe {
            (self.api.create_surfaces)(
                self.display,
                VA_RT_FORMAT_YUV420,
                width,
                height,
                surfaces.as_mut_ptr(),
                SURFACES as c_uint,
                std::ptr::null_mut(),
                0,
            )
        };
        self.check(status, "vaCreateSurfaces")?;
        self.surfaces = surfaces;

        let mut context = VA_INVALID_ID;
        // SAFETY: the surface array is live with its length passed; `context` is an out-value.
        let status = unsafe {
            (self.api.create_context)(
                self.display,
                self.config,
                width as c_int,
                height as c_int,
                VA_PROGRESSIVE,
                self.surfaces.as_mut_ptr(),
                SURFACES as c_int,
                &mut context,
            )
        };
        self.check(status, "vaCreateContext")?;
        self.context = context;

        self.coded_size = (width as usize * height as usize * 3 / 2).max(256 * 1024);
        let mut coded = VA_INVALID_ID;
        // SAFETY: a coded buffer takes its size and no initial data.
        let status = unsafe {
            (self.api.create_buffer)(
                self.display,
                self.context,
                VA_ENC_CODED_BUFFER_TYPE,
                self.coded_size as c_uint,
                1,
                std::ptr::null_mut(),
                &mut coded,
            )
        };
        self.check(status, "vaCreateBuffer (coded)")?;
        self.coded = coded;
        Ok(())
    }

    /// Creates a parameter buffer holding a copy of `data`, and remembers it for clean-up.
    fn buffer(&mut self, kind: c_int, data: &[u8]) -> Result<VABufferID, String> {
        let size = c_uint::try_from(data.len()).map_err(|_| "a buffer is too large".to_owned())?;
        let mut id = VA_INVALID_ID;
        // SAFETY: `data` is live for `size` bytes and libva copies it before returning.
        let status = unsafe {
            (self.api.create_buffer)(
                self.display,
                self.context,
                kind,
                size,
                1,
                data.as_ptr().cast_mut().cast::<c_void>(),
                &mut id,
            )
        };
        self.check(status, "vaCreateBuffer")?;
        self.pending.push(id);
        Ok(id)
    }

    fn misc(&mut self, kind: u32, payload: &[u8]) -> Result<VABufferID, String> {
        let mut data = kind.to_ne_bytes().to_vec();
        data.extend_from_slice(payload);
        self.buffer(VA_ENC_MISC_PARAMETER_BUFFER_TYPE, &data)
    }

    fn destroy_pending(&mut self) {
        for id in std::mem::take(&mut self.pending) {
            // SAFETY: `id` was created on this display and is destroyed once.
            unsafe { (self.api.destroy_buffer)(self.display, id) };
        }
    }

    /// Copies the picture into the input surface, repeating the last row and column into the
    /// padding up to a whole macroblock.
    fn upload(&mut self, rgba: &[u8]) -> Result<(), String> {
        let (width, height) = (self.request.width as usize, self.request.height as usize);
        let mut nv12 = std::mem::take(&mut self.nv12);
        let converted = rgba_to_nv12(
            rgba,
            self.request.width,
            self.request.height,
            &mut nv12,
            conversion_threads(),
        );
        let result = converted.and_then(|()| self.write_surface(&nv12, width, height));
        self.nv12 = nv12;
        result
    }

    fn write_surface(&self, nv12: &[u8], width: usize, height: usize) -> Result<(), String> {
        let surface = *self.surfaces.first().ok_or("no input surface")?;
        let mut image = zeroed::<VAImage>();
        // SAFETY: `surface` belongs to this display and `image` is a live out-value.
        let status = unsafe { (self.api.derive_image)(self.display, surface, &mut *image) };
        self.check(status, "vaDeriveImage")?;
        let result = self.fill_image(&image, nv12, width, height);
        // SAFETY: the image was derived above and is destroyed once.
        unsafe { (self.api.destroy_image)(self.display, image.image_id) };
        result
    }

    fn fill_image(
        &self,
        image: &VAImage,
        nv12: &[u8],
        width: usize,
        height: usize,
    ) -> Result<(), String> {
        let (aligned_w, aligned_h) = (self.aligned.0 as usize, self.aligned.1 as usize);
        if image.format.fourcc != FOURCC_NV12 || image.num_planes < 2 {
            return Err("the driver's input surfaces are not NV12".to_owned());
        }
        let size = image.data_size as usize;
        let luma = (image.offsets[0] as usize, image.pitches[0] as usize);
        let chroma = (image.offsets[1] as usize, image.pitches[1] as usize);
        let luma_end = luma
            .1
            .checked_mul(aligned_h)
            .and_then(|n| n.checked_add(luma.0));
        let chroma_end = chroma
            .1
            .checked_mul(aligned_h / 2)
            .and_then(|n| n.checked_add(chroma.0));
        match (luma_end, chroma_end) {
            (Some(a), Some(b))
                if a <= size && b <= size && luma.1 >= aligned_w && chroma.1 >= aligned_w => {}
            _ => return Err("the driver's input surface layout is implausible".to_owned()),
        }
        let mut mapped: *mut c_void = std::ptr::null_mut();
        // SAFETY: the image buffer belongs to this display; `mapped` is a live out-value.
        let status = unsafe { (self.api.map_buffer)(self.display, image.buf, &mut mapped) };
        self.check(status, "vaMapBuffer (image)")?;
        if mapped.is_null() {
            // SAFETY: the buffer was mapped by the call above.
            unsafe { (self.api.unmap_buffer)(self.display, image.buf) };
            return Err("the driver mapped no memory".to_owned());
        }
        // SAFETY: libva maps `data_size` bytes for the image buffer, and the layout above was
        // checked to lie inside them; nothing else touches the mapping until it is unmapped.
        let destination = unsafe { std::slice::from_raw_parts_mut(mapped.cast::<u8>(), size) };
        let (luma_in, chroma_in) = nv12.split_at(width * height);
        for row in 0..aligned_h {
            let source = &luma_in[row.min(height - 1) * width..][..width];
            let start = luma.0 + row * luma.1;
            let target = &mut destination[start..start + aligned_w];
            target[..width].copy_from_slice(source);
            target[width..].fill(source[width - 1]);
        }
        for row in 0..aligned_h / 2 {
            let source = &chroma_in[row.min(height / 2 - 1) * width..][..width];
            let start = chroma.0 + row * chroma.1;
            let target = &mut destination[start..start + aligned_w];
            target[..width].copy_from_slice(source);
            for pair in target[width..].chunks_exact_mut(2) {
                pair.copy_from_slice(&source[width - 2..]);
            }
        }
        // SAFETY: the buffer was mapped above and `destination` is not used again.
        let status = unsafe { (self.api.unmap_buffer)(self.display, image.buf) };
        self.check(status, "vaUnmapBuffer (image)")
    }

    fn sequence_buffer(&mut self) -> Result<VABufferID, String> {
        let mut sequence = zeroed::<VAEncSequenceParameterBufferH264>();
        sequence.level_idc = self.level;
        sequence.intra_period = self.period;
        sequence.intra_idr_period = self.period;
        sequence.ip_period = 1;
        sequence.bits_per_second = self.request.video_kbps.saturating_mul(1_000);
        sequence.max_num_ref_frames = 1;
        sequence.picture_width_in_mbs = self.shape.width_in_mbs() as u16;
        sequence.picture_height_in_mbs = self.shape.height_in_mbs() as u16;
        // chroma_format_idc 1, frame_mbs_only, direct_8x8_inference, the two log2 sizes,
        // picture order count type 0.
        sequence.seq_fields =
            1 | 1 << 2 | 1 << 5 | (LOG2_MAX_FRAME_NUM - 4) << 6 | (LOG2_MAX_POC_LSB - 4) << 12;
        if let Some((left, right, top, bottom)) = self.shape.crop() {
            sequence.frame_cropping_flag = 1;
            sequence.frame_crop_left_offset = left;
            sequence.frame_crop_right_offset = right;
            sequence.frame_crop_top_offset = top;
            sequence.frame_crop_bottom_offset = bottom;
        }
        self.buffer(VA_ENC_SEQUENCE_PARAMETER_BUFFER_TYPE, bytes_of(&*sequence))
    }

    fn packed(&mut self, kind: u32, nal: &[u8]) -> Result<[VABufferID; 2], String> {
        let mut data = vec![0, 0, 0, 1];
        data.extend_from_slice(nal);
        let mut header = zeroed::<VAEncPackedHeaderParameterBuffer>();
        header.kind = kind;
        header.bit_length = u32::try_from(data.len() * 8).map_err(|_| "header too large")?;
        // The writers in `h264_headers` already escape the payload.
        header.has_emulation_bytes = 1;
        let parameter = self.buffer(
            VA_ENC_PACKED_HEADER_PARAMETER_BUFFER_TYPE,
            bytes_of(&*header),
        )?;
        let payload = self.buffer(VA_ENC_PACKED_HEADER_DATA_BUFFER_TYPE, &data)?;
        Ok([parameter, payload])
    }

    fn rate_buffers(&mut self) -> Result<Vec<VABufferID>, String> {
        let bits = self.request.video_kbps.saturating_mul(1_000);
        let variable = self.rate_control == VA_RC_VBR;
        let mut rate = VAEncMiscParameterRateControl {
            bits_per_second: bits,
            target_percentage: 100,
            window_size: 1_000,
            initial_qp: PIC_INIT_QP,
            min_qp: 1,
            basic_unit_size: 0,
            rc_flags: 0,
            icq_quality_factor: 0,
            max_qp: 0,
            quality_factor: 0,
            target_frame_size: 0,
            va_reserved: [0; VA_PADDING_LOW],
        };
        if variable {
            // The peak is a quarter above the requested average.
            rate.bits_per_second = bits.saturating_add(bits / 4);
            rate.target_percentage = 80;
        }
        let frame_rate = VAEncMiscParameterFrameRate {
            framerate: self.request.fps,
            framerate_flags: 0,
            va_reserved: [0; VA_PADDING_LOW],
        };
        let hrd = VAEncMiscParameterHrd {
            initial_buffer_fullness: bits / 2,
            buffer_size: bits,
            va_reserved: [0; VA_PADDING_LOW],
        };
        Ok(vec![
            self.misc(VA_ENC_MISC_PARAMETER_TYPE_FRAME_RATE, bytes_of(&frame_rate))?,
            self.misc(VA_ENC_MISC_PARAMETER_TYPE_RATE_CONTROL, bytes_of(&rate))?,
            self.misc(VA_ENC_MISC_PARAMETER_TYPE_HRD, bytes_of(&hrd))?,
        ])
    }

    /// Everything libva needs to code the picture except the pixels.
    fn picture_buffers(
        &mut self,
        current: &VAPictureH264,
        idr: bool,
        frame_num: u32,
        poc: u32,
    ) -> Result<Vec<VABufferID>, String> {
        let mut ids = Vec::new();
        if idr {
            ids.push(self.sequence_buffer()?);
            ids.extend(self.rate_buffers()?);
            if self.packed_headers {
                let shape = self.shape;
                let sps = h264_headers::sps(shape, self.level);
                ids.extend(self.packed(VA_ENC_PACKED_HEADER_TYPE_SEQUENCE, &sps)?);
            }
        }

        let mut picture = zeroed::<VAEncPictureParameterBufferH264>();
        picture.curr_pic = *current;
        picture.reference_frames = [VAPictureH264::INVALID; 16];
        if let (false, Some(reference)) = (idr, self.reference) {
            picture.reference_frames[0] = reference;
        }
        picture.coded_buf = self.coded;
        picture.frame_num = frame_num as u16;
        picture.pic_init_qp = PIC_INIT_QP as u8;
        // idr_pic_flag, reference_pic_flag, deblocking_filter_control_present_flag.
        picture.pic_fields = u32::from(idr) | 1 << 1 | 1 << 9;
        ids.push(self.buffer(VA_ENC_PICTURE_PARAMETER_BUFFER_TYPE, bytes_of(&*picture))?);
        if self.packed_headers && idr {
            ids.extend(self.packed(VA_ENC_PACKED_HEADER_TYPE_PICTURE, &h264_headers::pps())?);
        }

        let mut slice = zeroed::<VAEncSliceParameterBufferH264>();
        slice.num_macroblocks = self.shape.width_in_mbs() * self.shape.height_in_mbs();
        slice.slice_type = if idr { 2 } else { 0 };
        slice.idr_pic_id = self.idr_pic_id;
        slice.pic_order_cnt_lsb = (poc % (1 << LOG2_MAX_POC_LSB)) as u16;
        slice.ref_pic_list0 = [VAPictureH264::INVALID; 32];
        slice.ref_pic_list1 = [VAPictureH264::INVALID; 32];
        if let (false, Some(reference)) = (idr, self.reference) {
            slice.ref_pic_list0[0] = reference;
        }
        ids.push(self.buffer(VA_ENC_SLICE_PARAMETER_BUFFER_TYPE, bytes_of(&*slice))?);
        Ok(ids)
    }

    fn encode_picture(&mut self, rgba: &[u8]) -> Result<HardwarePacket, String> {
        self.upload(rgba)?;
        let idr = self.since_idr == 0;
        let frame_num = self.since_idr % (1 << LOG2_MAX_FRAME_NUM);
        let poc = self.since_idr * 2;
        let recon = *self
            .surfaces
            .get(1 + (self.pictures % 2) as usize)
            .ok_or("no reconstruction surface")?;
        let input = *self.surfaces.first().ok_or("no input surface")?;
        let current = VAPictureH264 {
            picture_id: recon,
            frame_idx: frame_num,
            flags: VA_PICTURE_H264_SHORT_TERM_REFERENCE,
            top_field_order_cnt: poc as i32,
            bottom_field_order_cnt: poc as i32,
            va_reserved: [0; VA_PADDING_LOW],
        };

        let result = self.run_picture(input, &current, idr, frame_num, poc);
        self.destroy_pending();
        let annexb = result?;

        let converted = annexb_to_length_prefixed(&annexb)?;
        if self.avcc.is_empty() {
            // Prefer the parameter sets the driver actually put in the stream.
            let (sps, pps) = match (&converted.sps, &converted.pps) {
                (Some(sps), Some(pps)) => (sps.clone(), pps.clone()),
                _ => (
                    h264_headers::sps(self.shape, self.level),
                    h264_headers::pps(),
                ),
            };
            self.avcc = build_avcc(&sps, &pps)?;
        }
        self.reference = Some(current);
        if idr {
            self.idr_pic_id = self.idr_pic_id.wrapping_add(1);
        }
        self.pictures += 1;
        self.since_idr += 1;
        if self.since_idr >= self.period {
            self.since_idr = 0;
        }
        Ok(HardwarePacket {
            data: converted.data,
            keyframe: idr || converted.keyframe,
            reordered: false,
        })
    }

    fn run_picture(
        &mut self,
        input: VASurfaceID,
        current: &VAPictureH264,
        idr: bool,
        frame_num: u32,
        poc: u32,
    ) -> Result<Vec<u8>, String> {
        let mut ids = self.picture_buffers(current, idr, frame_num, poc)?;
        // The picture is coded from the input surface into the reconstruction surface named in
        // the parameters: libva takes the input here and the reference in the picture buffer.
        // SAFETY: context and surface belong to this display.
        let status = unsafe { (self.api.begin_picture)(self.display, self.context, input) };
        self.check(status, "vaBeginPicture")?;
        // SAFETY: `ids` is live with its length passed.
        let status = unsafe {
            (self.api.render_picture)(
                self.display,
                self.context,
                ids.as_mut_ptr(),
                ids.len() as c_int,
            )
        };
        let rendered = self.check(status, "vaRenderPicture");
        // SAFETY: a picture was begun; it must be ended even when rendering failed.
        let status = unsafe { (self.api.end_picture)(self.display, self.context) };
        rendered?;
        self.check(status, "vaEndPicture")?;
        // SAFETY: the surface belongs to this display.
        let status = unsafe { (self.api.sync_surface)(self.display, input) };
        self.check(status, "vaSyncSurface")?;
        self.read_coded()
    }

    fn read_coded(&self) -> Result<Vec<u8>, String> {
        let mut mapped: *mut c_void = std::ptr::null_mut();
        // SAFETY: the coded buffer belongs to this display; `mapped` is a live out-value.
        let status = unsafe { (self.api.map_buffer)(self.display, self.coded, &mut mapped) };
        self.check(status, "vaMapBuffer (coded)")?;
        let mut out = Vec::new();
        let mut segment = mapped.cast::<VACodedBufferSegment>();
        let mut problem = None;
        for _ in 0..MAX_SEGMENTS {
            if segment.is_null() {
                break;
            }
            // SAFETY: libva's mapped coded buffer is a linked list of segments, each valid until
            // the buffer is unmapped; the pointer is non-null here.
            let current = unsafe { *segment };
            let size = current.size as usize;
            if current.buf.is_null() || out.len() + size > self.coded_size {
                problem = Some("the driver returned an implausible coded segment");
                break;
            }
            // SAFETY: the segment says `size` bytes at `buf` are valid until unmapping.
            out.extend_from_slice(unsafe {
                std::slice::from_raw_parts(current.buf.cast::<u8>(), size)
            });
            segment = current.next.cast::<VACodedBufferSegment>();
        }
        // SAFETY: the buffer was mapped above; nothing borrowed from it is used afterwards.
        let status = unsafe { (self.api.unmap_buffer)(self.display, self.coded) };
        if let Some(problem) = problem {
            return Err(problem.to_owned());
        }
        self.check(status, "vaUnmapBuffer (coded)")?;
        if out.is_empty() {
            return Err("the encoder produced no data".to_owned());
        }
        Ok(out)
    }
}

impl HardwareEncoder for VaEncoder {
    fn encode(&mut self, rgba: &[u8], _index: u64) -> Result<Vec<HardwarePacket>, String> {
        Ok(vec![self.encode_picture(rgba)?])
    }

    fn flush(&mut self) -> Result<Vec<HardwarePacket>, String> {
        // IPPP: nothing is held back.
        Ok(Vec::new())
    }

    fn avcc(&self) -> Vec<u8> {
        self.avcc.clone()
    }

    fn name(&self) -> String {
        self.label.clone()
    }
}

impl Drop for VaEncoder {
    fn drop(&mut self) {
        if self.display.is_null() {
            return;
        }
        self.destroy_pending();
        // SAFETY: each id was created on this display, is valid here only if not the invalid
        // marker, and is destroyed once, in the order libva requires (buffers, context,
        // surfaces, config, display).
        unsafe {
            if self.coded != VA_INVALID_ID {
                (self.api.destroy_buffer)(self.display, self.coded);
            }
            if self.context != VA_INVALID_ID {
                (self.api.destroy_context)(self.display, self.context);
            }
            if !self.surfaces.is_empty() {
                (self.api.destroy_surfaces)(
                    self.display,
                    self.surfaces.as_mut_ptr(),
                    self.surfaces.len() as c_int,
                );
            }
            if self.config != VA_INVALID_ID {
                (self.api.destroy_config)(self.display, self.config);
            }
            if self.initialized {
                (self.api.terminate)(self.display);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! The encoder against a stand-in libva that records every call, so call order, buffer
    //! contents, uploaded pixels, and clean-up after a failure at any step are checked without a
    //! graphics card.
    use super::*;
    use std::ffi::c_char;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Mock {
        calls: Vec<String>,
        /// Name of the function that fails.
        fail: Option<&'static str>,
        live_buffers: i32,
        live_surfaces: i32,
        live_contexts: i32,
        live_configs: i32,
        live_images: i32,
        mapped: i32,
        initialized: i32,
        next_id: u32,
        /// (type, bytes) of every created buffer, in order.
        created: Vec<(c_int, Vec<u8>)>,
        /// What the next coded buffer holds.
        output: Vec<u8>,
        segment: Option<Held>,
        /// Backing memory for the derived image.
        surface: Vec<u8>,
        offered_rate: u32,
        offered_packed: u32,
        offered_entrypoints: Vec<c_int>,
        coded_ids: Vec<u32>,
        render_batches: Vec<Vec<u32>>,
    }

    /// A segment kept alive for the encoder to read; only one test thread holds it at a time.
    struct Held(#[allow(dead_code)] Box<VACodedBufferSegment>);
    // SAFETY: the mock is behind a mutex and the pointers inside are only read by the encoder
    // under the test lock.
    unsafe impl Send for Held {}

    static MOCK: Mutex<Option<Mock>> = Mutex::new(None);
    /// The mock is global, so tests that use it take turns.
    static TURN: Mutex<()> = Mutex::new(());

    fn with<R>(f: impl FnOnce(&mut Mock) -> R) -> R {
        let mut guard = MOCK.lock().unwrap_or_else(|e| e.into_inner());
        f(guard.get_or_insert_with(Mock::default))
    }

    /// Records the call and says whether it should fail.
    fn call(name: &'static str) -> bool {
        with(|m| {
            m.calls.push(name.to_owned());
            m.fail == Some(name)
        })
    }

    fn status(name: &'static str) -> VAStatus {
        if call(name) { 7 } else { 0 }
    }

    unsafe extern "C" fn get_display(_fd: c_int) -> VADisplay {
        if call("vaGetDisplayDRM") {
            return std::ptr::null_mut();
        }
        1 as VADisplay
    }
    unsafe extern "C" fn initialize(
        _d: VADisplay,
        major: *mut c_int,
        minor: *mut c_int,
    ) -> VAStatus {
        let s = status("vaInitialize");
        if s == 0 {
            with(|m| m.initialized += 1);
            // SAFETY: the encoder passes live integers.
            unsafe {
                *major = 1;
                *minor = 20;
            }
        }
        s
    }
    unsafe extern "C" fn terminate(_d: VADisplay) -> VAStatus {
        with(|m| m.initialized -= 1);
        status("vaTerminate")
    }
    unsafe extern "C" fn vendor(_d: VADisplay) -> *const c_char {
        c"Mesa Gallium driver for AMD Radeon".as_ptr()
    }
    unsafe extern "C" fn max_entrypoints(_d: VADisplay) -> c_int {
        8
    }
    unsafe extern "C" fn entrypoints(
        _d: VADisplay,
        _profile: c_int,
        list: *mut c_int,
        count: *mut c_int,
    ) -> VAStatus {
        let s = status("vaQueryConfigEntrypoints");
        if s == 0 {
            let offered = with(|m| m.offered_entrypoints.clone());
            // SAFETY: the encoder passes room for `vaMaxNumEntrypoints` (8) entries.
            unsafe {
                for (i, entry) in offered.iter().enumerate() {
                    *list.add(i) = *entry;
                }
                *count = offered.len() as c_int;
            }
        }
        s
    }
    unsafe extern "C" fn attributes(
        _d: VADisplay,
        _profile: c_int,
        _entry: c_int,
        list: *mut VAConfigAttrib,
        count: c_int,
    ) -> VAStatus {
        let s = status("vaGetConfigAttributes");
        if s == 0 {
            let (rate, packed) = with(|m| (m.offered_rate, m.offered_packed));
            for i in 0..count as usize {
                // SAFETY: the encoder passes `count` live attributes.
                let attribute = unsafe { &mut *list.add(i) };
                attribute.value = match attribute.kind {
                    VA_CONFIG_ATTRIB_RT_FORMAT => VA_RT_FORMAT_YUV420,
                    VA_CONFIG_ATTRIB_RATE_CONTROL => rate,
                    _ => packed,
                };
            }
        }
        s
    }
    unsafe extern "C" fn create_config(
        _d: VADisplay,
        _p: c_int,
        _e: c_int,
        _a: *mut VAConfigAttrib,
        _n: c_int,
        id: *mut VAConfigID,
    ) -> VAStatus {
        let s = status("vaCreateConfig");
        if s == 0 {
            with(|m| {
                m.live_configs += 1;
                // SAFETY: the encoder passes a live out-value.
                unsafe { *id = 100 };
            });
        }
        s
    }
    unsafe extern "C" fn destroy_config(_d: VADisplay, _c: VAConfigID) -> VAStatus {
        with(|m| m.live_configs -= 1);
        status("vaDestroyConfig")
    }
    unsafe extern "C" fn create_surfaces(
        _d: VADisplay,
        _f: c_uint,
        _w: c_uint,
        _h: c_uint,
        ids: *mut VASurfaceID,
        n: c_uint,
        _a: *mut c_void,
        _na: c_uint,
    ) -> VAStatus {
        let s = status("vaCreateSurfaces");
        if s == 0 {
            with(|m| {
                m.live_surfaces += n as i32;
                for i in 0..n {
                    // SAFETY: the encoder passes room for `n` ids.
                    unsafe { *ids.add(i as usize) = 10 + i };
                }
            });
        }
        s
    }
    unsafe extern "C" fn destroy_surfaces(
        _d: VADisplay,
        _ids: *mut VASurfaceID,
        n: c_int,
    ) -> VAStatus {
        with(|m| m.live_surfaces -= n);
        status("vaDestroySurfaces")
    }
    unsafe extern "C" fn create_context(
        _d: VADisplay,
        _c: VAConfigID,
        _w: c_int,
        _h: c_int,
        _f: c_int,
        _t: *mut VASurfaceID,
        _n: c_int,
        id: *mut VAContextID,
    ) -> VAStatus {
        let s = status("vaCreateContext");
        if s == 0 {
            with(|m| {
                m.live_contexts += 1;
                // SAFETY: the encoder passes a live out-value.
                unsafe { *id = 200 };
            });
        }
        s
    }
    unsafe extern "C" fn destroy_context(_d: VADisplay, _c: VAContextID) -> VAStatus {
        with(|m| m.live_contexts -= 1);
        status("vaDestroyContext")
    }
    unsafe extern "C" fn create_buffer(
        _d: VADisplay,
        _c: VAContextID,
        kind: c_int,
        size: c_uint,
        _n: c_uint,
        data: *mut c_void,
        id: *mut VABufferID,
    ) -> VAStatus {
        let s = status("vaCreateBuffer");
        if s == 0 {
            with(|m| {
                m.live_buffers += 1;
                m.next_id += 1;
                let new = 1_000 + m.next_id;
                let bytes = if data.is_null() {
                    Vec::new()
                } else {
                    // SAFETY: the encoder passes `size` readable bytes.
                    unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size as usize) }.to_vec()
                };
                if kind == VA_ENC_CODED_BUFFER_TYPE {
                    m.coded_ids.push(new);
                }
                m.created.push((kind, bytes));
                // SAFETY: the encoder passes a live out-value.
                unsafe { *id = new };
            });
        }
        s
    }
    unsafe extern "C" fn destroy_buffer(_d: VADisplay, _b: VABufferID) -> VAStatus {
        with(|m| m.live_buffers -= 1);
        status("vaDestroyBuffer")
    }
    unsafe extern "C" fn map_buffer(
        _d: VADisplay,
        id: VABufferID,
        out: *mut *mut c_void,
    ) -> VAStatus {
        let s = status("vaMapBuffer");
        if s == 0 {
            with(|m| {
                m.mapped += 1;
                let pointer: *mut c_void = if m.coded_ids.contains(&id) {
                    let segment = Box::new(VACodedBufferSegment {
                        size: m.output.len() as u32,
                        bit_offset: 0,
                        status: 0,
                        reserved: 0,
                        buf: m.output.as_mut_ptr().cast(),
                        next: std::ptr::null_mut(),
                        va_reserved: [0; VA_PADDING_LOW],
                    });
                    let raw = (&*segment as *const VACodedBufferSegment).cast_mut().cast();
                    m.segment = Some(Held(segment));
                    raw
                } else {
                    m.surface.as_mut_ptr().cast()
                };
                // SAFETY: the encoder passes a live out-value.
                unsafe { *out = pointer };
            });
        }
        s
    }
    unsafe extern "C" fn unmap_buffer(_d: VADisplay, _b: VABufferID) -> VAStatus {
        with(|m| m.mapped -= 1);
        status("vaUnmapBuffer")
    }
    unsafe extern "C" fn begin(_d: VADisplay, _c: VAContextID, _s: VASurfaceID) -> VAStatus {
        status("vaBeginPicture")
    }
    unsafe extern "C" fn render(
        _d: VADisplay,
        _c: VAContextID,
        ids: *mut VABufferID,
        n: c_int,
    ) -> VAStatus {
        with(|m| {
            // SAFETY: the encoder passes `n` buffer ids.
            let batch = unsafe { std::slice::from_raw_parts(ids, n as usize) }.to_vec();
            m.render_batches.push(batch);
        });
        status("vaRenderPicture")
    }
    unsafe extern "C" fn end(_d: VADisplay, _c: VAContextID) -> VAStatus {
        status("vaEndPicture")
    }
    unsafe extern "C" fn sync(_d: VADisplay, _s: VASurfaceID) -> VAStatus {
        status("vaSyncSurface")
    }
    unsafe extern "C" fn derive(_d: VADisplay, _s: VASurfaceID, image: *mut VAImage) -> VAStatus {
        let s = status("vaDeriveImage");
        if s == 0 {
            with(|m| {
                m.live_images += 1;
                // Pitch 64 for a 48-wide surface, to prove the pitch is honoured.
                let (pitch, rows) = (64usize, 32usize);
                m.surface = vec![0xAA; pitch * rows * 3 / 2];
                // SAFETY: the encoder passes a live out-value.
                let image = unsafe { &mut *image };
                image.image_id = 300;
                image.format.fourcc = FOURCC_NV12;
                image.buf = 400;
                image.data_size = m.surface.len() as u32;
                image.num_planes = 2;
                image.pitches = [pitch as u32, pitch as u32, 0];
                image.offsets = [0, (pitch * rows) as u32, 0];
            });
        }
        s
    }
    unsafe extern "C" fn destroy_image(_d: VADisplay, _i: VAImageID) -> VAStatus {
        with(|m| m.live_images -= 1);
        status("vaDestroyImage")
    }
    unsafe extern "C" fn error_str(_s: VAStatus) -> *const c_char {
        c"mock failure".as_ptr()
    }

    fn api() -> Arc<VaApi> {
        Arc::new(VaApi {
            get_display_drm: get_display,
            initialize,
            terminate,
            query_vendor_string: vendor,
            max_num_entrypoints: max_entrypoints,
            query_config_entrypoints: entrypoints,
            get_config_attributes: attributes,
            create_config,
            destroy_config,
            create_surfaces,
            destroy_surfaces,
            create_context,
            destroy_context,
            create_buffer,
            destroy_buffer,
            map_buffer,
            unmap_buffer,
            begin_picture: begin,
            render_picture: render,
            end_picture: end,
            sync_surface: sync,
            derive_image: derive,
            destroy_image,
            error_str,
            _libraries: None,
        })
    }

    fn request() -> HardwareRequest {
        HardwareRequest {
            codec: clipcraft_record::StreamCodec::H264,
            width: 40,
            height: 20,
            fps: 30,
            video_kbps: 4_000,
            constant_bitrate: true,
            keyframe_frames: 3,
            effort: clipcraft_record::EncoderSpeed::Balanced,
        }
    }

    fn annexb(idr: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if idr {
            for nal in [
                h264_headers::sps(
                    StreamShape {
                        width: 40,
                        height: 20,
                        fps: 30,
                    },
                    31,
                ),
                h264_headers::pps(),
            ] {
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(&nal);
            }
        }
        out.extend_from_slice(&[0, 0, 0, 1, if idr { 0x65 } else { 0x41 }, 0x88, 0x84, 0x21]);
        out
    }

    /// Resets the mock and returns the lock that keeps other tests out.
    fn start(configure: impl FnOnce(&mut Mock)) -> std::sync::MutexGuard<'static, ()> {
        let turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        *MOCK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Mock {
            offered_rate: VA_RC_CBR | VA_RC_VBR,
            offered_packed: VA_ENC_PACKED_HEADER_SEQUENCE | VA_ENC_PACKED_HEADER_PICTURE,
            offered_entrypoints: vec![VA_ENTRYPOINT_ENC_SLICE],
            ..Mock::default()
        });
        with(configure);
        turn
    }

    fn open_mock(request: &HardwareRequest) -> Result<VaEncoder, String> {
        VaEncoder::open_on(api(), 5, None, request)
    }

    fn picture() -> Vec<u8> {
        vec![200; 40 * 20 * 4]
    }

    fn live() -> [i32; 7] {
        with(|m| {
            [
                m.live_buffers,
                m.live_surfaces,
                m.live_contexts,
                m.live_configs,
                m.live_images,
                m.mapped,
                m.initialized,
            ]
        })
    }

    #[test]
    fn encodes_an_idr_picture_then_predicted_ones_and_releases_everything() {
        let _turn = start(|_| {});
        {
            let mut encoder = open_mock(&request()).unwrap();
            assert_eq!(encoder.name(), "AMD VA-API");
            for index in 0..7u64 {
                with(|m| m.output = annexb(index % 3 == 0));
                let packets = encoder.encode(&picture(), index).unwrap();
                assert_eq!(packets.len(), 1);
                assert_eq!(packets[0].keyframe, index % 3 == 0, "picture {index}");
                assert!(!packets[0].reordered);
                // Length-prefixed, parameter sets removed.
                assert_eq!(&packets[0].data[..4], &[0, 0, 0, 4]);
            }
            assert!(encoder.flush().unwrap().is_empty());
            assert_eq!(encoder.avcc()[0], 1);
            // Mid-life: only the coded buffer, context, surfaces and config remain.
            assert_eq!(live(), [1, 3, 1, 1, 0, 0, 1]);
        }
        assert_eq!(live(), [0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn sequence_data_and_slice_types_follow_the_gop() {
        let _turn = start(|_| {});
        let mut encoder = open_mock(&request()).unwrap();
        for index in 0..4u64 {
            with(|m| m.output = annexb(index % 3 == 0));
            encoder.encode(&picture(), index).unwrap();
        }
        let (batches, created) = with(|m| (m.render_batches.clone(), m.created.clone()));
        assert_eq!(batches.len(), 4);
        let kinds = |batch: &[u32]| -> Vec<c_int> {
            batch
                .iter()
                .map(|id| created[(id - 1_001) as usize].0)
                .collect()
        };
        let idr = kinds(&batches[0]);
        assert_eq!(idr[0], VA_ENC_SEQUENCE_PARAMETER_BUFFER_TYPE);
        assert!(idr.contains(&VA_ENC_PACKED_HEADER_DATA_BUFFER_TYPE));
        assert_eq!(
            idr.iter()
                .filter(|k| **k == VA_ENC_MISC_PARAMETER_BUFFER_TYPE)
                .count(),
            3
        );
        // A predicted picture has only the picture and slice parameters.
        assert_eq!(
            kinds(&batches[1]),
            vec![
                VA_ENC_PICTURE_PARAMETER_BUFFER_TYPE,
                VA_ENC_SLICE_PARAMETER_BUFFER_TYPE
            ]
        );
        assert_eq!(kinds(&batches[3])[0], VA_ENC_SEQUENCE_PARAMETER_BUFFER_TYPE);

        let slice = |batch: &[u32]| -> Box<VAEncSliceParameterBufferH264> {
            let id = batch.last().copied().unwrap_or(0);
            let bytes = &created[(id - 1_001) as usize].1;
            let mut slice = zeroed::<VAEncSliceParameterBufferH264>();
            assert_eq!(bytes.len(), size_of::<VAEncSliceParameterBufferH264>());
            // SAFETY: the buffer holds exactly one slice structure written by the encoder.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    (&mut *slice as *mut VAEncSliceParameterBufferH264).cast::<u8>(),
                    bytes.len(),
                );
            }
            slice
        };
        let (first, second, fourth) = (slice(&batches[0]), slice(&batches[1]), slice(&batches[3]));
        assert_eq!(
            (first.slice_type, second.slice_type, fourth.slice_type),
            (2, 0, 2)
        );
        assert_eq!(first.pic_order_cnt_lsb, 0);
        assert_eq!(second.pic_order_cnt_lsb, 2);
        assert_ne!(first.idr_pic_id, fourth.idr_pic_id);
        assert_eq!(first.ref_pic_list0[0].flags, VA_PICTURE_H264_INVALID);
        assert_ne!(second.ref_pic_list0[0].picture_id, VA_INVALID_SURFACE);
        assert_eq!(first.num_macroblocks, 3 * 2);
    }

    #[test]
    fn the_picture_is_uploaded_with_its_edges_repeated_into_the_padding() {
        let _turn = start(|_| {});
        let mut encoder = open_mock(&request()).unwrap();
        with(|m| m.output = annexb(true));
        // Left half white, right half black.
        let mut rgba = vec![0u8; 40 * 20 * 4];
        for (i, pixel) in rgba.chunks_exact_mut(4).enumerate() {
            let white = i % 40 < 20;
            pixel.copy_from_slice(&[
                if white { 255 } else { 0 },
                if white { 255 } else { 0 },
                if white { 255 } else { 0 },
                255,
            ]);
        }
        encoder.encode(&rgba, 0).unwrap();
        let surface = with(|m| m.surface.clone());
        let pitch = 64;
        let row = |y: usize| &surface[y * pitch..y * pitch + 48];
        assert_eq!(row(0)[0], 235);
        assert_eq!(row(0)[39], 16);
        // Columns 40..48 repeat column 39; rows 20..32 repeat row 19.
        assert!(row(0)[40..48].iter().all(|v| *v == 16));
        assert_eq!(row(25), row(19));
        // The bytes beyond the aligned width are untouched.
        assert_eq!(surface[48], 0xAA);
        // Chroma: pairs repeat into the padding.
        let chroma = pitch * 32;
        assert_eq!(surface[chroma + 46], surface[chroma + 38]);
        assert_eq!(surface[chroma + 47], surface[chroma + 39]);
    }

    #[test]
    fn works_without_packed_headers_and_with_the_low_power_entry_point() {
        let _turn = start(|m| {
            m.offered_packed = VA_ATTRIB_NOT_SUPPORTED;
            m.offered_entrypoints = vec![VA_ENTRYPOINT_ENC_SLICE_LP];
            m.offered_rate = VA_RC_VBR;
        });
        let mut encoder = open_mock(&request()).unwrap();
        with(|m| m.output = annexb(true));
        encoder.encode(&picture(), 0).unwrap();
        let created = with(|m| m.created.clone());
        assert!(
            !created
                .iter()
                .any(|(k, _)| *k == VA_ENC_PACKED_HEADER_DATA_BUFFER_TYPE)
        );
    }

    #[test]
    fn a_driver_without_h264_or_bitrate_control_is_refused_with_a_reason() {
        {
            let _turn = start(|m| m.offered_entrypoints = vec![]);
            let error = open_mock(&request()).err().unwrap();
            assert!(error.contains("no H.264 encoder"), "{error}");
            assert_eq!(live(), [0; 7]);
        }
        let _turn = start(|m| m.offered_rate = 0x10);
        let error = open_mock(&request()).err().unwrap();
        assert!(error.contains("CBR or VBR"), "{error}");
        assert_eq!(live(), [0; 7]);
    }

    #[test]
    fn a_failure_at_any_step_leaves_nothing_behind() {
        let steps = [
            "vaGetDisplayDRM",
            "vaInitialize",
            "vaQueryConfigEntrypoints",
            "vaGetConfigAttributes",
            "vaCreateConfig",
            "vaCreateSurfaces",
            "vaCreateContext",
            "vaCreateBuffer",
            "vaDeriveImage",
            "vaMapBuffer",
            "vaUnmapBuffer",
            "vaBeginPicture",
            "vaRenderPicture",
            "vaEndPicture",
            "vaSyncSurface",
        ];
        for step in steps {
            let _turn = start(|m| m.fail = Some(step));
            let result = open_mock(&request()).and_then(|mut encoder| {
                with(|m| m.output = annexb(true));
                encoder.encode(&picture(), 0).map(|_| ())
            });
            assert!(result.is_err(), "{step} should surface as an error");
            assert_eq!(live(), [0; 7], "{step} leaked");
        }
    }

    #[test]
    fn damaged_coded_data_is_an_error_not_a_crash() {
        let _turn = start(|_| {});
        let mut encoder = open_mock(&request()).unwrap();
        for output in [vec![], vec![1, 2, 3], vec![0, 0, 0, 1, 0x67, 1, 2, 3, 4]] {
            with(|m| m.output = output);
            assert!(encoder.encode(&picture(), 0).is_err());
        }
        assert!(encoder.encode(&[1, 2, 3], 0).is_err());
    }

    #[test]
    fn nonsense_requests_are_refused_before_any_libva_call() {
        let _turn = start(|_| {});
        for (width, height, fps, kbps) in [
            (0, 0, 30, 1),
            (3, 4, 30, 1),
            (100_000, 8, 30, 1),
            (64, 64, 0, 1),
            (64, 64, 30, 0),
        ] {
            let bad = HardwareRequest {
                width,
                height,
                fps,
                video_kbps: kbps,
                ..request()
            };
            assert!(open_mock(&bad).is_err());
        }
        assert!(with(|m| m.calls.is_empty()));
    }
}
