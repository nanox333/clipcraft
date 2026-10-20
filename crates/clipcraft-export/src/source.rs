//! The input clip as a list of streams of timed samples, whatever container it came in.

use std::fs::File;
use std::path::Path;

use clipcraft_media::ProbeError;
use filmcraft_isobmff::{AvcConfig, Mp4File, SampleEntry, TrackKind};
use filmcraft_matroska::{MkvFile, TrackKind as MkvKind};

use crate::ExportError;

/// Most samples accepted across all streams, so a hostile file cannot make the plan allocate
/// without bound.
const MAX_SAMPLES: usize = 4_000_000;
/// Timescale given to pictures from Matroska, which stamps in nanoseconds.
const MATROSKA_VIDEO_TIMESCALE: u32 = 90_000;
const EBML_MAGIC: [u8; 4] = [0x1a, 0x45, 0xdf, 0xa3];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StreamKind {
    Video,
    Audio,
}

/// One compressed picture or unit of sound.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Item {
    /// Decode time in the stream's timescale.
    pub dts: i64,
    /// Presentation time before the stream's offset is applied.
    pub pts: i64,
    /// Length, or 0 when the file does not say.
    pub duration: u32,
    pub sync: bool,
    pub size: u32,
}

pub(crate) struct Stream {
    pub kind: StreamKind,
    pub entry: SampleEntry,
    pub timescale: u32,
    /// Added to `pts` to get the time the sample is shown at (the container's edit list).
    pub offset: i64,
    pub samples: Vec<Item>,
    /// Which track of the container this is.
    track: usize,
}

impl Stream {
    /// The length of sample `index`, worked out from its neighbours when the file leaves it out.
    pub fn duration_of(&self, index: usize) -> u32 {
        let sample = &self.samples[index];
        if sample.duration > 0 {
            return sample.duration;
        }
        let gap = |a: usize, b: usize| {
            u32::try_from(self.samples[b].dts - self.samples[a].dts)
                .ok()
                .filter(|gap| *gap > 0)
        };
        if index + 1 < self.samples.len()
            && let Some(gap) = gap(index, index + 1)
        {
            return gap;
        }
        if index > 0
            && let Some(gap) = gap(index - 1, index)
        {
            return gap;
        }
        1
    }
}

enum Backend {
    Mp4(Mp4File),
    Matroska(Box<MkvFile>),
}

pub(crate) struct Source {
    pub streams: Vec<Stream>,
    backend: Backend,
    file: File,
}

fn probe_error(error: ProbeError) -> ExportError {
    match error {
        ProbeError::Unreadable => ExportError::Unreadable,
        ProbeError::TooLarge => ExportError::Damaged,
        ProbeError::Unsupported => {
            ExportError::Unsupported("it is not an MP4, MOV, MKV or WebM video")
        }
    }
}

impl Source {
    pub fn open(path: &Path) -> Result<Self, ExportError> {
        // The probe refuses oversized or damaged headers before the demuxers allocate for them.
        clipcraft_media::probe_file(path).map_err(probe_error)?;
        let file = File::open(path).map_err(|_| ExportError::Unreadable)?;
        let mut magic = [0_u8; 4];
        let matroska = filmcraft_isobmff::ByteSource::read_at(&file, 0, &mut magic).is_ok()
            && magic == EBML_MAGIC;
        let (streams, backend) = if matroska {
            let mkv = filmcraft_matroska::open(&file).map_err(|_| ExportError::Damaged)?;
            (matroska_streams(&mkv)?, Backend::Matroska(Box::new(mkv)))
        } else {
            let mp4 = filmcraft_isobmff::open(&file).map_err(|_| ExportError::Damaged)?;
            (mp4_streams(&mp4)?, Backend::Mp4(mp4))
        };
        let total: usize = streams.iter().map(|stream| stream.samples.len()).sum();
        if total > MAX_SAMPLES {
            return Err(ExportError::Unsupported(
                "the clip has too many pictures and sounds",
            ));
        }
        Ok(Self {
            streams,
            backend,
            file,
        })
    }

    pub fn read(&self, stream: usize, index: usize) -> Result<Vec<u8>, ExportError> {
        let track = self.streams.get(stream).ok_or(ExportError::Damaged)?.track;
        match &self.backend {
            Backend::Mp4(mp4) => mp4.read_sample(&self.file, track, index),
            Backend::Matroska(mkv) => mkv
                .read_sample(&self.file, track, index)
                .map_err(|_| filmcraft_isobmff::Error::Truncated("sample")),
        }
        .map_err(|_| ExportError::Damaged)
    }
}

fn mp4_streams(mp4: &Mp4File) -> Result<Vec<Stream>, ExportError> {
    let mut streams = Vec::new();
    for (track_index, track) in mp4.tracks.iter().enumerate() {
        let kind = match track.kind {
            TrackKind::Video => StreamKind::Video,
            TrackKind::Audio => StreamKind::Audio,
            _ => continue,
        };
        let Some(entry) = track.entries.first().cloned() else {
            continue;
        };
        if track.timescale == 0 || track.pcm_chunked {
            continue;
        }
        let samples = track
            .samples
            .iter()
            .map(|sample| Item {
                dts: sample.dts,
                pts: sample.pts,
                duration: sample.duration,
                sync: sample.is_sync,
                size: sample.size,
            })
            .collect();
        streams.push(Stream {
            kind,
            entry,
            timescale: track.timescale,
            offset: track.edit_offset,
            samples,
            track: track_index,
        });
    }
    if !streams
        .iter()
        .any(|stream| stream.kind == StreamKind::Video)
    {
        return Err(ExportError::Unsupported("the clip has no video"));
    }
    Ok(streams)
}

fn matroska_streams(mkv: &MkvFile) -> Result<Vec<Stream>, ExportError> {
    let mut streams = Vec::new();
    for (track_index, track) in mkv.tracks.iter().enumerate() {
        let (kind, entry, timescale) = match track.kind {
            MkvKind::Video if streams.iter().all(|s: &Stream| s.kind != StreamKind::Video) => {
                if track.codec_id != "V_MPEG4/ISO/AVC" {
                    return Err(ExportError::Unsupported(
                        "only H.264 video in Matroska can be cut",
                    ));
                }
                let video = track.video.as_ref().ok_or(ExportError::Damaged)?;
                let avc =
                    AvcConfig::parse(&track.codec_private).map_err(|_| ExportError::Damaged)?;
                let width = u16::try_from(video.pixel_width).map_err(|_| ExportError::Damaged)?;
                let height = u16::try_from(video.pixel_height).map_err(|_| ExportError::Damaged)?;
                (
                    StreamKind::Video,
                    SampleEntry::avc(avc, width, height),
                    MATROSKA_VIDEO_TIMESCALE,
                )
            }
            MkvKind::Audio if track.codec_id == "A_AAC" => {
                let audio = track.audio.as_ref().ok_or(ExportError::Damaged)?;
                let rate = audio.sampling_frequency.round();
                if !(8_000.0..=96_000.0).contains(&rate) {
                    return Err(ExportError::Damaged);
                }
                let rate = rate as u32;
                let channels = u32::try_from(audio.channels).map_err(|_| ExportError::Damaged)?;
                (
                    StreamKind::Audio,
                    SampleEntry::aac(track.codec_private.clone(), channels, rate),
                    rate,
                )
            }
            _ => continue,
        };
        let to_scale = |ticks: i64| -> i64 {
            let nanos = i128::from(track.ticks_to_ns(ticks));
            i64::try_from((nanos * i128::from(timescale) + 500_000_000).div_euclid(1_000_000_000))
                .unwrap_or(0)
        };
        let mut samples = Vec::with_capacity(track.samples.len());
        let mut previous = i64::MIN;
        for sample in &track.samples {
            let time = to_scale(sample.pts);
            if time < previous {
                return Err(ExportError::Unsupported(
                    "pictures out of order (B-frames) in a Matroska file",
                ));
            }
            previous = time;
            let duration = to_scale(i64::try_from(sample.duration).unwrap_or(0));
            samples.push(Item {
                dts: time,
                pts: time,
                duration: u32::try_from(duration).unwrap_or(0),
                sync: sample.keyframe,
                size: sample.size,
            });
        }
        streams.push(Stream {
            kind,
            entry,
            timescale,
            offset: 0,
            samples,
            track: track_index,
        });
    }
    if !streams
        .iter()
        .any(|stream| stream.kind == StreamKind::Video)
    {
        return Err(ExportError::Unsupported("the clip has no video"));
    }
    Ok(streams)
}
