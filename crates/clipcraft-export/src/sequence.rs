//! A bounded gapless sequence, rendered sequentially through FilmCraft's Rust codecs.
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clipcraft_time::{FrameRate, Tick};
use filmcraft_h264enc::{
    Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl, YuvFrame,
};
use filmcraft_isobmff::{
    AvcConfig, Brand, Edit, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions,
};
use filmcraft_media::reader::{FileReader, open_reader_within};
use filmcraft_media::{FrameRequest, SharedReader, SharedSource};
use serde::{Deserialize, Serialize};

use crate::ExportPreset;
use crate::compress::{Temporary, check_cancel, source_size, write_video};
use crate::hardware::{FastEncoder, FastOpener, FastPacket, FastRequest};
use crate::text::TextRaster;
use crate::{ExportError, ExportReport, SequenceQuality, io_error, mp4_error, sequence_settings};

const MIN_SEGMENT: Tick = Tick(clipcraft_time::TICKS_PER_SECOND / 30);
const AUDIO_RATE: u32 = 48_000;
const MOVIE_SCALE: u32 = 720_000; // Divisible by both 90 kHz video and 48 kHz audio.

/// Longest title, in characters.
pub const MAX_TITLE_CHARS: usize = 120;
/// Length of a fade; shorter clips use half their own length.
pub const FADE_LENGTH: Tick = Tick(clipcraft_time::TICKS_PER_SECOND / 2);

/// What a clip fades to at its start or end.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fade {
    #[default]
    None,
    Black,
    White,
}

/// A simple colour look applied to a segment's picture.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    #[default]
    None,
    BlackAndWhite,
    Sepia,
    Brighten,
    Darken,
    Contrast,
    Vivid,
    Invert,
}
impl Effect {
    pub const ALL: [Effect; 8] = [
        Effect::None,
        Effect::BlackAndWhite,
        Effect::Sepia,
        Effect::Brighten,
        Effect::Darken,
        Effect::Contrast,
        Effect::Vivid,
        Effect::Invert,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Effect::None => "None",
            Effect::BlackAndWhite => "Black & white",
            Effect::Sepia => "Sepia",
            Effect::Brighten => "Brighten",
            Effect::Darken => "Darken",
            Effect::Contrast => "Contrast",
            Effect::Vivid => "Vivid",
            Effect::Invert => "Invert",
        }
    }
    /// Recolour RGBA pixels in place; alpha is untouched.
    pub fn apply_rgba(self, pixels: &mut [u8]) {
        if self == Effect::None {
            return;
        }
        for pixel in pixels.chunks_exact_mut(4) {
            let (r, g, b) = (
                f32::from(pixel[0]),
                f32::from(pixel[1]),
                f32::from(pixel[2]),
            );
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            let out = match self {
                Effect::None => (r, g, b),
                Effect::BlackAndWhite => (luma, luma, luma),
                Effect::Sepia => (
                    0.393 * r + 0.769 * g + 0.189 * b,
                    0.349 * r + 0.686 * g + 0.168 * b,
                    0.272 * r + 0.534 * g + 0.131 * b,
                ),
                Effect::Brighten => (r + 40.0, g + 40.0, b + 40.0),
                Effect::Darken => (r * 0.6, g * 0.6, b * 0.6),
                Effect::Contrast => (
                    (r - 128.0) * 1.35 + 128.0,
                    (g - 128.0) * 1.35 + 128.0,
                    (b - 128.0) * 1.35 + 128.0,
                ),
                Effect::Vivid => (
                    luma + (r - luma) * 1.5,
                    luma + (g - luma) * 1.5,
                    luma + (b - luma) * 1.5,
                ),
                Effect::Invert => (255.0 - r, 255.0 - g, 255.0 - b),
            };
            pixel[0] = out.0.round().clamp(0.0, 255.0) as u8;
            pixel[1] = out.1.round().clamp(0.0, 255.0) as u8;
            pixel[2] = out.2.round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Simple per-clip looks: fades at either end and a colour effect.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SegmentLook {
    #[serde(default)]
    pub fade_in: Fade,
    #[serde(default)]
    pub fade_out: Fade,
    #[serde(default)]
    pub effect: Effect,
}

/// Smallest and largest text size, in thousandths of the picture height.
pub const TEXT_SIZE_RANGE: std::ops::RangeInclusive<u16> = 20..=250;
/// Most text layers in one edit.
pub const MAX_TEXT_LAYERS: usize = 256;

/// A text layer on its own track above the video: it has a time range, a place and look, and
/// takes fades and a colour effect like any other layer. A fade on a text layer is an opacity
/// ramp (black or white make no difference).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SequenceText {
    /// Where on the edit's timeline the text appears.
    pub start: Tick,
    pub end: Tick,
    pub text: String,
    /// Centre of the text across the picture, in thousandths (`0..=1000`).
    pub x: u16,
    /// Centre of the text down the picture, in thousandths (`0..=1000`).
    pub y: u16,
    /// Letter height as thousandths of the picture height.
    pub size: u16,
    pub color: [u8; 3],
    #[serde(default)]
    pub fade_in: Fade,
    #[serde(default)]
    pub fade_out: Fade,
    #[serde(default)]
    pub effect: Effect,
}
impl SequenceText {
    /// A white line in the middle of the picture, from `start` for `length`.
    pub fn new(start: Tick, length: Tick, text: &str) -> Self {
        Self {
            start,
            end: start + length,
            text: text.to_owned(),
            x: 500,
            y: 500,
            size: 75,
            color: [255, 255, 255],
            fade_in: Fade::None,
            fade_out: Fade::None,
            effect: Effect::None,
        }
    }
    pub fn duration(&self) -> Tick {
        self.end - self.start
    }
    /// True when the text, place and range are within what the export accepts.
    pub fn is_valid(&self) -> bool {
        self.start >= Tick::ZERO
            && self.end > self.start
            && self.end <= Tick::from_units(1800, 1)
            && self.text.chars().count() <= MAX_TITLE_CHARS
            && !self.text.chars().any(char::is_control)
            && self.x <= 1000
            && self.y <= 1000
            && TEXT_SIZE_RANGE.contains(&self.size)
    }
    /// How opaque the layer is `local` into its range (`1.0` when no fade applies).
    pub fn opacity_at(&self, local: Tick) -> f32 {
        fade_amount(self.fade_in, self.fade_out, local, self.duration())
            .map_or(1.0, |(_, amount)| 1.0 - amount)
    }
}

impl SegmentLook {
    /// Linear gain shared by preview and exported audio. Editing volume stays unchanged.
    pub fn audio_gain_at(&self, local: Tick, duration: Tick, volume_percent: u16) -> f32 {
        let fade = self
            .fade_at(local, duration)
            .map_or(1.0, |(_, amount)| 1.0 - amount);
        f32::from(volume_percent.min(100)) / 100.0 * fade
    }
    /// The fade colour and how much of it covers the picture (`0.0..=1.0`) `local` into a clip
    /// that lasts `duration`.
    pub fn fade_at(&self, local: Tick, duration: Tick) -> Option<(Fade, f32)> {
        fade_amount(self.fade_in, self.fade_out, local, duration)
    }
}

fn fade_amount(fade_in: Fade, fade_out: Fade, local: Tick, duration: Tick) -> Option<(Fade, f32)> {
    let length = FADE_LENGTH.0.min(duration.0 / 2);
    if length <= 0 {
        return None;
    }
    let into = (local.0 as f64 / length as f64) as f32;
    let before_end = ((duration.0 - local.0) as f64 / length as f64) as f32;
    let from_start = (fade_in != Fade::None && into < 1.0).then_some((fade_in, 1.0 - into));
    let to_end =
        (fade_out != Fade::None && before_end < 1.0).then_some((fade_out, 1.0 - before_end));
    let (kind, amount) = match (from_start, to_end) {
        (Some(a), Some(b)) => {
            if a.1 >= b.1 {
                a
            } else {
                b
            }
        }
        (a, b) => a.or(b)?,
    };
    Some((kind, amount.clamp(0.0, 1.0)))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SequenceSegment {
    pub input: PathBuf,
    pub start: Tick,
    pub end: Tick,
    pub volume_percent: u16,
    #[serde(default)]
    pub look: SegmentLook,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SequenceRequest {
    pub segments: Vec<SequenceSegment>,
    /// Text layers on the track above the video.
    #[serde(default)]
    pub texts: Vec<SequenceText>,
    /// An estimated file-size target. Absent uses the selected standard quality.
    #[serde(default)]
    pub preset: Option<ExportPreset>,
    #[serde(default)]
    pub quality: SequenceQuality,
    pub output: PathBuf,
}

/// The settings and actual result of a sequence export. File-size targets are estimates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequenceReport {
    pub clip: ExportReport,
    pub target_bytes: Option<u64>,
    pub within_target: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub video_kbps: u32,
    pub resolution_limited: bool,
    pub encoder: String,
    pub hardware_decode: bool,
}

/// Legacy 720p/30 canvas helper. New callers use `sequence_settings` for explicit quality.
pub fn sequence_canvas(
    first_width: u32,
    first_height: u32,
    preset: Option<ExportPreset>,
    duration: Tick,
) -> Result<(u32, u32), ExportError> {
    let settings = sequence_settings(
        first_width,
        first_height,
        preset,
        duration,
        SequenceQuality::default(),
    )?;
    Ok((settings.width, settings.height))
}

fn validate(request: &SequenceRequest) -> Result<Tick, ExportError> {
    if request.segments.is_empty()
        || request.segments.len() > 30 * 1800
        || request.output.as_os_str().is_empty()
        || request.output.as_os_str().len() > 4096
    {
        return Err(ExportError::InvalidSequence);
    }
    if request.texts.len() > MAX_TEXT_LAYERS || request.texts.iter().any(|t| !t.is_valid()) {
        return Err(ExportError::InvalidSequence);
    }
    let mut total = 0_i64;
    for segment in &request.segments {
        if segment.input.as_os_str().is_empty()
            || segment.input.as_os_str().len() > 4096
            || segment.start < Tick::ZERO
            || segment.end <= segment.start
            || segment.volume_percent > 100
        {
            return Err(ExportError::InvalidSequence);
        }
        let duration = segment
            .end
            .0
            .checked_sub(segment.start.0)
            .ok_or(ExportError::InvalidSequence)?;
        if duration < MIN_SEGMENT.0 {
            return Err(ExportError::InvalidSequence);
        }
        total = total
            .checked_add(duration)
            .ok_or(ExportError::InvalidSequence)?;
        if total > Tick::from_units(1800, 1).0 {
            return Err(ExportError::InvalidSequence);
        }
    }
    sequence_settings(1280, 720, request.preset, Tick(total), request.quality)?;
    Ok(Tick(total))
}

fn open(segment: &SequenceSegment) -> Result<SharedSource, ExportError> {
    // Apply the existing bounded container/header validation before entering decoder code.
    let source = crate::source::Source::open(&segment.input)?;
    if source.streams.iter().any(|s| match s.kind {
        crate::source::StreamKind::Video => {
            s.entry.format != *b"avc1" && s.entry.format != *b"avc3"
        }
        crate::source::StreamKind::Audio => s.entry.format != *b"mp4a",
    }) {
        return Err(ExportError::Unsupported(
            "the editor currently exports H.264/AAC clips",
        ));
    }
    drop(source);
    let reader: SharedReader =
        Arc::new(FileReader::open(&segment.input).map_err(|_| ExportError::Unreadable)?);
    let name = segment
        .input
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let decoder = open_reader_within(
        &name,
        reader,
        &filmcraft_codecs::reader_openers(),
        &filmcraft_codecs::openers(),
        64 * 1024 * 1024,
    )
    .map_err(|_| ExportError::EncodeFailed)?;
    let info = decoder.info();
    let video = info
        .video
        .as_ref()
        .ok_or(ExportError::Unsupported("the clip has no video"))?;
    source_size(video.width, video.height)?;
    if video.hdr.is_some() || video.par.0 == 0 || video.par.0 != video.par.1 {
        return Err(ExportError::Unsupported(
            "the editor needs SDR video with square pixels",
        ));
    }
    if segment.end > info.duration {
        return Err(ExportError::BadRange);
    }
    Ok(decoder)
}

/// Render a bounded sequence at the selected resolution and 30/60 FPS, with stereo AAC.
/// The first source determines canvas height; pictures fit inside it with black bars and no
/// upscaling. The first audio track is used; silent/muted clips contribute silence. No whole-clip
/// raw-frame or audio buffer is built, and existing files are never replaced.
pub fn sequence_to_mp4(
    request: &SequenceRequest,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    sequence_to_mp4_with(request, None, progress, cancel).map(|report| report.clip)
}

/// Renders a sequence with the requested size estimate, trying the supplied hardware encoder
/// before software. Device/bitstream failures discard the owned partial and retry from sources.
/// Cancellation and filesystem failures never start a second attempt.
pub fn sequence_to_mp4_with(
    request: &SequenceRequest,
    hardware: Option<&FastOpener>,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<SequenceReport, ExportError> {
    let duration = validate(request)?;
    if fs::symlink_metadata(&request.output).is_ok() {
        return Err(ExportError::AlreadyExists);
    }
    check_cancel(cancel)?;
    let mut highest = 0.0_f32;
    let mut report = |value: f32| {
        highest = highest.max(value);
        progress(highest);
    };
    if let Some(opener) = hardware {
        let result = catch_unwind(AssertUnwindSafe(|| {
            render(request, duration, Some(opener), &mut report, cancel)
        }))
        .unwrap_or(Err(ExportError::EncodeFailed));
        match result {
            Ok(done) => return Ok(done),
            Err(ExportError::EncodeFailed) => {}
            Err(error) => return Err(error),
        }
    }
    check_cancel(cancel)?;
    catch_unwind(AssertUnwindSafe(|| {
        render(request, duration, None, &mut report, cancel)
    }))
    .unwrap_or(Err(ExportError::EncodeFailed))
}

enum VideoEncoder {
    Software(Box<Encoder>, (Vec<u8>, Vec<u8>, Vec<u8>)),
    Hardware(Box<dyn FastEncoder>),
}
impl VideoEncoder {
    fn avcc(&self) -> Vec<u8> {
        match self {
            Self::Software(encoder, _) => encoder.avcc(),
            Self::Hardware(encoder) => encoder.avcc(),
        }
    }
    fn name(&self) -> String {
        match self {
            Self::Software(..) => "Software".into(),
            Self::Hardware(encoder) => encoder.name(),
        }
    }
    fn encode(
        &mut self,
        canvas: &image::RgbaImage,
        index: i64,
    ) -> Result<Vec<FastPacket>, ExportError> {
        match self {
            Self::Hardware(encoder) => encoder
                .encode(canvas.as_raw(), index as u64)
                .map_err(|_| ExportError::EncodeFailed),
            Self::Software(encoder, (y, u, v)) => {
                filmcraft_export::rgba_to_yuv420_8(
                    canvas.as_raw(),
                    canvas.width() as usize,
                    canvas.height() as usize,
                    y,
                    u,
                    v,
                );
                encoder
                    .encode(
                        &YuvFrame {
                            y,
                            u,
                            v,
                            y_stride: canvas.width() as usize,
                            uv_stride: canvas.width() as usize / 2,
                        },
                        index,
                    )
                    .map(software_packets)
                    .map_err(|_| ExportError::EncodeFailed)
            }
        }
    }
    fn flush(&mut self) -> Result<Vec<FastPacket>, ExportError> {
        match self {
            Self::Software(encoder, _) => Ok(software_packets(encoder.flush())),
            Self::Hardware(encoder) => encoder.flush().map_err(|_| ExportError::EncodeFailed),
        }
    }
}
fn software_packets(packets: Vec<filmcraft_h264enc::Packet>) -> Vec<FastPacket> {
    packets
        .into_iter()
        .map(|packet| FastPacket {
            data: packet.data,
            keyframe: packet.keyframe,
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn write_packets(
    mp4: &mut Mp4Writer<BufWriter<std::fs::File>>,
    video_id: usize,
    packets: Vec<FastPacket>,
    written: &mut i64,
    expected: i64,
    duration: Tick,
    rate: FrameRate,
) -> Result<(), ExportError> {
    if packets.len() > expected.saturating_sub(*written) as usize {
        return Err(ExportError::EncodeFailed);
    }
    for packet in packets {
        if (*written == 0 && !packet.keyframe) || packet.data.len() > 16 * 1024 * 1024 {
            return Err(ExportError::EncodeFailed);
        }
        // The seam uses four-byte length-prefixed NALs. Refuse incomplete/empty packets before
        // publishing; native adapters also reject reordered picture timestamps.
        let mut rest = packet.data.as_slice();
        let mut nal = false;
        while !rest.is_empty() {
            let prefix = rest.get(..4).ok_or(ExportError::EncodeFailed)?;
            let length = u32::from_be_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]) as usize;
            if length == 0 || length > rest.len().saturating_sub(4) {
                return Err(ExportError::EncodeFailed);
            }
            rest = &rest[4 + length..];
            nal = true;
        }
        if !nal {
            return Err(ExportError::EncodeFailed);
        }
        write_video(
            mp4,
            video_id,
            (&packet.data, packet.keyframe, *written),
            rate,
            duration,
        )?;
        *written += 1;
    }
    Ok(())
}

const TEXT_CACHE_BYTES: usize = 16 * 1024 * 1024;

#[allow(clippy::too_many_arguments)]
pub(crate) fn compose_frame(
    image: image::RgbaImage,
    canvas: [u32; 2],
    effect: Effect,
    fade: Option<(Fade, f32)>,
    texts: &[SequenceText],
    time: Tick,
    cache: &mut HashMap<usize, TextRaster>,
    compositor: &mut crate::compositor::Compositor,
    cancel: &AtomicBool,
) -> Result<image::RgbaImage, ExportError> {
    cache.retain(|index, raster| {
        texts.get(*index).is_some_and(|text| {
            time >= text.start && time < text.end && raster.matches(text, canvas[0], canvas[1])
        })
    });
    let mut bytes = cache.values().map(TextRaster::bytes).sum::<usize>();
    for (index, text) in texts.iter().enumerate() {
        check_cancel(cancel)?;
        if time < text.start || time >= text.end {
            continue;
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = cache.entry(index)
            && let Some(raster) = TextRaster::render(text, canvas[0], canvas[1])?
        {
            bytes = bytes
                .checked_add(raster.bytes())
                .ok_or(ExportError::InvalidSequence)?;
            if bytes > TEXT_CACHE_BYTES {
                return Err(ExportError::Unsupported(
                    "active text exceeds its cache budget; remove layers or reduce size",
                ));
            }
            entry.insert(raster);
        }
    }
    let mut plan = crate::compositor::FramePlan::plain(canvas, effect);
    plan.fade = fade;
    for (index, text) in texts.iter().enumerate() {
        if time < text.start || time >= text.end {
            continue;
        }
        if let Some(raster) = cache.get(&index) {
            let [w, h] = raster.size();
            plan.overlays.push(crate::compositor::Overlay {
                size: [w as u32, h as u32],
                rgba: raster.rgba(),
                position: raster.position(text.x, text.y),
                opacity: text.opacity_at(time - text.start),
            });
        }
    }
    check_cancel(cancel)?;
    let frame = compositor.render(image, &plan)?;
    check_cancel(cancel)?;
    Ok(frame)
}

#[allow(clippy::too_many_arguments)]
fn picture(
    request: &SequenceRequest,
    decoder: &SharedSource,
    segment: &SequenceSegment,
    time: Tick,
    offset: Tick,
    size: (u32, u32),
    cache: &mut HashMap<usize, TextRaster>,
    compositor: &mut crate::compositor::Compositor,
    cancel: &AtomicBool,
) -> Result<image::RgbaImage, ExportError> {
    let (width, height) = size;
    let source_time = segment.start + (time - offset).max(Tick::ZERO);
    let decoded = decoder
        .video_frame(FrameRequest::full(source_time))
        .map_err(|_| ExportError::EncodeFailed)?;
    source_size(decoded.width, decoded.height)?;
    check_cancel(cancel)?;
    let factor = (decoded.width / width).min(decoded.height / height).max(1) as usize;
    let small = decoded.box_decimated(factor);
    let frame = small.as_ref().unwrap_or(&decoded);
    let image = image::RgbaImage::from_raw(frame.width, frame.height, frame.to_rgba8())
        .ok_or(ExportError::Damaged)?;
    compose_frame(
        image,
        [width, height],
        segment.look.effect,
        segment
            .look
            .fade_at(time - offset, segment.end - segment.start),
        &request.texts,
        time,
        cache,
        compositor,
        cancel,
    )
}

/// Preserve the original thumbnail/alpha-composition result. Matching opaque pictures already
/// are the canvas: resizing and compositing them would allocate and traverse two full frames.
#[cfg(test)]
fn fit_picture(mut image: image::RgbaImage, size: (u32, u32), effect: Effect) -> image::RgbaImage {
    let (width, height) = size;
    if image.width() > width || image.height() > height {
        image = image::imageops::thumbnail(
            &image,
            width.min(image.width()),
            height.min(image.height()),
        );
    }
    effect.apply_rgba(&mut image);
    if image.dimensions() == size && image.pixels().all(|pixel| pixel[3] == 255) {
        return image;
    }
    // Smaller and transparent pictures still composite over opaque black exactly as before.
    let mut canvas = image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
    image::imageops::overlay(
        &mut canvas,
        &image,
        i64::from((width - image.width()) / 2),
        i64::from((height - image.height()) / 2),
    );
    canvas
}

fn render(
    request: &SequenceRequest,
    duration: Tick,
    hardware: Option<&FastOpener>,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<SequenceReport, ExportError> {
    let mut settings = sequence_settings(1280, 720, request.preset, duration, request.quality)?;
    // Validate every source before creating output. Keep one decoder/cache open at a time.
    for (index, segment) in request.segments.iter().enumerate() {
        check_cancel(cancel)?;
        let decoder = open(segment)?;
        if index == 0 {
            let video = decoder.info().video.as_ref().ok_or(ExportError::Damaged)?;
            settings = sequence_settings(
                video.width,
                video.height,
                request.preset,
                duration,
                request.quality,
            )?;
        }
    }
    let size = (settings.width, settings.height);
    let (width, height) = size;
    let kbps = settings.video_kbps;
    let rate = FrameRate {
        num: i64::from(settings.fps),
        den: 1,
    };
    let mut video = if let Some(opener) = hardware {
        VideoEncoder::Hardware(
            opener(&FastRequest {
                width,
                height,
                fps: settings.fps,
                video_kbps: kbps,
                keyframe_frames: settings.fps * 2,
            })
            .map_err(|_| ExportError::EncodeFailed)?,
        )
    } else {
        let mut config = EncoderConfig::new(width, height, settings.fps, 1);
        config.profile = Profile::Baseline;
        config.preset = Preset::Speed;
        config.bframes = 0;
        config.threads = 2;
        config.keyint = settings.fps * 2;
        config.format = PacketFormat::LengthPrefixed;
        config.rate = RateControl::Vbr {
            target_kbps: kbps,
            max_kbps: kbps.saturating_mul(2),
        };
        VideoEncoder::Software(
            Box::new(Encoder::new(config).map_err(|_| ExportError::EncodeFailed)?),
            (Vec::new(), Vec::new(), Vec::new()),
        )
    };
    let name = video.name();
    let mut text_cache = HashMap::new();
    let mut compositor = crate::compositor::Compositor::default();
    // Some native encoders only describe their stream after the first input. If that still
    // produces no usable description, software is safer than accumulating unknown packets.
    let mut first = if hardware.is_some() {
        let segment = request
            .segments
            .first()
            .ok_or(ExportError::InvalidSequence)?;
        let decoder = open(segment)?;
        let canvas = picture(
            request,
            &decoder,
            segment,
            Tick::ZERO,
            Tick::ZERO,
            size,
            &mut text_cache,
            &mut compositor,
            cancel,
        )?;
        check_cancel(cancel)?;
        Some(video.encode(&canvas, 0)?)
    } else {
        None
    };
    let avc = AvcConfig::parse(&video.avcc()).map_err(|_| ExportError::EncodeFailed)?;
    let mut audio =
        filmcraft_aac::Encoder::new(filmcraft_aac::EncoderConfig::cbr(AUDIO_RATE, 2, 160_000))
            .map_err(|_| ExportError::EncodeFailed)?;
    if let Some(parent) = request
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| io_error(&error))?;
    }
    let mut partial = request.output.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                ExportError::AlreadyExists
            } else {
                io_error(&error)
            }
        })?;
    let temporary = Temporary(partial);
    let mut options = WriterOptions::new(Brand::Mp4);
    options.movie_timescale = MOVIE_SCALE;
    let mut mp4 = Mp4Writer::new(BufWriter::new(file), options).map_err(mp4_error)?;
    let video_id = mp4
        .add_track(TrackConfig::new(
            SampleEntry::avc(avc, width as u16, height as u16),
            90_000,
        ))
        .map_err(mp4_error)?;
    let mut audio_config = TrackConfig::new(
        SampleEntry::aac(audio.audio_specific_config(), 2, AUDIO_RATE),
        AUDIO_RATE,
    );
    audio_config.edits = vec![Edit {
        segment_duration: duration.to_units_floor(i64::from(MOVIE_SCALE)) as u64,
        media_time: i64::from(audio.priming_samples()),
        media_rate: 0x0001_0000,
    }];
    let audio_id = mp4.add_track(audio_config).map_err(mp4_error)?;
    let mut frame = 0_i64;
    let mut written = 0_i64;
    let frames =
        rate.frame_at(duration) + i64::from(rate.tick_of(rate.frame_at(duration)) < duration);
    let mut audio_sample = 0_i64;
    let mut offset = Tick::ZERO;
    for segment in &request.segments {
        check_cancel(cancel)?;
        let decoder = open(segment)?;
        let end = offset + (segment.end - segment.start);
        let segment_audio_end = ceil_units(end, i64::from(AUDIO_RATE));
        while rate.tick_of(frame) < end {
            check_cancel(cancel)?;
            let time = rate.tick_of(frame);
            let packets = if frame == 0
                && let Some(packets) = first.take()
            {
                packets
            } else {
                let canvas = picture(
                    request,
                    &decoder,
                    segment,
                    time,
                    offset,
                    size,
                    &mut text_cache,
                    &mut compositor,
                    cancel,
                )?;
                check_cancel(cancel)?;
                video.encode(&canvas, frame)?
            };
            write_packets(
                &mut mp4,
                video_id,
                packets,
                &mut written,
                frames,
                duration,
                rate,
            )?;
            frame += 1;
            let until = ceil_units(rate.tick_of(frame).min(end), i64::from(AUDIO_RATE));
            write_audio_until(
                &decoder,
                segment,
                offset,
                until.min(segment_audio_end),
                &mut audio_sample,
                &mut audio,
                &mut mp4,
                audio_id,
                cancel,
            )?;
            progress((rate.tick_of(frame).0 as f64 / duration.0 as f64 * 0.98).min(0.98) as f32);
        }
        write_audio_until(
            &decoder,
            segment,
            offset,
            segment_audio_end,
            &mut audio_sample,
            &mut audio,
            &mut mp4,
            audio_id,
            cancel,
        )?;
        offset = end;
    }
    check_cancel(cancel)?;
    write_packets(
        &mut mp4,
        video_id,
        video.flush()?,
        &mut written,
        frames,
        duration,
        rate,
    )?;
    if written != frames {
        return Err(ExportError::EncodeFailed);
    }
    for data in audio.flush() {
        check_cancel(cancel)?;
        write_audio_packet(&mut mp4, audio_id, &data)?;
    }
    check_cancel(cancel)?;
    let mut out = mp4.finish().map_err(mp4_error)?;
    out.flush().map_err(|error| io_error(&error))?;
    out.get_ref().sync_all().map_err(|error| io_error(&error))?;
    let bytes = out
        .get_ref()
        .metadata()
        .map_err(|error| io_error(&error))?
        .len();
    drop(out);
    check_cancel(cancel)?;
    fs::hard_link(&temporary.0, &request.output).map_err(|error| match error.kind() {
        std::io::ErrorKind::AlreadyExists => ExportError::AlreadyExists,
        std::io::ErrorKind::Unsupported => {
            ExportError::Unsupported("choose a filesystem supporting hard links")
        }
        _ => io_error(&error),
    })?;
    progress(1.0);
    let target_bytes = request.preset.map(ExportPreset::target_bytes).transpose()?;
    Ok(SequenceReport {
        clip: ExportReport {
            path: request.output.clone(),
            duration,
            pictures: frame as u64,
            audio_tracks: 1,
            bytes,
        },
        target_bytes,
        within_target: target_bytes.is_none_or(|target| bytes <= target),
        width,
        height,
        fps: settings.fps,
        video_kbps: kbps,
        resolution_limited: settings.resolution_limited,
        encoder: name,
        hardware_decode: false,
    })
}

fn ceil_units(time: Tick, scale: i64) -> i64 {
    let units = time.to_units_floor(scale);
    units + i64::from(Tick::from_units(units, scale) < time)
}

#[allow(clippy::too_many_arguments)]
fn write_audio_until(
    decoder: &SharedSource,
    segment: &SequenceSegment,
    offset: Tick,
    until: i64,
    next: &mut i64,
    encoder: &mut filmcraft_aac::Encoder,
    mp4: &mut Mp4Writer<BufWriter<std::fs::File>>,
    id: usize,
    cancel: &AtomicBool,
) -> Result<(), ExportError> {
    while *next < until {
        check_cancel(cancel)?;
        let frames = (until - *next).min(1024) as usize;
        // At most one AAC block; avoid two heap allocations for every ~21 ms of sound.
        let mut left_storage = [0.0; 1024];
        let mut right_storage = [0.0; 1024];
        let left = &mut left_storage[..frames];
        let right = &mut right_storage[..frames];
        if segment.volume_percent > 0 && !decoder.info().audio_streams.is_empty() {
            let local = (Tick::from_units(*next, i64::from(AUDIO_RATE)) - offset).max(Tick::ZERO);
            let source_time = segment.start + local;
            // Sound fades with the picture, sampled once per block.
            let gain = segment.look.audio_gain_at(
                local,
                segment.end - segment.start,
                segment.volume_percent,
            );
            let buffer = decoder
                .audio_stream(
                    0,
                    source_time.to_units_floor(i64::from(AUDIO_RATE)),
                    frames,
                    AUDIO_RATE,
                )
                .map_err(|_| ExportError::EncodeFailed)?;
            let factor = gain;
            if let Some(channel) = buffer.channels.first() {
                for (out, sample) in left.iter_mut().zip(channel) {
                    *out = if sample.is_finite() {
                        (sample * factor).clamp(-1.0, 1.0)
                    } else {
                        0.0
                    };
                }
            }
            if let Some(channel) = buffer.channels.get(1) {
                for (out, sample) in right.iter_mut().zip(channel) {
                    *out = if sample.is_finite() {
                        (sample * factor).clamp(-1.0, 1.0)
                    } else {
                        0.0
                    };
                }
            } else {
                right.copy_from_slice(left);
            }
        }
        for data in encoder.encode(&[left, right]) {
            write_audio_packet(mp4, id, &data)?;
        }
        *next += frames as i64;
    }
    Ok(())
}
fn write_audio_packet(
    mp4: &mut Mp4Writer<BufWriter<std::fs::File>>,
    id: usize,
    data: &[u8],
) -> Result<(), ExportError> {
    mp4.write_sample(
        id,
        WriteSample {
            data,
            duration: 1024,
            composition_offset: 0,
            is_sync: true,
        },
    )
    .map_err(mp4_error)
}

pub(crate) fn blend_fade(canvas: &mut image::RgbaImage, fade: Fade, amount: f32) {
    let color = match fade {
        Fade::None => return,
        Fade::Black => 0.0,
        Fade::White => 255.0,
    };
    let amount = amount.clamp(0.0, 1.0);
    for pixel in canvas.pixels_mut() {
        for channel in pixel.0.iter_mut().take(3) {
            *channel = (f32::from(*channel) * (1.0 - amount) + color * amount).round() as u8;
        }
    }
}

#[cfg(test)]
#[path = "sequence_picture_tests.rs"]
mod picture_tests;
