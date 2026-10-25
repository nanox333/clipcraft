//! Bounded filmstrips and real first-track audio envelopes for a local source range.
//!
//! Call this on a worker: video is sampled at evenly spaced source times, while sound is read
//! sequentially in small chunks. No whole-range PCM or full-resolution RGBA copy is retained.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use clipcraft_time::Tick;
use filmcraft_media::{FrameRequest, SharedSource};
use serde::{Deserialize, Serialize};

use crate::{PlayerError, clip_info, open_source};

#[path = "timeline_mp4.rs"]
mod mp4;

pub const TIMELINE_WIDTH: u32 = 160;
pub const TIMELINE_HEIGHT: u32 = 90;
pub const MAX_TIMELINE_FRAMES: u16 = 12;
pub const MAX_WAVEFORM_BUCKETS: u16 = 2048;
const AUDIO_RATE: u32 = 8_000;
const AUDIO_CHUNK: usize = 4096;
const MAX_SOURCE_PIXELS: u64 = 7680 * 4320;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineRequest {
    pub path: PathBuf,
    pub start: Tick,
    pub end: Tick,
    pub frames: u16,
    pub buckets: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineFrame {
    /// Exact source time requested, before the decoder resolves its containing video frame.
    pub position: Tick,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct WaveformBucket {
    /// Finite normalized amplitude, quantized to -32767..=32767.
    pub min: i16,
    pub max: i16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimelineAnalysis {
    pub frames: Vec<TimelineFrame>,
    pub waveform: Vec<WaveformBucket>,
    pub has_audio: bool,
}
impl TimelineAnalysis {
    /// Payload accounting for the caller's cache; scalar metadata is a small additional cost.
    pub fn bytes(&self) -> usize {
        self.frames
            .iter()
            .fold(0_usize, |sum, frame| sum.saturating_add(frame.rgba.len()))
            .saturating_add(self.waveform.len() * std::mem::size_of::<WaveformBucket>())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineError {
    InvalidRequest,
    Unreadable,
    Unsupported,
    DecodeFailed,
    Cancelled,
    Crashed,
}
impl From<PlayerError> for TimelineError {
    fn from(value: PlayerError) -> Self {
        match value {
            PlayerError::Unreadable => Self::Unreadable,
            PlayerError::Unsupported => Self::Unsupported,
            PlayerError::DecodeFailed => Self::DecodeFailed,
            PlayerError::Crashed => Self::Crashed,
        }
    }
}
impl fmt::Display for TimelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => "Choose a source range from 1/30 second to 30 minutes, 1–12 thumbnails and 16–2048 waveform points.",
            Self::Unreadable => "The timeline source could not be read. Locate the original clip and try again.",
            Self::Unsupported => "Timeline previews do not support this clip's format or size. Try an MP4 clip or a shorter selection.",
            Self::DecodeFailed => "The timeline preview could not be decoded. The original clip is unchanged.",
            Self::Cancelled => "Timeline preview generation was cancelled.",
            Self::Crashed => "Timeline preview generation stopped unexpectedly. The original clip is unchanged.",
        })
    }
}
impl std::error::Error for TimelineError {}

fn check_cancel(cancel: &AtomicBool) -> Result<(), TimelineError> {
    if cancel.load(Ordering::Relaxed) {
        Err(TimelineError::Cancelled)
    } else {
        Ok(())
    }
}

fn background(cancel: &AtomicBool) -> Result<super::scheduling::Permit<'static>, TimelineError> {
    super::scheduling::background(cancel).map_err(|error| match error {
        super::scheduling::Error::Cancelled => TimelineError::Cancelled,
        super::scheduling::Error::Unavailable => TimelineError::Crashed,
    })
}

fn valid_dimensions(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= 8192
        && height <= 8192
        && u64::from(width) * u64::from(height) <= MAX_SOURCE_PIXELS
}

/// Bounded metadata from the same reader used for playback/analysis. Container-wide duration can
/// include an AAC padding tail; editor source boundaries must use the playable video timeline.
pub fn source_info(path: &Path) -> Result<crate::ClipInfoSummary, TimelineError> {
    catch_unwind(AssertUnwindSafe(|| {
        mp4::preflight(path, &AtomicBool::new(false))?;
        let source = open_source(path)?;
        let info = clip_info(&source)?;
        if !valid_dimensions(info.width, info.height)
            || info.duration <= Tick::ZERO
            || info.duration > Tick::from_units(86_400, 1)
        {
            return Err(TimelineError::Unsupported);
        }
        Ok((&info).into())
    }))
    .unwrap_or(Err(TimelineError::Crashed))
}

/// Reads local media only; never creates, copies or changes any source file.
pub fn analyze_timeline(
    request: &TimelineRequest,
    cancel: &AtomicBool,
) -> Result<TimelineAnalysis, TimelineError> {
    let duration = request
        .end
        .0
        .checked_sub(request.start.0)
        .ok_or(TimelineError::InvalidRequest)?;
    if request.path.as_os_str().is_empty()
        || request.path.as_os_str().len() > 4096
        || request.start < Tick::ZERO
        || duration < Tick::from_units(1, 30).0
        || duration > Tick::from_units(1800, 1).0
        || !(1..=MAX_TIMELINE_FRAMES).contains(&request.frames)
        || !(16..=MAX_WAVEFORM_BUCKETS).contains(&request.buckets)
    {
        return Err(TimelineError::InvalidRequest);
    }
    check_cancel(cancel)?;
    if !std::fs::metadata(&request.path)
        .map_err(|_| TimelineError::Unreadable)?
        .is_file()
    {
        return Err(TimelineError::Unreadable);
    }
    catch_unwind(AssertUnwindSafe(|| {
        filmcraft_media::cancel::with_background(true, || analyze_inner(request, cancel))
    }))
    .unwrap_or(Err(TimelineError::Crashed))
}

fn analyze_inner(
    request: &TimelineRequest,
    cancel: &AtomicBool,
) -> Result<TimelineAnalysis, TimelineError> {
    let (source, info) = {
        let _permit = background(cancel)?;
        mp4::preflight(&request.path, cancel)?;
        let source = open_source(&request.path)?;
        let info = clip_info(&source)?;
        (source, info)
    };
    if !valid_dimensions(info.width, info.height)
        || info.duration > Tick::from_units(86_400, 1)
        || info.duration <= Tick::ZERO
        || request.end > info.duration
    {
        return Err(TimelineError::Unsupported);
    }
    if source.info().audio_streams.first().is_some_and(|a| {
        a.channels == 0 || a.channels > 16 || a.sample_rate == 0 || a.sample_rate > 384_000
    }) {
        return Err(TimelineError::Unsupported);
    }
    let mp4 = matches!(source.info().container.as_str(), "MPEG-4" | "QuickTime");
    let audio = source.info().audio_streams.first().cloned();
    // Generic sources hide their internal packet caches. Keep that fallback short; MP4 uses a
    // packet-at-a-time reader below, so a long replay never builds minutes of retained PCM.
    if !mp4
        && audio.as_ref().is_some_and(|a| {
            let duration = request.end - request.start;
            duration > Tick::from_units(10, 1)
                || i128::from(duration.0) * i128::from(a.sample_rate) * i128::from(a.channels) * 4
                    > i128::from(clipcraft_time::TICKS_PER_SECOND) * 16 * 1024 * 1024
        })
    {
        return Err(TimelineError::Unsupported);
    }
    let duration = request.end - request.start;
    let mut frames = Vec::with_capacity(usize::from(request.frames));
    for index in 0..request.frames {
        check_cancel(cancel)?;
        let position = request.start
            + duration.mul_ratio(i64::from(index) * 2 + 1, i64::from(request.frames) * 2);
        let _permit = background(cancel)?;
        check_cancel(cancel)?;
        frames.push(thumbnail(&source, position)?);
    }
    let waveform = if let Some(audio) = audio.filter(|_| mp4) {
        drop(source);
        mp4::waveform(request, &audio, cancel)?
    } else if info.has_audio {
        waveform(&source, request.start, request.end, request.buckets, cancel)?
    } else {
        vec![WaveformBucket::default(); usize::from(request.buckets)]
    };
    check_cancel(cancel)?;
    Ok(TimelineAnalysis {
        frames,
        waveform,
        has_audio: info.has_audio,
    })
}

fn thumbnail(source: &SharedSource, position: Tick) -> Result<TimelineFrame, TimelineError> {
    let decoded = source
        .video_frame(FrameRequest::full(position))
        .map_err(|_| TimelineError::DecodeFailed)?;
    if !valid_dimensions(decoded.width, decoded.height) {
        return Err(TimelineError::Unsupported);
    }
    let factor = (decoded.width / TIMELINE_WIDTH)
        .max(decoded.height / TIMELINE_HEIGHT)
        .max(1) as usize;
    let reduced = decoded.box_decimated(factor);
    let (width, height, rgba) = if let Some(frame) = reduced.as_ref() {
        (frame.width, frame.height, frame.to_rgba8())
    } else {
        // RGBA sources have no native planar box reducer. Convert only decimated pixels, not a
        // full-size RGBA copy, so even an 8K float source does not expand on our worker.
        let (width, height, linear) = decoded.to_linear_f32_decimated(factor);
        let pixels = width
            .checked_mul(height)
            .and_then(|p| p.checked_mul(4))
            .filter(|&n| n <= 320 * 180 * 4)
            .ok_or(TimelineError::DecodeFailed)?;
        let mut rgba = vec![0; pixels];
        filmcraft_frame::linear_premul_to_srgb8(&linear, &mut rgba);
        (width as u32, height as u32, rgba)
    };
    if width > 320 || height > 180 {
        return Err(TimelineError::DecodeFailed);
    }
    let picture =
        image::RgbaImage::from_raw(width, height, rgba).ok_or(TimelineError::DecodeFailed)?;
    let scale_width = TIMELINE_WIDTH.min(width);
    let scale_height = TIMELINE_HEIGHT.min(height);
    let fitted = image::imageops::thumbnail(&picture, scale_width, scale_height);
    let mut canvas =
        image::RgbaImage::from_pixel(TIMELINE_WIDTH, TIMELINE_HEIGHT, image::Rgba([0, 0, 0, 255]));
    image::imageops::overlay(
        &mut canvas,
        &fitted,
        i64::from((TIMELINE_WIDTH - fitted.width()) / 2),
        i64::from((TIMELINE_HEIGHT - fitted.height()) / 2),
    );
    Ok(TimelineFrame {
        position,
        width: TIMELINE_WIDTH,
        height: TIMELINE_HEIGHT,
        rgba: canvas.into_raw(),
    })
}

fn waveform(
    source: &SharedSource,
    start: Tick,
    end: Tick,
    buckets: u16,
    cancel: &AtomicBool,
) -> Result<Vec<WaveformBucket>, TimelineError> {
    let mut output = vec![WaveformBucket::default(); usize::from(buckets)];
    let first = start.to_units_floor(i64::from(AUDIO_RATE));
    let last = end.to_units_floor(i64::from(AUDIO_RATE));
    let count = last
        .checked_sub(first)
        .filter(|&n| n > 0)
        .ok_or(TimelineError::InvalidRequest)?;
    let mut cursor = first;
    while cursor < last {
        let _permit = background(cancel)?;
        check_cancel(cancel)?;
        let frames = usize::try_from((last - cursor).min(AUDIO_CHUNK as i64))
            .map_err(|_| TimelineError::InvalidRequest)?;
        let audio = source
            .audio_stream(0, cursor, frames, AUDIO_RATE)
            .map_err(|_| TimelineError::DecodeFailed)?;
        if audio.sample_rate != AUDIO_RATE
            || audio.channels.is_empty()
            || audio.channels.len() > 16
            || audio.channels.iter().any(|c| c.len() > frames)
        {
            return Err(TimelineError::DecodeFailed);
        }
        for i in 0..frames {
            let bucket = ((cursor - first + i as i64) * i64::from(buckets) / count) as usize;
            let Some(out) = output.get_mut(bucket) else {
                return Err(TimelineError::DecodeFailed);
            };
            for channel in &audio.channels {
                let sample = channel.get(i).copied().unwrap_or(0.0);
                add_amplitude(out, sample);
            }
        }
        cursor += frames as i64;
    }
    Ok(output)
}

fn add_amplitude(out: &mut WaveformBucket, sample: f32) {
    if sample.is_finite() {
        let sample = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        out.min = out.min.min(sample);
        out.max = out.max.max(sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "clipcraft-timeline-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn request(path: PathBuf) -> TimelineRequest {
        TimelineRequest {
            path,
            start: Tick::ZERO,
            end: Tick::from_units(1, 1),
            frames: 4,
            buckets: 32,
        }
    }
    fn centre_red(frame: &TimelineFrame) -> u8 {
        frame.rgba[((TIMELINE_HEIGHT / 2 * TIMELINE_WIDTH + TIMELINE_WIDTH / 2) * 4) as usize]
    }

    #[test]
    fn real_filmstrips_follow_source_times_and_trim_without_changing_media() {
        let scratch = Scratch::new();
        let bytes = crate::tests::make_clip();
        let path = scratch.file("changing.mp4", &bytes);
        let mut request = request(path.clone());
        let analysis = analyze_timeline(&request, &AtomicBool::new(false)).unwrap();
        assert!(!analysis.has_audio);
        assert!(
            analysis
                .waveform
                .iter()
                .all(|v| *v == WaveformBucket::default())
        );
        assert!(
            analysis.frames.windows(2).all(
                |w| w[0].position < w[1].position && centre_red(&w[0]) + 20 < centre_red(&w[1])
            )
        );
        assert!(
            analysis
                .frames
                .iter()
                .all(|f| (f.width, f.height, f.rgba.len()) == (160, 90, 160 * 90 * 4))
        );
        assert_eq!(
            analysis.frames[0].rgba[0..4],
            [0, 0, 0, 255],
            "small sources are letterboxed, not enlarged"
        );
        request.start = Tick::from_units(1, 2);
        request.frames = 2;
        let trimmed = analyze_timeline(&request, &AtomicBool::new(false)).unwrap();
        assert_eq!(trimmed.frames[0].position, Tick::from_units(5, 8));
        assert!(centre_red(&trimmed.frames[0]) > centre_red(&analysis.frames[1]));
        assert!(analysis.bytes() < 1024 * 1024);
        request.frames = MAX_TIMELINE_FRAMES;
        request.buckets = MAX_WAVEFORM_BUCKETS;
        let largest = analyze_timeline(&request, &AtomicBool::new(false)).unwrap();
        assert_eq!(largest.frames.len(), usize::from(MAX_TIMELINE_FRAMES));
        assert_eq!(largest.waveform.len(), usize::from(MAX_WAVEFORM_BUCKETS));
        assert!(largest.bytes() < 1024 * 1024);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn cancellation_stops_between_bounded_audio_reads() {
        use filmcraft_frame::{AudioBuffer, VideoFrame};
        use filmcraft_media::{MediaInfo, MediaSource};
        use std::sync::Arc;

        struct CancelAfterRead {
            inner: SharedSource,
            cancel: Arc<AtomicBool>,
            reads: AtomicU64,
        }
        impl MediaSource for CancelAfterRead {
            fn info(&self) -> &MediaInfo {
                self.inner.info()
            }
            fn video_frame(
                &self,
                request: FrameRequest,
            ) -> filmcraft_media::Result<Arc<VideoFrame>> {
                self.inner.video_frame(request)
            }
            fn audio(
                &self,
                start: i64,
                frames: usize,
                rate: u32,
            ) -> filmcraft_media::Result<AudioBuffer> {
                assert!(frames <= AUDIO_CHUNK);
                self.reads.fetch_add(1, Ordering::Relaxed);
                let audio = self.inner.audio(start, frames, rate)?;
                self.cancel.store(true, Ordering::Relaxed);
                Ok(audio)
            }
        }

        let scratch = Scratch::new();
        let path = scratch.file("cancel.mp4", &crate::tests::make_clip_with_sound());
        let cancel = Arc::new(AtomicBool::new(false));
        let source = Arc::new(CancelAfterRead {
            inner: open_source(&path).unwrap(),
            cancel: Arc::clone(&cancel),
            reads: AtomicU64::new(0),
        });
        let shared: SharedSource = source.clone();
        assert_eq!(
            waveform(&shared, Tick::ZERO, Tick::from_units(1, 1), 16, &cancel),
            Err(TimelineError::Cancelled)
        );
        assert_eq!(source.reads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn waveform_decodes_the_first_track_and_a_trimmed_real_impulse() {
        let scratch = Scratch::new();
        let tone = scratch.file("tone.mp4", &crate::tests::make_clip_with_sound());
        let analysis = analyze_timeline(&request(tone), &AtomicBool::new(false)).unwrap();
        assert!(analysis.has_audio);
        assert!(
            analysis
                .waveform
                .iter()
                .all(|b| b.min < -8191 && b.max > 8191)
        );
        let silence_first = scratch.file(
            "tracks.mp4",
            &crate::tests::make_clip_with_tracks(&[0.0, 440.0]),
        );
        let first = analyze_timeline(&request(silence_first), &AtomicBool::new(false)).unwrap();
        assert!(first.has_audio);
        assert!(first.waveform.iter().all(|b| b.min == 0 && b.max == 0));

        // Exact 8 kHz PCM avoids resampling uncertainty: only the middle bucket contains sound.
        let samples: Vec<i16> = (0..8000)
            .map(|i| {
                if (4000..4100).contains(&i) {
                    if i % 2 == 0 { 24000 } else { -24000 }
                } else {
                    0
                }
            })
            .collect();
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + samples.len() as u32 * 2).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16_u32.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(AUDIO_RATE.to_le_bytes());
        wav.extend((AUDIO_RATE * 2).to_le_bytes());
        wav.extend(2_u16.to_le_bytes());
        wav.extend(16_u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend((samples.len() as u32 * 2).to_le_bytes());
        for sample in samples {
            wav.extend(sample.to_le_bytes());
        }
        let impulse = open_source(&scratch.file("impulse.wav", &wav)).unwrap();
        let envelope = waveform(
            &impulse,
            Tick::ZERO,
            Tick::from_units(1, 1),
            16,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(envelope[8].min < -22936 && envelope[8].max > 22936);
        assert!(
            envelope
                .iter()
                .enumerate()
                .all(|(i, b)| i == 8 || *b == WaveformBucket::default())
        );
        let trimmed = waveform(
            &impulse,
            Tick::from_units(1, 2),
            Tick::from_units(3, 4),
            16,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(trimmed[0].max > 22936);
        assert_eq!(trimmed[8].max, 0);
    }

    #[test]
    fn invalid_requests_missing_and_hostile_media_and_cancellation_fail_without_output() {
        let scratch = Scratch::new();
        let bytes = crate::tests::make_clip();
        let path = scratch.file("clip.mp4", &bytes);
        let inspected = source_info(&path).unwrap();
        assert!(inspected.duration > Tick::ZERO);
        assert!(inspected.width > 0 && inspected.height > 0);
        assert!(source_info(&scratch.0.join("missing.mp4")).is_err());
        assert!(source_info(&scratch.0).is_err());
        assert!(source_info(&scratch.file("broken-info.mp4", b"not a video")).is_err());
        for (start, end) in [
            (Tick(-1), Tick::from_units(1, 1)),
            (Tick::ZERO, Tick::ZERO),
            (Tick(i64::MIN), Tick(i64::MAX)),
            (Tick::ZERO, Tick::from_units(1801, 1)),
        ] {
            let mut r = request(path.clone());
            (r.start, r.end) = (start, end);
            assert_eq!(
                analyze_timeline(&r, &AtomicBool::new(false)),
                Err(TimelineError::InvalidRequest)
            );
        }
        for (frames, buckets) in [(0, 16), (13, 16), (1, 0), (1, 2049), (u16::MAX, u16::MAX)] {
            let mut r = request(path.clone());
            (r.frames, r.buckets) = (frames, buckets);
            assert_eq!(
                analyze_timeline(&r, &AtomicBool::new(false)),
                Err(TimelineError::InvalidRequest)
            );
        }
        assert_eq!(
            analyze_timeline(&request(path.clone()), &AtomicBool::new(true)),
            Err(TimelineError::Cancelled)
        );
        assert!(
            analyze_timeline(
                &request(scratch.0.join("missing.mp4")),
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert_eq!(
            analyze_timeline(&request(scratch.0.clone()), &AtomicBool::new(false)),
            Err(TimelineError::Unreadable)
        );
        let mut past_end = request(path.clone());
        past_end.end = Tick::from_units(2, 1);
        assert_eq!(
            analyze_timeline(&past_end, &AtomicBool::new(false)),
            Err(TimelineError::Unsupported)
        );
        assert!(
            analyze_timeline(
                &request(scratch.file("broken.mp4", b"not a video")),
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn oversized_prores_sample_is_rejected_before_source_metadata_probing() {
        let scratch = Scratch::new();
        let mut bytes = crate::tests::make_clip();
        let stsd = bytes.windows(4).position(|w| w == b"stsd").unwrap();
        assert_eq!(&bytes[stsd + 16..stsd + 20], b"avc1");
        bytes[stsd + 16..stsd + 20].copy_from_slice(b"apcn");
        let stsz = bytes.windows(4).position(|w| w == b"stsz").unwrap();
        assert_eq!(&bytes[stsz + 8..stsz + 12], &[0; 4]);
        bytes[stsz + 16..stsz + 20].copy_from_slice(&u32::MAX.to_be_bytes());
        let path = scratch.file("forged-prores.mp4", &bytes);
        assert_eq!(
            analyze_timeline(&request(path.clone()), &AtomicBool::new(false)),
            Err(TimelineError::Unsupported)
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
