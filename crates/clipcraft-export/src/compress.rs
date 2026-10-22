//! Bounded software H.264 export. Audio is copied, so its exact byte count is reserved before
//! choosing a video bitrate. A target is an estimate, never a promise about a service's limit.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clipcraft_time::{FrameRate, Tick};
use filmcraft_h264enc::{
    Encoder, EncoderConfig, PacketFormat, Preset, Profile, RateControl, YuvFrame,
};
use filmcraft_isobmff::{
    AvcConfig, Brand, Edit, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions,
};
use filmcraft_media::reader::{FileReader, open_reader_within};
use filmcraft_media::{FrameRequest, SharedReader};
use serde::{Deserialize, Serialize};

use crate::source::{Source, StreamKind};
use crate::{ExportError, ExportReport, Plan, TrimRequest, io_error, mp4_error, plan};

const VIDEO_TIMESCALE: u32 = 90_000;
const MAX_FRAMES: i64 = 54_000;
/// Threads preparing pictures (decode, resize, colour conversion) ahead of the encoder.
const PREPARERS: u32 = 2;

/// Estimated file-size choices. MB means 1,000,000 bytes, independent of any third-party limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "preset", rename_all = "snake_case")]
pub enum ExportPreset {
    /// Estimated 10 MB file.
    Small,
    /// Estimated 25 MB file.
    Medium,
    /// Estimated size from 1 to 500 MB.
    Custom { megabytes: u16 },
}

impl ExportPreset {
    /// Validates and returns the requested target, in bytes.
    pub fn target_bytes(self) -> Result<u64, ExportError> {
        let megabytes = match self {
            Self::Small => 10,
            Self::Medium => 25,
            Self::Custom { megabytes } => megabytes,
        };
        if !(1..=500).contains(&megabytes) {
            return Err(ExportError::BadPreset);
        }
        Ok(u64::from(megabytes) * 1_000_000)
    }
}

/// Actual export size and the settings chosen for its estimate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompressReport {
    /// The new clip and its actual size.
    pub clip: ExportReport,
    /// Estimated file-size target; variable-bitrate output may differ.
    pub target_bytes: u64,
    /// Whether the actual output fits the requested target.
    pub within_target: bool,
    /// Output width, without upscaling.
    pub width: u32,
    /// Output height, at most 720 pixels.
    pub height: u32,
}

/// Re-encodes a selected range as H.264 MP4, preserving its AAC audio tracks.
///
/// Supports ClipCraft's H.264/AAC MP4/MOV/MKV inputs, up to 30 minutes per job. Output is at most
/// 1280×720 and 30 fps, keeping aspect ratio and never upscaling. Work is sequential and decoded
/// pictures are not accumulated. Existing outputs and temporary files are never replaced.
pub fn compress_to_mp4(
    request: &TrimRequest,
    preset: ExportPreset,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<CompressReport, ExportError> {
    let target = preset.target_bytes()?;
    if request.start < Tick::ZERO || request.end <= request.start {
        return Err(ExportError::BadRange);
    }
    if fs::symlink_metadata(&request.output).is_ok() {
        return Err(ExportError::AlreadyExists);
    }
    check_cancel(cancel)?;
    // Open and validate before creating any output. Decoder panics become a fallible job result.
    catch_unwind(AssertUnwindSafe(|| {
        export_inner(request, target, progress, cancel)
    }))
    .unwrap_or(Err(ExportError::EncodeFailed))
}

pub(crate) struct Temporary(pub(crate) PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Bilinear resize of interleaved 8-bit samples (`channels` per pixel) with fixed-point weights,
/// about ten times faster than the general resampler for the modest reductions (under 2x) left
/// after box decimation.
fn bilinear(
    raw: &[u8],
    (sw, sh): (usize, usize),
    (dw, dh): (usize, usize),
    channels: usize,
) -> Vec<u8> {
    // For each output column: the two source columns and the weight of the second, in 1/256ths.
    let axis = |from: usize, to: usize| -> Vec<(usize, usize, u32)> {
        (0..to)
            .map(|n| {
                // Pixel centres: (n + 0.5) * from / to - 0.5, in 1/256ths.
                let position = ((2 * n + 1) * from * 256 / (2 * to)).saturating_sub(128);
                let low = (position >> 8).min(from.saturating_sub(1));
                let high = (low + 1).min(from.saturating_sub(1));
                (low, high, (position & 255) as u32)
            })
            .collect()
    };
    let columns = axis(sw, dw);
    let rows = axis(sh, dh);
    let stride = sw * channels;
    let mut out = vec![0_u8; dw * dh * channels];
    if raw.len() < stride * sh || dw == 0 || dh == 0 {
        return out;
    }
    if channels == 1 {
        bilinear_plane(raw, sw, &columns, &rows, &mut out);
        return out;
    }
    for (y, (low, high, wy)) in rows.iter().enumerate() {
        let top = &raw[low * stride..(low + 1) * stride];
        let bottom = &raw[high * stride..(high + 1) * stride];
        let line = &mut out[y * dw * channels..(y + 1) * dw * channels];
        for (pixel, (a, b, wx)) in line.chunks_exact_mut(channels).zip(&columns) {
            for (channel, slot) in pixel.iter_mut().enumerate() {
                let t = u32::from(top[a * channels + channel]) * (256 - wx)
                    + u32::from(top[b * channels + channel]) * wx;
                let u = u32::from(bottom[a * channels + channel]) * (256 - wx)
                    + u32::from(bottom[b * channels + channel]) * wx;
                *slot = ((t * (256 - wy) + u * wy + 32_768) >> 16) as u8;
            }
        }
    }
    out
}

/// One-channel case: each source row is interpolated across once and reused by the output rows
/// that need it, with the vertical blend done on those short rows.
fn bilinear_plane(
    raw: &[u8],
    sw: usize,
    columns: &[(usize, usize, u32)],
    rows: &[(usize, usize, u32)],
    out: &mut [u8],
) {
    let across = |row: usize| -> Vec<u16> {
        let line = &raw[row * sw..(row + 1) * sw];
        columns
            .iter()
            .map(|&(a, b, wx)| (u32::from(line[a]) * (256 - wx) + u32::from(line[b]) * wx) as u16)
            .collect()
    };
    let dw = columns.len();
    // The two interpolated rows of the previous output row; neighbouring output rows mostly share.
    let mut previous: Vec<(usize, Vec<u16>)> = Vec::new();
    for (y, &(low, high, wy)) in rows.iter().enumerate() {
        let take = |row: usize, previous: &mut Vec<(usize, Vec<u16>)>| -> Vec<u16> {
            match previous.iter().position(|(r, _)| *r == row) {
                Some(position) => previous.swap_remove(position).1,
                None => across(row),
            }
        };
        let top = take(low, &mut previous);
        let bottom = if high == low {
            top.clone()
        } else {
            take(high, &mut previous)
        };
        let line = &mut out[y * dw..(y + 1) * dw];
        for ((slot, &t), &b) in line.iter_mut().zip(&top).zip(&bottom) {
            *slot = ((u32::from(t) * (256 - wy) + u32::from(b) * wy + 32_768) >> 16) as u8;
        }
        previous.clear();
        previous.push((low, top));
        if high != low {
            previous.push((high, bottom));
        }
    }
}

pub(crate) fn resize_bilinear(
    source: &image::RgbaImage,
    width: u32,
    height: u32,
) -> image::RgbaImage {
    let size = (source.width() as usize, source.height() as usize);
    let out = bilinear(source.as_raw(), size, (width as usize, height as usize), 4);
    image::RgbaImage::from_raw(width, height, out)
        .unwrap_or_else(|| image::RgbaImage::new(width, height))
}

/// Scaled 8-bit 4:2:0 planes straight from a decoded BT.709 limited-range picture, skipping the
/// round trip through RGBA. `None` for any other layout, which then takes the RGBA path.
fn scale_yuv420(
    frame: &filmcraft_frame::VideoFrame,
    width: u32,
    height: u32,
) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    use filmcraft_color::{Matrix, Range};
    use filmcraft_frame::{Chroma, PixelData};
    let PixelData::Yuv8 {
        planes,
        chroma: Chroma::C420,
        alpha: None,
    } = &frame.data
    else {
        return None;
    };
    if frame.color.matrix != Matrix::Bt709
        || frame.color.range != Range::Limited
        || !width.is_multiple_of(2)
        || !height.is_multiple_of(2)
    {
        return None;
    }
    let (sw, sh) = (frame.width as usize, frame.height as usize);
    let (scw, sch) = (sw.div_ceil(2), sh.div_ceil(2));
    let [luma, blue, red] = planes;
    if luma.len() < sw * sh || blue.len() < scw * sch || red.len() < scw * sch {
        return None;
    }
    let (dw, dh) = (width as usize, height as usize);
    let plane = |data: &[u8], from: (usize, usize), to: (usize, usize)| {
        if from == to {
            data.to_vec()
        } else {
            bilinear(data, from, to, 1)
        }
    };
    let mut y = plane(luma, (sw, sh), (dw, dh));
    let mut u = plane(blue, (scw, sch), (dw / 2, dh / 2));
    let mut v = plane(red, (scw, sch), (dw / 2, dh / 2));
    // The encoder's own conversion keeps samples inside 1..=254; do the same.
    for sample in y.iter_mut().chain(u.iter_mut()).chain(v.iter_mut()) {
        *sample = (*sample).clamp(1, 254);
    }
    Some((y, u, v))
}

/// Encoder threads: every core but one (so the app and a running game keep responding), 2 to 8.
pub(crate) fn worker_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(2, |cores| cores.get().saturating_sub(1))
        .clamp(2, 8)
}

pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<(), ExportError> {
    if cancel.load(Ordering::Relaxed) {
        Err(ExportError::Cancelled)
    } else {
        Ok(())
    }
}

pub(crate) fn source_size(width: u32, height: u32) -> Result<(), ExportError> {
    if width < 2
        || height < 2
        || width > 7_680
        || height > 7_680
        || u64::from(width) * u64::from(height) > 33_177_600
    {
        return Err(ExportError::Unsupported(
            "the picture dimensions are outside export limits",
        ));
    }
    Ok(())
}

fn export_inner(
    request: &TrimRequest,
    target: u64,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<CompressReport, ExportError> {
    let source = Source::open(&request.input)?;
    let plans = plan(&source, request.start, request.end)?;
    let video = plans
        .iter()
        .find(|p| source.streams[p.stream].kind == StreamKind::Video)
        .ok_or(ExportError::Unsupported("the clip has no video"))?;
    let stream = &source.streams[video.stream];
    if stream.entry.format != *b"avc1" && stream.entry.format != *b"avc3" {
        return Err(ExportError::Unsupported(
            "small-file export currently needs H.264 video",
        ));
    }
    let dimensions = stream.entry.video.as_ref().ok_or(ExportError::Damaged)?;
    source_size(u32::from(dimensions.width), u32::from(dimensions.height))?;
    let audio: Vec<&Plan> = plans
        .iter()
        .filter(|p| source.streams[p.stream].kind == StreamKind::Audio)
        .collect();
    if audio.len() > 16
        || audio
            .iter()
            .any(|p| source.streams[p.stream].entry.format != *b"mp4a")
    {
        return Err(ExportError::Unsupported(
            "small-file export supports up to 16 AAC audio tracks",
        ));
    }
    let duration = Tick::from_units(
        i64::try_from(video.segment).map_err(|_| ExportError::BadPreset)?,
        1_000,
    );
    if duration > Tick::from_units(1_800, 1) || duration < Tick::from_units(100, 1_000) {
        return Err(ExportError::BadPreset);
    }
    let reader: SharedReader =
        Arc::new(FileReader::open(&request.input).map_err(|_| ExportError::Unreadable)?);
    let name = request
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
    let info = decoder.info().video.as_ref().ok_or(ExportError::Damaged)?;
    source_size(info.width, info.height)?;
    if info.hdr.is_some() || info.par.0 == 0 || info.par.1 == 0 || info.par.0 != info.par.1 {
        return Err(ExportError::Unsupported(
            "small-file export currently needs SDR video with square pixels",
        ));
    }
    let source_rate = info.frame_rate.sane();
    let rate = if source_rate.num > source_rate.den.saturating_mul(30) {
        FrameRate { num: 30, den: 1 }
    } else {
        source_rate
    };
    let frames =
        rate.frame_at(duration) + i64::from(rate.tick_of(rate.frame_at(duration)) < duration);
    if !(1..=MAX_FRAMES).contains(&frames) {
        return Err(ExportError::BadPreset);
    }
    let audio_bytes: u64 = audio
        .iter()
        .map(|p| {
            source.streams[p.stream].samples[p.first..=p.last]
                .iter()
                .map(|s| u64::from(s.size))
                .sum::<u64>()
        })
        .sum();
    let source_bytes: u64 = stream.samples[video.first..=video.last]
        .iter()
        .map(|s| u64::from(s.size))
        .sum();
    let source_kbps = source_bytes.saturating_mul(8) / duration.to_units_floor(1_000).max(1) as u64;
    let budget_kbps = video_budget(target, audio_bytes, duration)?;
    let kbps = budget_kbps.min(source_kbps.clamp(100, 12_000) as u32);
    let (width, height) = output_size(info.width, info.height, budget_kbps);
    let fps_num = u32::try_from(rate.num).map_err(|_| ExportError::BadPreset)?;
    let fps_den = u32::try_from(rate.den).map_err(|_| ExportError::BadPreset)?;
    let mut config = EncoderConfig::new(width, height, fps_num, fps_den);
    config.profile = Profile::Baseline;
    config.preset = Preset::Speed;
    config.bframes = 0;
    config.threads = worker_threads();
    // Variance-adaptive quantisation and scene-cut search cost time for little gain at these sizes.
    config.aq_strength = 0.0;
    config.scenecut = false;
    config.keyint = 60;
    config.format = PacketFormat::LengthPrefixed;
    config.rate = RateControl::Vbr {
        target_kbps: kbps,
        max_kbps: kbps.saturating_mul(2),
    };
    let mut encoder = Encoder::new(config).map_err(|_| ExportError::EncodeFailed)?;
    let avc = AvcConfig::parse(&encoder.avcc()).map_err(|_| ExportError::EncodeFailed)?;

    if let Some(folder) = request
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        fs::create_dir_all(folder).map_err(|e| io_error(&e))?;
    }
    let mut partial = request.output.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    // Own the cleanup guard only after create_new succeeds: never remove someone else's file.
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
    let mut mp4 =
        Mp4Writer::new(BufWriter::new(file), WriterOptions::new(Brand::Mp4)).map_err(mp4_error)?;
    let video_id = mp4
        .add_track(TrackConfig::new(
            SampleEntry::avc(avc, width as u16, height as u16),
            VIDEO_TIMESCALE,
        ))
        .map_err(mp4_error)?;
    let mut tracks = Vec::new();
    for p in &audio {
        let stream = &source.streams[p.stream];
        let mut config = TrackConfig::new(stream.entry.clone(), stream.timescale);
        config.edits = vec![Edit {
            segment_duration: p.segment,
            media_time: p.media_time,
            media_rate: 0x0001_0000,
        }];
        tracks.push((mp4.add_track(config).map_err(mp4_error)?, p.first));
    }
    // Decoding, resizing and colour conversion run on a second thread, a few pictures ahead of
    // the encoder, so the two overlap. The queue is small, so memory stays bounded.
    std::thread::scope(|scope| -> Result<(), ExportError> {
        // Two preparers take every other picture; the encoder reads them back in order.
        let lanes = i64::from(PREPARERS);
        let mut receivers = Vec::new();
        let decoder = &decoder;
        for lane in 0..lanes {
            let (sender, receiver) = std::sync::mpsc::sync_channel::<
                Result<(Vec<u8>, Vec<u8>, Vec<u8>), ExportError>,
            >(3);
            receivers.push(receiver);
            scope.spawn(move || {
                for index in (lane..frames).step_by(PREPARERS as usize) {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let made = (|| {
                        let decoded = decoder
                            .video_frame(FrameRequest::full(request.start + rate.tick_of(index)))
                            .map_err(|_| ExportError::EncodeFailed)?;
                        source_size(decoded.width, decoded.height)?;
                        let factor =
                            (decoded.width / width).min(decoded.height / height).max(1) as usize;
                        let small = decoded.box_decimated(factor);
                        let frame = small.as_ref().unwrap_or(&decoded);
                        if let Some(planes) = scale_yuv420(frame, width, height) {
                            return Ok(planes);
                        }
                        let image =
                            image::RgbaImage::from_raw(frame.width, frame.height, frame.to_rgba8())
                                .ok_or(ExportError::Damaged)?;
                        // A picture that is already the output size needs no resampling pass.
                        let resized = if (frame.width, frame.height) == (width, height) {
                            image
                        } else {
                            resize_bilinear(&image, width, height)
                        };
                        let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
                        filmcraft_export::rgba_to_yuv420_8(
                            resized.as_raw(),
                            width as usize,
                            height as usize,
                            &mut y,
                            &mut u,
                            &mut v,
                        );
                        Ok((y, u, v))
                    })();
                    let failed = made.is_err();
                    if sender.send(made).is_err() || failed {
                        return;
                    }
                }
            });
        }
        for index in 0..frames {
            check_cancel(cancel)?;
            let time = rate.tick_of(index);
            let (y, u, v) = receivers
                .get((index % lanes) as usize)
                .ok_or(ExportError::EncodeFailed)?
                .recv()
                .map_err(|_| ExportError::EncodeFailed)??;
            let packets = encoder
                .encode(
                    &YuvFrame {
                        y: &y,
                        u: &u,
                        v: &v,
                        y_stride: width as usize,
                        uv_stride: width as usize / 2,
                    },
                    index,
                )
                .map_err(|_| ExportError::EncodeFailed)?;
            for packet in packets {
                write_video(&mut mp4, video_id, &packet, rate, duration)?;
            }
            copy_audio(&source, &audio, &mut tracks, &mut mp4, Some(time), cancel)?;
            progress(((index + 1) as f32 / frames as f32 * 0.98).min(0.98));
        }
        Ok(())
    })?;
    check_cancel(cancel)?;
    for packet in encoder.flush() {
        write_video(&mut mp4, video_id, &packet, rate, duration)?;
    }
    copy_audio(&source, &audio, &mut tracks, &mut mp4, None, cancel)?;
    let mut out = mp4.finish().map_err(mp4_error)?;
    out.flush().map_err(|e| io_error(&e))?;
    out.get_ref().sync_all().map_err(|e| io_error(&e))?;
    let bytes = out.get_ref().metadata().map_err(|e| io_error(&e))?.len();
    drop(out);
    check_cancel(cancel)?;
    // Publish without replacement, including an output created after the preflight check.
    fs::hard_link(&temporary.0, &request.output).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => ExportError::AlreadyExists,
        std::io::ErrorKind::Unsupported => ExportError::Unsupported(
            "choose an output folder on a filesystem supporting hard links",
        ),
        _ => io_error(&e),
    })?;
    progress(1.0);
    Ok(CompressReport {
        clip: ExportReport {
            path: request.output.clone(),
            duration,
            pictures: frames as u64,
            audio_tracks: audio.len(),
            bytes,
        },
        target_bytes: target,
        within_target: bytes <= target,
        width,
        height,
    })
}

fn video_budget(target: u64, audio_bytes: u64, duration: Tick) -> Result<u32, ExportError> {
    // Reserve 10% plus 64 KB for indexes, metadata, bitrate variation, and mux overhead.
    let room = target.saturating_mul(9) / 10;
    let room = room
        .checked_sub(audio_bytes.saturating_add(65_536))
        .ok_or(ExportError::TargetTooSmall)?;
    let millis = duration.to_units_floor(1_000);
    if millis <= 0 {
        return Err(ExportError::BadRange);
    }
    let kbps = room.saturating_mul(8) / millis as u64;
    if kbps < 100 {
        return Err(ExportError::TargetTooSmall);
    }
    Ok(kbps.min(12_000) as u32)
}

fn output_size(width: u32, height: u32, kbps: u32) -> (u32, u32) {
    // Fewer pixels also mean faster export, and at a low bitrate they look better than 720p.
    let cap = if kbps < 350 {
        360
    } else if kbps < 900 {
        480
    } else if kbps < 2_500 {
        540
    } else {
        720
    };
    let max_width = cap * 16 / 9;
    let (w, h) = if u64::from(width) * u64::from(cap) > u64::from(height) * u64::from(max_width) {
        let w = width.min(max_width);
        (
            w,
            (u64::from(height) * u64::from(w) / u64::from(width)) as u32,
        )
    } else {
        let h = height.min(cap);
        (
            (u64::from(width) * u64::from(h) / u64::from(height)) as u32,
            h,
        )
    };
    ((w / 2 * 2).max(2), (h / 2 * 2).max(2))
}

fn write_video(
    mp4: &mut Mp4Writer<BufWriter<File>>,
    id: usize,
    packet: &filmcraft_h264enc::Packet,
    rate: FrameRate,
    duration: Tick,
) -> Result<(), ExportError> {
    let start = rate
        .tick_of(packet.pts)
        .to_units_floor(i64::from(VIDEO_TIMESCALE));
    let end = rate
        .tick_of(packet.pts + 1)
        .min(duration)
        .to_units_floor(i64::from(VIDEO_TIMESCALE));
    let span = u32::try_from(end - start)
        .ok()
        .filter(|v| *v > 0)
        .ok_or(ExportError::EncodeFailed)?;
    mp4.write_sample(
        id,
        WriteSample {
            data: &packet.data,
            duration: span,
            composition_offset: 0,
            is_sync: packet.keyframe,
        },
    )
    .map_err(mp4_error)
}

fn copy_audio(
    source: &Source,
    plans: &[&Plan],
    tracks: &mut [(usize, usize)],
    mp4: &mut Mp4Writer<BufWriter<File>>,
    until: Option<Tick>,
    cancel: &AtomicBool,
) -> Result<(), ExportError> {
    for (p, (id, next)) in plans.iter().zip(tracks.iter_mut()) {
        let stream = &source.streams[p.stream];
        while *next <= p.last {
            check_cancel(cancel)?;
            let sample = &stream.samples[*next];
            let at = Tick::from_units(
                sample.dts - stream.samples[p.first].dts,
                i64::from(stream.timescale),
            );
            if until.is_some_and(|time| at > time) {
                break;
            }
            let data = source.read(p.stream, *next)?;
            mp4.write_sample(
                *id,
                WriteSample {
                    data: &data,
                    duration: stream.duration_of(*next),
                    composition_offset: 0,
                    is_sync: sample.sync,
                },
            )
            .map_err(mp4_error)?;
            *next += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_and_audio_budget_are_bounded() {
        assert_eq!(ExportPreset::Small.target_bytes(), Ok(10_000_000));
        assert!(
            ExportPreset::Custom { megabytes: 0 }
                .target_bytes()
                .is_err()
        );
        assert!(
            ExportPreset::Custom { megabytes: 501 }
                .target_bytes()
                .is_err()
        );
        let duration = Tick::from_units(60, 1);
        assert_eq!(
            video_budget(1_000_000, 1_000_000, duration),
            Err(ExportError::TargetTooSmall)
        );
        assert!(video_budget(10_000_000, 960_000, duration).is_ok());
    }

    #[test]
    fn bilinear_resize_keeps_flat_colour_and_edges_and_handles_tiny_sources() {
        let flat = image::RgbaImage::from_pixel(1920, 1080, image::Rgba([10, 120, 250, 255]));
        let out = resize_bilinear(&flat, 1280, 720);
        assert_eq!((out.width(), out.height()), (1280, 720));
        assert!(out.pixels().all(|p| p.0 == [10, 120, 250, 255]));
        // A black-to-white split stays a split, with only a thin blend between them.
        let mut split = image::RgbaImage::new(300, 100);
        for (x, _, p) in split.enumerate_pixels_mut() {
            *p = if x < 150 {
                image::Rgba([0, 0, 0, 255])
            } else {
                image::Rgba([255, 255, 255, 255])
            };
        }
        let out = resize_bilinear(&split, 200, 66);
        assert_eq!(out.get_pixel(10, 5).0[0], 0);
        assert_eq!(out.get_pixel(190, 5).0[0], 255);
        // One pixel in, many out, and the reverse, never panic or leave the buffer.
        let one = image::RgbaImage::from_pixel(1, 1, image::Rgba([7, 7, 7, 255]));
        assert!(
            resize_bilinear(&one, 8, 8)
                .pixels()
                .all(|p| p.0 == [7, 7, 7, 255])
        );
        assert_eq!(resize_bilinear(&flat, 2, 2).width(), 2);
    }

    #[test]
    fn the_one_channel_resize_matches_the_general_one() {
        let (w, h) = (97_usize, 61_usize);
        let raw: Vec<u8> = (0..w * h).map(|n| (n * 37 % 251) as u8).collect();
        let fast = bilinear(&raw, (w, h), (64, 40), 1);
        // The same samples as four equal channels take the general path.
        let wide: Vec<u8> = raw.iter().flat_map(|&v| [v; 4]).collect();
        let general = bilinear(&wide, (w, h), (64, 40), 4);
        assert!(
            fast.iter()
                .zip(general.chunks_exact(4))
                .all(|(a, g)| *a == g[0])
        );
        // Upscaling and a 1-pixel row work too.
        assert_eq!(bilinear(&raw[..w], (w, 1), (10, 3), 1).len(), 30);
    }

    #[test]
    fn size_keeps_aspect_and_never_upscales() {
        assert_eq!(output_size(1920, 1080, 3_000), (1280, 720));
        assert_eq!(output_size(1920, 1080, 2_000), (960, 540));
        assert_eq!(output_size(1080, 1920, 3_000), (404, 720));
        assert_eq!(output_size(1080, 1920, 2_000), (302, 540));
        assert_eq!(output_size(160, 90, 2_000), (160, 90));
        assert_eq!(output_size(1920, 1080, 200), (640, 360));
        assert!(source_size(u32::MAX, 100).is_err());
    }
}
