//! Writes buffered pictures to an MP4 or MKV file, atomically.

use std::fs::{self, File};
use std::io::{BufWriter, ErrorKind};
use std::path::{Path, PathBuf};

use filmcraft_isobmff::{
    AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions,
};
use filmcraft_matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};

use crate::RecordError;
use crate::audio::{AAC_FRAME, AudioPacket, AudioTrackInfo};
use crate::replay::EncodedSample;

/// The file format of a saved clip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipFormat {
    /// MP4 with H.264.
    Mp4,
    /// Matroska with H.264.
    Mkv,
}

impl ClipFormat {
    /// The file extension, without a dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
        }
    }
}

/// What the encoder produced, needed to describe the stream in a file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoInfo {
    /// Picture width.
    pub width: u32,
    /// Picture height.
    pub height: u32,
    /// Frames per second; the unit of every sample time.
    pub fps: u32,
    /// The H.264 decoder configuration record (`avcC`).
    pub avcc: Vec<u8>,
}

/// One track of sound for a clip: how it is described, and its encoded units.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipAudio {
    /// The track's format and name.
    pub info: AudioTrackInfo,
    /// Units that cover the clip, starting with the one before it (see `AudioBuffer::snapshot`).
    pub packets: Vec<AudioPacket>,
}

/// A clip that was written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedClip {
    /// Where it is.
    pub path: PathBuf,
    /// Pictures in the clip.
    pub frames: u64,
    /// Length in frame units (`frames` plus any dropped frames).
    pub duration_frames: u64,
    /// File size in bytes.
    pub bytes: u64,
    /// Tracks of sound in the file.
    pub audio_tracks: usize,
    /// Time of the first picture, in frames since recording started.
    pub start_frame: i64,
}

/// A failure from the MP4 writer. A disk that filled up or a folder that cannot be written to
/// says so, instead of a vague write failure.
fn mp4_error(error: filmcraft_isobmff::Error) -> RecordError {
    match error {
        filmcraft_isobmff::Error::Io(error) => io_error(&error),
        _ => RecordError::WriteFailed,
    }
}

/// The same for the Matroska writer.
fn mkv_error(error: filmcraft_matroska::Error) -> RecordError {
    match error {
        filmcraft_matroska::Error::Io(error) => io_error(&error),
        _ => RecordError::WriteFailed,
    }
}

fn io_error(error: &std::io::Error) -> RecordError {
    match error.kind() {
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => RecordError::DiskFull,
        ErrorKind::PermissionDenied => RecordError::FolderNotWritable,
        _ => RecordError::WriteFailed,
    }
}

/// Writes `samples` to `path`. The file appears only when complete; an existing file is never
/// overwritten.
pub fn write_clip(
    samples: &[EncodedSample],
    video: &VideoInfo,
    audio: &[ClipAudio],
    format: ClipFormat,
    path: &Path,
) -> Result<SavedClip, RecordError> {
    let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
        return Err(RecordError::NothingBuffered);
    };
    if !first.keyframe || video.fps == 0 {
        return Err(RecordError::BadStream);
    }
    if path.exists() {
        return Err(RecordError::AlreadyExists);
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| io_error(&error))?;
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);

    if let Err(error) = write_partial(samples, video, audio, format, &partial) {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    if let Err(error) = fs::rename(&partial, path) {
        let _ = fs::remove_file(&partial);
        return Err(io_error(&error));
    }
    let bytes = fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    Ok(SavedClip {
        path: path.to_path_buf(),
        frames: samples.len() as u64,
        duration_frames: u64::try_from(last.pts - first.pts + 1).unwrap_or(0),
        bytes,
        audio_tracks: audio
            .iter()
            .filter(|track| !track.packets.is_empty())
            .count(),
        start_frame: first.pts,
    })
}

/// The input sample at which the clip's first picture is shown, and the one just after its last.
fn clip_span(samples: &[EncodedSample], video: &VideoInfo, rate: u32) -> (i64, i64) {
    let to_sample = |pts: i64| -> i64 {
        i64::try_from(i128::from(pts) * i128::from(rate) / i128::from(video.fps.max(1)))
            .unwrap_or(i64::MAX)
    };
    let first = samples.first().map_or(0, |sample| sample.pts);
    let last = samples.last().map_or(0, |sample| sample.pts);
    (to_sample(first), to_sample(last.saturating_add(1)))
}

fn write_partial(
    samples: &[EncodedSample],
    video: &VideoInfo,
    audio: &[ClipAudio],
    format: ClipFormat,
    partial: &Path,
) -> Result<(), RecordError> {
    let file = File::create(partial).map_err(|error| io_error(&error))?;
    let writer = BufWriter::new(file);
    let origin = samples.first().map_or(0, |sample| sample.pts);
    let audio: Vec<&ClipAudio> = audio
        .iter()
        .filter(|track| !track.packets.is_empty())
        .collect();
    match format {
        ClipFormat::Mp4 => {
            let avc = AvcConfig::parse(&video.avcc).map_err(|_| RecordError::BadStream)?;
            let mut mp4 =
                Mp4Writer::new(writer, WriterOptions::new(Brand::Mp4)).map_err(mp4_error)?;
            let entry = SampleEntry::avc(
                avc,
                u16::try_from(video.width).map_err(|_| RecordError::BadStream)?,
                u16::try_from(video.height).map_err(|_| RecordError::BadStream)?,
            );
            let track = mp4
                .add_track(TrackConfig::new(entry, video.fps))
                .map_err(mp4_error)?;
            // Each sound track: its own timescale (the sample rate) and an edit that skips the
            // decoder's warm-up unit and the part before the first picture.
            let mut sound = Vec::new();
            for clip_audio in &audio {
                let info = &clip_audio.info;
                let (from, until) = clip_span(samples, video, info.sample_rate);
                let units = trimmed(&clip_audio.packets, until);
                let Some(first) = units.first() else { continue };
                let mut config = TrackConfig::new(
                    SampleEntry::aac(
                        info.config.clone(),
                        u32::from(info.channels),
                        info.sample_rate,
                    ),
                    info.sample_rate,
                );
                config.media_start = Some((from - first.start).max(0));
                let id = mp4.add_track(config).map_err(mp4_error)?;
                sound.push((id, units, info.sample_rate, from));
            }
            // Pictures and sound in time order, so the file is interleaved.
            let mut cursors = vec![0_usize; sound.len()];
            let mut previous_duration = 1_u32;
            for (index, sample) in samples.iter().enumerate() {
                let duration = match samples.get(index + 1) {
                    Some(next) => u32::try_from(next.pts - sample.pts).unwrap_or(1).max(1),
                    None => previous_duration,
                };
                previous_duration = duration;
                mp4.write_sample(
                    track,
                    WriteSample {
                        data: &sample.data,
                        duration,
                        composition_offset: 0,
                        is_sync: sample.keyframe,
                    },
                )
                .map_err(mp4_error)?;
                let picture_end = (sample.pts - origin + i64::from(duration)) as f64
                    / f64::from(video.fps.max(1));
                for ((id, units, rate, from), cursor) in sound.iter().zip(&mut cursors) {
                    while let Some(unit) = units.get(*cursor) {
                        let unit_time = (unit.start - from) as f64 / f64::from(*rate);
                        if unit_time > picture_end && index + 1 < samples.len() {
                            break;
                        }
                        mp4.write_sample(
                            *id,
                            WriteSample {
                                data: &unit.data,
                                duration: AAC_FRAME as u32,
                                composition_offset: 0,
                                is_sync: true,
                            },
                        )
                        .map_err(mp4_error)?;
                        *cursor += 1;
                    }
                }
            }
            let mut out = mp4.finish().map_err(mp4_error)?;
            std::io::Write::flush(&mut out).map_err(|error| io_error(&error))?;
            out.get_ref().sync_all().map_err(|error| io_error(&error))?;
        }
        ClipFormat::Mkv => {
            let mut specs = Vec::new();
            let mut spec = TrackSpec::new(TrackKind::Video, "V_MPEG4/ISO/AVC");
            spec.codec_private = video.avcc.clone();
            spec.video_size = Some((video.width, video.height));
            spec.default_duration_ns = Some(1_000_000_000 / u64::from(video.fps));
            specs.push(spec);
            let mut sound = Vec::new();
            for clip_audio in &audio {
                let info = &clip_audio.info;
                let (from, until) = clip_span(samples, video, info.sample_rate);
                // Matroska stamps every block, so the warm-up unit is simply left out.
                let units: Vec<&AudioPacket> = trimmed(&clip_audio.packets, until)
                    .into_iter()
                    .filter(|unit| unit.start + AAC_FRAME > from)
                    .collect();
                if units.is_empty() {
                    continue;
                }
                let mut spec = TrackSpec::new(TrackKind::Audio, "A_AAC");
                spec.codec_private = info.config.clone();
                spec.audio = Some((f64::from(info.sample_rate), u64::from(info.channels), None));
                spec.name = Some(info.name.clone());
                specs.push(spec);
                sound.push((specs.len() - 1, units, info.sample_rate, from));
            }
            let options = MuxOptions {
                doc_type: "matroska".to_owned(),
                writing_app: "ClipCraft".to_owned(),
                ..MuxOptions::default()
            };
            let mut mkv = MkvWriter::new(writer, specs, options).map_err(mkv_error)?;
            let fps = i128::from(video.fps);
            let to_ns = |frames: i64| -> i64 {
                i64::try_from(i128::from(frames) * 1_000_000_000 / fps).unwrap_or(i64::MAX)
            };
            let sound_ns = |samples: i64, rate: u32| -> i64 {
                i64::try_from(i128::from(samples) * 1_000_000_000 / i128::from(rate.max(1)))
                    .unwrap_or(0)
            };
            // Every block with its time, merged into time order.
            let mut blocks: Vec<(i64, usize, usize, usize)> = Vec::new();
            for (index, sample) in samples.iter().enumerate() {
                blocks.push((to_ns(sample.pts - origin), 0, 0, index));
            }
            for (lane, (track, units, rate, from)) in sound.iter().enumerate() {
                for (index, unit) in units.iter().enumerate() {
                    let time = sound_ns((unit.start - from).max(0), *rate);
                    blocks.push((time, *track, lane, index));
                }
            }
            blocks.sort_by_key(|block| (block.0, block.1, block.3));
            for (time, track, lane, index) in blocks {
                if track == 0 {
                    let Some(sample) = samples.get(index) else {
                        continue;
                    };
                    let duration = samples
                        .get(index + 1)
                        .map(|next| to_ns(next.pts - sample.pts))
                        .or_else(|| Some(to_ns(1)))
                        .and_then(|ns| u64::try_from(ns).ok());
                    mkv.write_frame(0, time, sample.keyframe, &sample.data, duration)
                        .map_err(mkv_error)?;
                } else if let Some((_, units, rate, _)) = sound.get(lane) {
                    let Some(unit) = units.get(index) else {
                        continue;
                    };
                    let duration = u64::try_from(sound_ns(AAC_FRAME, *rate)).ok();
                    mkv.write_frame(track, time, true, &unit.data, duration)
                        .map_err(mkv_error)?;
                }
            }
            let mut out = mkv.finish().map_err(mkv_error)?;
            std::io::Write::flush(&mut out).map_err(|error| io_error(&error))?;
            out.get_ref().sync_all().map_err(|error| io_error(&error))?;
        }
    }
    Ok(())
}

/// The units that begin before sample `until`.
fn trimmed(packets: &[AudioPacket], until: i64) -> Vec<&AudioPacket> {
    packets.iter().filter(|unit| unit.start < until).collect()
}
