//! Writing H.264 parameter sets for encoders that leave that to the application (VA-API): a bit
//! writer, emulation prevention, the sequence and picture parameter sets for Constrained
//! Baseline, and the level table.
//!
//! Pure functions, tested on every platform against FilmCraft's own H.264 parser.

/// Writes bits, most significant first.
#[derive(Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    used: u32,
}

impl BitWriter {
    /// An empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes the low `count` bits of `value` (at most 32).
    pub fn bits(&mut self, value: u32, count: u32) {
        for shift in (0..count.min(32)).rev() {
            let bit = ((value >> shift) & 1) as u8;
            if self.used == 0 {
                self.bytes.push(0);
            }
            if let Some(last) = self.bytes.last_mut() {
                *last |= bit << (7 - self.used);
            }
            self.used = (self.used + 1) % 8;
        }
    }

    /// Writes one bit.
    pub fn flag(&mut self, on: bool) {
        self.bits(u32::from(on), 1);
    }

    /// Writes an unsigned Exp-Golomb code.
    pub fn ue(&mut self, value: u32) {
        let shifted = u64::from(value) + 1;
        let length = 64 - shifted.leading_zeros();
        self.bits(0, length - 1);
        for shift in (0..length).rev() {
            self.flag((shifted >> shift) & 1 == 1);
        }
    }

    /// Writes a signed Exp-Golomb code.
    pub fn se(&mut self, value: i32) {
        let mapped = if value > 0 {
            (value as u32) * 2 - 1
        } else {
            value.unsigned_abs() * 2
        };
        self.ue(mapped);
    }

    /// Writes the trailing one bit and pads with zeros to a whole byte.
    pub fn finish(mut self) -> Vec<u8> {
        self.flag(true);
        self.bytes
    }
}

/// Inserts emulation prevention bytes: after two zero bytes, a byte of 0..=3 gets a 0x03 first.
pub fn escape(rbsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rbsp.len() + rbsp.len() / 64 + 2);
    let mut zeros = 0;
    for &byte in rbsp {
        if zeros >= 2 && byte <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// What the writers need to know about the stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamShape {
    /// Visible picture width in pixels.
    pub width: u32,
    /// Visible picture height in pixels.
    pub height: u32,
    /// Frames per second.
    pub fps: u32,
}

/// log2 of the largest `frame_num` plus one, as written in the sequence parameter set.
pub const LOG2_MAX_FRAME_NUM: u32 = 8;
/// log2 of the largest picture order count lsb.
pub const LOG2_MAX_POC_LSB: u32 = 8;
/// The initial quantizer written in the picture parameter set.
pub const PIC_INIT_QP: u32 = 26;

impl StreamShape {
    /// Width in 16x16 macroblocks.
    pub fn width_in_mbs(self) -> u32 {
        self.width.div_ceil(16)
    }

    /// Height in 16x16 macroblocks.
    pub fn height_in_mbs(self) -> u32 {
        self.height.div_ceil(16)
    }

    /// Crop offsets (left, right, top, bottom) in the units the sequence parameter set uses
    /// (two pixels, for 4:2:0 frame coding), or `None` when the size is a whole number of
    /// macroblocks.
    pub fn crop(self) -> Option<(u32, u32, u32, u32)> {
        let right = (self.width_in_mbs() * 16 - self.width) / 2;
        let bottom = (self.height_in_mbs() * 16 - self.height) / 2;
        (right > 0 || bottom > 0).then_some((0, right, 0, bottom))
    }
}

/// The lowest level whose limits hold this picture size and frame rate, or the highest known.
pub fn level_for(shape: StreamShape) -> u8 {
    crate::h264_stream::level_for(shape.width, shape.height, shape.fps)
}

fn nal(header: u8, rbsp: Vec<u8>) -> Vec<u8> {
    let mut nal = vec![header];
    nal.extend(escape(&rbsp));
    nal
}

/// The sequence parameter set NAL unit (with its header byte, emulation-prevented, without a
/// start code) for Constrained Baseline, one reference frame, picture order count type 0.
pub fn sps(shape: StreamShape, level: u8) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.bits(66, 8); // profile_idc: Baseline
    w.bits(0x40, 8); // constraint_set1_flag: Constrained Baseline
    w.bits(u32::from(level), 8);
    w.ue(0); // seq_parameter_set_id
    w.ue(LOG2_MAX_FRAME_NUM - 4);
    w.ue(0); // pic_order_cnt_type
    w.ue(LOG2_MAX_POC_LSB - 4);
    w.ue(1); // max_num_ref_frames
    w.flag(false); // gaps_in_frame_num_value_allowed_flag
    w.ue(shape.width_in_mbs() - 1);
    w.ue(shape.height_in_mbs() - 1);
    w.flag(true); // frame_mbs_only_flag
    w.flag(true); // direct_8x8_inference_flag
    match shape.crop() {
        Some((left, right, top, bottom)) => {
            w.flag(true);
            w.ue(left);
            w.ue(right);
            w.ue(top);
            w.ue(bottom);
        }
        None => w.flag(false),
    }
    w.flag(false); // vui_parameters_present_flag
    nal(0x67, w.finish())
}

/// The picture parameter set NAL unit for CAVLC, one reference index, and the deblocking filter
/// controlled per slice.
pub fn pps() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.ue(0); // pic_parameter_set_id
    w.ue(0); // seq_parameter_set_id
    w.flag(false); // entropy_coding_mode_flag: CAVLC
    w.flag(false); // bottom_field_pic_order_in_frame_present_flag
    w.ue(0); // num_slice_groups_minus1
    w.ue(0); // num_ref_idx_l0_default_active_minus1
    w.ue(0); // num_ref_idx_l1_default_active_minus1
    w.flag(false); // weighted_pred_flag
    w.bits(0, 2); // weighted_bipred_idc
    w.se(PIC_INIT_QP as i32 - 26);
    w.se(0); // pic_init_qs_minus26
    w.se(0); // chroma_qp_index_offset
    w.flag(true); // deblocking_filter_control_present_flag
    w.flag(false); // constrained_intra_pred_flag
    w.flag(false); // redundant_pic_cnt_present_flag
    nal(0x68, w.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_h264::params::{Pps, Sps};

    /// The RBSP of a NAL unit: header dropped, emulation prevention removed.
    fn rbsp(nal: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut zeros = 0;
        for &byte in &nal[1..] {
            if zeros >= 2 && byte == 3 {
                zeros = 0;
                continue;
            }
            out.push(byte);
            zeros = if byte == 0 { zeros + 1 } else { 0 };
        }
        out
    }

    #[test]
    fn exp_golomb_codes_are_the_standards() {
        let bytes = |build: fn(&mut BitWriter)| {
            let mut w = BitWriter::new();
            build(&mut w);
            w.finish()
        };
        // ue(0)=1, ue(1)=010, ue(2)=011, ue(3)=00100 then the stop bit.
        assert_eq!(bytes(|w| w.ue(0)), [0b1100_0000]);
        assert_eq!(
            bytes(|w| {
                w.ue(1);
                w.ue(2);
            }),
            [0b0100_1110]
        );
        assert_eq!(bytes(|w| w.ue(3)), [0b0010_0100]);
        // se(1)=ue(1), se(-1)=ue(2), se(2)=ue(3), se(0)=ue(0).
        assert_eq!(
            bytes(|w| {
                w.se(1);
                w.se(-1);
            }),
            [0b0100_1110]
        );
        assert_eq!(bytes(|w| w.se(2)), [0b0010_0100]);
        assert_eq!(bytes(|w| w.se(0)), [0b1100_0000]);
        // A large value round-trips through the length rule.
        let big = bytes(|w| w.ue(u32::MAX));
        assert_eq!(big.len(), 9);
    }

    #[test]
    fn emulation_prevention_protects_start_code_look_alikes() {
        assert_eq!(escape(&[0, 0, 0]), [0, 0, 3, 0]);
        assert_eq!(escape(&[0, 0, 1]), [0, 0, 3, 1]);
        assert_eq!(escape(&[0, 0, 2, 0, 0, 3]), [0, 0, 3, 2, 0, 0, 3, 3]);
        assert_eq!(escape(&[0, 0, 4]), [0, 0, 4]);
        assert_eq!(escape(&[1, 0, 0, 0, 0, 0]), [1, 0, 0, 3, 0, 0, 3, 0]);
        assert_eq!(escape(&[]), Vec::<u8>::new());
    }

    #[test]
    fn the_sequence_parameter_set_parses_back_to_what_was_asked() {
        for (width, height, fps) in [
            (1920, 1080, 60),
            (1280, 720, 30),
            (640, 360, 24),
            (3840, 2160, 60),
            (66, 34, 30),
        ] {
            let shape = StreamShape { width, height, fps };
            let level = level_for(shape);
            let bytes = sps(shape, level);
            assert_eq!(bytes[0], 0x67);
            let parsed = Sps::parse(&rbsp(&bytes)).unwrap();
            assert_eq!(parsed.profile_idc, 66);
            assert_eq!(parsed.constraint_flags & 0x40, 0x40);
            assert_eq!(parsed.level_idc, level);
            assert_eq!(parsed.log2_max_frame_num, LOG2_MAX_FRAME_NUM);
            assert_eq!(parsed.pic_order_cnt_type, 0);
            assert_eq!(parsed.log2_max_poc_lsb, LOG2_MAX_POC_LSB);
            assert_eq!(parsed.max_num_ref_frames, 1);
            assert!(
                parsed.frame_mbs_only
                    && parsed.direct_8x8_inference
                    && !parsed.gaps_in_frame_num_allowed
            );
            assert_eq!(parsed.pic_width_in_mbs, shape.width_in_mbs());
            assert_eq!(parsed.pic_height_in_map_units, shape.height_in_mbs());
            assert_eq!(parsed.frame_crop, shape.crop());
            assert!(parsed.vui.is_none());
            // The visible size the decoder derives is the one asked for.
            let (_, right, _, bottom) = parsed.frame_crop.unwrap_or((0, 0, 0, 0));
            assert_eq!(parsed.pic_width_in_mbs * 16 - right * 2, width);
            assert_eq!(parsed.pic_height_in_map_units * 16 - bottom * 2, height);
        }
    }

    #[test]
    fn the_picture_parameter_set_parses_back_to_what_the_slices_assume() {
        let shape = StreamShape {
            width: 1280,
            height: 720,
            fps: 30,
        };
        let sps_nal = sps(shape, level_for(shape));
        let spss = vec![Some(Sps::parse(&rbsp(&sps_nal)).unwrap())];
        let bytes = pps();
        assert_eq!(bytes[0], 0x68);
        let parsed = Pps::parse(&rbsp(&bytes), &spss).unwrap();
        assert_eq!((parsed.id, parsed.sps_id), (0, 0));
        assert!(
            !parsed.entropy_coding_mode && !parsed.weighted_pred && !parsed.constrained_intra_pred
        );
        assert_eq!(
            (
                parsed.num_ref_idx_l0_default_active,
                parsed.num_ref_idx_l1_default_active
            ),
            (1, 1)
        );
        assert_eq!(parsed.pic_init_qp, PIC_INIT_QP as i32);
        assert!(parsed.deblocking_filter_control_present && !parsed.redundant_pic_cnt_present);
        assert_eq!(parsed.chroma_qp_index_offset, 0);
    }

    #[test]
    fn levels_grow_with_size_and_rate() {
        let level = |width, height, fps| level_for(StreamShape { width, height, fps });
        assert_eq!(level(640, 360, 30), 31);
        assert_eq!(level(1920, 1080, 30), 40);
        assert_eq!(level(1920, 1080, 60), 42);
        assert_eq!(level(3840, 2160, 30), 51);
        assert_eq!(level(3840, 2160, 60), 52);
        assert_eq!(level(7680, 4320, 240), 52);
        assert_eq!(level(0, 0, 0), 31);
    }

    #[test]
    fn crop_covers_only_the_padding() {
        let crop = |width, height| {
            StreamShape {
                width,
                height,
                fps: 30,
            }
            .crop()
        };
        assert_eq!(crop(1920, 1088), None);
        assert_eq!(crop(1920, 1080), Some((0, 0, 0, 4)));
        assert_eq!(crop(1918, 1080), Some((0, 1, 0, 4)));
        assert_eq!(crop(16, 16), None);
    }
}
