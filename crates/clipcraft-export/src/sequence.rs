//! A bounded gapless sequence, rendered sequentially through FilmCraft's Rust codecs.
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

use crate::compress::{Temporary, check_cancel, source_size, write_video};
use crate::{ExportError, ExportReport, io_error, mp4_error};

const RATE: FrameRate = FrameRate { num: 30, den: 1 };
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

/// Where a clip's title sits in the picture.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitlePlace {
    Top,
    Center,
    #[default]
    Bottom,
}

/// Simple per-clip looks: fades at either end and a title shown for the whole clip.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SegmentLook {
    #[serde(default)]
    pub fade_in: Fade,
    #[serde(default)]
    pub fade_out: Fade,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub title_place: TitlePlace,
}
impl SegmentLook {
    /// The fade colour and how much of it covers the picture (`0.0..=1.0`) `local` into a clip
    /// that lasts `duration`.
    pub fn fade_at(&self, local: Tick, duration: Tick) -> Option<(Fade, f32)> {
        let length = FADE_LENGTH.0.min(duration.0 / 2);
        if length <= 0 {
            return None;
        }
        let into = (local.0 as f64 / length as f64) as f32;
        let before_end = ((duration.0 - local.0) as f64 / length as f64) as f32;
        let from_start =
            (self.fade_in != Fade::None && into < 1.0).then_some((self.fade_in, 1.0 - into));
        let to_end = (self.fade_out != Fade::None && before_end < 1.0)
            .then_some((self.fade_out, 1.0 - before_end));
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
    pub output: PathBuf,
}

fn validate(request: &SequenceRequest) -> Result<Tick, ExportError> {
    if request.segments.is_empty()
        || request.segments.len() > 30 * 1800
        || request.output.as_os_str().is_empty()
        || request.output.as_os_str().len() > 4096
    {
        return Err(ExportError::InvalidSequence);
    }
    let mut total = 0_i64;
    for segment in &request.segments {
        if segment.input.as_os_str().is_empty()
            || segment.input.as_os_str().len() > 4096
            || segment.start < Tick::ZERO
            || segment.end <= segment.start
            || segment.volume_percent > 100
            || segment.look.title.chars().count() > MAX_TITLE_CHARS
            || segment.look.title.chars().any(char::is_control)
        {
            return Err(ExportError::InvalidSequence);
        }
        let duration = segment
            .end
            .0
            .checked_sub(segment.start.0)
            .ok_or(ExportError::InvalidSequence)?;
        if duration < RATE.frame_duration().0 {
            return Err(ExportError::InvalidSequence);
        }
        total = total
            .checked_add(duration)
            .ok_or(ExportError::InvalidSequence)?;
        if total > Tick::from_units(1800, 1).0 {
            return Err(ExportError::InvalidSequence);
        }
    }
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

/// Render a 16:9 sequence at 30 fps, at most 720p, with one 48 kHz stereo AAC track.
/// The first source determines canvas height; pictures fit inside it with black bars and no
/// upscaling. The first audio track is used; silent/muted clips contribute silence. No whole-clip
/// raw-frame or audio buffer is built, and existing files are never replaced.
pub fn sequence_to_mp4(
    request: &SequenceRequest,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    let duration = validate(request)?;
    if fs::symlink_metadata(&request.output).is_ok() {
        return Err(ExportError::AlreadyExists);
    }
    check_cancel(cancel)?;
    catch_unwind(AssertUnwindSafe(|| {
        render(request, duration, progress, cancel)
    }))
    .unwrap_or(Err(ExportError::EncodeFailed))
}

fn render(
    request: &SequenceRequest,
    duration: Tick,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    let mut height = 720;
    // Validate every source before creating output. Keep just one decoder/cache open at a time.
    for (index, segment) in request.segments.iter().enumerate() {
        check_cancel(cancel)?;
        let decoder = open(segment)?;
        if index == 0 {
            height = decoder
                .info()
                .video
                .as_ref()
                .ok_or(ExportError::Damaged)?
                .height
                .min(720)
                / 2
                * 2;
        }
    }
    let width = (height * 16 / 9 / 2 * 2).max(2);
    let mut config = EncoderConfig::new(width, height, 30, 1);
    config.profile = Profile::Baseline;
    config.preset = Preset::Speed;
    config.bframes = 0;
    config.threads = 2;
    config.keyint = 60;
    config.format = PacketFormat::LengthPrefixed;
    config.rate = RateControl::Vbr {
        target_kbps: 4000,
        max_kbps: 8000,
    };
    let mut video = Encoder::new(config).map_err(|_| ExportError::EncodeFailed)?;
    let avc = AvcConfig::parse(&video.avcc()).map_err(|_| ExportError::EncodeFailed)?;
    let mut audio =
        filmcraft_aac::Encoder::new(filmcraft_aac::EncoderConfig::cbr(AUDIO_RATE, 2, 160_000))
            .map_err(|_| ExportError::EncodeFailed)?;
    if let Some(parent) = request
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|e| io_error(&e))?;
    }
    let mut partial = request.output.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                ExportError::AlreadyExists
            } else {
                io_error(&e)
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
    let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
    let mut picture = 0_i64;
    let mut audio_sample = 0_i64;
    let mut offset = Tick::ZERO;
    for segment in &request.segments {
        check_cancel(cancel)?;
        let decoder = open(segment)?;
        let end = offset + (segment.end - segment.start);
        let title = title_mask(&segment.look, width, height);
        let segment_audio_end = ceil_units(end, i64::from(AUDIO_RATE));
        while RATE.tick_of(picture) < end {
            check_cancel(cancel)?;
            let time = RATE.tick_of(picture);
            let source_time = segment.start + (time - offset).max(Tick::ZERO);
            let decoded = decoder
                .video_frame(FrameRequest::full(source_time))
                .map_err(|_| ExportError::EncodeFailed)?;
            source_size(decoded.width, decoded.height)?;
            let factor = (decoded.width / width).min(decoded.height / height).max(1) as usize;
            let small = decoded.box_decimated(factor);
            let frame = small.as_ref().unwrap_or(&decoded);
            let image = image::RgbaImage::from_raw(frame.width, frame.height, frame.to_rgba8())
                .ok_or(ExportError::Damaged)?;
            let fitted = image::imageops::thumbnail(
                &image,
                width.min(image.width()),
                height.min(image.height()),
            );
            let mut canvas =
                image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 255]));
            image::imageops::overlay(
                &mut canvas,
                &fitted,
                i64::from((width - fitted.width()) / 2),
                i64::from((height - fitted.height()) / 2),
            );
            if let Some(title) = &title {
                title.draw(&mut canvas);
            }
            if let Some((fade, amount)) = segment
                .look
                .fade_at(time - offset, segment.end - segment.start)
            {
                blend_fade(&mut canvas, fade, amount);
            }
            filmcraft_export::rgba_to_yuv420_8(
                canvas.as_raw(),
                width as usize,
                height as usize,
                &mut y,
                &mut u,
                &mut v,
            );
            for packet in video
                .encode(
                    &YuvFrame {
                        y: &y,
                        u: &u,
                        v: &v,
                        y_stride: width as usize,
                        uv_stride: width as usize / 2,
                    },
                    picture,
                )
                .map_err(|_| ExportError::EncodeFailed)?
            {
                write_video(
                    &mut mp4,
                    video_id,
                    (&packet.data, packet.keyframe, packet.pts),
                    RATE,
                    duration,
                )?;
            }
            picture += 1;
            let until = ceil_units(RATE.tick_of(picture).min(end), i64::from(AUDIO_RATE));
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
            progress((RATE.tick_of(picture).0 as f64 / duration.0 as f64 * 0.98).min(0.98) as f32);
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
    for packet in video.flush() {
        check_cancel(cancel)?;
        write_video(
            &mut mp4,
            video_id,
            (&packet.data, packet.keyframe, packet.pts),
            RATE,
            duration,
        )?;
    }
    for data in audio.flush() {
        check_cancel(cancel)?;
        write_audio_packet(&mut mp4, audio_id, &data)?;
    }
    check_cancel(cancel)?;
    let mut out = mp4.finish().map_err(mp4_error)?;
    out.flush().map_err(|e| io_error(&e))?;
    out.get_ref().sync_all().map_err(|e| io_error(&e))?;
    let bytes = out.get_ref().metadata().map_err(|e| io_error(&e))?.len();
    drop(out);
    check_cancel(cancel)?;
    fs::hard_link(&temporary.0, &request.output).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => ExportError::AlreadyExists,
        std::io::ErrorKind::Unsupported => {
            ExportError::Unsupported("choose a filesystem supporting hard links")
        }
        _ => io_error(&e),
    })?;
    progress(1.0);
    Ok(ExportReport {
        path: request.output.clone(),
        duration,
        pictures: picture as u64,
        audio_tracks: 1,
        bytes,
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
        let mut left = vec![0.0; frames];
        let mut right = vec![0.0; frames];
        if segment.volume_percent > 0 && !decoder.info().audio_streams.is_empty() {
            let local = (Tick::from_units(*next, i64::from(AUDIO_RATE)) - offset).max(Tick::ZERO);
            let source_time = segment.start + local;
            // Sound fades with the picture, sampled once per block.
            let fade_gain = segment
                .look
                .fade_at(local, segment.end - segment.start)
                .map_or(1.0, |(_, amount)| 1.0 - amount);
            let buffer = decoder
                .audio_stream(
                    0,
                    source_time.to_units_floor(i64::from(AUDIO_RATE)),
                    frames,
                    AUDIO_RATE,
                )
                .map_err(|_| ExportError::EncodeFailed)?;
            let factor = f32::from(segment.volume_percent) / 100.0 * fade_gain;
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
                right.clone_from(&left);
            }
        }
        for data in encoder.encode(&[&left, &right]) {
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

/// A rendered title: a soft shadow and white letters, placed once per clip.
struct Title {
    mask: filmcraft_text::Mask,
    x: i64,
    y: i64,
}
impl Title {
    fn draw(&self, canvas: &mut image::RgbaImage) {
        let shadow = (self.mask.h as f32 / 12.0).clamp(1.0, 4.0) as i64;
        for (dx, dy, color, strength) in [(shadow, shadow, 0.0_f32, 0.65_f32), (0, 0, 255.0, 1.0)] {
            for row in 0..self.mask.h {
                let y = self.y + row as i64 + dy;
                let Ok(py) = u32::try_from(y) else { continue };
                if py >= canvas.height() {
                    continue;
                }
                for col in 0..self.mask.w {
                    let coverage = self
                        .mask
                        .a
                        .get(row * self.mask.w + col)
                        .copied()
                        .unwrap_or(0.0)
                        * strength;
                    if coverage <= 0.0 {
                        continue;
                    }
                    let x = self.x + col as i64 + dx;
                    let Ok(px) = u32::try_from(x) else { continue };
                    if px >= canvas.width() {
                        continue;
                    }
                    let pixel = canvas.get_pixel_mut(px, py);
                    let amount = coverage.clamp(0.0, 1.0);
                    for channel in pixel.0.iter_mut().take(3) {
                        *channel = (f32::from(*channel) * (1.0 - amount) + color * amount)
                            .round()
                            .clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
}

fn title_mask(look: &SegmentLook, width: u32, height: u32) -> Option<Title> {
    let text = look.title.trim();
    if text.is_empty() {
        return None;
    }
    let style = filmcraft_text::TextStyle {
        size: (height as f32 * 0.075).max(10.0),
        style: "Bold".into(),
        ..Default::default()
    };
    let paragraph = filmcraft_text::ParagraphStyle {
        align: filmcraft_text::Align::Center,
        width: Some(width as f32 * 0.86),
        ..Default::default()
    };
    let layout = filmcraft_text::layout(text, &style, &paragraph);
    let (mask, left, top) =
        filmcraft_text::render::rasterize(&layout, &filmcraft_text::render::at(0.0, 0.0));
    let margin = (height as f32 * 0.08) as i64;
    let x = (i64::from(width) - mask.w as i64) / 2;
    let y = match look.title_place {
        TitlePlace::Top => margin,
        TitlePlace::Center => (i64::from(height) - mask.h as i64) / 2,
        TitlePlace::Bottom => i64::from(height) - mask.h as i64 - margin,
    };
    let _ = (left, top);
    Some(Title { mask, x, y })
}

fn blend_fade(canvas: &mut image::RgbaImage, fade: Fade, amount: f32) {
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
