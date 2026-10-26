//! Stream plumbing for hardware H.265 (HEVC) encoders that speak the Windows Media Foundation
//! dialect: the Annex B output becomes the length-prefixed samples MP4 and Matroska hold, and the
//! decoder configuration record (`hvcC`) is built from the parameter sets. The H.264 counterpart
//! is `h264_stream`.
//!
//! Pure functions with no operating-system code, so they are tested on every platform.

// Only the Windows Media Foundation encoder calls these; the unit tests run everywhere, so other
// platforms see them as unused outside tests.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::h264_stream::split_nal_units;

/// Largest NAL-unit count accepted in one encoded picture.
const MAX_NAL_UNITS: usize = 4_096;

/// HEVC NAL unit type codes used here.
const VPS: u8 = 32;
const SPS: u8 = 33;
const PPS: u8 = 34;
const ACCESS_UNIT_DELIMITER: u8 = 35;
const FILLER: u8 = 38;
/// The first and last of the intra random access point types (BLA, IDR, CRA). A picture of one of
/// these types can start playback.
const FIRST_IRAP: u8 = 16;
const LAST_IRAP: u8 = 21;

/// The type of an HEVC NAL unit, from its two-byte header.
pub fn nal_type(unit: &[u8]) -> u8 {
    unit.first().map_or(0, |byte| (byte >> 1) & 0x3f)
}

/// One encoded picture converted for a file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConvertedHevc {
    /// Length-prefixed NAL units (4-byte big-endian lengths), without parameter sets or delimiters.
    pub data: Vec<u8>,
    /// True when the picture can start playback.
    pub keyframe: bool,
    /// The video parameter set found in the picture, without a start code.
    pub vps: Option<Vec<u8>>,
    /// The sequence parameter set found in the picture, without a start code.
    pub sps: Option<Vec<u8>>,
    /// The picture parameter set found in the picture, without a start code.
    pub pps: Option<Vec<u8>>,
}

/// Converts one encoded picture from Annex B to length-prefixed NAL units. Parameter sets, access
/// unit delimiters and filler are taken out (the sets are reported separately). Errors when no
/// picture data is left.
pub fn annexb_to_length_prefixed(annexb: &[u8]) -> Result<ConvertedHevc, String> {
    let units = split_nal_units(annexb);
    if units.is_empty() {
        return Err("the encoder returned data without start codes".to_owned());
    }
    if units.len() > MAX_NAL_UNITS {
        return Err("the encoder returned an implausible number of NAL units".to_owned());
    }
    let mut picture = ConvertedHevc::default();
    for unit in units {
        if unit.len() < 2 {
            continue;
        }
        let kind = nal_type(unit);
        match kind {
            VPS => picture.vps = Some(unit.to_vec()),
            SPS => picture.sps = Some(unit.to_vec()),
            PPS => picture.pps = Some(unit.to_vec()),
            ACCESS_UNIT_DELIMITER | FILLER => {}
            _ => {
                if (FIRST_IRAP..=LAST_IRAP).contains(&kind) {
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

/// Removes the emulation-prevention bytes (`00 00 03` becomes `00 00`) from the start of a NAL
/// unit, enough of it to read the profile, tier and level.
fn unescape_head(unit: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    let mut zeros = 0;
    for &byte in unit.iter().take(48) {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}

/// Builds the `HEVCDecoderConfigurationRecord` (the MP4 `hvcC` payload) for one VPS, SPS and PPS,
/// all without start codes, with four-byte NAL lengths. Everything fed to these encoders is 8-bit
/// 4:2:0 (NV12), so the record says that.
pub fn build_hvcc(vps: &[u8], sps: &[u8], pps: &[u8]) -> Result<Vec<u8>, String> {
    if vps.len() < 2 || nal_type(vps) != VPS {
        return Err("the video parameter set is damaged".to_owned());
    }
    if sps.len() < 3 || nal_type(sps) != SPS {
        return Err("the sequence parameter set is damaged".to_owned());
    }
    if pps.len() < 2 || nal_type(pps) != PPS {
        return Err("the picture parameter set is damaged".to_owned());
    }
    // Two header bytes, one byte of ids and flags, then the 12-byte profile, tier and level.
    let head = unescape_head(sps);
    if head.len() < 15 {
        return Err("the sequence parameter set is too short".to_owned());
    }
    let flags = head[2];
    let temporal_layers = ((flags >> 1) & 7) + 1;
    let nested = flags & 1;
    let profile = &head[3..15];
    let mut record = vec![1_u8];
    record.push(profile[0]);
    record.extend_from_slice(&profile[1..5]);
    record.extend_from_slice(&profile[5..11]);
    record.push(profile[11]);
    // Spatial segmentation unknown, no parallelism claim, 4:2:0, 8-bit luma and chroma, frame
    // rate unspecified.
    record.extend_from_slice(&[0xf0, 0x00, 0xfc, 0xfd, 0xf8, 0xf8, 0, 0]);
    // Not constant frame rate, the temporal layers, nesting, and four-byte lengths.
    record.push((temporal_layers.min(7) << 3) | (nested << 2) | 3);
    record.push(3);
    for (kind, unit) in [(VPS, vps), (SPS, sps), (PPS, pps)] {
        let length =
            u16::try_from(unit.len()).map_err(|_| "a parameter set is too large".to_owned())?;
        record.push(0x80 | kind);
        record.extend_from_slice(&1_u16.to_be_bytes());
        record.extend_from_slice(&length.to_be_bytes());
        record.extend_from_slice(unit);
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real-shaped Main profile, level 4.1 SPS head: header, ids, then 12 bytes of profile.
    fn sps() -> Vec<u8> {
        let mut unit = vec![0x42, 0x01, 0x01];
        unit.extend_from_slice(&[
            0x01, 0x60, 0x00, 0x00, 0x00, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00, 123,
        ]);
        unit.extend_from_slice(&[0xa0, 0x03, 0xc0, 0x80]);
        unit
    }

    #[test]
    fn nal_types_come_from_the_two_byte_header() {
        assert_eq!(nal_type(&[0x40, 0x01]), VPS);
        assert_eq!(nal_type(&[0x42, 0x01]), SPS);
        assert_eq!(nal_type(&[0x44, 0x01]), PPS);
        assert_eq!(nal_type(&[0x26, 0x01]), 19);
        assert_eq!(nal_type(&[]), 0);
    }

    #[test]
    fn a_picture_loses_its_parameter_sets_and_delimiters_and_keeps_its_slices() {
        let mut stream = Vec::new();
        for unit in [
            &[0x46_u8, 0x01, 0x50][..], // access unit delimiter
            &[0x40, 0x01, 0x0c][..],
            &[0x42, 0x01, 0x01, 0x01][..],
            &[0x44, 0x01, 0xc1][..],
            &[0x26, 0x01, 0xaf, 0x08][..], // IDR_W_RADL
        ] {
            stream.extend_from_slice(&[0, 0, 0, 1]);
            stream.extend_from_slice(unit);
        }
        let picture = annexb_to_length_prefixed(&stream).unwrap();
        assert!(picture.keyframe);
        assert_eq!(picture.vps.as_deref(), Some(&[0x40, 0x01, 0x0c][..]));
        assert_eq!(picture.sps.as_deref(), Some(&[0x42, 0x01, 0x01, 0x01][..]));
        assert_eq!(picture.pps.as_deref(), Some(&[0x44, 0x01, 0xc1][..]));
        assert_eq!(picture.data, [0, 0, 0, 4, 0x26, 0x01, 0xaf, 0x08]);
    }

    #[test]
    fn a_predicted_picture_is_not_a_keyframe() {
        let stream = [0, 0, 0, 1, 0x02, 0x01, 0xaa];
        let picture = annexb_to_length_prefixed(&stream).unwrap();
        assert!(!picture.keyframe);
        assert_eq!(picture.data, [0, 0, 0, 3, 0x02, 0x01, 0xaa]);
    }

    #[test]
    fn data_with_no_picture_or_no_start_codes_is_refused() {
        assert!(annexb_to_length_prefixed(&[1, 2, 3]).is_err());
        let sets_only = [0, 0, 0, 1, 0x40, 0x01, 0x0c];
        assert!(annexb_to_length_prefixed(&sets_only).is_err());
    }

    #[test]
    fn the_record_carries_profile_level_and_all_three_sets() {
        let vps = [0x40, 0x01, 0x0c, 0x01];
        let pps = [0x44, 0x01, 0xc1];
        let record = build_hvcc(&vps, &sps(), &pps).unwrap();
        assert_eq!(record[0], 1);
        assert_eq!(record[1], 0x01, "profile space, tier and Main profile");
        assert_eq!(&record[2..6], &[0x60, 0x00, 0x00, 0x00]);
        assert_eq!(record[12], 123, "level 4.1");
        assert_eq!(record[21] & 3, 3, "four-byte lengths");
        assert_eq!(record[22], 3, "three arrays");
        // The arrays follow in order: VPS, SPS, PPS.
        assert_eq!(record[23], 0x80 | VPS);
        let parsed = filmcraft_isobmff::HevcConfig::parse(&record).unwrap();
        assert_eq!(parsed.general_profile_idc, 1);
        assert_eq!(parsed.general_level_idc, 123);
        assert_eq!(parsed.chroma_format_idc, 1);
        assert_eq!(parsed.bit_depth_luma, 8);
        assert_eq!(parsed.length_size, 4);
        assert_eq!(parsed.vps(), vec![&vps[..]]);
        assert_eq!(parsed.sps(), vec![&sps()[..]]);
    }

    #[test]
    fn damaged_parameter_sets_are_refused() {
        let vps = [0x40, 0x01, 0x0c];
        let pps = [0x44, 0x01, 0xc1];
        assert!(build_hvcc(&pps, &sps(), &pps).is_err());
        assert!(build_hvcc(&vps, &[0x42, 0x01], &pps).is_err());
        assert!(build_hvcc(&vps, &sps(), &vps).is_err());
        assert!(build_hvcc(&vps, &[0x42, 0x01, 0x01, 0x01], &pps).is_err());
    }

    #[test]
    fn emulation_prevention_bytes_are_removed_before_reading_the_profile() {
        let mut escaped = vec![0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x00];
        escaped.extend_from_slice(&[0x03, 0x90, 0, 0, 3, 0, 0, 0, 123, 0xa0]);
        let head = unescape_head(&escaped);
        assert_eq!(&head[3..8], &[0x01, 0x60, 0x00, 0x00, 0x00]);
    }
}
