use super::*;
use filmcraft_h264enc::{Encoder, EncoderConfig, PacketFormat, RateControl, YuvFrame};
use filmcraft_isobmff::{
    AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions,
};
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
const FPS: u32 = 30;
const FRAMES: u32 = 30;

/// Brightness of synthetic frame `index`: every frame is visibly different from its neighbours.
fn brightness(index: u32) -> u8 {
    (24 + index * 7).min(250) as u8
}

/// Encodes a one-second flat-gray clip whose brightness rises every frame, with the pure-Rust encoder.
pub(crate) fn make_clip() -> Vec<u8> {
    build_clip(false)
}

/// The same clip with one second of 440 Hz sound: 16-bit mono PCM at 48 kHz.
pub(crate) fn make_clip_with_sound() -> Vec<u8> {
    build_clip(true)
}

/// The same clip with one second of sound per entry: a sine tone at each pitch, as separate audio tracks.
pub(crate) fn make_clip_with_tracks(tones: &[f64]) -> Vec<u8> {
    build_clip_with(tones)
}

const SOUND_RATE: u32 = 48_000;
const TONE_HZ: f64 = 440.0;

fn build_clip(sound: bool) -> Vec<u8> {
    build_clip_with(if sound { &[TONE_HZ] } else { &[] })
}

fn build_clip_with(tones: &[f64]) -> Vec<u8> {
    let mut config = EncoderConfig::new(WIDTH, HEIGHT, FPS, 1);
    config.rate = RateControl::Crf(10.0);
    config.bframes = 0;
    config.keyint = 10;
    config.format = PacketFormat::LengthPrefixed;
    let mut encoder = Encoder::new(config).expect("encoder");
    let (sps, pps) = encoder.sps_pps();

    let mut writer =
        Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).expect("writer");
    let entry = SampleEntry::avc(
        AvcConfig::new(vec![strip(&sps)], vec![strip(&pps)], 4),
        WIDTH as u16,
        HEIGHT as u16,
    );
    let track = writer
        .add_track(TrackConfig::new(entry, FPS))
        .expect("track");
    let sound_tracks: Vec<usize> = tones
        .iter()
        .map(|_| {
            let pcm = filmcraft_isobmff::PcmConfig {
                bits: 16,
                float: false,
                big_endian: false,
                signed: true,
                channels: 1,
                sample_rate: f64::from(SOUND_RATE),
            };
            writer
                .add_track(TrackConfig::new(SampleEntry::pcm(pcm), SOUND_RATE))
                .expect("sound track")
        })
        .collect();

    let mut write = |packets: Vec<filmcraft_h264enc::Packet>| {
        for packet in packets {
            writer
                .write_sample(
                    track,
                    WriteSample {
                        data: &packet.data,
                        duration: 1,
                        composition_offset: i32::try_from(packet.pts - packet.dts).unwrap_or(0),
                        is_sync: packet.keyframe,
                    },
                )
                .expect("sample");
        }
    };
    for index in 0..FRAMES {
        let luma = vec![brightness(index); (WIDTH * HEIGHT) as usize];
        let chroma = vec![128_u8; (WIDTH * HEIGHT / 4) as usize];
        let frame = YuvFrame {
            y: &luma,
            u: &chroma,
            v: &chroma,
            y_stride: WIDTH as usize,
            uv_stride: (WIDTH / 2) as usize,
        };
        write(encoder.encode(&frame, i64::from(index)).expect("encode"));
    }
    write(encoder.flush());
    for (&sound_track, &tone) in sound_tracks.iter().zip(tones) {
        // One second of tone in 0.1 s chunks.
        for chunk in 0..10_u32 {
            let bytes: Vec<u8> = (0..SOUND_RATE / 10)
                .flat_map(|n| {
                    let index = f64::from(chunk * (SOUND_RATE / 10) + n);
                    let value =
                        (index * tone * std::f64::consts::TAU / f64::from(SOUND_RATE)).sin();
                    ((value * 12_000.0) as i16).to_le_bytes()
                })
                .collect();
            writer
                .write_sample(
                    sound_track,
                    WriteSample {
                        data: &bytes,
                        duration: 0,
                        composition_offset: 0,
                        is_sync: true,
                    },
                )
                .expect("sound sample");
        }
    }
    writer.finish().expect("finish").into_inner()
}

/// The encoder may return parameter sets with Annex-B start codes; the MP4 config wants bare NALs.
fn strip(nal: &[u8]) -> Vec<u8> {
    let trimmed = nal
        .strip_prefix(&[0, 0, 0, 1])
        .or_else(|| nal.strip_prefix(&[0, 0, 1]))
        .unwrap_or(nal);
    trimmed.to_vec()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "clipcraft-player-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("scratch directory");
        Self(path)
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).expect("fixture writes");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Collects events until `done` says to stop or ten seconds pass.
fn collect(player: &Player, mut done: impl FnMut(&[PlayerEvent]) -> bool) -> Vec<PlayerEvent> {
    let mut all = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        all.extend(player.poll());
        if done(&all) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    all
}

fn frames(events: &[PlayerEvent]) -> Vec<&DecodedFrame> {
    events
        .iter()
        .filter_map(|event| match event {
            PlayerEvent::Frame(frame) => Some(frame),
            _ => None,
        })
        .collect()
}

fn mean_red(frame: &DecodedFrame) -> f64 {
    let reds: Vec<u64> = frame
        .rgba
        .chunks_exact(4)
        .map(|pixel| u64::from(pixel[0]))
        .collect();
    reds.iter().sum::<u64>() as f64 / reds.len() as f64
}

#[test]
fn cancellation_acknowledges_both_video_and_audio_worker_retirement() {
    let scratch = Scratch::new();
    let path = scratch.file("retire.mp4", &make_clip_with_sound());
    let player = Player::start(path, || {});
    let events = collect(&player, |events| !frames(events).is_empty());
    assert!(!frames(&events).is_empty());
    assert!(!player.worker_finished());
    let feed = player.audio_feed().unwrap();
    player.request_stop();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !player.worker_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(player.worker_finished());
    assert!(feed.worker_finished());
}

#[test]
fn editor_resume_keeps_the_final_source_frame_instead_of_restarting_at_zero() {
    let scratch = Scratch::new();
    let path = scratch.file("last-frame.mp4", &make_clip());
    let player = Player::start(path, || {});
    collect(&player, |events| !frames(events).is_empty());
    let target = Tick::from_units(29, 30);
    player.seek(target);
    let seeked = collect(&player, |events| {
        frames(events).iter().any(|frame| frame.pts == target)
    });
    assert!(frames(&seeked).iter().any(|frame| frame.pts == target));
    player.play_from_current();
    let played = collect(&player, |events| {
        events.iter().any(|e| matches!(e, PlayerEvent::Ended))
    });
    assert!(played.iter().any(|e| matches!(e, PlayerEvent::Ended)));
    assert!(frames(&played).iter().all(|frame| frame.pts >= target));
    player.request_stop();
}

#[test]
fn opens_a_clip_reports_its_facts_and_shows_the_first_frame() {
    let scratch = Scratch::new();
    let path = scratch.file("clip.mp4", &make_clip());
    let player = Player::start(path, || {});

    let events = collect(&player, |events| !frames(events).is_empty());

    let Some(PlayerEvent::Opened(info)) = events.first() else {
        panic!("first event should be Opened, got {events:?}");
    };
    assert_eq!((info.width, info.height), (WIDTH, HEIGHT));
    assert_eq!(info.frame_rate(), FrameRate::FPS_30);
    assert!((info.duration.seconds() - 1.0).abs() < 0.05);
    assert!(!info.has_audio);
    let first = frames(&events)[0];
    assert_eq!(first.pts, Tick::ZERO);
    assert_eq!(first.rgba.len(), (WIDTH * HEIGHT * 4) as usize);
}

#[test]
fn seeking_shows_the_picture_at_that_time_and_stays_paused() {
    let scratch = Scratch::new();
    let player = Player::start(scratch.file("clip.mp4", &make_clip()), || {});
    let first = collect(&player, |events| !frames(events).is_empty());
    let early = mean_red(frames(&first)[0]);

    player.seek(Tick::from_seconds_f64(0.8));
    let after = collect(&player, |events| !frames(events).is_empty());
    let late_frame = frames(&after)[0];
    assert!(
        late_frame.pts.seconds() > 0.7 && late_frame.pts.seconds() < 0.9,
        "pts was {}",
        late_frame.pts.seconds()
    );
    assert!(
        mean_red(late_frame) > early + 40.0,
        "later frame should be brighter"
    );

    // Paused: nothing else arrives on its own.
    thread::sleep(Duration::from_millis(300));
    assert!(frames(&player.poll()).is_empty());
}

#[test]
fn playing_advances_in_order_and_stops_at_the_end() {
    let scratch = Scratch::new();
    let player = Player::start(scratch.file("clip.mp4", &make_clip()), || {});
    collect(&player, |events| !frames(events).is_empty());

    player.play();
    let events = collect(&player, |events| events.contains(&PlayerEvent::Ended));

    assert!(
        events.contains(&PlayerEvent::Ended),
        "playback should reach the end"
    );
    let shown = frames(&events);
    assert!(
        shown.len() >= 3,
        "expected several frames, got {}",
        shown.len()
    );
    assert!(
        shown.windows(2).all(|pair| pair[0].pts <= pair[1].pts),
        "frames must not go backwards"
    );
    assert!(mean_red(shown[shown.len() - 1]) > mean_red(shown[0]));

    // Playing again from the end starts over at the first frame.
    player.play();
    let restarted = collect(&player, |events| !frames(events).is_empty());
    assert_eq!(frames(&restarted)[0].pts, Tick::ZERO);
}

#[test]
fn unreadable_and_unsupported_files_fail_cleanly() {
    let scratch = Scratch::new();

    let missing = Player::start(scratch.0.join("missing.mp4"), || {});
    let events = collect(&missing, |events| !events.is_empty());
    assert_eq!(events, [PlayerEvent::Failed(PlayerError::Unreadable)]);

    let garbage = Player::start(
        scratch.file("garbage.mp4", b"definitely not a video"),
        || {},
    );
    let events = collect(&garbage, |events| !events.is_empty());
    assert!(
        matches!(
            events.as_slice(),
            [PlayerEvent::Failed(
                PlayerError::Unsupported | PlayerError::DecodeFailed
            )]
        ),
        "got {events:?}"
    );

    let empty = Player::start(scratch.file("empty.mp4", b""), || {});
    let events = collect(&empty, |events| !events.is_empty());
    assert!(
        matches!(events.as_slice(), [PlayerEvent::Failed(_)]),
        "got {events:?}"
    );
}

#[test]
fn damaged_streams_never_panic_the_caller() {
    let scratch = Scratch::new();
    let good = make_clip();
    // Corrupt bytes spread across the file, including the picture data, then try to play each.
    for (number, position) in (0..good.len())
        .step_by((good.len() / 24).max(1))
        .enumerate()
    {
        let mut damaged = good.clone();
        damaged[position] ^= 0xFF;
        let path = scratch.file(&format!("damaged-{number}.mp4"), &damaged);
        let player = Player::start(path, || {});
        player.play();
        // Every damaged file must produce a picture or a failure promptly. A corrupted header may
        // legitimately claim a long duration, so reaching the end of playback is not required.
        let started = Instant::now();
        let events = collect(&player, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    PlayerEvent::Failed(_) | PlayerEvent::Ended | PlayerEvent::Frame(_)
                )
            })
        });
        assert!(
            events
                .iter()
                .any(|event| !matches!(event, PlayerEvent::Opened(_))),
            "byte {position} left the player silent: {events:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "byte {position} took too long"
        );
    }
}

#[test]
fn notify_is_called_for_every_event() {
    let scratch = Scratch::new();
    let count = Arc::new(AtomicU64::new(0));
    let counter = count.clone();
    let player = Player::start(scratch.file("clip.mp4", &make_clip()), move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let events = collect(&player, |events| !frames(events).is_empty());
    assert!(count.load(Ordering::SeqCst) >= events.len() as u64);
}

mod thumbnails {
    use super::*;
    use crate::thumbnail::{self, ThumbnailError};

    #[test]
    fn a_thumbnail_is_generated_cached_and_reloaded() {
        let scratch = Scratch::new();
        let clip = scratch.file("clip.mp4", &make_clip());
        let cache = scratch.0.join("thumbs");

        let first = thumbnail::ensure_cached(&clip, &cache).expect("thumbnail");
        assert!(first.created);
        assert!(first.file.starts_with(&cache));
        assert!(std::fs::metadata(&first.file).expect("cache file").len() > 0);

        // Second call reuses the cache file without decoding again.
        let second = thumbnail::ensure_cached(&clip, &cache).expect("cached");
        assert!(!second.created);
        assert_eq!(second.file, first.file);

        let loaded = thumbnail::load(&first.file).expect("load");
        // The 64x48 test clip is narrower than the thumbnail width, so it keeps its size.
        assert_eq!((loaded.width, loaded.height), (WIDTH, HEIGHT));
        assert_eq!(loaded.rgba.len(), (WIDTH * HEIGHT * 4) as usize);
        // About a tenth in: frame 3 of 30, brightness 24 + 3 * 7 = 45 (limited-range gray).
        let mean = loaded
            .rgba
            .chunks_exact(4)
            .map(|p| f64::from(p[0]))
            .sum::<f64>()
            / f64::from(WIDTH * HEIGHT);
        assert!(
            mean > 30.0 && mean < 80.0,
            "mean red {mean} is not an early, dark frame"
        );
        assert!(loaded.rgba.chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn replacing_a_clip_gets_a_new_thumbnail() {
        let scratch = Scratch::new();
        let clip = scratch.file("clip.mp4", &make_clip());
        let cache = scratch.0.join("thumbs");
        let before = thumbnail::ensure_cached(&clip, &cache).expect("thumbnail");

        // Same path, different size: the key changes, so the old picture is not reused.
        let mut longer = make_clip();
        longer.extend_from_slice(&[0; 16]);
        std::fs::write(&clip, longer).expect("rewrite clip");
        let after = thumbnail::ensure_cached(&clip, &cache).expect("thumbnail");
        assert!(after.created);
        assert_ne!(after.file, before.file);
    }

    #[test]
    fn bad_clips_report_errors_and_leave_no_cache_files() {
        let scratch = Scratch::new();
        let cache = scratch.0.join("thumbs");
        assert_eq!(
            thumbnail::ensure_cached(&scratch.0.join("missing.mp4"), &cache),
            Err(ThumbnailError::Unreadable)
        );
        let garbage = scratch.file("garbage.mp4", b"not a video");
        assert!(thumbnail::ensure_cached(&garbage, &cache).is_err());
        assert!(thumbnail::ensure_cached(&scratch.0, &cache).is_err());
        let leftovers = std::fs::read_dir(&cache).map_or(0, Iterator::count);
        assert_eq!(leftovers, 0, "failures must not leave cache files");
    }

    #[test]
    fn damaged_clips_never_panic_the_thumbnailer() {
        let scratch = Scratch::new();
        let cache = scratch.0.join("thumbs");
        let good = make_clip();
        for (number, position) in (0..good.len())
            .step_by((good.len() / 24).max(1))
            .enumerate()
        {
            let mut damaged = good.clone();
            damaged[position] ^= 0xFF;
            let path = scratch.file(&format!("damaged-{number}.mp4"), &damaged);
            // Either a thumbnail or a clean error; never a panic or a hang.
            let _ = thumbnail::ensure_cached(&path, &cache);
        }
    }

    #[test]
    fn unreadable_cache_files_are_refused() {
        let scratch = Scratch::new();
        assert_eq!(
            thumbnail::load(&scratch.0.join("missing.jpg")),
            Err(ThumbnailError::Unreadable)
        );
        let fake = scratch.file("fake.jpg", b"not a jpeg");
        assert_eq!(thumbnail::load(&fake), Err(ThumbnailError::DecodeFailed));
        let huge = scratch.file("huge.jpg", &vec![0; 2 * 1024 * 1024]);
        assert_eq!(thumbnail::load(&huge), Err(ThumbnailError::Unreadable));
    }

    #[test]
    fn pruning_removes_only_the_oldest_jpg_files() {
        let scratch = Scratch::new();
        let cache = scratch.0.join("thumbs");
        std::fs::create_dir_all(&cache).expect("cache dir");
        for number in 0..20 {
            std::fs::write(cache.join(format!("{number:02}.jpg")), b"x").expect("jpg");
            // Distinct modified times so "oldest" is well defined.
            std::thread::sleep(Duration::from_millis(15));
        }
        std::fs::write(cache.join("keep.txt"), b"not a thumbnail").expect("txt");

        thumbnail::prune(&cache, 10);

        let mut left: Vec<String> = std::fs::read_dir(&cache)
            .expect("read cache")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert!(
            left.contains(&"keep.txt".to_owned()),
            "non-jpg files are never deleted"
        );
        let jpgs: Vec<_> = left.iter().filter(|name| name.ends_with(".jpg")).collect();
        assert_eq!(jpgs.len(), 9, "keeps max minus 10% headroom: {jpgs:?}");
        assert!(
            jpgs.iter().all(|name| name.as_str() >= "11.jpg"),
            "oldest go first: {jpgs:?}"
        );
    }
}

mod sound {
    use super::*;
    use crate::audio::AudioFeed;
    use std::sync::atomic::AtomicBool;

    /// A stand-in for a sound card: pulls 10 ms of stereo at 48 kHz every 10 ms, like a callback.
    struct FakeDevice {
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<Vec<f32>>>,
    }

    impl FakeDevice {
        fn start(feed: Arc<AudioFeed>) -> Self {
            assert!(feed.configure_output(SOUND_RATE, 2));
            feed.set_output_active(true);
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let thread = thread::spawn(move || {
                let mut heard = Vec::new();
                let mut buffer = vec![0.0_f32; 480 * 2];
                let mut next = Instant::now();
                while !flag.load(Ordering::SeqCst) {
                    feed.fill(&mut buffer);
                    heard.extend_from_slice(&buffer);
                    next += Duration::from_millis(10);
                    thread::sleep(next.saturating_duration_since(Instant::now()));
                }
                heard
            });
            Self {
                stop,
                thread: Some(thread),
            }
        }

        fn stop(mut self) -> Vec<f32> {
            self.stop.store(true, Ordering::SeqCst);
            self.thread
                .take()
                .expect("device thread")
                .join()
                .expect("join")
        }
    }

    /// Estimated frequency of the left channel from rising zero crossings.
    fn frequency(stereo: &[f32]) -> f64 {
        let left: Vec<f32> = stereo.iter().step_by(2).copied().collect();
        let crossings = left
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        crossings as f64 / (left.len() as f64 / f64::from(SOUND_RATE))
    }

    fn open_with_feed(scratch: &Scratch) -> (Player, Arc<AudioFeed>) {
        let player = Player::start(scratch.file("clip.mp4", &make_clip_with_sound()), || {});
        let events = collect(&player, |events| {
            events.iter().any(|e| matches!(e, PlayerEvent::Opened(_)))
        });
        let Some(PlayerEvent::Opened(info)) = events.first() else {
            panic!("expected Opened, got {events:?}");
        };
        assert!(info.has_audio);
        let feed = player.audio_feed().expect("a clip with sound has a feed");
        (player, feed)
    }

    #[test]
    fn a_clip_without_sound_has_no_feed() {
        let scratch = Scratch::new();
        let player = Player::start(scratch.file("clip.mp4", &make_clip()), || {});
        collect(&player, |events| !frames(events).is_empty());
        assert!(player.audio_feed().is_none());
    }

    #[test]
    fn the_tone_is_decoded_played_at_the_right_pitch_and_the_picture_follows_it() {
        let scratch = Scratch::new();
        let (player, feed) = open_with_feed(&scratch);
        let device = FakeDevice::start(feed.clone());

        player.play();
        let events = collect(&player, |events| events.contains(&PlayerEvent::Ended));
        assert!(events.contains(&PlayerEvent::Ended));
        let heard = device.stop();

        // Real sound reached the device, not silence.
        let loud = heard.iter().filter(|s| s.abs() > 0.1).count();
        assert!(
            loud > heard.len() / 3,
            "only {loud} of {} samples were audible",
            heard.len()
        );
        assert!(
            heard.iter().all(|s| s.abs() <= 1.0),
            "samples must stay in range"
        );
        // The pitch is the encoded 440 Hz (measured on the audible stretch).
        let first = heard
            .iter()
            .position(|s| s.abs() > 0.1)
            .expect("sound starts");
        let tone = &heard[first - first % 2..];
        let pitch = frequency(&tone[..(SOUND_RATE as usize * 2 / 2).min(tone.len())]);
        assert!((pitch - TONE_HZ).abs() < 15.0, "pitch was {pitch} Hz");
        // Mono sound is spread to both channels.
        assert!(heard.chunks_exact(2).all(|f| (f[0] - f[1]).abs() < 1e-6));
        // About one second of sound was consumed in total.
        let seconds = feed.position().seconds();
        assert!(seconds > 0.85 && seconds < 1.1, "clock reached {seconds}");
        // The picture followed the sound clock and ended on the last frame.
        let shown = frames(&events);
        assert!(shown.len() >= 3);
        assert!(shown.windows(2).all(|pair| pair[0].pts <= pair[1].pts));
        assert!(shown[shown.len() - 1].pts.seconds() > 0.9);
    }

    #[test]
    fn seeking_while_playing_moves_the_sound_and_the_picture_together() {
        let scratch = Scratch::new();
        let (player, feed) = open_with_feed(&scratch);
        let device = FakeDevice::start(feed.clone());
        player.play();
        collect(&player, |events| frames(events).len() >= 2);

        player.seek(Tick::from_seconds_f64(0.6));
        let events = collect(&player, |events| {
            frames(events).iter().any(|f| f.pts.seconds() >= 0.6)
        });
        let landed = frames(&events)
            .iter()
            .find(|f| f.pts.seconds() >= 0.6)
            .map(|f| f.pts.seconds())
            .expect("a frame at or after the seek target");
        assert!(landed < 0.8, "picture landed at {landed}");
        let clock = feed.position().seconds();
        assert!((0.6..0.95).contains(&clock), "sound clock {clock}");
        drop(device.stop());
    }

    #[test]
    fn playback_finishes_on_the_wall_clock_when_no_device_ever_runs() {
        let scratch = Scratch::new();
        let (player, feed) = open_with_feed(&scratch);
        assert!(!feed.driving());
        player.play();
        let events = collect(&player, |events| events.contains(&PlayerEvent::Ended));
        assert!(
            events.contains(&PlayerEvent::Ended),
            "must not wait for a device that is absent"
        );
    }

    #[test]
    fn a_device_that_fails_mid_play_hands_the_clock_back_and_playback_still_ends() {
        let scratch = Scratch::new();
        let (player, feed) = open_with_feed(&scratch);
        let device = FakeDevice::start(feed.clone());
        player.play();
        collect(&player, |events| frames(events).len() >= 2);

        // The device dies: its callbacks stop and it reports itself inactive.
        feed.set_output_active(false);
        drop(device.stop());
        let events = collect(&player, |events| events.contains(&PlayerEvent::Ended));
        assert!(events.contains(&PlayerEvent::Ended));
    }

    #[test]
    fn a_device_that_starts_late_picks_up_at_the_current_position() {
        let scratch = Scratch::new();
        let (player, feed) = open_with_feed(&scratch);
        player.play();
        thread::sleep(Duration::from_millis(250));

        let device = FakeDevice::start(feed.clone());
        thread::sleep(Duration::from_millis(250));
        let clock = feed.position().seconds();
        drop(device.stop());
        // The sound joined at roughly where the picture already was (0.25 s in), not at zero.
        assert!(
            clock > 0.3,
            "sound clock {clock} did not join the running playback"
        );
    }
}

mod controls {
    use super::*;
    use crate::audio::AudioFeed;
    use std::sync::atomic::AtomicBool;

    /// Opens the clip and waits until both `Opened` and the first picture have arrived, so a
    /// test starts from a known, quiet state (paused on the first picture, nothing pending).
    fn open_player(scratch: &Scratch, clip: &[u8]) -> (Player, ClipInfoSummary) {
        let player = Player::start(scratch.file("clip.mp4", clip), || {});
        let events = collect(&player, |events| {
            events.iter().any(|e| matches!(e, PlayerEvent::Opened(_))) && !frames(events).is_empty()
        });
        let info = events
            .into_iter()
            .find_map(|event| match event {
                PlayerEvent::Opened(info) => Some(info),
                _ => None,
            })
            .expect("clip did not open");
        (player, info)
    }

    /// Waits for the next picture after `skip` already collected ones and returns its time.
    fn next_pts(player: &Player) -> Tick {
        let events = collect(player, |events| !frames(events).is_empty());
        frames(&events)[0].pts
    }

    /// A stand-in sound card (same as in the sound tests) that records what it was given.
    fn record(feed: Arc<AudioFeed>) -> (Arc<AtomicBool>, thread::JoinHandle<Vec<f32>>) {
        assert!(feed.configure_output(SOUND_RATE, 2));
        feed.set_output_active(true);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = thread::spawn(move || {
            let mut heard = Vec::new();
            let mut buffer = vec![0.0_f32; 480 * 2];
            let mut next = Instant::now();
            while !flag.load(Ordering::SeqCst) {
                feed.fill(&mut buffer);
                heard.extend_from_slice(&buffer);
                next += Duration::from_millis(10);
                thread::sleep(next.saturating_duration_since(Instant::now()));
            }
            heard
        });
        (stop, handle)
    }

    fn pitch_of_audible(stereo: &[f32]) -> f64 {
        let loud: Vec<f32> = stereo
            .chunks_exact(2)
            .map(|f| f[0])
            .skip_while(|s| s.abs() < 0.05)
            .collect();
        // Trim the ends, which carry silence and window ramps.
        let trim = (SOUND_RATE / 20) as usize;
        let body: Vec<f32> = loud.iter().copied().filter(|s| s.abs() >= 0.0).collect();
        let body = &body[trim.min(body.len())..body.len().saturating_sub(trim)];
        let crossings = body
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        crossings as f64 / (body.len() as f64 / f64::from(SOUND_RATE))
    }

    #[test]
    fn frame_stepping_lands_exactly_on_picture_boundaries_and_clamps() {
        let scratch = Scratch::new();
        let (player, info) = open_player(&scratch, &make_clip());
        let rate = info.frame_rate();
        player.step(1);
        assert_eq!(next_pts(&player), rate.tick_of(1));
        player.step(3);
        assert_eq!(next_pts(&player), rate.tick_of(4));
        player.step(-2);
        assert_eq!(next_pts(&player), rate.tick_of(2));
        player.step(-100);
        assert_eq!(
            next_pts(&player),
            Tick::ZERO,
            "stepping back stops at the first picture"
        );
        player.step(10_000);
        let last = next_pts(&player);
        assert_eq!(
            last,
            rate.tick_of(i64::from(FRAMES) - 1),
            "stepping forward stops at the last picture"
        );
        // Stepping leaves playback paused: nothing else arrives by itself.
        thread::sleep(Duration::from_millis(300));
        assert!(frames(&player.poll()).is_empty());
    }

    #[test]
    fn stepping_while_playing_pauses_on_the_next_picture() {
        let scratch = Scratch::new();
        let (player, _) = open_player(&scratch, &make_clip());
        player.play();
        collect(&player, |events| frames(events).len() >= 3);
        player.step(1);
        thread::sleep(Duration::from_millis(300));
        let after = player.poll();
        // Pictures already in flight may arrive, but playback then stops.
        thread::sleep(Duration::from_millis(300));
        assert!(
            frames(&player.poll()).is_empty(),
            "still playing after a step: {after:?}"
        );
    }

    #[test]
    fn double_speed_finishes_in_about_half_the_time_and_slow_motion_takes_longer() {
        let scratch = Scratch::new();
        let (player, _) = open_player(&scratch, &make_clip());

        player.set_speed(2.0);
        let started = Instant::now();
        player.play();
        collect(&player, |events| events.contains(&PlayerEvent::Ended));
        let fast = started.elapsed().as_secs_f64();
        assert!(fast > 0.3 && fast < 0.9, "2x took {fast} s for a 1 s clip");

        player.set_speed(0.5);
        let started = Instant::now();
        player.play();
        collect(&player, |events| events.contains(&PlayerEvent::Ended));
        let slow = started.elapsed().as_secs_f64();
        assert!(
            slow > 1.5 && slow < 3.0,
            "0.5x took {slow} s for a 1 s clip"
        );
    }

    #[test]
    fn changing_speed_mid_play_does_not_jump_the_position() {
        let scratch = Scratch::new();
        let (player, _) = open_player(&scratch, &make_clip());
        player.set_speed(0.25);
        player.play();
        let before = collect(&player, |events| frames(events).len() >= 2);
        let at = frames(&before).last().expect("frame").pts;
        player.set_speed(4.0);
        let after = collect(&player, |events| frames(events).len() >= 2);
        let resumed = frames(&after)[0].pts;
        assert!(
            resumed >= at,
            "position went backwards: {at:?} then {resumed:?}"
        );
        assert!(
            resumed.seconds() - at.seconds() < 0.3,
            "position jumped from {} to {}",
            at.seconds(),
            resumed.seconds()
        );
    }

    #[test]
    fn looping_restarts_instead_of_ending_and_turning_it_off_ends_normally() {
        let scratch = Scratch::new();
        let (player, _) = open_player(&scratch, &make_clip());
        player.set_loop(true);
        player.play();

        // A 1 s clip played for 2.6 s must wrap at least twice, and never report the end.
        let started = Instant::now();
        let mut pts = Vec::new();
        let mut ended = false;
        while started.elapsed() < Duration::from_millis(2600) {
            for event in player.poll() {
                match event {
                    PlayerEvent::Frame(frame) => pts.push(frame.pts),
                    PlayerEvent::Ended => ended = true,
                    _ => {}
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!ended, "a looping clip must not end");
        let wraps = pts.windows(2).filter(|pair| pair[1] < pair[0]).count();
        assert!(wraps >= 2, "expected at least two wraps, saw {wraps}");

        player.set_loop(false);
        let events = collect(&player, |events| events.contains(&PlayerEvent::Ended));
        assert!(
            events.contains(&PlayerEvent::Ended),
            "with looping off it ends at the end"
        );
    }

    #[test]
    fn faster_and_slower_speeds_keep_the_pitch_through_the_whole_player() {
        for (speed, seconds) in [(2.0, 0.5), (0.5, 2.0)] {
            let scratch = Scratch::new();
            let (player, _) = open_player(&scratch, &make_clip_with_sound());
            let feed = player.audio_feed().expect("feed");
            let (stop, device) = record(feed.clone());
            player.set_speed(speed);
            player.play();
            collect(&player, |events| events.contains(&PlayerEvent::Ended));
            stop.store(true, Ordering::SeqCst);
            let heard = device.join().expect("device");

            let audible = heard.chunks_exact(2).filter(|f| f[0].abs() > 0.02).count();
            let audible_seconds = audible as f64 / f64::from(SOUND_RATE);
            assert!(
                (audible_seconds - seconds).abs() < 0.3,
                "{speed}x: {audible_seconds} s of sound, expected about {seconds}"
            );
            let measured = pitch_of_audible(&heard);
            assert!(
                (measured - TONE_HZ).abs() < 25.0,
                "{speed}x changed the pitch to {measured} Hz"
            );
        }
    }

    #[test]
    fn selecting_an_audio_track_switches_what_is_heard() {
        let scratch = Scratch::new();
        let (player, info) = open_player(&scratch, &make_clip_with_tracks(&[440.0, 880.0]));
        assert_eq!(
            info.audio_tracks.len(),
            2,
            "labels: {:?}",
            info.audio_tracks
        );
        assert!(info.audio_tracks[0].contains("Track 1"));
        let feed = player.audio_feed().expect("feed");

        let (stop, device) = record(feed.clone());
        player.select_audio(1);
        player.play();
        collect(&player, |events| events.contains(&PlayerEvent::Ended));
        stop.store(true, Ordering::SeqCst);
        let heard = device.join().expect("device");
        let measured = pitch_of_audible(&heard);
        assert!(
            (measured - 880.0).abs() < 25.0,
            "track 2 should sound at 880 Hz, got {measured}"
        );

        // An out-of-range track is ignored rather than breaking playback.
        player.select_audio(99);
        player.play();
        let events = collect(&player, |events| events.contains(&PlayerEvent::Ended));
        assert!(events.contains(&PlayerEvent::Ended));
    }

    #[test]
    fn a_device_that_stops_pulling_without_an_error_does_not_freeze_playback() {
        let scratch = Scratch::new();
        let (player, _) = open_player(&scratch, &make_clip_with_sound());
        let feed = player.audio_feed().expect("feed");
        // "Active" but nobody ever calls fill: a stalled driver or a suspended machine.
        assert!(feed.configure_output(SOUND_RATE, 2));
        feed.set_output_active(true);

        let started = Instant::now();
        player.play();
        let events = collect(&player, |events| events.contains(&PlayerEvent::Ended));
        assert!(
            events.contains(&PlayerEvent::Ended),
            "playback froze behind a dead sound clock"
        );
        let took = started.elapsed().as_secs_f64();
        // About 1.5 s of stall detection plus the clip itself; far less than the 10 s timeout.
        assert!(took > 1.0 && took < 6.0, "took {took} s");
    }

    #[test]
    fn a_stalled_device_that_wakes_up_gets_the_sound_back_in_sync() {
        let scratch = Scratch::new();
        let (player, _) = open_player(&scratch, &make_clip_with_sound());
        player.set_loop(true);
        let feed = player.audio_feed().expect("feed");
        assert!(feed.configure_output(SOUND_RATE, 2));
        feed.set_output_active(true);
        player.play();
        // Let the stall be detected and playback continue on the wall clock.
        collect(&player, |events| frames(events).len() >= 3);
        thread::sleep(Duration::from_millis(1800));

        // The device comes back and starts pulling.
        let (stop, device) = record(feed.clone());
        thread::sleep(Duration::from_millis(600));
        stop.store(true, Ordering::SeqCst);
        let heard = device.join().expect("device");
        feed.set_output_active(false);
        let audible = heard.chunks_exact(2).filter(|f| f[0].abs() > 0.05).count();
        assert!(
            audible > 5_000,
            "only {audible} audible frames after the device woke up"
        );
    }
}

#[test]
fn an_unpolled_player_reaches_the_end_with_only_its_final_picture_pending() {
    let scratch = Scratch::new();
    let player = Player::start(scratch.file("stalled-ui.mp4", &make_clip()), || {});
    player.play();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !player.events.ended() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(player.events.ended(), "unpolled playback did not finish");
    let events = player.poll();
    assert_eq!(
        events.len(),
        3,
        "only Opened/latest picture/Ended may be retained"
    );
    assert!(matches!(events[0], PlayerEvent::Opened(_)));
    let PlayerEvent::Frame(frame) = &events[1] else {
        panic!("final picture missing");
    };
    assert_eq!(frame.pts, FrameRate::FPS_30.tick_of(i64::from(FRAMES - 1)));
    assert_eq!(events[2], PlayerEvent::Ended);
    assert!(player.poll().is_empty());
    player.seek(Tick::from_units(10, 30));
    let sought = collect(&player, |events| !frames(events).is_empty());
    assert_eq!(frames(&sought)[0].pts, Tick::from_units(10, 30));
    assert!(!sought.contains(&PlayerEvent::Ended));
    player.request_stop();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !player.worker_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(player.worker_finished());
}

#[test]
fn picture_dimensions_are_bounded_before_rgba_conversion() {
    for (width, height) in [
        (0, 1),
        (1, 0),
        (u32::MAX, u32::MAX),
        (8193, 1),
        (7680, 4321),
    ] {
        assert_eq!(frame_bytes(width, height), Err(PlayerError::Unsupported));
    }
    assert_eq!(frame_bytes(7680, 4320), Ok(132_710_400));
    assert_eq!(frame_bytes(4320, 7680), Ok(132_710_400));
    assert_eq!(frame_bytes(64, 48), Ok(64 * 48 * 4));
}

#[test]
fn playback_and_sound_finish_while_thumbnail_and_waveform_workers_share_admission() {
    let scratch = Scratch::new();
    let path = scratch.file("priorities.mp4", &make_clip_with_sound());
    let original = std::fs::read(&path).unwrap();
    let player = Player::start(path.clone(), || {});
    player.play();
    let background: Vec<_> = (0..3)
        .map(|_| {
            let path = path.clone();
            thread::spawn(move || {
                for _ in 0..3 {
                    let thumbnail = thumbnail::generate(&path).unwrap();
                    assert_eq!(thumbnail.width, WIDTH);
                    let analysis = timeline::analyze_timeline(
                        &timeline::TimelineRequest {
                            path: path.clone(),
                            start: Tick::ZERO,
                            end: Tick::from_units(1, 1),
                            frames: 4,
                            buckets: 64,
                        },
                        &std::sync::atomic::AtomicBool::new(false),
                    )
                    .unwrap();
                    assert_eq!(analysis.frames.len(), 4);
                    assert_eq!(analysis.waveform.len(), 64);
                    assert!(analysis.has_audio);
                }
            })
        })
        .collect();
    let events = collect(&player, |events| events.contains(&PlayerEvent::Ended));
    assert!(events.contains(&PlayerEvent::Ended));
    assert!(frames(&events).len() >= 2);
    for worker in background {
        worker.join().unwrap();
    }
    assert_eq!(std::fs::read(&path).unwrap(), original);
    player.request_stop();
}

#[test]
fn optional_frame_processing_failures_and_malformed_output_do_not_publish_or_touch_sources() {
    for malformed in [false, true] {
        let scratch = Scratch::new();
        let original = make_clip();
        let path = scratch.file("renderer.mp4", &original);
        let player = Player::start_transformed(
            path.clone(),
            || {},
            move |mut frame| {
                if malformed {
                    frame.rgba.clear();
                    Ok(frame)
                } else {
                    Err(PlayerError::RenderFailed)
                }
            },
        );
        let events = collect(&player, |events| {
            events
                .iter()
                .any(|event| matches!(event, PlayerEvent::Failed(_)))
        });
        assert!(events.contains(&PlayerEvent::Failed(PlayerError::RenderFailed)));
        assert!(frames(&events).is_empty());
        assert_eq!(std::fs::read(path).unwrap(), original);
        player.request_stop();
    }
}

#[test]
fn a_superseded_render_failure_does_not_reject_a_newer_seek_recipe() {
    let scratch = Scratch::new();
    let path = scratch.file("stale-render.mp4", &make_clip());
    let (ready, waiting) = mpsc::sync_channel(1);
    let release = Arc::new(std::sync::Barrier::new(2));
    let worker_release = release.clone();
    let mut first = true;
    let player = Player::start_transformed(
        path,
        || {},
        move |frame| {
            if first {
                first = false;
                ready.send(()).unwrap();
                worker_release.wait();
                Err(PlayerError::RenderFailed)
            } else {
                Ok(frame)
            }
        },
    );
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    let target = Tick::from_units(1, 2);
    player.seek(target);
    release.wait();
    let events = collect(&player, |events| {
        frames(events).iter().any(|frame| frame.pts == target)
    });
    assert!(frames(&events).iter().any(|frame| frame.pts == target));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, PlayerEvent::Failed(_)))
    );
    player.request_stop();
}
