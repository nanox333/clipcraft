//! MP4 waveforms discard packet PCM immediately instead of populating MediaSource's audio cache.

use super::{TimelineError, TimelineRequest, WaveformBucket, add_amplitude, check_cancel};
use clipcraft_time::{TICKS_PER_SECOND, Tick};
use filmcraft_codecs::audio::{self, FixedFrames, PacketDecoder, PacketTime};
use filmcraft_isobmff::{CodecConfig, Mp4File, Track, TrackKind};
use filmcraft_media::AudioStreamInfo;
use std::sync::atomic::AtomicBool;

#[path = "timeline_mp4_preflight.rs"]
mod preflight;
pub(super) use preflight::preflight;

const MAX_PACKET_BYTES: u32 = 1024 * 1024;
const MAX_PACKET_SCALARS: usize = 1024 * 1024;
const MAX_PACKET_ENTRIES: usize = 1_000_000;

fn playable(track: &Track, kind: TrackKind) -> bool {
    track.kind == kind
        && track.timescale > 0
        && !track.samples.is_empty()
        && !track.entries.is_empty()
}

fn packet(
    file: &Mp4File,
    reader: &(impl filmcraft_isobmff::ByteSource + ?Sized),
    track: usize,
    index: usize,
) -> Result<Vec<u8>, TimelineError> {
    let sample = file
        .tracks
        .get(track)
        .and_then(|t| t.samples.get(index))
        .ok_or(TimelineError::DecodeFailed)?;
    let limit = if matches!(
        file.tracks.get(track).and_then(Track::codec),
        Some(CodecConfig::Mp3 | CodecConfig::Ac3 { .. } | CodecConfig::Eac3 { .. })
    ) {
        64 * 1024
    } else {
        MAX_PACKET_BYTES
    };
    if sample.size > limit {
        return Err(TimelineError::Unsupported);
    }
    file.read_sample(reader, track, index)
        .map_err(|_| TimelineError::DecodeFailed)
}

fn offset_units(track: &Track) -> i64 {
    match track.codec() {
        Some(CodecConfig::Opus(o)) if track.edit_offset == 0 => {
            -i64::from(o.pre_skip) * i64::from(track.timescale) / i64::from(audio::OPUS_RATE)
        }
        _ => track.edit_offset,
    }
}

fn offset(track: &Track) -> Tick {
    Tick::from_units(offset_units(track), i64::from(track.timescale))
}

// Match the source clock: unedited video with a positive composition delay is rebased together
// with every audio track by the earliest positive stream start.
fn rebase(file: &Mp4File) -> Tick {
    let Some(video) = file
        .tracks
        .iter()
        .find(|t| playable(t, TrackKind::Video))
        .filter(|t| t.edits.is_empty())
    else {
        return Tick::ZERO;
    };
    let Some(first) = video
        .samples
        .iter()
        .map(|s| s.pts)
        .min()
        .and_then(|v| v.checked_add(video.edit_offset))
        .filter(|&v| v > 0)
    else {
        return Tick::ZERO;
    };
    file.tracks
        .iter()
        .filter(|t| playable(t, TrackKind::Audio))
        .take(filmcraft_media::MAX_AUDIO_STREAMS)
        .fold(
            Tick::from_units(first, i64::from(video.timescale)),
            |start, t| start.min(offset(t).max(Tick::ZERO)),
        )
}

fn starts(
    track: &Track,
    rate: u32,
    first: &[u8],
    cancel: &AtomicBool,
) -> Result<Vec<i64>, TimelineError> {
    let mut total = 0_i128;
    let mut starts = Vec::with_capacity(track.samples.len());
    for (i, sample) in track.samples.iter().enumerate() {
        if i.is_multiple_of(1024) {
            check_cancel(cancel)?;
        }
        starts.push(
            i64::try_from(total * i128::from(rate) / i128::from(track.timescale))
                .map_err(|_| TimelineError::Unsupported)?,
        );
        total += i128::from(sample.duration);
    }
    let fixed = match track.codec() {
        Some(CodecConfig::Aac(a)) => Some(FixedFrames::Aac(&a.asc)),
        Some(CodecConfig::Mp3) => Some(FixedFrames::MpegAudio),
        Some(CodecConfig::Ac3 { .. }) => Some(FixedFrames::Ac3),
        Some(CodecConfig::Eac3 { .. }) => Some(FixedFrames::Eac3),
        _ => None,
    }
    .and_then(|codec| audio::fixed_packet_samples(codec, first, rate));
    Ok(if let Some(frames) = fixed {
        let unit = (i64::from(rate) + i64::from(track.timescale) - 1) / i64::from(track.timescale);
        audio::contiguous_starts(
            starts.into_iter().map(|stamp| PacketTime {
                stamp,
                stamped: true,
                samples: Some(frames),
            }),
            (frames / 2).max(unit * 2 + 1),
        )
    } else {
        starts
    })
}

pub(super) fn waveform(
    request: &TimelineRequest,
    info: &AudioStreamInfo,
    cancel: &AtomicBool,
) -> Result<Vec<WaveformBucket>, TimelineError> {
    check_cancel(cancel)?;
    if info.sample_rate == 0
        || info.sample_rate > 384_000
        || info.channels == 0
        || info.channels > 16
    {
        return Err(TimelineError::Unsupported);
    }
    let reader = std::fs::File::open(&request.path).map_err(|_| TimelineError::Unreadable)?;
    waveform_reader(request, info, cancel, &reader)
}

fn waveform_reader(
    request: &TimelineRequest,
    info: &AudioStreamInfo,
    cancel: &AtomicBool,
    reader: &impl filmcraft_isobmff::ByteSource,
) -> Result<Vec<WaveformBucket>, TimelineError> {
    preflight::reader(reader, cancel)?;
    let file = filmcraft_isobmff::open(reader).map_err(|_| TimelineError::DecodeFailed)?;
    let (ti, track) = file
        .tracks
        .iter()
        .enumerate()
        .find(|(_, t)| playable(t, TrackKind::Audio))
        .ok_or(TimelineError::Unsupported)?;
    if track.samples.len() > MAX_PACKET_ENTRIES {
        return Err(TimelineError::Unsupported);
    }
    let codec = track.codec().ok_or(TimelineError::Unsupported)?;
    // Symphonia allocates ALAC's frame buffer while constructing the decoder, before decode.
    // Its cookie frame_length is an untrusted u32; validate it before calling the factory.
    if let CodecConfig::Alac { cookie } = codec {
        let frame: [u8; 4] = cookie
            .get(..4)
            .and_then(|b| b.try_into().ok())
            .ok_or(TimelineError::Unsupported)?;
        let channels = cookie.get(9).copied().ok_or(TimelineError::Unsupported)?;
        if !matches!(cookie.len(), 24 | 48)
            || u32::from(channels) != info.channels
            || u32::from_be_bytes(frame) == 0
            || u64::from(u32::from_be_bytes(frame)) * u64::from(channels)
                > MAX_PACKET_SCALARS as u64
        {
            return Err(TimelineError::Unsupported);
        }
    }
    let first_packet = packet(&file, reader, ti, 0)?;
    let positions = starts(track, info.sample_rate, &first_packet, cancel)?;
    drop(first_packet);
    let adjusted_offset = offset_units(track)
        .saturating_sub(rebase(&file).to_rational_round(1, i64::from(track.timescale)));
    let origin = Tick::from_units(adjusted_offset, i64::from(track.timescale));
    let ceil_sample = |time: Tick| -> Result<i64, TimelineError> {
        let n = i128::from(time.0) * i128::from(info.sample_rate);
        i64::try_from(-(-n).div_euclid(i128::from(TICKS_PER_SECOND)))
            .map_err(|_| TimelineError::Unsupported)
    };
    let first = ceil_sample(request.start - origin)?;
    let last = ceil_sample(request.end - origin)?;
    let preroll = if matches!(codec, CodecConfig::Opus(_)) {
        i64::from(audio::OPUS_PRE_ROLL)
    } else {
        0
    };
    let from = positions
        .partition_point(|&p| p <= first.saturating_sub(preroll))
        .saturating_sub(1);
    let from = if preroll == 0 {
        from.saturating_sub(1)
    } else {
        from
    };
    let mut decoder = if matches!(codec, CodecConfig::Pcm(_)) {
        None
    } else {
        Some(
            PacketDecoder::for_isobmff(codec, info.sample_rate)
                .map_err(|_| TimelineError::Unsupported)?,
        )
    };
    let mut output = vec![WaveformBucket::default(); usize::from(request.buckets)];
    for (i, &position) in positions.iter().enumerate().skip(from) {
        check_cancel(cancel)?;
        if position >= last {
            break;
        }
        let sample = track.samples.get(i).ok_or(TimelineError::DecodeFailed)?;
        let declared =
            u128::from(sample.duration) * u128::from(info.sample_rate) * u128::from(info.channels)
                / u128::from(track.timescale);
        if declared > MAX_PACKET_SCALARS as u128 {
            return Err(TimelineError::Unsupported);
        }
        let bytes = packet(&file, reader, ti, i)?;
        let pcm = match codec {
            CodecConfig::Pcm(cfg)
                if cfg.channels == info.channels
                    && matches!(
                        (cfg.float, cfg.bits),
                        (false, 8 | 16 | 24 | 32) | (true, 32 | 64)
                    ) =>
            {
                if bytes.len() / usize::from(cfg.bits).div_ceil(8) > MAX_PACKET_SCALARS {
                    return Err(TimelineError::Unsupported);
                }
                audio::decode_pcm(&bytes, cfg)
            }
            CodecConfig::Pcm(_) => return Err(TimelineError::Unsupported),
            _ => decoder
                .as_mut()
                .ok_or(TimelineError::Unsupported)?
                .decode(&bytes, position.max(0) as u64)
                .map_err(|_| TimelineError::DecodeFailed)?,
        };
        if pcm.is_empty()
            || pcm.len() > 16
            || pcm
                .iter()
                .try_fold(0_usize, |sum, c| sum.checked_add(c.len()))
                .is_none_or(|sum| sum > MAX_PACKET_SCALARS)
        {
            return Err(TimelineError::Unsupported);
        }
        for channel in pcm {
            for (k, sample) in channel.into_iter().enumerate() {
                if k.is_multiple_of(4096) {
                    check_cancel(cancel)?;
                }
                let pos = position
                    .checked_add(k as i64)
                    .ok_or(TimelineError::DecodeFailed)?;
                if pos < first || pos >= last {
                    continue;
                }
                let time = Tick::from_units(pos, i64::from(info.sample_rate)) + origin;
                let bucket = i128::from((time - request.start).0) * i128::from(request.buckets)
                    / i128::from((request.end - request.start).0);
                if let Ok(bucket) = usize::try_from(bucket)
                    && let Some(out) = output.get_mut(bucket)
                {
                    add_amplitude(out, sample);
                }
            }
        }
    }
    check_cancel(cancel)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_isobmff::{
        Brand, ByteSource, Mp4Writer, PcmConfig, SampleEntry, TrackConfig, WriteSample,
        WriterOptions,
    };
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn long_pcm() -> Vec<u8> {
        let mut writer =
            Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
        let cfg = PcmConfig {
            bits: 16,
            float: false,
            big_endian: false,
            signed: true,
            channels: 1,
            sample_rate: 8.0,
        };
        let track = writer
            .add_track(TrackConfig::new(SampleEntry::pcm(cfg), 8))
            .unwrap();
        for i in 0..1800 {
            let sample = if i == 900 { 24000_i16 } else { 0_i16 };
            let data: Vec<u8> = (0..8).flat_map(|_| sample.to_le_bytes()).collect();
            writer
                .write_sample(
                    track,
                    WriteSample {
                        data: &data,
                        duration: 0,
                        composition_offset: 0,
                        is_sync: true,
                    },
                )
                .unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn request() -> TimelineRequest {
        TimelineRequest {
            path: "unused.mp4".into(),
            start: Tick::ZERO,
            end: Tick::from_units(1800, 1),
            frames: 1,
            buckets: 1800,
        }
    }

    fn info() -> AudioStreamInfo {
        AudioStreamInfo {
            sample_rate: 8,
            channels: 1,
            codec: "PCM".into(),
            bits_per_sample: Some(16),
        }
    }

    #[test]
    fn full_thirty_minute_audio_stream_discards_packets_and_places_a_trimmed_peak() {
        let bytes = long_pcm();
        let mut request = request();
        let envelope = waveform_reader(&request, &info(), &AtomicBool::new(false), &bytes).unwrap();
        assert!(envelope[900].max > 22936);
        assert!(
            envelope
                .iter()
                .enumerate()
                .all(|(i, b)| i == 900 || *b == WaveformBucket::default())
        );
        request.start = Tick::from_units(900, 1);
        request.end = Tick::from_units(902, 1);
        request.buckets = 16;
        let trimmed = waveform_reader(&request, &info(), &AtomicBool::new(false), &bytes).unwrap();
        assert!(trimmed[..8].iter().all(|b| b.max > 22936));
        assert!(trimmed[8..].iter().all(|b| *b == WaveformBucket::default()));
    }

    #[test]
    fn packet_size_guards_and_mid_packet_cancellation_stop_without_source_changes() {
        struct CancelAfterPacket<'a> {
            bytes: &'a [u8],
            offset: u64,
            reads: AtomicUsize,
            flag: AtomicBool,
        }
        impl ByteSource for CancelAfterPacket<'_> {
            fn len(&self) -> u64 {
                self.bytes.len() as u64
            }
            fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<()> {
                self.bytes.read_at(offset, buffer)?;
                if offset == self.offset && self.reads.fetch_add(1, Ordering::Relaxed) == 1 {
                    self.flag.store(true, Ordering::Relaxed);
                }
                Ok(())
            }
        }
        let bytes = long_pcm();
        let original = bytes.clone();
        let mut file = filmcraft_isobmff::open(&bytes).unwrap();
        let sample = file.tracks[0].samples[0];
        file.tracks[0].samples[0].size = MAX_PACKET_BYTES + 1;
        assert_eq!(packet(&file, &bytes, 0, 0), Err(TimelineError::Unsupported));
        let reader = CancelAfterPacket {
            bytes: &bytes,
            offset: sample.offset,
            reads: AtomicUsize::new(0),
            flag: AtomicBool::new(false),
        };
        assert_eq!(
            waveform_reader(&request(), &info(), &reader.flag, &reader),
            Err(TimelineError::Cancelled)
        );
        assert_eq!(reader.reads.load(Ordering::Relaxed), 2);
        assert_eq!(bytes, original);
    }

    #[test]
    fn oversized_alac_cookie_is_rejected_before_allocating_decoder_buffers() {
        use filmcraft_isobmff::FourCc;
        for length in [24, 48] {
            let mut cookie = vec![0; length];
            cookie[..4].copy_from_slice(&u32::MAX.to_be_bytes());
            cookie[5] = 16;
            cookie[9] = 1;
            cookie[20..24].copy_from_slice(&8_u32.to_be_bytes());
            let entry =
                SampleEntry::audio(FourCc(*b"alac"), CodecConfig::Alac { cookie }, 1, 8.0, 16);
            let mut writer =
                Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
            let track = writer.add_track(TrackConfig::new(entry, 8)).unwrap();
            writer
                .write_sample(
                    track,
                    WriteSample {
                        data: &[0; 4],
                        duration: 8,
                        composition_offset: 0,
                        is_sync: true,
                    },
                )
                .unwrap();
            let bytes = writer.finish().unwrap().into_inner();
            assert_eq!(
                waveform_reader(&request(), &info(), &AtomicBool::new(false), &bytes),
                Err(TimelineError::Unsupported)
            );
        }
    }
}
