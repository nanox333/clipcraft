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
fn make_clip() -> Vec<u8> {
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
