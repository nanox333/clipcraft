//! One recording crosses the production save/library/player/edit/export/recovery boundaries.
use super::*;
use clipcraft_capture::SyntheticSource;
use clipcraft_player::{Player, PlayerEvent};
use clipcraft_record::{AudioFormat, AudioProvider, ToneSource};
use clipcraft_time::Tick;
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "clipcraft-workflow-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn library(command: Command) -> LibraryScanResult {
    let CommandOutput::LibraryScan(scan) = execute(command).unwrap() else {
        panic!("expected library");
    };
    scan
}
fn edit(session: EditorSession, action: EditorAction) -> EditorSession {
    let CommandOutput::EditorSession(session) = execute(Command::EditSequence {
        session: Box::new(session),
        action,
    })
    .unwrap() else {
        panic!("expected edit");
    };
    *session
}
fn draft(path: &Path) -> EditorDraft {
    let CommandOutput::EditorDraft(Some(draft)) =
        execute(Command::LoadEditorDraft { path: path.into() }).unwrap()
    else {
        panic!("expected recovery record");
    };
    *draft
}

fn playable_with_sound(path: &Path) {
    let player = Player::start(path.into(), || {});
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut opened = false;
    let mut picture = false;
    let mut audible = false;
    while !(opened && picture && audible) && Instant::now() < deadline {
        for event in player.poll() {
            match event {
                PlayerEvent::Opened(info) => {
                    assert!(info.has_audio);
                    opened = true;
                    player.play();
                }
                PlayerEvent::Frame(frame) => {
                    picture |= !frame.rgba.is_empty();
                }
                PlayerEvent::Failed(error) => panic!("playback failed: {error}"),
                _ => {}
            }
        }
        if let Some(feed) = player.audio_feed() {
            if !feed.driving() {
                assert!(feed.configure_output(48_000, 2));
                feed.set_output_active(true);
            }
            let mut samples = [0.0; 960];
            feed.fill(&mut samples);
            audible |= samples.iter().any(|s| s.abs() > 0.01);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    player.request_stop();
    while !player.worker_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(player.worker_finished());
    assert!(
        opened && picture && audible,
        "opened={opened}, picture={picture}, audible={audible}"
    );
}

#[test]
fn game_replay_library_editor_export_and_recovery_share_one_recording() {
    let dir = Scratch::new();
    let clips = dir.file("clips");
    let index = dir.file("library.json");
    fs::create_dir(&clips).unwrap();
    let settings = RecordingSettings {
        quality: QualityMode::Custom,
        video_encoder: VideoEncoder::Cpu,
        height: 180,
        frames_per_second: 30,
        video_kbps: 500,
        replay_seconds: 15,
        keyframe_seconds: 1,
        capture_game_audio: true,
        capture_microphone: false,
        capture_folder: clips.to_string_lossy().into(),
        ..Default::default()
    };
    execute(Command::SaveRecordingSettings {
        path: dir.file("settings.json"),
        settings: Box::new(settings.clone()),
    })
    .unwrap();
    let CommandOutput::RecordingSettings(loaded_settings) =
        execute(Command::LoadRecordingSettings {
            path: dir.file("settings.json"),
        })
        .unwrap()
    else {
        panic!("expected settings");
    };
    assert_eq!(*loaded_settings, settings.clone().sanitized());

    let game = GameProcessMatch {
        game_name: "Workflow game".into(),
        process_id: 42,
    };
    let mut arming = GameArmingState::default();
    assert_eq!(arming.observe(&[]), None);
    assert_eq!(
        arming.observe(std::slice::from_ref(&game)),
        Some(GameArmingDecision::Start(game.clone()))
    );
    let mut capture = CaptureSession::new();
    capture
        .start_with_audio(
            &settings,
            Box::new(SyntheticSource::new(320, 180, 30)),
            None,
            Some(AudioProvider::new(|_, _| {
                Ok(Box::new(ToneSource::new(
                    AudioFormat {
                        sample_rate: 48_000,
                        channels: 2,
                    },
                    440.0,
                    0.3,
                )))
            })),
        )
        .unwrap();
    arming.capture_started(game.clone());
    let deadline = Instant::now() + Duration::from_secs(30);
    while capture.status().is_none_or(|s| s.buffered_seconds < 1.3) {
        assert!(
            Instant::now() < deadline,
            "capture did not produce a replay"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let job = capture
        .prepare_save(&settings, &clips)
        .unwrap()
        .with_game(arming.recording().map(|g| g.game_name.as_str()));
    // Exiting before the save finishes must not lose the game snapshot or retained media.
    assert_eq!(arming.observe(&[]), Some(GameArmingDecision::Stop));
    capture.stop();
    arming.capture_stopped_automatically();
    let saved = job.run().unwrap();
    assert_eq!(saved.recorded_game.as_deref(), Some("Workflow game"));
    assert_eq!(saved.clip.audio_tracks, 1);
    let input = saved.clip.path.clone();
    let original = fs::read(&input).unwrap();
    let duration = Tick::from_units(saved.clip.duration_frames as i64, 30);
    assert!(duration >= Tick::from_units(1, 1));
    library(Command::RegisterSavedClip {
        index_path: index.clone(),
        directory: clips.clone(),
        path: input.clone(),
        game: saved.recorded_game.unwrap(),
    });
    let path = input.to_string_lossy().to_string();
    for change in [
        MetaEdit::SetFavorite {
            path: path.clone(),
            favorite: true,
        },
        MetaEdit::CreateCollection {
            name: "Highlights".into(),
        },
        MetaEdit::AddToCollection {
            path: path.clone(),
            name: "Highlights".into(),
        },
    ] {
        library(Command::EditLibraryMeta {
            index_path: index.clone(),
            directory: clips.to_string_lossy().into(),
            edit: change,
        });
    }
    let CommandOutput::ClipProbe(probe) = execute(Command::ProbeClip {
        path: input.clone(),
    })
    .unwrap() else {
        panic!("expected probe");
    };
    library(Command::StoreProbes {
        index_path: index.clone(),
        directory: clips.to_string_lossy().into(),
        probes: vec![(path.clone(), probe)],
    });
    let CommandOutput::Thumbnail(thumbnail) = execute(Command::EnsureThumbnail {
        path: input.clone(),
        cache_dir: dir.file("thumbs"),
    })
    .unwrap() else {
        panic!("expected thumbnail");
    };
    assert!(fs::metadata(&thumbnail.file).unwrap().len() > 0);
    playable_with_sound(&input);

    let CommandOutput::EditorMedia(media) = execute(Command::ImportEditorMedia {
        paths: vec![input.clone()],
    })
    .unwrap() else {
        panic!("expected editor import");
    };
    assert_eq!(media.len(), 1);
    assert!(media[0].probe.is_some() && !media[0].offline);
    // Use exactly the duration offered by native import, including recordings with AAC padding.
    let imported_duration = media[0].probe.as_ref().unwrap().duration.unwrap();
    execute(Command::AnalyzeEditorTimeline {
        request: Box::new(TimelineRequest {
            path: input.clone(),
            start: Tick::ZERO,
            end: imported_duration,
            frames: 2,
            buckets: 16,
        }),
    })
    .unwrap();
    let full_request = SequenceRequest {
        segments: vec![SequenceSegment {
            input: input.clone(),
            start: Tick::ZERO,
            end: imported_duration,
            volume_percent: 100,
            look: Default::default(),
        }],
        texts: vec![],
        preset: None,
        quality: SequenceQuality::default(),
        output: dir.file("full-replay.mp4"),
    };
    execute(Command::ExportSequence {
        request: Box::new(full_request),
    })
    .unwrap();
    let session = edit(
        EditorSession::default(),
        EditorAction::Append {
            input: input.clone(),
            name: "Replay".into(),
            duration: imported_duration,
        },
    );
    let first = session.clips()[0].id;
    let session = edit(
        session,
        EditorAction::Trim {
            id: first,
            start: Tick::ZERO,
            end: Tick::from_units(1, 1),
        },
    );
    let session = edit(
        session,
        EditorAction::Split {
            at: Tick::from_units(1, 2),
        },
    );
    let second = session.clips()[1].id;
    let session = edit(
        session,
        EditorAction::Trim {
            id: second,
            start: Tick::from_units(3, 5),
            end: Tick::from_units(1, 1),
        },
    );
    let session = edit(
        session,
        EditorAction::AddText {
            start: Tick::ZERO,
            duration: Tick::from_units(9, 10),
            text: "Offline".into(),
        },
    );
    assert_eq!(session.duration(), Tick::from_units(9, 10));
    let recovery = dir.file("editor-draft.json");
    let snapshot = EditorDraft::new(
        &session,
        vec![input.clone()],
        "Offline highlight".into(),
        Tick::from_units(1, 3),
        Some(ExportPreset::Small),
    )
    .unwrap()
    .with_quality(SequenceQuality {
        resolution: SequenceResolution::Source,
        fps: 60,
    })
    .unwrap();
    execute(Command::SaveEditorDraft {
        path: recovery.clone(),
        draft: Box::new(snapshot.clone()),
    })
    .unwrap();
    drop(session);
    let restored = draft(&recovery);
    assert_eq!(restored, snapshot);
    assert!(!restored.session.can_undo() && !restored.session.can_redo());
    let request = SequenceRequest {
        segments: restored
            .session
            .clips()
            .iter()
            .map(|c| SequenceSegment {
                input: c.input.clone(),
                start: c.start,
                end: c.end,
                volume_percent: c.volume_percent,
                look: c.look.clone(),
            })
            .collect(),
        texts: restored
            .session
            .texts()
            .iter()
            .map(|t| t.layer.clone())
            .collect(),
        preset: restored.preset,
        quality: restored.quality,
        output: clips.join("highlight.mp4"),
    };
    let command = |req: SequenceRequest| Command::ExportSequence {
        request: Box::new(req),
    };
    assert!(matches!(
        execute_cancellable(command(request.clone()), &AtomicBool::new(true)),
        Err(EngineError::Export(
            clipcraft_export::ExportError::Cancelled
        ))
    ));
    assert!(!request.output.exists());
    assert!(!request.output.with_extension("mp4.partial").exists());
    let held = dir.file("disconnected-source.mp4");
    fs::rename(&input, &held).unwrap();
    assert_eq!(
        draft(&recovery),
        snapshot,
        "missing media must retain the edit"
    );
    assert!(execute(command(request.clone())).is_err());
    assert!(!request.output.exists());
    fs::rename(&held, &input).unwrap();
    let CommandOutput::SequenceExported(report) = execute(command(request.clone())).unwrap() else {
        panic!("expected completed export");
    };
    assert_eq!(
        (report.fps, report.clip.pictures, report.clip.duration_ms),
        (60, 54, 900)
    );
    playable_with_sound(&request.output);
    let exported = fs::read(&request.output).unwrap();
    assert!(matches!(
        execute(command(request.clone())),
        Err(EngineError::Export(
            clipcraft_export::ExportError::AlreadyExists
        ))
    ));
    assert_eq!(fs::read(&request.output).unwrap(), exported);
    assert_eq!(fs::read(&input).unwrap(), original);
    assert_eq!(draft(&recovery), snapshot);
    let reloaded = library(Command::RefreshLibrary {
        directory: clips.clone(),
        index_path: index.clone(),
    });
    assert_eq!(reloaded.entries.len(), 2);
    let reloaded = library(Command::LoadLibrary { index_path: index });
    let entry = reloaded
        .entries
        .iter()
        .find(|entry| entry.path == path)
        .unwrap();
    assert!(entry.favorite && !entry.offline && entry.probe.is_some());
    assert_eq!(entry.recorded_game.as_deref(), Some("Workflow game"));
    assert_eq!(entry.collections, ["Highlights"]);
}
