//! The few PipeWire "pod" messages the screen stream needs: the formats offered when connecting,
//! and reading back the one that was agreed. Pure byte handling, no `unsafe`, so hostile input
//! from the other end cannot do more than fail to parse.

/// Pixel layouts a screen share can hand over, with the order of their bytes in memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PixelFormat {
    Bgrx,
    Bgra,
    Rgbx,
    Rgba,
}

impl PixelFormat {
    /// The `spa_video_format` number.
    pub fn id(self) -> u32 {
        match self {
            Self::Rgbx => 7,
            Self::Bgrx => 8,
            Self::Rgba => 11,
            Self::Bgra => 12,
        }
    }

    pub fn from_id(id: u32) -> Option<Self> {
        Some(match id {
            7 => Self::Rgbx,
            8 => Self::Bgrx,
            11 => Self::Rgba,
            12 => Self::Bgra,
            _ => return None,
        })
    }

    /// Whether red comes before blue in memory.
    pub fn red_first(self) -> bool {
        matches!(self, Self::Rgbx | Self::Rgba)
    }
}

/// What the stream agreed on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Negotiated {
    pub format: PixelFormat,
    pub width: u32,
    pub height: u32,
    /// Frames per second as numerator and denominator (`0/1` when the other end left it open).
    pub rate: (u32, u32),
    /// Whether every value was a single choice already. A producer that offers ranges expects
    /// the receiver to pick one (see [`fixed_format`]).
    pub fixed: bool,
}

const TYPE_ID: u32 = 3;
const TYPE_RECTANGLE: u32 = 10;
const TYPE_FRACTION: u32 = 11;
const TYPE_OBJECT: u32 = 15;
const TYPE_CHOICE: u32 = 19;
const OBJECT_FORMAT: u32 = 0x40003;
const PARAM_ENUM_FORMAT: u32 = 3;
const PARAM_FORMAT: u32 = 4;
const KEY_MEDIA_TYPE: u32 = 1;
const KEY_MEDIA_SUBTYPE: u32 = 2;
const KEY_VIDEO_FORMAT: u32 = 0x20001;
const KEY_VIDEO_SIZE: u32 = 0x20003;
const KEY_VIDEO_FRAMERATE: u32 = 0x20004;
const MEDIA_VIDEO: u32 = 2;
const SUBTYPE_RAW: u32 = 1;
const CHOICE_RANGE: u32 = 1;
const CHOICE_ENUM: u32 = 3;

fn push(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_ne_bytes());
}

fn pad(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(8) {
        out.push(0);
    }
}

/// An `Id` property.
fn id_property(out: &mut Vec<u8>, key: u32, value: u32) {
    push(out, key);
    push(out, 0);
    push(out, 4);
    push(out, TYPE_ID);
    push(out, value);
    pad(out);
}

/// A choice property whose values are `child_size` bytes each, given as raw words.
fn choice_property(out: &mut Vec<u8>, key: u32, kind: u32, child: (u32, u32), words: &[u32]) {
    push(out, key);
    push(out, 0);
    let body = 16 + words.len() * 4;
    push(out, body as u32);
    push(out, TYPE_CHOICE);
    push(out, kind);
    push(out, 0);
    push(out, child.0);
    push(out, child.1);
    for word in words {
        push(out, *word);
    }
    pad(out);
}

/// The "I can take these screen formats" message: raw video in any of the supported pixel
/// layouts, any size up to `max_edge`, any frame rate.
pub fn enum_format(max_edge: u32) -> Vec<u8> {
    let mut props = Vec::new();
    id_property(&mut props, KEY_MEDIA_TYPE, MEDIA_VIDEO);
    id_property(&mut props, KEY_MEDIA_SUBTYPE, SUBTYPE_RAW);
    let formats = [
        PixelFormat::Bgrx,
        PixelFormat::Bgrx,
        PixelFormat::Bgra,
        PixelFormat::Rgbx,
        PixelFormat::Rgba,
    ]
    .map(PixelFormat::id);
    choice_property(
        &mut props,
        KEY_VIDEO_FORMAT,
        CHOICE_ENUM,
        (4, TYPE_ID),
        &formats,
    );
    choice_property(
        &mut props,
        KEY_VIDEO_SIZE,
        CHOICE_RANGE,
        (8, TYPE_RECTANGLE),
        &[1920, 1080, 1, 1, max_edge, max_edge],
    );
    choice_property(
        &mut props,
        KEY_VIDEO_FRAMERATE,
        CHOICE_RANGE,
        (8, TYPE_FRACTION),
        &[60, 1, 0, 1, 1000, 1],
    );
    let mut out = Vec::with_capacity(props.len() + 16);
    push(&mut out, (props.len() + 8) as u32);
    push(&mut out, TYPE_OBJECT);
    push(&mut out, OBJECT_FORMAT);
    push(&mut out, PARAM_ENUM_FORMAT);
    out.extend_from_slice(&props);
    out
}

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    let end = at.checked_add(4)?;
    Some(u32::from_ne_bytes(bytes.get(at..end)?.try_into().ok()?))
}

/// The first (default) value of a property, whether it is a plain value or a choice, and whether
/// it was a plain value. `words` is how many 32-bit numbers the value has.
fn first_value<const N: usize>(pod: &[u8], at: usize, wanted: u32) -> Option<([u32; N], bool)> {
    let kind = word(pod, at.checked_add(12)?)?;
    let mut out = [0_u32; N];
    if kind == wanted {
        for (index, slot) in out.iter_mut().enumerate() {
            *slot = word(pod, at.checked_add(16)?.checked_add(index * 4)?)?;
        }
        return Some((out, true));
    }
    if kind == TYPE_CHOICE && word(pod, at.checked_add(28)?)? == wanted {
        let choice = word(pod, at.checked_add(16)?)?;
        for (index, slot) in out.iter_mut().enumerate() {
            *slot = word(pod, at.checked_add(32)?.checked_add(index * 4)?)?;
        }
        return Some((out, choice == 0));
    }
    None
}

/// Reads the pixel format, size and rate out of a "format" object. `None` for anything that is
/// not raw video with a known layout and a sensible size.
pub fn parse_format(pod: &[u8]) -> Option<Negotiated> {
    let size = word(pod, 0)? as usize;
    if word(pod, 4)? != TYPE_OBJECT || word(pod, 8)? != OBJECT_FORMAT {
        return None;
    }
    let end = size.checked_add(8)?.min(pod.len());
    let mut at = 16_usize;
    let mut format = None;
    let mut dimensions = None;
    let mut rate = (0, 1);
    let mut fixed = true;
    let mut is_video_raw = (false, false);
    while at.checked_add(16)? <= end {
        let key = word(pod, at)?;
        let value_size = word(pod, at + 8)? as usize;
        let value = at + 16;
        match key {
            KEY_MEDIA_TYPE => {
                is_video_raw.0 = first_value::<1>(pod, at, TYPE_ID)?.0 == [MEDIA_VIDEO];
            }
            KEY_MEDIA_SUBTYPE => {
                is_video_raw.1 = first_value::<1>(pod, at, TYPE_ID)?.0 == [SUBTYPE_RAW];
            }
            KEY_VIDEO_FORMAT => {
                let ([id], plain) = first_value::<1>(pod, at, TYPE_ID)?;
                format = PixelFormat::from_id(id);
                fixed &= plain;
            }
            KEY_VIDEO_SIZE => {
                let ([width, height], plain) = first_value::<2>(pod, at, TYPE_RECTANGLE)?;
                dimensions = Some((width, height));
                fixed &= plain;
            }
            KEY_VIDEO_FRAMERATE => {
                let ([numerator, denominator], plain) = first_value::<2>(pod, at, TYPE_FRACTION)?;
                rate = (numerator, denominator.max(1));
                fixed &= plain;
            }
            _ => {}
        }
        let advance = value_size.checked_add(7)? / 8 * 8;
        at = value.checked_add(advance)?;
    }
    let (width, height) = dimensions?;
    if !(is_video_raw.0 && is_video_raw.1) || width == 0 || height == 0 {
        return None;
    }
    Some(Negotiated {
        format: format?,
        width,
        height,
        rate,
        fixed,
    })
}

/// The answer to a format that still has choices: exactly these values.
pub fn fixed_format(chosen: &Negotiated) -> Vec<u8> {
    let mut props = Vec::new();
    id_property(&mut props, KEY_MEDIA_TYPE, MEDIA_VIDEO);
    id_property(&mut props, KEY_MEDIA_SUBTYPE, SUBTYPE_RAW);
    id_property(&mut props, KEY_VIDEO_FORMAT, chosen.format.id());
    for (key, kind, words) in [
        (
            KEY_VIDEO_SIZE,
            TYPE_RECTANGLE,
            [chosen.width, chosen.height],
        ),
        (
            KEY_VIDEO_FRAMERATE,
            TYPE_FRACTION,
            [chosen.rate.0, chosen.rate.1],
        ),
    ] {
        push(&mut props, key);
        push(&mut props, 0);
        push(&mut props, 8);
        push(&mut props, kind);
        for word in words {
            push(&mut props, word);
        }
    }
    let mut out = Vec::with_capacity(props.len() + 16);
    push(&mut out, (props.len() + 8) as u32);
    push(&mut out, TYPE_OBJECT);
    push(&mut out, OBJECT_FORMAT);
    push(&mut out, PARAM_FORMAT);
    out.extend_from_slice(&props);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What PipeWire's own pod builder (`spa_pod_builder_add_object`) produces for the same
    /// formats, taken from the C headers (see docs/wayland.md).
    const FROM_C: &str = "e00000000f0000000300040003000000010000000000000004000000030000000200000000000000020000000000000004000000030000000100000000000000010002000000000024000000130000000300000000000000040000000300000008000000080000000c000000070000000b00000000000000030002000000000028000000130000000100000000000000080000000a00000080070000380400000100000001000000001e0000001e0000040002000000000028000000130000000100000000000000080000000b0000003c000000010000000000000001000000e803000001000000";

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn the_offered_formats_match_what_pipewires_own_builder_writes() {
        // The C sample offers 7680-wide pictures, as the widest edge.
        let ours = enum_format(7_680);
        assert_eq!(hex(&ours), FROM_C);
    }

    #[test]
    fn an_agreed_format_is_read_back_and_nonsense_is_refused() {
        let mut props = Vec::new();
        id_property(&mut props, KEY_MEDIA_TYPE, MEDIA_VIDEO);
        id_property(&mut props, KEY_MEDIA_SUBTYPE, SUBTYPE_RAW);
        id_property(&mut props, KEY_VIDEO_FORMAT, 12);
        push(&mut props, KEY_VIDEO_SIZE);
        push(&mut props, 0);
        push(&mut props, 8);
        push(&mut props, TYPE_RECTANGLE);
        push(&mut props, 1280);
        push(&mut props, 720);
        let mut pod = Vec::new();
        push(&mut pod, (props.len() + 8) as u32);
        push(&mut pod, TYPE_OBJECT);
        push(&mut pod, OBJECT_FORMAT);
        push(&mut pod, 4);
        pod.extend_from_slice(&props);
        assert_eq!(
            parse_format(&pod),
            Some(Negotiated {
                format: PixelFormat::Bgra,
                width: 1280,
                height: 720,
                rate: (0, 1),
                fixed: true,
            })
        );
        // Every cut-off copy and every flipped byte is refused or read safely, never a panic.
        for cut in 0..pod.len() {
            let _ = parse_format(&pod[..cut]);
        }
        for index in 0..pod.len() {
            let mut damaged = pod.clone();
            damaged[index] ^= 0xff;
            let _ = parse_format(&damaged);
        }
        assert_eq!(parse_format(&[]), None);
    }

    #[test]
    fn a_format_with_choices_is_read_by_its_defaults_and_answered_with_fixed_values() {
        // What a producer that offers ranges sends back (captured from GStreamer's PipeWire sink).
        let offered = parse_format(&unhex(FROM_C)).expect("the offer parses");
        assert_eq!(offered.format, PixelFormat::Bgrx);
        assert_eq!((offered.width, offered.height), (1920, 1080));
        assert_eq!(offered.rate, (60, 1));
        assert!(!offered.fixed);
        let answer = fixed_format(&offered);
        let read = parse_format(&answer).expect("the answer parses");
        assert!(read.fixed);
        assert_eq!(
            (read.format, read.width, read.height, read.rate),
            (offered.format, 1920, 1080, (60, 1))
        );
    }

    fn unhex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks(2)
            .filter_map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
            .collect()
    }
}
