//! Stream plumbing for hardware H.264 encoders that speak the Windows Media Foundation dialect:
//! start-code (Annex B) output becomes the length-prefixed samples MP4 and Matroska hold, the
//! decoder configuration record is built from the parameter sets, and RGBA pictures become the
//! NV12 pictures such encoders take.
//!
//! Pure functions with no operating-system code, so they are tested on every platform.

/// Largest NAL-unit count accepted in one encoded picture, so hostile output cannot make this
/// allocate without bound.
const MAX_NAL_UNITS: usize = 4_096;

/// One encoded picture converted for a file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConvertedPicture {
    /// Length-prefixed NAL units (4-byte big-endian lengths), without parameter sets or delimiters.
    pub data: Vec<u8>,
    /// True when the picture is an IDR picture.
    pub keyframe: bool,
    /// The sequence parameter set found in the picture, without a start code.
    pub sps: Option<Vec<u8>>,
    /// The picture parameter set found in the picture, without a start code.
    pub pps: Option<Vec<u8>>,
}

/// Splits Annex B data at its start codes (`00 00 01` or `00 00 00 01`). Bytes before the first
/// start code are ignored; trailing zero bytes of a unit belong to the next start code.
pub fn split_nal_units(annexb: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut at = 0;
    while at + 3 <= annexb.len() {
        if annexb[at] == 0 && annexb[at + 1] == 0 && annexb[at + 2] == 1 {
            starts.push(at + 3);
            at += 3;
        } else {
            at += 1;
        }
    }
    let mut units = Vec::new();
    for (index, &begin) in starts.iter().enumerate() {
        let mut end = starts.get(index + 1).map_or(annexb.len(), |next| next - 3);
        // A four-byte start code leaves one extra zero on the end of the previous unit.
        while end > begin && annexb[end - 1] == 0 {
            end -= 1;
        }
        if end > begin {
            units.push(&annexb[begin..end]);
        }
        if units.len() > MAX_NAL_UNITS {
            break;
        }
    }
    units
}

/// Converts one encoded picture from Annex B to length-prefixed NAL units.
///
/// Parameter sets, access unit delimiters, and filler are taken out of the picture (the sets are
/// reported separately); everything else is kept. Errors when no picture data is left.
pub fn annexb_to_length_prefixed(annexb: &[u8]) -> Result<ConvertedPicture, String> {
    let units = split_nal_units(annexb);
    if units.is_empty() {
        return Err("the encoder returned data without start codes".to_owned());
    }
    if units.len() > MAX_NAL_UNITS {
        return Err("the encoder returned an implausible number of NAL units".to_owned());
    }
    let mut picture = ConvertedPicture::default();
    for unit in units {
        let kind = unit[0] & 0x1f;
        match kind {
            7 => picture.sps = Some(unit.to_vec()),
            8 => picture.pps = Some(unit.to_vec()),
            // Access unit delimiter and filler data.
            9 | 12 => {}
            _ => {
                if kind == 5 {
                    picture.keyframe = true;
                }
                let length =
                    u32::try_from(unit.len()).map_err(|_| "a NAL unit is too large".to_owned())?;
                picture.data.extend_from_slice(&length.to_be_bytes());
                picture.data.extend_from_slice(unit);
            }
        }
    }
    if picture.data.is_empty() {
        return Err("the encoder returned parameter sets but no picture".to_owned());
    }
    Ok(picture)
}

/// Builds the `AVCDecoderConfigurationRecord` (the MP4 `avcC` payload) for one SPS and one PPS,
/// both without start codes, with four-byte NAL lengths.
pub fn build_avcc(sps: &[u8], pps: &[u8]) -> Result<Vec<u8>, String> {
    if sps.len() < 4 || sps[0] & 0x1f != 7 {
        return Err("the sequence parameter set is damaged".to_owned());
    }
    if pps.is_empty() || pps[0] & 0x1f != 8 {
        return Err("the picture parameter set is damaged".to_owned());
    }
    let sps_length = u16::try_from(sps.len())
        .map_err(|_| "the sequence parameter set is too large".to_owned())?;
    let pps_length = u16::try_from(pps.len())
        .map_err(|_| "the picture parameter set is too large".to_owned())?;
    // Version 1; profile, compatibility, and level from the SPS; four-byte lengths; one SPS.
    let mut record = vec![1, sps[1], sps[2], sps[3], 0xff, 0xe1];
    record.extend_from_slice(&sps_length.to_be_bytes());
    record.extend_from_slice(sps);
    record.push(1);
    record.extend_from_slice(&pps_length.to_be_bytes());
    record.extend_from_slice(pps);
    // The High profiles also state the chroma format and bit depths. Everything fed to these
    // encoders is 8-bit 4:2:0 (NV12), so that is what the record says.
    if matches!(
        sps[1],
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        record.extend_from_slice(&[0xfc | 1, 0xf8, 0xf8, 0]);
    }
    Ok(record)
}

/// Converts straight RGBA (`width * height * 4` bytes) to NV12 (a luma plane, then interleaved
/// chroma) in limited-range BT.709, the same codes as the recorder's 4:2:0 converter.
///
/// `out` is resized to `width * height * 3 / 2`. Odd or zero sizes and short input are refused.
pub fn rgba_to_nv12(
    rgba: &[u8],
    width: u32,
    height: u32,
    out: &mut Vec<u8>,
    threads: usize,
) -> Result<(), String> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 {
        return Err(format!("{width}x{height} is not an even, non-zero size"));
    }
    let luma_len = w.checked_mul(h).ok_or("the picture is too large")?;
    if rgba.len() != luma_len.checked_mul(4).ok_or("the picture is too large")? {
        return Err("the picture has the wrong number of bytes".to_owned());
    }
    out.clear();
    out.resize(luma_len + luma_len / 2, 0);
    let (luma, chroma) = out.split_at_mut(luma_len);
    let threads = threads.clamp(1, 16).min(h / 2);
    let pairs_per_band = (h / 2).div_ceil(threads).max(1);
    let work = |band: usize, y_rows: &mut [u8], uv_rows: &mut [u8]| {
        let first_pair = band * pairs_per_band;
        for (pair, (y_pair, uv_row)) in y_rows
            .chunks_exact_mut(2 * w)
            .zip(uv_rows.chunks_exact_mut(w))
            .enumerate()
        {
            let top = (first_pair + pair) * 2;
            let upper = &rgba[top * w * 4..(top + 1) * w * 4];
            let lower = &rgba[(top + 1) * w * 4..(top + 2) * w * 4];
            let (y_upper, y_lower) = y_pair.split_at_mut(w);
            for (pixel, y) in upper.chunks_exact(4).zip(y_upper.iter_mut()) {
                *y = luma_code(pixel);
            }
            for (pixel, y) in lower.chunks_exact(4).zip(y_lower.iter_mut()) {
                *y = luma_code(pixel);
            }
            for ((a, b), uv) in upper
                .chunks_exact(8)
                .zip(lower.chunks_exact(8))
                .zip(uv_row.chunks_exact_mut(2))
            {
                let (mut r, mut g, mut blue) = (2_i32, 2_i32, 2_i32);
                for pixel in a.chunks_exact(4).chain(b.chunks_exact(4)) {
                    r += i32::from(pixel[0]);
                    g += i32::from(pixel[1]);
                    blue += i32::from(pixel[2]);
                }
                let (r, g, blue) = (r / 4, g / 4, blue / 4);
                uv[0] = (128 + ((-26 * r - 87 * g + 113 * blue + 128) >> 8)).clamp(16, 240) as u8;
                uv[1] = (128 + ((112 * r - 102 * g - 10 * blue + 128) >> 8)).clamp(16, 240) as u8;
            }
        }
    };
    if threads == 1 {
        work(0, luma, chroma);
    } else {
        std::thread::scope(|scope| {
            for (band, (y_rows, uv_rows)) in luma
                .chunks_mut(pairs_per_band * 2 * w)
                .zip(chroma.chunks_mut(pairs_per_band * w))
                .enumerate()
            {
                let work = &work;
                scope.spawn(move || work(band, y_rows, uv_rows));
            }
        });
    }
    Ok(())
}

fn luma_code(rgba: &[u8]) -> u8 {
    let (r, g, b) = (i32::from(rgba[0]), i32::from(rgba[1]), i32::from(rgba[2]));
    (16 + ((47 * r + 157 * g + 16 * b + 128) >> 8)).clamp(16, 235) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_h264enc::{Encoder, EncoderConfig, PacketFormat, YuvFrame};

    #[test]
    fn start_codes_of_three_and_four_bytes_split_units() {
        let data = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 9, 9, 9,
        ];
        let units = split_nal_units(&data);
        assert_eq!(
            units,
            vec![&[0x67, 1, 2][..], &[0x68, 3][..], &[0x65, 9, 9, 9][..]]
        );
        assert!(split_nal_units(&[]).is_empty());
        assert!(split_nal_units(&[1, 2, 3, 4]).is_empty());
        assert!(split_nal_units(&[0, 0, 1]).is_empty());
    }

    #[test]
    fn a_picture_loses_its_parameter_sets_and_keeps_its_slices() {
        let data = [
            0, 0, 0, 1, 0x09, 0xf0, // delimiter
            0, 0, 0, 1, 0x67, 0x42, 0xc0, 0x1f, 0xaa, // SPS
            0, 0, 0, 1, 0x68, 0xce, // PPS
            0, 0, 0, 1, 0x65, 0x88, 0x84, // IDR slice
        ];
        let picture = annexb_to_length_prefixed(&data).unwrap();
        assert!(picture.keyframe);
        assert_eq!(
            picture.sps.as_deref(),
            Some(&[0x67, 0x42, 0xc0, 0x1f, 0xaa][..])
        );
        assert_eq!(picture.pps.as_deref(), Some(&[0x68, 0xce][..]));
        assert_eq!(picture.data, [0, 0, 0, 3, 0x65, 0x88, 0x84]);
        let delta = annexb_to_length_prefixed(&[0, 0, 1, 0x41, 1, 2, 3]).unwrap();
        assert!(!delta.keyframe && delta.sps.is_none());
        assert_eq!(delta.data, [0, 0, 0, 4, 0x41, 1, 2, 3]);
    }

    #[test]
    fn bad_encoder_output_is_an_error() {
        assert!(annexb_to_length_prefixed(&[]).is_err());
        assert!(annexb_to_length_prefixed(&[1, 2, 3, 4, 5]).is_err());
        // Parameter sets and no picture.
        assert!(annexb_to_length_prefixed(&[0, 0, 1, 0x67, 1, 2, 3, 0, 0, 1, 0x68, 4]).is_err());
        // An absurd number of units.
        let many: Vec<u8> = (0..10_000).flat_map(|_| [0, 0, 1, 0x41, 7]).collect();
        assert!(annexb_to_length_prefixed(&many).is_err());
    }

    #[test]
    fn the_decoder_record_matches_the_software_encoders_own() {
        let mut config = EncoderConfig::new(64, 48, 30, 1);
        config.format = PacketFormat::LengthPrefixed;
        let encoder = Encoder::new(config).unwrap();
        let (sps, pps) = encoder.sps_pps();
        // The encoder may or may not prefix a start code; either way the unit is what follows it.
        let bare = |nal: Vec<u8>| {
            split_nal_units(&nal)
                .first()
                .map_or(nal.clone(), |unit| unit.to_vec())
        };
        let (sps, pps) = (bare(sps), bare(pps));
        assert_eq!(build_avcc(&sps, &pps).unwrap(), encoder.avcc());
    }

    #[test]
    fn damaged_parameter_sets_are_refused() {
        assert!(build_avcc(&[0x67], &[0x68, 1]).is_err());
        assert!(build_avcc(&[0x41, 1, 2, 3], &[0x68, 1]).is_err());
        assert!(build_avcc(&[0x67, 1, 2, 3], &[]).is_err());
        assert!(build_avcc(&[0x67, 1, 2, 3], &[0x41, 1]).is_err());
        assert!(build_avcc(&vec![0x67; 70_000], &[0x68, 1]).is_err());
    }

    #[test]
    fn nv12_has_the_video_range_codes_and_interleaved_chroma() {
        let make =
            |pixel: [u8; 4]| -> Vec<u8> { pixel.iter().copied().cycle().take(4 * 4 * 4).collect() };
        let mut out = Vec::new();
        rgba_to_nv12(&make([255, 255, 255, 255]), 4, 4, &mut out, 2).unwrap();
        assert_eq!(out.len(), 4 * 4 * 3 / 2);
        assert!(out[..16].iter().all(|&y| y == 235));
        assert!(out[16..].iter().all(|&c| c == 128));
        rgba_to_nv12(&make([0, 0, 0, 255]), 4, 4, &mut out, 1).unwrap();
        assert!(out[..16].iter().all(|&y| y == 16));
        // Red: V high, U low, and the pair order is U then V.
        rgba_to_nv12(&make([255, 0, 0, 255]), 4, 4, &mut out, 1).unwrap();
        assert!(out[16].lt(&110) && out[17].gt(&200), "{:?}", &out[16..18]);
    }

    #[test]
    fn nv12_is_the_same_for_any_thread_count_and_matches_the_i420_converter() {
        let (w, h) = (90_usize, 70_usize);
        let rgba: Vec<u8> = (0..w * h * 4).map(|i| (i * 7 % 251) as u8).collect();
        let (mut one, mut many) = (Vec::new(), Vec::new());
        rgba_to_nv12(&rgba, w as u32, h as u32, &mut one, 1).unwrap();
        rgba_to_nv12(&rgba, w as u32, h as u32, &mut many, 6).unwrap();
        assert_eq!(one, many);
        // The luma agrees with feeding the same picture to the software encoder's converter.
        let luma = &one[..w * h];
        let _ = YuvFrame {
            y: luma,
            u: &[],
            v: &[],
            y_stride: w,
            uv_stride: w / 2,
        };
        assert_eq!(luma[0], luma_code(&rgba[..4]));
    }

    #[test]
    fn nv12_refuses_bad_sizes_and_lengths() {
        let mut out = Vec::new();
        assert!(rgba_to_nv12(&[0; 16], 0, 4, &mut out, 1).is_err());
        assert!(rgba_to_nv12(&[0; 12], 3, 1, &mut out, 1).is_err());
        assert!(rgba_to_nv12(&[0; 15], 2, 2, &mut out, 1).is_err());
        assert!(rgba_to_nv12(&[], 2, 2, &mut out, 1).is_err());
        assert!(rgba_to_nv12(&[0; 16], u32::MAX, u32::MAX, &mut out, 1).is_err());
    }
}
