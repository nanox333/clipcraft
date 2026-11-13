//! Damaged and hostile clip files: every reader that takes a file from the person's disk (the
//! header probe, the thumbnail maker and the player) is fed truncated and bit-flipped copies of a
//! real clip. None may panic (even where the panic would be caught and reported), run far longer
//! than it should, or use unbounded memory (a hang or a huge allocation shows up as a timeout).
//!
//! It is a fixed, repeatable mutation pass, not random fuzzing: the same files every run.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use clipcraft_capture::SyntheticSource;
use clipcraft_engine::{CaptureSession, Container, QualityMode, RecordingSettings, VideoEncoder};

static PANICS: AtomicUsize = AtomicUsize::new(0);

/// A real clip in `container`, about two seconds long.
fn make_clip(folder: &Path, container: Container) -> Vec<u8> {
    let settings = RecordingSettings {
        quality: QualityMode::Custom,
        video_encoder: VideoEncoder::Cpu,
        container,
        height: 90,
        frames_per_second: 30,
        video_kbps: 400,
        replay_seconds: 10,
        keyframe_seconds: 1,
        capture_game_audio: false,
        capture_microphone: false,
        ..RecordingSettings::default()
    };
    let mut session = CaptureSession::new();
    session
        .start(&settings, Box::new(SyntheticSource::new(160, 90, 30)), None)
        .expect("recording starts");
    let deadline = Instant::now() + Duration::from_secs(30);
    while session
        .status()
        .is_none_or(|status| status.buffered_seconds < 2.0)
    {
        assert!(Instant::now() < deadline, "the recording did not fill");
        std::thread::sleep(Duration::from_millis(50));
    }
    let saved = session
        .prepare_save(&settings, folder)
        .expect("save prepares")
        .run()
        .expect("the clip is saved");
    session.stop();
    std::fs::read(&saved.clip.path).expect("the clip reads")
}

/// A small deterministic generator, so every run mutates the same bytes.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// The damaged copies of `original`: cut short at many places, a few bytes flipped at many
/// places (headers get most of them, where the damage matters most), and zeroed stretches.
fn variants(original: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let length = original.len();
    for part in 1..=24 {
        out.push(original[..length * part / 25].to_vec());
    }
    for cut in [0, 1, 3, 4, 7, 8, 11, 12, 15, 16, 31, 32, 63, 64, 127, 128] {
        out.push(original[..cut.min(length)].to_vec());
    }
    let mut random = Xorshift(0x9E37_79B9_7F4A_7C15);
    for round in 0..120_u64 {
        let mut copy = original.to_vec();
        // Most damage lands in the first few kilobytes, where the headers are.
        let window = if round % 3 == 0 {
            length
        } else {
            length.min(4_096)
        };
        for _ in 0..(1 + round % 6) {
            let at = usize::try_from(random.next() % window as u64).unwrap_or(0);
            copy[at] ^= 1 << (random.next() % 8);
        }
        out.push(copy);
    }
    for round in 0..16_usize {
        let mut copy = original.to_vec();
        let start = usize::try_from(random.next() % length as u64).unwrap_or(0);
        let end = (start + 16 + round * 40).min(length);
        copy[start..end].fill(0);
        out.push(copy);
    }
    out
}

fn run_readers(path: &Path, cache: &Path, with_player: bool) {
    let begin = Instant::now();
    let _ = clipcraft_media::probe_file(path);
    let _ = clipcraft_player::thumbnail::ensure_cached(path, cache);
    if !with_player {
        return;
    }
    // The player opens on its own thread; give it a moment, then ask it to stop and make sure it
    // does.
    let player = clipcraft_player::Player::start(path.to_path_buf(), || {});
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        let _ = player.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    player.request_stop();
    let stop_by = Instant::now() + Duration::from_secs(10);
    while !player.worker_finished() && Instant::now() < stop_by {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        player.worker_finished(),
        "the player did not stop for {}",
        path.display()
    );
    assert!(
        begin.elapsed() < Duration::from_secs(30),
        "reading {} took {:?}",
        path.display(),
        begin.elapsed()
    );
}

fn pass(container: Container, extension: &str) {
    let folder = std::env::temp_dir().join(format!(
        "clipcraft-hostile-{}-{extension}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("scratch folder");
    let original = make_clip(&folder, container);
    assert!(original.len() > 1_000, "the clip is too small to damage");
    let cache = folder.join("cache");
    let damaged: PathBuf = folder.join(format!("damaged.{extension}"));
    for (index, bytes) in variants(&original).iter().enumerate() {
        std::fs::write(&damaged, bytes).expect("damaged copy writes");
        // The header probe and the thumbnail maker see every variant; the player, which needs a
        // moment each, every third one.
        run_readers(&damaged, &cache, index % 3 == 0);
        assert_eq!(
            PANICS.load(Ordering::SeqCst),
            0,
            "variant {index} ({} bytes) caused a panic",
            bytes.len()
        );
    }
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn damaged_mp4_and_mkv_files_never_panic_hang_or_balloon() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        PANICS.fetch_add(1, Ordering::SeqCst);
        default_hook(info);
    }));
    pass(Container::Mp4, "mp4");
    pass(Container::Mkv, "mkv");
}
