//! Local media file probing for ClipCraft.
//!
//! [`probe_file`] reads only a clip's container headers (never its picture or sound data) and
//! reports duration, size, frame rate and codecs. MP4 and MOV go through FilmCraft's clean-room
//! ISO BMFF demuxer. Every number comes from an untrusted file, so header sizes are capped before
//! the demuxer allocates and all arithmetic is checked.

use clipcraft_time::{FrameRate, Tick};
use filmcraft_isobmff::{ByteSource, Mp4File, Track, TrackKind};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::File;
use std::path::Path;

/// Largest `moov`/`moof` header box that will be handed to the demuxer.
pub const MAX_HEADER_BOX_BYTES: u64 = 128 * 1024 * 1024;
/// Most top-level boxes inspected while checking header sizes.
const MAX_TOP_LEVEL_BOXES: usize = 4_096;

/// What a probe learned about a clip without decoding it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaProbe {
    /// `mp4` or `mov`.
    pub container: String,
    /// Total duration, or `None` when the file does not state one.
    pub duration: Option<Tick>,
    /// Picture width in pixels of the first video track, when there is one.
    pub width: Option<u32>,
    /// Picture height in pixels of the first video track, when there is one.
    pub height: Option<u32>,
    /// Average frame rate of the first video track, snapped to a standard rate when close.
    pub frame_rate: Option<FrameRate>,
    /// Sample entry format of the first video track, such as `avc1`.
    pub video_codec: Option<String>,
    /// Number of audio tracks.
    pub audio_tracks: u32,
}

/// Why a file could not be probed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeError {
    /// The path is not a readable regular file.
    Unreadable,
    /// A header box is larger than ClipCraft will read.
    TooLarge,
    /// The file is not an MP4/MOV container, or its headers are damaged.
    Unsupported,
}

impl fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable => formatter.write_str("The file could not be read."),
            Self::TooLarge => {
                formatter.write_str("The file's headers are too large to read safely.")
            }
            Self::Unsupported => {
                formatter.write_str("This file is not a readable MP4 or MOV video.")
            }
        }
    }
}

impl std::error::Error for ProbeError {}

/// Probes the MP4/MOV file at `path`.
pub fn probe_file(path: &Path) -> Result<MediaProbe, ProbeError> {
    let file = File::open(path).map_err(|_| ProbeError::Unreadable)?;
    let metadata = file.metadata().map_err(|_| ProbeError::Unreadable)?;
    if !metadata.is_file() {
        return Err(ProbeError::Unreadable);
    }
    probe_source(file)
}

/// Probes an in-memory or file-backed container.
pub fn probe_source<S: ByteSource>(source: S) -> Result<MediaProbe, ProbeError> {
    check_header_sizes(&source)?;
    let mp4 = filmcraft_isobmff::open(&source).map_err(|error| match error {
        filmcraft_isobmff::Error::Io(_) => ProbeError::Unreadable,
        filmcraft_isobmff::Error::TooLarge(_) => ProbeError::TooLarge,
        _ => ProbeError::Unsupported,
    })?;
    Ok(summarize(&mp4))
}

/// Walks top-level box headers and refuses oversized `moov`/`moof` boxes before the demuxer
/// allocates for them. Truncated or odd headers are left for the demuxer to reject.
fn check_header_sizes<S: ByteSource>(source: &S) -> Result<(), ProbeError> {
    let length = source.len();
    let mut offset = 0_u64;
    for _ in 0..MAX_TOP_LEVEL_BOXES {
        let Some(remaining) = length.checked_sub(offset).filter(|left| *left >= 8) else {
            return Ok(());
        };
        let mut header = [0_u8; 16];
        let wanted = usize::try_from(remaining.min(16)).unwrap_or(16);
        let Some(buffer) = header.get_mut(..wanted) else {
            return Ok(());
        };
        source
            .read_at(offset, buffer)
            .map_err(|_| ProbeError::Unreadable)?;
        let declared = u64::from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]));
        let size = match declared {
            0 => remaining,
            1 if wanted >= 16 => u64::from_be_bytes([
                header[8], header[9], header[10], header[11], header[12], header[13], header[14],
                header[15],
            ]),
            1 => return Ok(()),
            other => other,
        };
        let kind = [header[4], header[5], header[6], header[7]];
        if (&kind == b"moov" || &kind == b"moof") && size > MAX_HEADER_BOX_BYTES {
            return Err(ProbeError::TooLarge);
        }
        if size < 8 {
            return Ok(());
        }
        offset = match offset.checked_add(size) {
            Some(next) => next,
            None => return Ok(()),
        };
    }
    Ok(())
}

fn summarize(mp4: &Mp4File) -> MediaProbe {
    let video = mp4
        .tracks
        .iter()
        .find(|track| track.kind == TrackKind::Video);
    let entry = video.and_then(|track| track.entries.first());
    let picture = entry.and_then(|entry| entry.video.as_ref());
    let width = picture
        .map(|params| u32::from(params.width))
        .or_else(|| video.map(|track| track.width))
        .filter(|value| *value > 0);
    let height = picture
        .map(|params| u32::from(params.height))
        .or_else(|| video.map(|track| track.height))
        .filter(|value| *value > 0);

    MediaProbe {
        container: if mp4.is_quicktime { "mov" } else { "mp4" }.to_owned(),
        duration: movie_duration(mp4),
        width,
        height,
        frame_rate: video.and_then(average_frame_rate),
        video_codec: entry.map(|entry| String::from_utf8_lossy(&entry.format.0).into_owned()),
        audio_tracks: u32::try_from(
            mp4.tracks
                .iter()
                .filter(|track| track.kind == TrackKind::Audio)
                .count(),
        )
        .unwrap_or(u32::MAX),
    }
}

fn movie_duration(mp4: &Mp4File) -> Option<Tick> {
    if mp4.timescale == 0 {
        return None;
    }
    let units = i64::try_from(mp4.duration).ok()?;
    Some(Tick::from_rational(units, 1, i64::from(mp4.timescale)))
}

fn average_frame_rate(track: &Track) -> Option<FrameRate> {
    let frames = i64::try_from(track.samples.len()).ok().filter(|n| *n > 1)?;
    let duration = i64::try_from(track.duration).ok().filter(|d| *d > 0)?;
    if track.timescale == 0 {
        return None;
    }
    let fps = (frames as f64) * f64::from(track.timescale) / (duration as f64);
    (fps.is_finite() && fps > 0.0 && fps < 10_000.0).then(|| FrameRate::from_f64(fps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_isobmff::{
        AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions,
    };
    use std::io::Cursor;

    /// Writes a small MP4: `frames` video samples at `fps_num/fps_den`, plus optional audio.
    fn make_mp4(
        frames: u32,
        fps_num: u32,
        fps_den: u32,
        with_audio: bool,
        brand: Brand,
    ) -> Vec<u8> {
        let mut writer =
            Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(brand)).expect("writer");
        let video_entry = SampleEntry::avc(
            AvcConfig::new(
                vec![vec![0x67, 0x64, 0, 0x28, 1, 2]],
                vec![vec![0x68, 3]],
                4,
            ),
            1920,
            1080,
        );
        let video = writer
            .add_track(TrackConfig::new(video_entry, fps_num))
            .expect("video track");
        let audio = with_audio.then(|| {
            writer
                .add_track(TrackConfig::new(
                    SampleEntry::aac(vec![0x11, 0x90], 2, 48_000),
                    48_000,
                ))
                .expect("audio track")
        });
        for _ in 0..frames {
            writer
                .write_sample(
                    video,
                    WriteSample {
                        data: &[0, 0, 0, 2, 0x65, 0],
                        duration: fps_den,
                        composition_offset: 0,
                        is_sync: true,
                    },
                )
                .expect("video sample");
        }
        if let Some(audio) = audio {
            writer
                .write_sample(
                    audio,
                    WriteSample {
                        data: &[1, 2, 3],
                        duration: 1_024,
                        composition_offset: 0,
                        is_sync: true,
                    },
                )
                .expect("audio sample");
        }
        writer.finish().expect("finish").into_inner()
    }

    #[test]
    fn probes_duration_size_rate_and_codecs() {
        let bytes = make_mp4(120, 60_000, 1_001, true, Brand::Mp4);
        let probe = probe_source(bytes).expect("probe");

        assert_eq!(probe.container, "mp4");
        assert_eq!((probe.width, probe.height), (Some(1920), Some(1080)));
        assert_eq!(probe.frame_rate, Some(FrameRate::FPS_59_94));
        assert_eq!(probe.video_codec.as_deref(), Some("avc1"));
        assert_eq!(probe.audio_tracks, 1);
        // 120 frames at 59.94 fps is 2.002 s; the movie header is in milliseconds.
        let seconds = probe.duration.expect("duration").seconds();
        assert!((seconds - 2.002).abs() < 0.002, "duration was {seconds}");
    }

    #[test]
    fn quicktime_container_is_reported() {
        let probe = probe_source(make_mp4(30, 30, 1, false, Brand::Mov)).expect("probe");
        assert_eq!(probe.container, "mov");
        assert_eq!(probe.frame_rate, Some(FrameRate::FPS_30));
        assert_eq!(probe.audio_tracks, 0);
    }

    #[test]
    fn garbage_empty_and_truncated_files_are_unsupported_not_panics() {
        assert_eq!(probe_source(Vec::<u8>::new()), Err(ProbeError::Unsupported));
        assert_eq!(probe_source(vec![0_u8; 3]), Err(ProbeError::Unsupported));
        assert_eq!(
            probe_source(b"this is not a video file at all".to_vec()),
            Err(ProbeError::Unsupported)
        );
        let good = make_mp4(30, 30, 1, true, Brand::Mp4);
        for cut in [8, 16, good.len() / 2, good.len() - 1] {
            // Truncation may or may not leave a parsable header; it must never panic.
            let _ = probe_source(good[..cut].to_vec());
        }
    }

    #[test]
    fn every_single_byte_corruption_of_a_valid_header_is_handled() {
        let good = make_mp4(10, 25, 1, true, Brand::Mp4);
        for position in 0..good.len() {
            let mut damaged = good.clone();
            damaged[position] ^= 0xFF;
            let _ = probe_source(damaged);
        }
    }

    #[test]
    fn oversized_header_boxes_are_refused_before_allocation() {
        // A `moov` box that claims 2 GiB in a 16-byte file.
        let mut hostile = Vec::new();
        hostile.extend_from_slice(&0x8000_0000_u32.to_be_bytes());
        hostile.extend_from_slice(b"moov");
        hostile.extend_from_slice(&[0; 8]);
        assert_eq!(probe_source(hostile), Err(ProbeError::TooLarge));

        // The same claim through a 64-bit extended size.
        let mut extended = Vec::new();
        extended.extend_from_slice(&1_u32.to_be_bytes());
        extended.extend_from_slice(b"moov");
        extended.extend_from_slice(&(u64::MAX / 2).to_be_bytes());
        assert_eq!(probe_source(extended), Err(ProbeError::TooLarge));
    }

    #[test]
    fn missing_and_non_file_paths_are_unreadable() {
        assert_eq!(
            probe_file(Path::new("/definitely/not/here.mp4")),
            Err(ProbeError::Unreadable)
        );
        assert_eq!(
            probe_file(&std::env::temp_dir()),
            Err(ProbeError::Unreadable)
        );
    }
}
