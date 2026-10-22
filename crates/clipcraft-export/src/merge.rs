//! Joins clips recorded with the same settings into one MP4 without re-encoding.
//!
//! Every clip must have the same tracks with identical codec configuration (which is the case for
//! clips ClipCraft records with unchanged settings). Samples are copied back to back, and one edit
//! per clip tells players where that clip's visible part starts and how long it is, so a clip that
//! itself began between keyframes is joined frame accurately. Sound and picture are placed by
//! their own edits, so they may differ from each other by under one AAC unit (about 23 ms) per join.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use clipcraft_time::Tick;
use filmcraft_isobmff::{Brand, Edit, Mp4Writer, TrackConfig, WriteSample, WriterOptions};

use crate::compress::{Temporary, check_cancel};
use crate::source::{Source, StreamKind};
use crate::{ExportError, ExportReport, Plan, io_error, mp4_error, plan};

/// Most clips joined in one go.
pub const MAX_MERGE_CLIPS: usize = 16;

/// Far enough past any clip to mean "to the end", without overflowing a `Tick`.
fn whole_clip_end() -> Tick {
    Tick::from_units(1_000_000, 1)
}

struct Chosen {
    clip: usize,
    plan: usize,
    index: usize,
    time_seconds: f64,
}

/// Writes the clips in `inputs`, in order, as one MP4 at `output`.
///
/// Two to sixteen clips are needed. The inputs are only read, an existing output is never
/// replaced, and a cancelled or failed merge leaves no file behind.
pub fn merge_to_mp4(
    inputs: &[PathBuf],
    output: &Path,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    if !(2..=MAX_MERGE_CLIPS).contains(&inputs.len()) {
        return Err(ExportError::BadPreset);
    }
    if fs::symlink_metadata(output).is_ok() {
        return Err(ExportError::AlreadyExists);
    }
    check_cancel(cancel)?;
    let mut sources = Vec::new();
    let mut plans: Vec<Vec<Plan>> = Vec::new();
    for input in inputs {
        let source = Source::open(input)?;
        plans.push(plan(&source, Tick::ZERO, whole_clip_end())?);
        sources.push(source);
    }
    check_matching(&sources, &plans)?;
    if let Some(folder) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(folder).map_err(|e| io_error(&e))?;
    }
    let mut partial = output.as_os_str().to_owned();
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
    let (pictures, audio_tracks, millis) = write_all(&sources, &plans, file, progress, cancel)?;
    check_cancel(cancel)?;
    fs::hard_link(&temporary.0, output).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => ExportError::AlreadyExists,
        std::io::ErrorKind::Unsupported => ExportError::Unsupported(
            "choose an output folder on a filesystem supporting hard links",
        ),
        _ => io_error(&e),
    })?;
    let bytes = fs::metadata(output).map_or(0, |meta| meta.len());
    progress(1.0);
    Ok(ExportReport {
        path: output.to_path_buf(),
        duration: Tick::from_units(millis, 1_000),
        pictures,
        audio_tracks,
        bytes,
    })
}

/// Every clip must have the same tracks, with identical codec configuration and timescale.
fn check_matching(sources: &[Source], plans: &[Vec<Plan>]) -> Result<(), ExportError> {
    let (Some(first_source), Some(first_plan)) = (sources.first(), plans.first()) else {
        return Err(ExportError::BadPreset);
    };
    for (source, clip_plans) in sources.iter().zip(plans).skip(1) {
        if clip_plans.len() != first_plan.len() {
            return Err(ExportError::NotMatching);
        }
        for (a, b) in first_plan.iter().zip(clip_plans) {
            let x = first_source
                .streams
                .get(a.stream)
                .ok_or(ExportError::Damaged)?;
            let y = source.streams.get(b.stream).ok_or(ExportError::Damaged)?;
            if x.kind != y.kind || x.timescale != y.timescale || x.entry != y.entry {
                return Err(ExportError::NotMatching);
            }
        }
    }
    Ok(())
}

fn write_all(
    sources: &[Source],
    plans: &[Vec<Plan>],
    file: File,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<(u64, usize, i64), ExportError> {
    let mut mp4 =
        Mp4Writer::new(BufWriter::new(file), WriterOptions::new(Brand::Mp4)).map_err(mp4_error)?;
    let first_plan = plans.first().ok_or(ExportError::BadPreset)?;
    let first_source = sources.first().ok_or(ExportError::BadPreset)?;
    // Per output track: the media time (in its timescale) where the next clip's samples begin.
    let mut cursor = vec![0_i64; first_plan.len()];
    let mut edits: Vec<Vec<Edit>> = vec![Vec::new(); first_plan.len()];
    let mut chosen: Vec<Chosen> = Vec::new();
    let mut clip_start_seconds = 0.0_f64;
    let mut total_bytes = 0_u64;
    let mut movie_millis = 0_i64;
    for (clip, (source, clip_plans)) in sources.iter().zip(plans).enumerate() {
        let mut longest = 0.0_f64;
        for (number, p) in clip_plans.iter().enumerate() {
            let stream = source.streams.get(p.stream).ok_or(ExportError::Damaged)?;
            let base = stream.samples.get(p.first).ok_or(ExportError::Damaged)?.dts;
            let scale = f64::from(stream.timescale);
            let length: i64 = (p.first..=p.last)
                .map(|i| i64::from(stream.duration_of(i)))
                .sum();
            let at = cursor.get_mut(number).ok_or(ExportError::Damaged)?;
            let list = edits.get_mut(number).ok_or(ExportError::Damaged)?;
            list.push(Edit {
                segment_duration: p.segment,
                media_time: at.saturating_add(p.media_time),
                media_rate: 0x0001_0000,
            });
            for index in p.first..=p.last {
                let sample = stream.samples.get(index).ok_or(ExportError::Damaged)?;
                total_bytes += u64::from(sample.size);
                chosen.push(Chosen {
                    clip,
                    plan: number,
                    index,
                    time_seconds: clip_start_seconds + (sample.dts - base) as f64 / scale,
                });
            }
            *at = at.saturating_add(length);
            longest = longest.max(length as f64 / scale);
        }
        clip_start_seconds += longest;
        let video_segment = clip_plans
            .iter()
            .find(|p| {
                source
                    .streams
                    .get(p.stream)
                    .is_some_and(|s| s.kind == StreamKind::Video)
            })
            .map_or(0, |p| p.segment);
        movie_millis = movie_millis.saturating_add(i64::try_from(video_segment).unwrap_or(0));
    }
    let mut ids = Vec::new();
    for (number, p) in first_plan.iter().enumerate() {
        let stream = first_source
            .streams
            .get(p.stream)
            .ok_or(ExportError::Damaged)?;
        let mut config = TrackConfig::new(stream.entry.clone(), stream.timescale);
        config.edits = edits
            .get_mut(number)
            .map(std::mem::take)
            .unwrap_or_default();
        ids.push(
            mp4.add_track(config)
                .map_err(|_| ExportError::Unsupported("the codec cannot be stored in an MP4"))?,
        );
    }
    chosen.sort_by(|a, b| {
        a.time_seconds
            .total_cmp(&b.time_seconds)
            .then(a.clip.cmp(&b.clip))
            .then(a.plan.cmp(&b.plan))
            .then(a.index.cmp(&b.index))
    });
    let mut done = 0_u64;
    let mut pictures = 0_u64;
    for item in &chosen {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        let source = sources.get(item.clip).ok_or(ExportError::Damaged)?;
        let stream_number = plans
            .get(item.clip)
            .and_then(|clip_plans| clip_plans.get(item.plan))
            .ok_or(ExportError::Damaged)?
            .stream;
        let stream = source
            .streams
            .get(stream_number)
            .ok_or(ExportError::Damaged)?;
        let sample = stream.samples.get(item.index).ok_or(ExportError::Damaged)?;
        let data = source.read(stream_number, item.index)?;
        let offset = i32::try_from(sample.pts - sample.dts).map_err(|_| ExportError::Damaged)?;
        mp4.write_sample(
            *ids.get(item.plan).ok_or(ExportError::Damaged)?,
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
    out.flush().map_err(|e| io_error(&e))?;
    out.get_ref().sync_all().map_err(|e| io_error(&e))?;
    let audio = first_plan
        .iter()
        .filter(|p| {
            first_source
                .streams
                .get(p.stream)
                .is_some_and(|s| s.kind == StreamKind::Audio)
        })
        .count();
    Ok((pictures, audio, movie_millis))
}
