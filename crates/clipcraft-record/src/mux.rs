//! Writes buffered pictures to an MP4 or MKV file, atomically.

use std::fs::{self, File};
use std::io::{BufWriter, ErrorKind};
use std::path::{Path, PathBuf};

use filmcraft_isobmff::{
    AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions,
};
use filmcraft_matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};

use crate::RecordError;
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

    if let Err(error) = write_partial(samples, video, format, &partial) {
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
    })
}

fn write_partial(
    samples: &[EncodedSample],
    video: &VideoInfo,
    format: ClipFormat,
    partial: &Path,
) -> Result<(), RecordError> {
    let file = File::create(partial).map_err(|error| io_error(&error))?;
    let writer = BufWriter::new(file);
    let origin = samples.first().map_or(0, |sample| sample.pts);
    match format {
        ClipFormat::Mp4 => {
            let avc = AvcConfig::parse(&video.avcc).map_err(|_| RecordError::BadStream)?;
            let mut mp4 = Mp4Writer::new(writer, WriterOptions::new(Brand::Mp4))
                .map_err(|_| RecordError::WriteFailed)?;
            let entry = SampleEntry::avc(
                avc,
                u16::try_from(video.width).map_err(|_| RecordError::BadStream)?,
                u16::try_from(video.height).map_err(|_| RecordError::BadStream)?,
            );
            let track = mp4
                .add_track(TrackConfig::new(entry, video.fps))
                .map_err(|_| RecordError::WriteFailed)?;
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
                .map_err(|_| RecordError::WriteFailed)?;
            }
            let mut out = mp4.finish().map_err(|_| RecordError::WriteFailed)?;
            std::io::Write::flush(&mut out).map_err(|error| io_error(&error))?;
            out.get_ref().sync_all().map_err(|error| io_error(&error))?;
        }
        ClipFormat::Mkv => {
            let mut spec = TrackSpec::new(TrackKind::Video, "V_MPEG4/ISO/AVC");
            spec.codec_private = video.avcc.clone();
            spec.video_size = Some((video.width, video.height));
            spec.default_duration_ns = Some(1_000_000_000 / u64::from(video.fps));
            let options = MuxOptions {
                doc_type: "matroska".to_owned(),
                writing_app: "ClipCraft".to_owned(),
                ..MuxOptions::default()
            };
            let mut mkv = MkvWriter::new(writer, vec![spec], options)
                .map_err(|_| RecordError::WriteFailed)?;
            let fps = i128::from(video.fps);
            let to_ns = |frames: i64| -> i64 {
                i64::try_from(i128::from(frames) * 1_000_000_000 / fps).unwrap_or(i64::MAX)
            };
            for (index, sample) in samples.iter().enumerate() {
                let duration = samples
                    .get(index + 1)
                    .map(|next| to_ns(next.pts - sample.pts))
                    .or_else(|| Some(to_ns(1)))
                    .and_then(|ns| u64::try_from(ns).ok());
                mkv.write_frame(
                    0,
                    to_ns(sample.pts - origin),
                    sample.keyframe,
                    &sample.data,
                    duration,
                )
                .map_err(|_| RecordError::WriteFailed)?;
            }
            let mut out = mkv.finish().map_err(|_| RecordError::WriteFailed)?;
            std::io::Write::flush(&mut out).map_err(|error| io_error(&error))?;
            out.get_ref().sync_all().map_err(|error| io_error(&error))?;
        }
    }
    Ok(())
}
