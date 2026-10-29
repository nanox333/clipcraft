//! Writes buffered pictures to an MP4 or MKV file, atomically.

use std::fs::{self, File};
use std::io::{BufWriter, ErrorKind};
use std::path::{Path, PathBuf};

use filmcraft_isobmff::{
    AvcConfig, Brand, FragmentedWriter, HevcConfig, Mp4Writer, SampleEntry, TrackConfig,
    WriteSample, WriterOptions,
};
use filmcraft_matroska::{MkvWriter, MuxOptions, TrackKind, TrackSpec};

use crate::RecordError;
use crate::audio::{AAC_FRAME, AudioPacket, AudioTrackInfo};
use crate::hardware::StreamCodec;
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
    /// The decoder configuration record: `avcC` for H.264, `hvcC` for H.265.
    pub avcc: Vec<u8>,
    /// Which of the two `avcc` is.
    pub codec: StreamCodec,
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

    // An MP4 keeps its index at the end, so a cut-off one cannot be played. A crash-safe copy is
    // written first: fragmented, playable up to its last fragment, and turned back into a clip
    // at the next launch. It goes once the clip is complete.
    let recovery = (format == ClipFormat::Mp4).then(|| {
        let mut name = path.as_os_str().to_owned();
        name.push(".recovery");
        PathBuf::from(name)
    });
    let cleanup = |recovery: &Option<PathBuf>| {
        let _ = fs::remove_file(&partial);
        if let Some(recovery) = recovery {
            let _ = fs::remove_file(recovery);
        }
    };
    // Claim the temporary names exclusively: another save of the same name must not share or
    // truncate them, and a failed claim never removes a file this save did not create.
    let claim = |name: &Path| {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(name)
            .map(drop)
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    RecordError::AlreadyExists
                } else {
                    io_error(&error)
                }
            })
    };
    claim(&partial)?;
    if let Some(recovery) = &recovery
        && let Err(error) = claim(recovery)
    {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    if let Some(recovery) = &recovery
        && let Err(error) = write_recovery_copy(samples, video, audio, recovery, None)
    {
        cleanup(&Some(recovery.clone()));
        return Err(error);
    }
    if let Err(error) = write_partial(samples, video, audio, format, &partial) {
        cleanup(&recovery);
        return Err(error);
    }
    if let Err(error) = publish_new(&partial, path) {
        cleanup(&recovery);
        return Err(error);
    }
    let _ = fs::remove_file(&partial);
    if let Some(recovery) = &recovery {
        let _ = fs::remove_file(recovery);
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

/// Moves `from` to `to` without replacing an existing file: a hard link is atomic, and where the
/// filesystem has none, a checked rename is the fallback.
fn publish_new(from: &Path, to: &Path) -> Result<(), RecordError> {
    match fs::hard_link(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(RecordError::AlreadyExists)
        }
        Err(_) => {
            if fs::symlink_metadata(to).is_ok() {
                return Err(RecordError::AlreadyExists);
            }
            fs::rename(from, to).map_err(|error| io_error(&error))
        }
    }
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
            let mut mp4 =
                Mp4Writer::new(writer, WriterOptions::new(Brand::Mp4)).map_err(mp4_error)?;
            write_mp4_samples(&mut mp4, samples, video, &audio)?;
            let mut out = mp4.finish().map_err(mp4_error)?;
            std::io::Write::flush(&mut out).map_err(|error| io_error(&error))?;
            out.get_ref().sync_all().map_err(|error| io_error(&error))?;
        }
        ClipFormat::Mkv => {
            let mut specs = Vec::new();
            let mut spec = TrackSpec::new(
                TrackKind::Video,
                match video.codec {
                    StreamCodec::H264 => "V_MPEG4/ISO/AVC",
                    StreamCodec::H265 => "V_MPEGH/ISO/HEVC",
                },
            );
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

/// Where MP4 samples go: the ordinary writer, or the fragmented one whose file stays playable up
/// to its last complete fragment if the program dies while writing.
trait Mp4Sink {
    fn add(&mut self, config: TrackConfig) -> Result<usize, RecordError>;
    fn put(&mut self, track: usize, sample: WriteSample<'_>) -> Result<(), RecordError>;
    /// Called before every picture that starts a group of pictures (not the first). The
    /// fragmented writer ends a fragment here. True when the caller should stop, which is how
    /// tests simulate a crash.
    fn group_boundary(&mut self) -> Result<bool, RecordError>;
}

impl<W: std::io::Write + std::io::Seek> Mp4Sink for Mp4Writer<W> {
    fn add(&mut self, config: TrackConfig) -> Result<usize, RecordError> {
        self.add_track(config).map_err(mp4_error)
    }
    fn put(&mut self, track: usize, sample: WriteSample<'_>) -> Result<(), RecordError> {
        self.write_sample(track, sample).map_err(mp4_error)
    }
    fn group_boundary(&mut self) -> Result<bool, RecordError> {
        Ok(false)
    }
}

struct Fragments<W: std::io::Write> {
    writer: FragmentedWriter<W>,
    written: usize,
    stop_after: Option<usize>,
}

impl<W: std::io::Write> Mp4Sink for Fragments<W> {
    fn add(&mut self, config: TrackConfig) -> Result<usize, RecordError> {
        self.writer.add_track(config).map_err(mp4_error)
    }
    fn put(&mut self, track: usize, sample: WriteSample<'_>) -> Result<(), RecordError> {
        self.writer.write_sample(track, sample).map_err(mp4_error)
    }
    fn group_boundary(&mut self) -> Result<bool, RecordError> {
        self.writer.flush_fragment().map_err(mp4_error)?;
        self.written += 1;
        Ok(self.stop_after.is_some_and(|limit| self.written >= limit))
    }
}

/// Describes the pictures and sound to `mp4` and feeds it every sample in time order. False when
/// the sink asked to stop early.
fn write_mp4_samples<S: Mp4Sink>(
    mp4: &mut S,
    samples: &[EncodedSample],
    video: &VideoInfo,
    audio: &[&ClipAudio],
) -> Result<bool, RecordError> {
    let origin = samples.first().map_or(0, |sample| sample.pts);
    let (width, height) = (
        u16::try_from(video.width).map_err(|_| RecordError::BadStream)?,
        u16::try_from(video.height).map_err(|_| RecordError::BadStream)?,
    );
    let entry = match video.codec {
        StreamCodec::H264 => SampleEntry::avc(
            AvcConfig::parse(&video.avcc).map_err(|_| RecordError::BadStream)?,
            width,
            height,
        ),
        StreamCodec::H265 => SampleEntry::hevc(
            HevcConfig::parse(&video.avcc).map_err(|_| RecordError::BadStream)?,
            width,
            height,
        ),
    };
    let track = mp4.add(TrackConfig::new(entry, video.fps))?;
    // Each sound track: its own timescale (the sample rate) and an edit that skips the decoder's
    // warm-up unit and the part before the first picture.
    let mut sound = Vec::new();
    for clip_audio in audio {
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
        let id = mp4.add(config)?;
        sound.push((id, units, info.sample_rate, from));
    }
    // Pictures and sound in time order, so the file is interleaved.
    let mut cursors = vec![0_usize; sound.len()];
    let mut previous_duration = 1_u32;
    for (index, sample) in samples.iter().enumerate() {
        if index > 0 && sample.keyframe && mp4.group_boundary()? {
            return Ok(false);
        }
        let duration = match samples.get(index + 1) {
            Some(next) => u32::try_from(next.pts - sample.pts).unwrap_or(1).max(1),
            None => previous_duration,
        };
        previous_duration = duration;
        mp4.put(
            track,
            WriteSample {
                data: &sample.data,
                duration,
                composition_offset: 0,
                is_sync: sample.keyframe,
            },
        )?;
        let picture_end =
            (sample.pts - origin + i64::from(duration)) as f64 / f64::from(video.fps.max(1));
        for ((id, units, rate, from), cursor) in sound.iter().zip(&mut cursors) {
            while let Some(unit) = units.get(*cursor) {
                let unit_time = (unit.start - from) as f64 / f64::from(*rate);
                if unit_time > picture_end && index + 1 < samples.len() {
                    break;
                }
                mp4.put(
                    *id,
                    WriteSample {
                        data: &unit.data,
                        duration: AAC_FRAME as u32,
                        composition_offset: 0,
                        is_sync: true,
                    },
                )?;
                *cursor += 1;
            }
        }
    }
    Ok(true)
}

/// A file writer that asks the disk to keep what was written every few megabytes, so a power cut
/// loses little of a crash-safe copy.
struct SyncingFile {
    file: File,
    since_sync: usize,
}

impl std::io::Write for SyncingFile {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let written = self.file.write(data)?;
        self.since_sync += written;
        if self.since_sync >= 4 * 1024 * 1024 {
            self.file.sync_data()?;
            self.since_sync = 0;
        }
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Writes the clip as a fragmented MP4, one fragment per group of pictures. If the program dies
/// part way, everything up to the last complete fragment can be played and turned back into an
/// ordinary clip ([`ClipFormat::Mp4`] saves write this first). `stop_after` ends the write after
/// that many fragments without finishing, which is how tests simulate a crash.
fn write_recovery_copy(
    samples: &[EncodedSample],
    video: &VideoInfo,
    audio: &[ClipAudio],
    path: &Path,
    stop_after: Option<usize>,
) -> Result<(), RecordError> {
    let file = File::create(path).map_err(|error| io_error(&error))?;
    let sink = BufWriter::with_capacity(
        256 * 1024,
        SyncingFile {
            file,
            since_sync: 0,
        },
    );
    let mut fragments = Fragments {
        writer: FragmentedWriter::new(sink, WriterOptions::new(Brand::Mp4)),
        written: 0,
        stop_after,
    };
    let audio: Vec<&ClipAudio> = audio
        .iter()
        .filter(|track| !track.packets.is_empty())
        .collect();
    if !write_mp4_samples(&mut fragments, samples, video, &audio)? {
        // Stopped early: the buffered part reaches the file when the writer is dropped, as it
        // would have if the program had died right after this fragment.
        return Ok(());
    }
    let mut sink = fragments.writer.finish().map_err(mp4_error)?;
    std::io::Write::flush(&mut sink).map_err(|error| io_error(&error))?;
    sink.get_ref()
        .file
        .sync_all()
        .map_err(|error| io_error(&error))
}

/// Writes the first `fragments` fragments of the clip's crash-safe copy to `path` and stops, as
/// if the program had died there. For tests of recovery only.
#[doc(hidden)]
pub fn write_clip_interrupted(
    samples: &[EncodedSample],
    video: &VideoInfo,
    audio: &[ClipAudio],
    path: &Path,
    fragments: usize,
) -> Result<(), RecordError> {
    write_recovery_copy(samples, video, audio, path, Some(fragments.max(1)))
}
