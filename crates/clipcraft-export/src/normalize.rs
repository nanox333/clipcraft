//! Evens out a clip's loudness. The picture is copied untouched; the sound is decoded, scaled by
//! one gain chosen so the whole programme reaches the target loudness (ITU-R BS.1770 integrated
//! loudness, gated), and encoded again as AAC. All tracks get the same gain, measured on their
//! sum, so the balance between game sound and microphone is kept. A gain that would push the
//! true peak above -1 dBTP is held back, so the result never clips; a quiet clip with loud peaks
//! may therefore end up below the target.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use clipcraft_time::Tick;
use filmcraft_aac::{Decoder, Encoder, EncoderConfig};
use filmcraft_audio_dsp::loudness::LoudnessMeter;
use filmcraft_isobmff::{
    Brand, CodecConfig, Edit, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions,
};

use crate::compress::{Temporary, check_cancel};
use crate::source::{Source, StreamKind};
use crate::{ExportError, ExportReport, Plan, TrimRequest, io_error, mp4_error, plan};

/// Samples in one AAC access unit.
const UNIT: i64 = 1_024;
/// The target range accepted, in LUFS.
pub const LOUDNESS_TARGETS: std::ops::RangeInclusive<f64> = -30.0..=-8.0;
/// The loudest true peak a result may have, in dBTP.
const PEAK_CEILING_DB: f64 = -1.0;
/// The largest change, either way, in dB.
const MAX_GAIN_DB: f64 = 30.0;
/// Most sound tracks adjusted together.
const MAX_TRACKS: usize = 8;

/// What was measured and applied.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizeReport {
    /// The new clip.
    pub clip: ExportReport,
    /// Integrated loudness of the range before the change, in LUFS.
    pub measured_lufs: f64,
    /// The gain applied to every sound track, in dB.
    pub gain_db: f64,
    /// True when the gain was held back to keep the peaks below clipping.
    pub peak_limited: bool,
}

/// Writes `request.start..request.end` with its sound brought to `target_lufs` as a new MP4.
///
/// The picture is copied, not re-encoded. The input is only read, an existing output is never
/// replaced, and a cancelled or failed export leaves no file behind.
pub fn normalize_to_mp4(
    request: &TrimRequest,
    target_lufs: f64,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<NormalizeReport, ExportError> {
    if !LOUDNESS_TARGETS.contains(&target_lufs) {
        return Err(ExportError::BadPreset);
    }
    if request.start < Tick::ZERO || request.end <= request.start {
        return Err(ExportError::BadRange);
    }
    if fs::symlink_metadata(&request.output).is_ok() {
        return Err(ExportError::AlreadyExists);
    }
    check_cancel(cancel)?;
    catch_unwind(AssertUnwindSafe(|| {
        normalize_inner(request, target_lufs, progress, cancel)
    }))
    .unwrap_or(Err(ExportError::EncodeFailed))
}

/// One sound track's facts, from its source stream.
struct Track<'a> {
    plan: &'a Plan,
    asc: Vec<u8>,
}

fn aac_tracks<'a>(
    source: &Source,
    plans: &'a [Plan],
) -> Result<(Vec<Track<'a>>, usize, u32), ExportError> {
    let mut tracks = Vec::new();
    let mut layout: Option<(usize, u32)> = None;
    for p in plans {
        let stream = source.streams.get(p.stream).ok_or(ExportError::Damaged)?;
        if stream.kind != StreamKind::Audio {
            continue;
        }
        let CodecConfig::Aac(aac) = &stream.entry.codec else {
            return Err(ExportError::Unsupported(
                "only AAC sound can have its loudness adjusted",
            ));
        };
        let audio = stream.entry.audio.as_ref().ok_or(ExportError::Damaged)?;
        let channels = usize::try_from(audio.channels).map_err(|_| ExportError::Damaged)?;
        let found = (channels, stream.timescale);
        if !(1..=2).contains(&channels) || *layout.get_or_insert(found) != found {
            return Err(ExportError::Unsupported(
                "the sound tracks differ in channels or sample rate",
            ));
        }
        tracks.push(Track {
            plan: p,
            asc: aac.asc.clone(),
        });
    }
    let (channels, rate) = layout.ok_or(ExportError::Unsupported("the clip has no sound"))?;
    if tracks.len() > MAX_TRACKS {
        return Err(ExportError::Unsupported(
            "the clip has too many sound tracks",
        ));
    }
    Ok((tracks, channels, rate))
}

fn decoder_for(track: &Track<'_>) -> Result<Decoder, ExportError> {
    Decoder::new(&track.asc).map_err(|_| ExportError::Unsupported("this AAC sound cannot be read"))
}

fn normalize_inner(
    request: &TrimRequest,
    target_lufs: f64,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<NormalizeReport, ExportError> {
    let source = Source::open(&request.input)?;
    let plans = plan(&source, request.start, request.end)?;
    let (tracks, channels, rate) = aac_tracks(&source, &plans)?;

    // Pass 1: measure the sum of every track, a unit at a time.
    let mut decoders = tracks
        .iter()
        .map(decoder_for)
        .collect::<Result<Vec<_>, _>>()?;
    let longest = tracks
        .iter()
        .map(|t| t.plan.last - t.plan.first + 1)
        .max()
        .unwrap_or(0);
    let mut meter = LoudnessMeter::new(f64::from(rate), channels);
    for unit in 0..longest {
        check_cancel(cancel)?;
        let mut mix = vec![vec![0.0_f32; UNIT as usize]; channels];
        for (track, decoder) in tracks.iter().zip(&mut decoders) {
            let index = track.plan.first + unit;
            if index > track.plan.last {
                continue;
            }
            let data = source.read(track.plan.stream, index)?;
            let planar = decoder.decode(&data).map_err(|_| ExportError::Damaged)?;
            for (out, channel) in mix.iter_mut().zip(&planar) {
                for (sum, sample) in out.iter_mut().zip(channel) {
                    *sum += *sample;
                }
            }
        }
        let refs: Vec<&[f32]> = mix.iter().map(Vec::as_slice).collect();
        meter.process(&refs);
        progress(0.3 * (unit + 1) as f32 / longest as f32);
    }
    let measured = meter.integrated();
    if !measured.is_finite() {
        return Err(ExportError::Unsupported(
            "the sound is too quiet to measure",
        ));
    }
    let wanted = (target_lufs - measured).clamp(-MAX_GAIN_DB, MAX_GAIN_DB);
    let room = PEAK_CEILING_DB - meter.true_peak_dbtp();
    let (gain_db, peak_limited) = if wanted > room {
        (room.max(-MAX_GAIN_DB), true)
    } else {
        (wanted, false)
    };
    let gain = 10.0_f64.powf(gain_db / 20.0) as f32;

    // Pass 2: scale, encode, and keep the compressed units (small) until the file is written.
    let bits = if channels == 1 { 96_000 } else { 160_000 };
    let mut encoded: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut config_bytes: Vec<Vec<u8>> = Vec::new();
    let mut priming = 0_i64;
    for (number, track) in tracks.iter().enumerate() {
        let mut decoder = decoder_for(track)?;
        let mut encoder = Encoder::new(EncoderConfig::cbr(rate, channels, bits))
            .map_err(|_| ExportError::EncodeFailed)?;
        priming = i64::from(encoder.priming_samples());
        config_bytes.push(encoder.audio_specific_config());
        let mut units = Vec::new();
        for index in track.plan.first..=track.plan.last {
            check_cancel(cancel)?;
            let data = source.read(track.plan.stream, index)?;
            let mut planar = decoder.decode(&data).map_err(|_| ExportError::Damaged)?;
            for channel in &mut planar {
                for sample in channel.iter_mut() {
                    *sample = (*sample * gain).clamp(-1.0, 1.0);
                }
            }
            let refs: Vec<&[f32]> = planar.iter().map(Vec::as_slice).collect();
            units.extend(encoder.encode(&refs));
            progress(
                0.3 + 0.3
                    * (number as f32
                        + (index - track.plan.first + 1) as f32
                            / (track.plan.last - track.plan.first + 1) as f32)
                    / tracks.len() as f32,
            );
        }
        units.extend(encoder.flush());
        encoded.push(units);
    }

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
    let pictures = write_file(
        &source,
        &plans,
        &tracks,
        &encoded,
        &config_bytes,
        (channels, rate, priming),
        file,
        progress,
        cancel,
    )?;
    check_cancel(cancel)?;
    crate::compress::publish_new(&temporary.0, &request.output)?;
    let bytes = fs::metadata(&request.output).map_or(0, |meta| meta.len());
    let video_span = plans
        .iter()
        .find(|p| {
            source
                .streams
                .get(p.stream)
                .is_some_and(|s| s.kind == StreamKind::Video)
        })
        .map_or(0, |p| p.segment);
    progress(1.0);
    Ok(NormalizeReport {
        clip: ExportReport {
            path: request.output.clone(),
            duration: Tick::from_units(i64::try_from(video_span).unwrap_or(0), 1_000),
            pictures,
            audio_tracks: tracks.len(),
            bytes,
        },
        measured_lufs: measured,
        gain_db,
        peak_limited,
    })
}

enum Piece {
    Original { stream: usize, index: usize },
    Encoded { track: usize, unit: usize },
}

struct Timed {
    seconds: f64,
    order: usize,
    piece: Piece,
}

#[allow(clippy::too_many_arguments)]
fn write_file(
    source: &Source,
    plans: &[Plan],
    tracks: &[Track<'_>],
    encoded: &[Vec<Vec<u8>>],
    configs: &[Vec<u8>],
    (channels, rate, priming): (usize, u32, i64),
    file: File,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<u64, ExportError> {
    let mut mp4 =
        Mp4Writer::new(BufWriter::new(file), WriterOptions::new(Brand::Mp4)).map_err(mp4_error)?;
    let mut ids = Vec::new();
    let mut items: Vec<Timed> = Vec::new();
    let mut audio_number = 0_usize;
    let mut total = 0_u64;
    for (order, p) in plans.iter().enumerate() {
        let stream = source.streams.get(p.stream).ok_or(ExportError::Damaged)?;
        let video = stream.kind == StreamKind::Video;
        let (entry, scale, media_time) = if video {
            (stream.entry.clone(), stream.timescale, p.media_time)
        } else {
            let asc = configs
                .get(audio_number)
                .ok_or(ExportError::Damaged)?
                .clone();
            (
                SampleEntry::aac(
                    asc,
                    u32::try_from(channels).map_err(|_| ExportError::Damaged)?,
                    rate,
                ),
                rate,
                p.media_time + priming,
            )
        };
        let mut config = TrackConfig::new(entry, scale);
        config.edits = vec![Edit {
            segment_duration: p.segment,
            media_time,
            media_rate: 0x0001_0000,
        }];
        ids.push(
            mp4.add_track(config)
                .map_err(|_| ExportError::Unsupported("the codec cannot be stored in an MP4"))?,
        );
        if video {
            let base = stream.samples.get(p.first).ok_or(ExportError::Damaged)?.dts;
            for index in p.first..=p.last {
                let sample = stream.samples.get(index).ok_or(ExportError::Damaged)?;
                total += u64::from(sample.size);
                items.push(Timed {
                    seconds: (sample.dts - base) as f64 / f64::from(stream.timescale),
                    order,
                    piece: Piece::Original {
                        stream: p.stream,
                        index,
                    },
                });
            }
        } else {
            let units = encoded.get(audio_number).ok_or(ExportError::Damaged)?;
            for (unit, data) in units.iter().enumerate() {
                total += data.len() as u64;
                items.push(Timed {
                    seconds: unit as f64 * UNIT as f64 / f64::from(rate),
                    order,
                    piece: Piece::Encoded {
                        track: audio_number,
                        unit,
                    },
                });
            }
            audio_number += 1;
        }
    }
    debug_assert_eq!(audio_number, tracks.len());
    items.sort_by(|a, b| a.seconds.total_cmp(&b.seconds).then(a.order.cmp(&b.order)));
    let mut done = 0_u64;
    let mut pictures = 0_u64;
    for item in &items {
        if cancel.load(Ordering::Relaxed) {
            return Err(ExportError::Cancelled);
        }
        let id = *ids.get(item.order).ok_or(ExportError::Damaged)?;
        match item.piece {
            Piece::Original { stream, index } => {
                let s = source.streams.get(stream).ok_or(ExportError::Damaged)?;
                let sample = s.samples.get(index).ok_or(ExportError::Damaged)?;
                let data = source.read(stream, index)?;
                let offset =
                    i32::try_from(sample.pts - sample.dts).map_err(|_| ExportError::Damaged)?;
                mp4.write_sample(
                    id,
                    WriteSample {
                        data: &data,
                        duration: s.duration_of(index),
                        composition_offset: offset,
                        is_sync: sample.sync,
                    },
                )
                .map_err(mp4_error)?;
                pictures += 1;
                done += u64::from(sample.size);
            }
            Piece::Encoded { track, unit } => {
                let data = encoded
                    .get(track)
                    .and_then(|units| units.get(unit))
                    .ok_or(ExportError::Damaged)?;
                mp4.write_sample(
                    id,
                    WriteSample {
                        data,
                        duration: UNIT as u32,
                        composition_offset: 0,
                        is_sync: true,
                    },
                )
                .map_err(mp4_error)?;
                done += data.len() as u64;
            }
        }
        progress(0.6 + 0.39 * (done as f64 / total.max(1) as f64).min(1.0) as f32);
    }
    let mut out = mp4.finish().map_err(mp4_error)?;
    out.flush().map_err(|e| io_error(&e))?;
    out.get_ref().sync_all().map_err(|e| io_error(&e))?;
    Ok(pictures)
}
