//! Cuts a clip down to a range and writes the result as a new MP4, without decoding or
//! re-encoding anything.
//!
//! The pictures are copied from the keyframe at or before the start, and an edit list tells
//! players to begin at the exact start frame, so a cut is frame accurate at the start and ends on
//! the exact last frame. Sound is copied the same way, with the one AAC unit before the cut kept
//! so the decoder has its overlap. Sources are MP4 or MOV (any codec the MP4 format holds) and
//! Matroska or WebM holding H.264 and AAC, which is what ClipCraft records.
//!
//! The input is never touched. The output appears only when complete, is never written over an
//! existing file, and can be cancelled at any moment. Everything read from the input is bounded.

mod compress;
mod gif;
mod hardware;
mod normalize;
mod sequence;
mod source;
pub use sequence::{SequenceRequest, SequenceSegment, sequence_to_mp4};

pub use compress::{CompressReport, ExportPreset, compress_to_mp4, compress_to_mp4_with};
pub use gif::{GifOptions, MAX_GIF_FPS, MAX_GIF_SECONDS, export_gif};
pub use hardware::{FastEncoder, FastOpener, FastPacket, FastRequest};
pub use normalize::{LOUDNESS_TARGETS, NormalizeReport, normalize_to_mp4};

use std::fs::{self, File};
use std::io::{BufWriter, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use clipcraft_time::Tick;
use filmcraft_isobmff::{Brand, Edit, Mp4Writer, TrackConfig, WriteSample, WriterOptions};
use source::{Source, StreamKind};

/// Timescale of the movie header, and so of an edit's duration.
const MOVIE_TIMESCALE: u32 = 1_000;

/// What to cut.
#[derive(Clone, Debug)]
pub struct TrimRequest {
    /// The clip to cut. It is only read.
    pub input: PathBuf,
    /// Where to write the new file. Must not exist.
    pub output: PathBuf,
    /// Where the new clip begins, from the start of the input.
    pub start: Tick,
    /// Where it ends. Past the end of the input means "to the end".
    pub end: Tick,
}

/// A finished export.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportReport {
    /// The file written.
    pub path: PathBuf,
    /// Length of the new clip.
    pub duration: Tick,
    /// Pictures written, including the few before the start that only the decoder needs.
    pub pictures: u64,
    /// Tracks of sound written.
    pub audio_tracks: usize,
    /// Size of the file.
    pub bytes: u64,
}

/// Why an export failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportError {
    /// The edit exceeds the bounded single-track contract.
    InvalidSequence,
    /// The input could not be read.
    Unreadable,
    /// The input is not a video ClipCraft can cut, and what is missing.
    Unsupported(&'static str),
    /// The input's headers are damaged, or it points outside itself.
    Damaged,
    /// The start is not before the end, or lies beyond the input.
    BadRange,
    /// The output file exists already; ClipCraft never overwrites.
    AlreadyExists,
    /// The disk is full.
    DiskFull,
    /// The output folder cannot be written to.
    FolderNotWritable,
    /// The output could not be written.
    WriteFailed,
    /// Cancelled; nothing was left behind.
    Cancelled,
    /// The requested size or amount of work is outside the supported limits.
    BadPreset,
    /// The selected audio and minimum video bitrate do not fit the target.
    TargetTooSmall,
    /// Decoding or encoding failed. The original is preserved.
    EncodeFailed,
    /// A GIF can cover at most 30 seconds.
    GifTooLong,
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSequence => formatter.write_str(
                "Use valid segments, at least one frame each, and at most 30 minutes in the edit.",
            ),
            Self::Unreadable => formatter.write_str("The clip could not be read."),
            Self::Unsupported(what) => {
                write!(formatter, "This clip cannot be exported yet: {what}.")
            }
            Self::Damaged => formatter.write_str("The clip is damaged and cannot be exported."),
            Self::BadRange => formatter.write_str("Choose a start that comes before the end."),
            Self::AlreadyExists => formatter.write_str("A file with that name already exists."),
            Self::DiskFull => {
                formatter.write_str("The disk is full. Free some space and try again.")
            }
            Self::FolderNotWritable => {
                formatter.write_str("ClipCraft cannot write to that folder.")
            }
            Self::WriteFailed => formatter.write_str("The new clip could not be written."),
            Self::Cancelled => formatter.write_str("Export cancelled."),
            Self::BadPreset => formatter.write_str(
                "Choose a target from 1 to 500 MB and a range from 0.1 seconds to 30 minutes.",
            ),
            Self::TargetTooSmall => formatter.write_str(
                "The audio and video need more room. Choose a larger target or a shorter range.",
            ),
            Self::GifTooLong => formatter.write_str("A GIF can be at most 30 seconds long."),
            Self::EncodeFailed => formatter.write_str(
                "The clip could not be decoded or compressed. The original was preserved.",
            ),
        }
    }
}

impl std::error::Error for ExportError {}

fn mp4_error(error: filmcraft_isobmff::Error) -> ExportError {
    match error {
        filmcraft_isobmff::Error::Io(error) => io_error(&error),
        _ => ExportError::WriteFailed,
    }
}

fn io_error(error: &std::io::Error) -> ExportError {
    match error.kind() {
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => ExportError::DiskFull,
        ErrorKind::PermissionDenied => ExportError::FolderNotWritable,
        _ => ExportError::WriteFailed,
    }
}

/// One sample chosen for the output.
struct Chosen {
    stream: usize,
    index: usize,
    time_seconds: f64,
}

/// What `trim` decided for one stream.
struct Plan {
    stream: usize,
    first: usize,
    last: usize,
    /// Media time the edit starts at, and the length of the edit in the movie timescale.
    media_time: i64,
    segment: u64,
}

/// Writes `request.start..request.end` of the input to the output as an MP4.
///
/// `progress` is called with a fraction from 0 to 1 as samples are copied. Setting `cancel`
/// stops the export and removes the partial file.
pub fn trim_to_mp4(
    request: &TrimRequest,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    if request.end <= request.start || request.start < Tick::ZERO {
        return Err(ExportError::BadRange);
    }
    if request.output.exists() {
        return Err(ExportError::AlreadyExists);
    }
    let source = Source::open(&request.input)?;
    let plans = plan(&source, request.start, request.end)?;
    if let Some(parent) = request
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| io_error(&error))?;
    }
    let mut partial = request.output.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let outcome = write(&source, &plans, &partial, progress, cancel);
    let (pictures, audio_tracks) = match outcome {
        Ok(counts) => counts,
        Err(error) => {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
    };
    if let Err(error) = fs::rename(&partial, &request.output) {
        let _ = fs::remove_file(&partial);
        return Err(io_error(&error));
    }
    let bytes = fs::metadata(&request.output).map_or(0, |meta| meta.len());
    let video_span = plans
        .iter()
        .find(|plan| {
            source
                .streams
                .get(plan.stream)
                .is_some_and(|s| s.kind == StreamKind::Video)
        })
        .map_or(0, |plan| plan.segment);
    let millis = i64::try_from(video_span).unwrap_or(0);
    progress(1.0);
    Ok(ExportReport {
        path: request.output.clone(),
        duration: Tick::from_units(millis, i64::from(MOVIE_TIMESCALE)),
        pictures,
        audio_tracks,
        bytes,
    })
}

/// Decides which samples of each stream go in, and where its edit starts.
fn plan(source: &Source, start: Tick, end: Tick) -> Result<Vec<Plan>, ExportError> {
    let mut plans = Vec::new();
    let mut video_seen = false;
    for (number, stream) in source.streams.iter().enumerate() {
        let scale = i64::from(stream.timescale);
        let s = start.to_rational_round(1, scale);
        let e = end.to_rational_round(1, scale);
        let presentation = |index: usize| stream.samples[index].pts + stream.offset;
        let samples = &stream.samples;
        if samples.is_empty() {
            continue;
        }
        let first_time = (0..samples.len()).map(presentation).min().unwrap_or(0);
        let content_end = samples
            .iter()
            .enumerate()
            .map(|(index, sample)| presentation(index) + i64::from(sample.duration))
            .max()
            .unwrap_or(0);
        if s >= content_end {
            return Err(ExportError::BadRange);
        }
        // The last sample, in decoding order, that is shown before the end.
        if !(0..samples.len()).any(|index| presentation(index) < e) {
            continue;
        }
        let last_decoded = (0..samples.len())
            .filter(|index| presentation(*index) < e)
            .max()
            .unwrap_or(0);
        let first = match stream.kind {
            StreamKind::Video => {
                // The last keyframe shown at or before the start.
                let keyframe = (0..samples.len())
                    .filter(|index| samples[*index].sync && presentation(*index) <= s)
                    .max();
                keyframe.unwrap_or(0)
            }
            StreamKind::Audio => {
                // The unit that covers the start, and the one before it for the decoder.
                let covering = (0..samples.len())
                    .find(|index| presentation(*index) + i64::from(samples[*index].duration) > s)
                    .unwrap_or(0);
                covering.saturating_sub(1)
            }
        };
        if first > last_decoded {
            continue;
        }
        if stream.kind == StreamKind::Video {
            video_seen = true;
        }
        let effective_start = s.max(first_time);
        let effective_end = e.min(content_end);
        if effective_end <= effective_start {
            return Err(ExportError::BadRange);
        }
        let media_time = (effective_start - stream.offset - samples[first].dts).max(0);
        let span = effective_end - effective_start;
        let segment = u64::try_from(span.saturating_mul(i64::from(MOVIE_TIMESCALE)) / scale)
            .unwrap_or(0)
            .max(1);
        plans.push(Plan {
            stream: number,
            first,
            last: last_decoded,
            media_time,
            segment,
        });
    }
    if !video_seen {
        return Err(ExportError::Unsupported("the clip has no video"));
    }
    Ok(plans)
}

fn write(
    source: &Source,
    plans: &[Plan],
    partial: &Path,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<(u64, usize), ExportError> {
    let file = File::create(partial).map_err(|error| io_error(&error))?;
    let mut mp4 =
        Mp4Writer::new(BufWriter::new(file), WriterOptions::new(Brand::Mp4)).map_err(mp4_error)?;
    let mut ids = Vec::new();
    let mut chosen: Vec<Chosen> = Vec::new();
    let mut total_bytes = 0_u64;
    for plan in plans {
        let stream = &source.streams[plan.stream];
        let mut config = TrackConfig::new(stream.entry.clone(), stream.timescale);
        config.edits = vec![Edit {
            segment_duration: plan.segment,
            media_time: plan.media_time,
            media_rate: 0x0001_0000,
        }];
        ids.push(
            mp4.add_track(config)
                .map_err(|_| ExportError::Unsupported("the codec cannot be stored in an MP4"))?,
        );
        let base = stream.samples[plan.first].dts;
        for index in plan.first..=plan.last {
            let sample = &stream.samples[index];
            total_bytes += u64::from(sample.size);
            chosen.push(Chosen {
                stream: plan.stream,
                index,
                time_seconds: (sample.dts - base) as f64 / f64::from(stream.timescale),
            });
        }
    }
    // Interleave by time; within a stream the order is the decoding order.
    chosen.sort_by(|a, b| {
        a.time_seconds
            .total_cmp(&b.time_seconds)
            .then(a.stream.cmp(&b.stream))
            .then(a.index.cmp(&b.index))
    });
    let mut done = 0_u64;
    let mut pictures = 0_u64;
    for item in &chosen {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        let stream = &source.streams[item.stream];
        let sample = &stream.samples[item.index];
        let data = source.read(item.stream, item.index)?;
        let track = plans
            .iter()
            .position(|plan| plan.stream == item.stream)
            .and_then(|position| ids.get(position))
            .copied()
            .ok_or(ExportError::Damaged)?;
        let offset = i32::try_from(sample.pts - sample.dts).map_err(|_| ExportError::Damaged)?;
        mp4.write_sample(
            track,
            WriteSample {
                data: &data,
                duration: stream.duration_of(item.index),
                composition_offset: offset,
                is_sync: sample.sync,
            },
        )
        .map_err(mp4_error)?;
        if stream.kind == StreamKind::Video {
            pictures += 1;
        }
        done += u64::from(sample.size);
        progress((done as f64 / total_bytes.max(1) as f64).min(0.99) as f32);
    }
    let mut out = mp4.finish().map_err(mp4_error)?;
    out.flush().map_err(|error| io_error(&error))?;
    out.get_ref().sync_all().map_err(|error| io_error(&error))?;
    let audio = plans
        .iter()
        .filter(|plan| source.streams[plan.stream].kind == StreamKind::Audio)
        .count();
    Ok((pictures, audio))
}
