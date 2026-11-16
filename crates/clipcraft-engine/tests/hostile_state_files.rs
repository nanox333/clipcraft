//! Damaged and hostile state files: the settings file and the editor's recovery draft are cut
//! short, bit-flipped and given extreme values (one number at a time becomes 0, -1, a huge value,
//! a string, null...). Loading must never panic, and settings that do load must be safe to record
//! with: a recording started from them must not end in an internal error.
//!
//! A fixed, repeatable pass, not random fuzzing.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clipcraft_capture::SyntheticSource;
use clipcraft_engine::{
    AppAudioRow, CaptureSession, Command, CommandOutput, RecordingSettings, VideoEncoder, execute,
    load_editor_draft,
};
use clipcraft_record::{RecordError, RecordState};

fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "clipcraft-hostile-state-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("scratch folder");
    folder
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

/// The damaged copies of a JSON file: cut short, bit-flipped, with each number replaced by an
/// extreme value in turn, and nested very deeply.
fn variants(original: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let length = original.len();
    for part in 1..=40 {
        out.push(original[..length * part / 41].to_vec());
    }
    let mut random = Xorshift(0x1234_5678_9ABC_DEF1);
    for round in 0..200_u64 {
        let mut copy = original.to_vec();
        for _ in 0..(1 + round % 4) {
            let at = usize::try_from(random.next() % length as u64).unwrap_or(0);
            copy[at] ^= 1 << (random.next() % 8);
        }
        out.push(copy);
    }
    // Every number (a run of digits, maybe signed or with a fraction) becomes each extreme.
    let text = String::from_utf8_lossy(original).into_owned();
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_digit() && (at == 0 || !bytes[at - 1].is_ascii_alphanumeric()) {
            let start = at;
            while at < bytes.len()
                && (bytes[at].is_ascii_digit() || matches!(bytes[at], b'.' | b'e' | b'E'))
            {
                at += 1;
            }
            spans.push((start, at));
        } else {
            at += 1;
        }
    }
    let extremes = [
        "0",
        "-1",
        "1",
        "255",
        "65536",
        "4294967295",
        "4294967296",
        "9007199254740993",
        "18446744073709551615",
        "-9223372036854775808",
        "1e308",
        "-1e308",
        "0.0000001",
        "null",
        "\"x\"",
        "[]",
        "{}",
        "true",
    ];
    for (start, end) in spans.iter().take(400) {
        for extreme in extremes {
            out.push(format!("{}{}{}", &text[..*start], extreme, &text[*end..]).into_bytes());
        }
    }
    out.push(format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000)).into_bytes());
    out.push(vec![0; 70_000]);
    out.push(Vec::new());
    out
}

/// A recording from `settings` for a moment: it must not end in an internal error.
fn record_briefly(settings: &RecordingSettings) {
    // Never the graphics card or a real microphone: those are not what is being tested here.
    let mut settings = settings.clone();
    settings.video_encoder = VideoEncoder::Cpu;
    settings.capture_microphone = false;
    settings.capture_game_audio = false;
    settings.audio_apps.clear();
    let mut session = CaptureSession::new();
    if session
        .start(&settings, Box::new(SyntheticSource::new(160, 90, 30)), None)
        .is_err()
    {
        return; // refused with words: fine
    }
    let until = Instant::now() + Duration::from_millis(250);
    while Instant::now() < until {
        if let Some(status) = session.status()
            && let RecordState::Failed(error) = status.state
        {
            assert_ne!(
                error,
                RecordError::Internal,
                "a recording from loadable settings panicked: {settings:?}"
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    session.stop();
}

#[test]
fn damaged_settings_files_never_panic_and_loadable_ones_are_safe_to_record_with() {
    let folder = scratch("settings");
    let path = folder.join("recording-settings.json");
    let rich = RecordingSettings {
        audio_apps: vec![
            AppAudioRow::for_program("discord.exe"),
            AppAudioRow::for_game(),
        ],
        ..RecordingSettings::default()
    };
    execute(Command::SaveRecordingSettings {
        path: path.clone(),
        settings: Box::new(rich),
    })
    .expect("settings save");
    let original = std::fs::read(&path).expect("settings file");
    let mut tried = HashSet::new();
    let mut recorded = 0;
    for bytes in variants(&original) {
        std::fs::write(&path, &bytes).expect("damaged copy writes");
        // Damaged files are set aside beside it; clear them so they do not pile up.
        let outcome = execute(Command::LoadRecordingSettings { path: path.clone() });
        clear_aside(&folder, &path);
        if let Ok(CommandOutput::RecordingSettings(settings)) = outcome {
            let settings = settings.sanitized();
            // Sanitising twice changes nothing: it is a fixed point.
            assert_eq!(settings.clone().sanitized(), settings);
            // Recording from a distinct, loadable value, up to a limit (each takes a moment).
            let key = serde_json::to_string(&settings).unwrap_or_default();
            if recorded < 60 && tried.insert(key) {
                recorded += 1;
                record_briefly(&settings);
            }
        }
    }
    assert!(recorded > 5, "the pass tried too few loadable values");
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn damaged_editor_drafts_never_panic() {
    let folder = scratch("draft");
    let path = folder.join("editor-draft.json");
    let original = serde_json::to_vec(&clipcraft_engine::EditorDraft::default()).expect("draft");
    std::fs::write(&path, &original).expect("draft file");
    assert!(load_editor_draft(&path).is_ok());
    for bytes in variants(&original) {
        std::fs::write(&path, &bytes).expect("damaged copy writes");
        // An error or an empty answer are both fine; a panic is not.
        let _ = load_editor_draft(&path);
    }
    let _ = std::fs::remove_dir_all(&folder);
}

fn clear_aside(folder: &Path, keep: &Path) {
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            if entry.path() != keep {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}
