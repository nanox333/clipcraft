//! The slice of libva's C interface used for H.264 encoding: types, constants, the structures of
//! `va.h` and `va_enc_h264.h`, and a table of the functions, loaded from the system's `libva.so.2`
//! and `libva-drm.so.2` at run time (so a computer without libva simply has no VA-API encoder).
//!
//! Every structure's size and the offset of every field the encoder writes are checked against
//! the numbers a C compiler gives for the real headers (`abi_tests`).

use std::ffi::{c_char, c_int, c_uint, c_void};

pub type VADisplay = *mut c_void;
pub type VAStatus = c_int;
pub type VAGenericID = c_uint;
pub type VAConfigID = VAGenericID;
pub type VAContextID = VAGenericID;
pub type VASurfaceID = VAGenericID;
pub type VABufferID = VAGenericID;
pub type VAImageID = VAGenericID;

pub const VA_STATUS_SUCCESS: VAStatus = 0;
pub const VA_INVALID_ID: c_uint = 0xffff_ffff;
pub const VA_INVALID_SURFACE: VASurfaceID = VA_INVALID_ID;
pub const VA_PADDING_LOW: usize = 4;

pub const VA_PROFILE_H264_CONSTRAINED_BASELINE: c_int = 13;
pub const VA_ENTRYPOINT_ENC_SLICE: c_int = 6;
pub const VA_ENTRYPOINT_ENC_SLICE_LP: c_int = 8;

pub const VA_CONFIG_ATTRIB_RT_FORMAT: c_int = 0;
pub const VA_CONFIG_ATTRIB_RATE_CONTROL: c_int = 5;
pub const VA_CONFIG_ATTRIB_ENC_PACKED_HEADERS: c_int = 10;
pub const VA_ATTRIB_NOT_SUPPORTED: c_uint = 0x8000_0000;

pub const VA_RT_FORMAT_YUV420: c_uint = 0x1;
pub const VA_RC_CBR: c_uint = 0x2;
pub const VA_RC_VBR: c_uint = 0x4;
pub const VA_ENC_PACKED_HEADER_SEQUENCE: c_uint = 0x1;
pub const VA_ENC_PACKED_HEADER_PICTURE: c_uint = 0x2;

pub const VA_PROGRESSIVE: c_int = 0x1;

pub const VA_ENC_CODED_BUFFER_TYPE: c_int = 21;
pub const VA_ENC_SEQUENCE_PARAMETER_BUFFER_TYPE: c_int = 22;
pub const VA_ENC_PICTURE_PARAMETER_BUFFER_TYPE: c_int = 23;
pub const VA_ENC_SLICE_PARAMETER_BUFFER_TYPE: c_int = 24;
pub const VA_ENC_PACKED_HEADER_PARAMETER_BUFFER_TYPE: c_int = 25;
pub const VA_ENC_PACKED_HEADER_DATA_BUFFER_TYPE: c_int = 26;
pub const VA_ENC_MISC_PARAMETER_BUFFER_TYPE: c_int = 27;

pub const VA_ENC_PACKED_HEADER_TYPE_SEQUENCE: u32 = 1;
pub const VA_ENC_PACKED_HEADER_TYPE_PICTURE: u32 = 2;

pub const VA_ENC_MISC_PARAMETER_TYPE_FRAME_RATE: u32 = 0;
pub const VA_ENC_MISC_PARAMETER_TYPE_RATE_CONTROL: u32 = 1;
pub const VA_ENC_MISC_PARAMETER_TYPE_HRD: u32 = 5;

pub const VA_PICTURE_H264_INVALID: u32 = 0x1;
pub const VA_PICTURE_H264_SHORT_TERM_REFERENCE: u32 = 0x8;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VAConfigAttrib {
    pub kind: c_int,
    pub value: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VAImageFormat {
    pub fourcc: u32,
    pub byte_order: u32,
    pub bits_per_pixel: u32,
    pub depth: u32,
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub alpha_mask: u32,
    pub va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VAImage {
    pub image_id: VAImageID,
    pub format: VAImageFormat,
    pub buf: VABufferID,
    pub width: u16,
    pub height: u16,
    pub data_size: u32,
    pub num_planes: u32,
    pub pitches: [u32; 3],
    pub offsets: [u32; 3],
    pub num_palette_entries: i32,
    pub entry_bytes: i32,
    pub component_order: [i8; 4],
    pub va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VACodedBufferSegment {
    pub size: u32,
    pub bit_offset: u32,
    pub status: u32,
    pub reserved: u32,
    pub buf: *mut c_void,
    pub next: *mut c_void,
    pub va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VAPictureH264 {
    pub picture_id: VASurfaceID,
    pub frame_idx: u32,
    pub flags: u32,
    pub top_field_order_cnt: i32,
    pub bottom_field_order_cnt: i32,
    pub va_reserved: [u32; VA_PADDING_LOW],
}

impl VAPictureH264 {
    /// An unused entry of a reference list.
    pub const INVALID: Self = Self {
        picture_id: VA_INVALID_SURFACE,
        frame_idx: 0,
        flags: VA_PICTURE_H264_INVALID,
        top_field_order_cnt: 0,
        bottom_field_order_cnt: 0,
        va_reserved: [0; VA_PADDING_LOW],
    };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VAEncSequenceParameterBufferH264 {
    pub seq_parameter_set_id: u8,
    pub level_idc: u8,
    pub intra_period: u32,
    pub intra_idr_period: u32,
    pub ip_period: u32,
    pub bits_per_second: u32,
    pub max_num_ref_frames: u32,
    pub picture_width_in_mbs: u16,
    pub picture_height_in_mbs: u16,
    pub seq_fields: u32,
    pub bit_depth_luma_minus8: u8,
    pub bit_depth_chroma_minus8: u8,
    pub num_ref_frames_in_pic_order_cnt_cycle: u8,
    pub offset_for_non_ref_pic: i32,
    pub offset_for_top_to_bottom_field: i32,
    pub offset_for_ref_frame: [i32; 256],
    pub frame_cropping_flag: u8,
    pub frame_crop_left_offset: u32,
    pub frame_crop_right_offset: u32,
    pub frame_crop_top_offset: u32,
    pub frame_crop_bottom_offset: u32,
    pub vui_parameters_present_flag: u8,
    pub vui_fields: u32,
    pub aspect_ratio_idc: u8,
    pub sar_width: u32,
    pub sar_height: u32,
    pub num_units_in_tick: u32,
    pub time_scale: u32,
    pub va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VAEncPictureParameterBufferH264 {
    pub curr_pic: VAPictureH264,
    pub reference_frames: [VAPictureH264; 16],
    pub coded_buf: VABufferID,
    pub pic_parameter_set_id: u8,
    pub seq_parameter_set_id: u8,
    pub last_picture: u8,
    pub frame_num: u16,
    pub pic_init_qp: u8,
    pub num_ref_idx_l0_active_minus1: u8,
    pub num_ref_idx_l1_active_minus1: u8,
    pub chroma_qp_index_offset: i8,
    pub second_chroma_qp_index_offset: i8,
    pub pic_fields: u32,
    pub va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VAEncSliceParameterBufferH264 {
    pub macroblock_address: u32,
    pub num_macroblocks: u32,
    pub macroblock_info: VABufferID,
    pub slice_type: u8,
    pub pic_parameter_set_id: u8,
    pub idr_pic_id: u16,
    pub pic_order_cnt_lsb: u16,
    pub delta_pic_order_cnt_bottom: i32,
    pub delta_pic_order_cnt: [i32; 2],
    pub direct_spatial_mv_pred_flag: u8,
    pub num_ref_idx_active_override_flag: u8,
    pub num_ref_idx_l0_active_minus1: u8,
    pub num_ref_idx_l1_active_minus1: u8,
    pub ref_pic_list0: [VAPictureH264; 32],
    pub ref_pic_list1: [VAPictureH264; 32],
    pub luma_log2_weight_denom: u8,
    pub chroma_log2_weight_denom: u8,
    pub luma_weight_l0_flag: u8,
    pub luma_weight_l0: [i16; 32],
    pub luma_offset_l0: [i16; 32],
    pub chroma_weight_l0_flag: u8,
    pub chroma_weight_l0: [[i16; 2]; 32],
    pub chroma_offset_l0: [[i16; 2]; 32],
    pub luma_weight_l1_flag: u8,
    pub luma_weight_l1: [i16; 32],
    pub luma_offset_l1: [i16; 32],
    pub chroma_weight_l1_flag: u8,
    pub chroma_weight_l1: [[i16; 2]; 32],
    pub chroma_offset_l1: [[i16; 2]; 32],
    pub cabac_init_idc: u8,
    pub slice_qp_delta: i8,
    pub disable_deblocking_filter_idc: u8,
    pub slice_alpha_c0_offset_div2: i8,
    pub slice_beta_offset_div2: i8,
    pub va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VAEncPackedHeaderParameterBuffer {
    pub kind: u32,
    pub bit_length: u32,
    pub has_emulation_bytes: u8,
    pub va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VAEncMiscParameterRateControl {
    pub bits_per_second: u32,
    pub target_percentage: u32,
    pub window_size: u32,
    pub initial_qp: u32,
    pub min_qp: u32,
    pub basic_unit_size: u32,
    pub rc_flags: u32,
    pub icq_quality_factor: u32,
    pub max_qp: u32,
    pub quality_factor: u32,
    pub target_frame_size: u32,
    pub va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VAEncMiscParameterFrameRate {
    pub framerate: u32,
    pub framerate_flags: u32,
    pub va_reserved: [u32; VA_PADDING_LOW],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VAEncMiscParameterHrd {
    pub initial_buffer_fullness: u32,
    pub buffer_size: u32,
    pub va_reserved: [u32; VA_PADDING_LOW],
}

/// The libva functions the encoder calls. Real ones come from the shared libraries; tests build a
/// table of stand-ins.
pub struct VaApi {
    pub get_display_drm: unsafe extern "C" fn(c_int) -> VADisplay,
    pub initialize: unsafe extern "C" fn(VADisplay, *mut c_int, *mut c_int) -> VAStatus,
    pub terminate: unsafe extern "C" fn(VADisplay) -> VAStatus,
    pub query_vendor_string: unsafe extern "C" fn(VADisplay) -> *const c_char,
    pub max_num_entrypoints: unsafe extern "C" fn(VADisplay) -> c_int,
    pub query_config_entrypoints:
        unsafe extern "C" fn(VADisplay, c_int, *mut c_int, *mut c_int) -> VAStatus,
    pub get_config_attributes:
        unsafe extern "C" fn(VADisplay, c_int, c_int, *mut VAConfigAttrib, c_int) -> VAStatus,
    pub create_config: unsafe extern "C" fn(
        VADisplay,
        c_int,
        c_int,
        *mut VAConfigAttrib,
        c_int,
        *mut VAConfigID,
    ) -> VAStatus,
    pub destroy_config: unsafe extern "C" fn(VADisplay, VAConfigID) -> VAStatus,
    pub create_surfaces: unsafe extern "C" fn(
        VADisplay,
        c_uint,
        c_uint,
        c_uint,
        *mut VASurfaceID,
        c_uint,
        *mut c_void,
        c_uint,
    ) -> VAStatus,
    pub destroy_surfaces: unsafe extern "C" fn(VADisplay, *mut VASurfaceID, c_int) -> VAStatus,
    pub create_context: unsafe extern "C" fn(
        VADisplay,
        VAConfigID,
        c_int,
        c_int,
        c_int,
        *mut VASurfaceID,
        c_int,
        *mut VAContextID,
    ) -> VAStatus,
    pub destroy_context: unsafe extern "C" fn(VADisplay, VAContextID) -> VAStatus,
    pub create_buffer: unsafe extern "C" fn(
        VADisplay,
        VAContextID,
        c_int,
        c_uint,
        c_uint,
        *mut c_void,
        *mut VABufferID,
    ) -> VAStatus,
    pub destroy_buffer: unsafe extern "C" fn(VADisplay, VABufferID) -> VAStatus,
    pub map_buffer: unsafe extern "C" fn(VADisplay, VABufferID, *mut *mut c_void) -> VAStatus,
    pub unmap_buffer: unsafe extern "C" fn(VADisplay, VABufferID) -> VAStatus,
    pub begin_picture: unsafe extern "C" fn(VADisplay, VAContextID, VASurfaceID) -> VAStatus,
    pub render_picture:
        unsafe extern "C" fn(VADisplay, VAContextID, *mut VABufferID, c_int) -> VAStatus,
    pub end_picture: unsafe extern "C" fn(VADisplay, VAContextID) -> VAStatus,
    pub sync_surface: unsafe extern "C" fn(VADisplay, VASurfaceID) -> VAStatus,
    pub derive_image: unsafe extern "C" fn(VADisplay, VASurfaceID, *mut VAImage) -> VAStatus,
    pub destroy_image: unsafe extern "C" fn(VADisplay, VAImageID) -> VAStatus,
    pub error_str: unsafe extern "C" fn(VAStatus) -> *const c_char,
    /// Keeps the shared libraries loaded for as long as the table is used.
    pub(super) _libraries: Option<(libloading::Library, libloading::Library)>,
}

impl VaApi {
    /// Loads libva from the system, or says why it cannot.
    pub fn load() -> Result<Self, String> {
        // SAFETY: loading the system's libva runs its initializers, which is what any program
        // using it does; the symbols are then looked up by their documented names and cast to
        // their documented C signatures.
        unsafe {
            let va = libloading::Library::new("libva.so.2")
                .map_err(|_| "libva is not installed (libva.so.2)".to_owned())?;
            let drm = libloading::Library::new("libva-drm.so.2")
                .map_err(|_| "libva-drm is not installed (libva-drm.so.2)".to_owned())?;
            macro_rules! symbol {
                ($library:expr, $name:literal) => {
                    *$library
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|_| format!("libva has no {}", $name))?
                };
            }
            Ok(Self {
                get_display_drm: symbol!(drm, "vaGetDisplayDRM"),
                initialize: symbol!(va, "vaInitialize"),
                terminate: symbol!(va, "vaTerminate"),
                query_vendor_string: symbol!(va, "vaQueryVendorString"),
                max_num_entrypoints: symbol!(va, "vaMaxNumEntrypoints"),
                query_config_entrypoints: symbol!(va, "vaQueryConfigEntrypoints"),
                get_config_attributes: symbol!(va, "vaGetConfigAttributes"),
                create_config: symbol!(va, "vaCreateConfig"),
                destroy_config: symbol!(va, "vaDestroyConfig"),
                create_surfaces: symbol!(va, "vaCreateSurfaces"),
                destroy_surfaces: symbol!(va, "vaDestroySurfaces"),
                create_context: symbol!(va, "vaCreateContext"),
                destroy_context: symbol!(va, "vaDestroyContext"),
                create_buffer: symbol!(va, "vaCreateBuffer"),
                destroy_buffer: symbol!(va, "vaDestroyBuffer"),
                map_buffer: symbol!(va, "vaMapBuffer"),
                unmap_buffer: symbol!(va, "vaUnmapBuffer"),
                begin_picture: symbol!(va, "vaBeginPicture"),
                render_picture: symbol!(va, "vaRenderPicture"),
                end_picture: symbol!(va, "vaEndPicture"),
                sync_surface: symbol!(va, "vaSyncSurface"),
                derive_image: symbol!(va, "vaDeriveImage"),
                destroy_image: symbol!(va, "vaDestroyImage"),
                error_str: symbol!(va, "vaErrorStr"),
                _libraries: Some((va, drm)),
            })
        }
    }
}

#[cfg(test)]
mod abi_tests {
    //! The sizes and offsets below come from compiling `va.h` and `va_enc_h264.h` from the libva
    //! project with a C compiler and printing `sizeof` and `offsetof`.
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn structures_match_the_c_headers() {
        assert_eq!(size_of::<VAConfigAttrib>(), 8);
        assert_eq!(size_of::<VAImageFormat>(), 48);
        assert_eq!(size_of::<VAImage>(), 120);
        assert_eq!(offset_of!(VAImage, buf), 52);
        assert_eq!(offset_of!(VAImage, width), 56);
        assert_eq!(offset_of!(VAImage, data_size), 60);
        assert_eq!(offset_of!(VAImage, num_planes), 64);
        assert_eq!(offset_of!(VAImage, pitches), 68);
        assert_eq!(offset_of!(VAImage, offsets), 80);
        assert_eq!(size_of::<VAPictureH264>(), 36);
        if size_of::<usize>() == 8 {
            assert_eq!(size_of::<VACodedBufferSegment>(), 48);
            assert_eq!(offset_of!(VACodedBufferSegment, buf), 16);
            assert_eq!(offset_of!(VACodedBufferSegment, next), 24);
        }

        assert_eq!(size_of::<VAEncSequenceParameterBufferH264>(), 1132);
        assert_eq!(
            offset_of!(VAEncSequenceParameterBufferH264, intra_period),
            4
        );
        assert_eq!(
            offset_of!(VAEncSequenceParameterBufferH264, bits_per_second),
            16
        );
        assert_eq!(
            offset_of!(VAEncSequenceParameterBufferH264, picture_width_in_mbs),
            24
        );
        assert_eq!(offset_of!(VAEncSequenceParameterBufferH264, seq_fields), 28);
        assert_eq!(
            offset_of!(VAEncSequenceParameterBufferH264, bit_depth_luma_minus8),
            32
        );
        assert_eq!(
            offset_of!(VAEncSequenceParameterBufferH264, offset_for_non_ref_pic),
            36
        );
        assert_eq!(
            offset_of!(VAEncSequenceParameterBufferH264, offset_for_ref_frame),
            44
        );
        assert_eq!(
            offset_of!(VAEncSequenceParameterBufferH264, frame_cropping_flag),
            1068
        );
        assert_eq!(
            offset_of!(VAEncSequenceParameterBufferH264, frame_crop_left_offset),
            1072
        );
        assert_eq!(
            offset_of!(
                VAEncSequenceParameterBufferH264,
                vui_parameters_present_flag
            ),
            1088
        );
        assert_eq!(
            offset_of!(VAEncSequenceParameterBufferH264, vui_fields),
            1092
        );
        assert_eq!(
            offset_of!(VAEncSequenceParameterBufferH264, aspect_ratio_idc),
            1096
        );
        assert_eq!(
            offset_of!(VAEncSequenceParameterBufferH264, time_scale),
            1112
        );

        assert_eq!(size_of::<VAEncPictureParameterBufferH264>(), 648);
        assert_eq!(
            offset_of!(VAEncPictureParameterBufferH264, reference_frames),
            36
        );
        assert_eq!(offset_of!(VAEncPictureParameterBufferH264, coded_buf), 612);
        assert_eq!(
            offset_of!(VAEncPictureParameterBufferH264, pic_parameter_set_id),
            616
        );
        assert_eq!(offset_of!(VAEncPictureParameterBufferH264, frame_num), 620);
        assert_eq!(
            offset_of!(VAEncPictureParameterBufferH264, pic_init_qp),
            622
        );
        assert_eq!(
            offset_of!(VAEncPictureParameterBufferH264, chroma_qp_index_offset),
            625
        );
        assert_eq!(offset_of!(VAEncPictureParameterBufferH264, pic_fields), 628);

        assert_eq!(size_of::<VAEncSliceParameterBufferH264>(), 3140);
        assert_eq!(
            offset_of!(VAEncSliceParameterBufferH264, macroblock_info),
            8
        );
        assert_eq!(offset_of!(VAEncSliceParameterBufferH264, slice_type), 12);
        assert_eq!(offset_of!(VAEncSliceParameterBufferH264, idr_pic_id), 14);
        assert_eq!(
            offset_of!(VAEncSliceParameterBufferH264, pic_order_cnt_lsb),
            16
        );
        assert_eq!(
            offset_of!(VAEncSliceParameterBufferH264, delta_pic_order_cnt_bottom),
            20
        );
        assert_eq!(
            offset_of!(VAEncSliceParameterBufferH264, direct_spatial_mv_pred_flag),
            32
        );
        assert_eq!(offset_of!(VAEncSliceParameterBufferH264, ref_pic_list0), 36);
        assert_eq!(
            offset_of!(VAEncSliceParameterBufferH264, ref_pic_list1),
            1188
        );
        assert_eq!(
            offset_of!(VAEncSliceParameterBufferH264, luma_log2_weight_denom),
            2340
        );
        assert_eq!(
            offset_of!(VAEncSliceParameterBufferH264, luma_weight_l0),
            2344
        );
        assert_eq!(
            offset_of!(VAEncSliceParameterBufferH264, cabac_init_idc),
            3118
        );

        assert_eq!(size_of::<VAEncPackedHeaderParameterBuffer>(), 28);
        assert_eq!(offset_of!(VAEncPackedHeaderParameterBuffer, bit_length), 4);
        assert_eq!(
            offset_of!(VAEncPackedHeaderParameterBuffer, has_emulation_bytes),
            8
        );
        assert_eq!(size_of::<VAEncMiscParameterRateControl>(), 60);
        assert_eq!(offset_of!(VAEncMiscParameterRateControl, rc_flags), 24);
        assert_eq!(offset_of!(VAEncMiscParameterRateControl, max_qp), 32);
        assert_eq!(size_of::<VAEncMiscParameterFrameRate>(), 24);
        assert_eq!(size_of::<VAEncMiscParameterHrd>(), 24);
    }
}
