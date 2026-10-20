//! Recording from the app: starting and stopping the replay buffer, saving a clip, the hotkeys
//! that do both, and the controls in the top bar.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clipcraft_capture::{CaptureTarget, TargetKind};
use clipcraft_engine::{
    CaptureSession, HotkeyAction, RecordingSettings, SavedClipReport, default_capture_folder,
};
use clipcraft_platform::{CaptureOptions, HotkeyBinding, HotkeyListener};
use clipcraft_record::{RecordError, RecordState, RecordStatus};
use eframe::egui;
use egui::RichText;

use crate::ClipCraftApp;
use crate::kit::{self, DANGER, FAINT, Icon, LIME, MUTED, TEXT};
use crate::settings::GpuProbe;

/// Hotkey ids handed to the platform listener.
const SAVE_ID: u32 = 1;
const TOGGLE_ID: u32 = 2;
const MICROPHONE_ID: u32 = 3;
const TALK_ID: u32 = 4;

/// Opens microphones and the computer's own sound for the recorder.
fn audio_provider() -> clipcraft_record::AudioProvider {
    use clipcraft_audio_out::{CaptureKind, DeviceCapture};
    use clipcraft_record::AudioKind;
    clipcraft_record::AudioProvider::new(|kind, device| {
        // On Linux the sound server's own "monitor" records what is playing; the generic device
        // layer (ALSA) cannot reach it on most systems.
        #[cfg(target_os = "linux")]
        let mut pulse_problem = String::new();
        #[cfg(target_os = "linux")]
        if kind == AudioKind::SystemOutput {
            match clipcraft_platform::open_system_monitor(device) {
                Ok(source) => return Ok(source),
                Err(why) => pulse_problem = why,
            }
        }
        let kind = match kind {
            AudioKind::Microphone => CaptureKind::Microphone,
            AudioKind::SystemOutput => CaptureKind::SystemOutput,
        };
        DeviceCapture::open(kind, device)
            .map(|capture| Box::new(capture) as Box<dyn clipcraft_record::AudioSource>)
            .map_err(|error| {
                #[cfg(target_os = "linux")]
                if !pulse_problem.is_empty() {
                    return format!("{pulse_problem}; {error}");
                }
                error.to_string()
            })
    })
}

/// A short message shown in the top bar.
pub struct Notice {
    pub text: String,
    pub error: bool,
    pub at: Instant,
    pub path: Option<PathBuf>,
}

/// Everything about recording the app keeps between frames.
#[derive(Default)]
pub struct CaptureState {
    session: CaptureSession,
    started_with: Option<RecordingSettings>,
    pub status: Option<RecordStatus>,
    pub notice: Option<Notice>,
    saving: Option<mpsc::Receiver<Result<SavedClipReport, String>>>,
    /// When the clip being saved will be written, if it is waiting for the post-roll.
    saving_at: Option<Instant>,
    listener: Option<HotkeyListener>,
    listened_for: Vec<(u32, String)>,
    pub targets: Vec<CaptureTarget>,
    targets_loaded: bool,
    /// Whether the graphics card can encode video, found by trying it once.
    pub gpu: GpuProbe,
    gpu_receiver: Option<mpsc::Receiver<Result<String, String>>>,
    launched: bool,
    last_hotkey: Option<(HotkeyAction, Instant)>,
    /// Set after a clip was saved into the library folder, so the library is rescanned.
    pub saved_into: Option<PathBuf>,
}

impl CaptureState {
    /// True while recording runs (or has failed but can still save what it has).
    pub fn is_active(&self) -> bool {
        self.session.is_active()
    }

    /// True when the open recording was started with different settings than the current ones.
    pub fn is_stale(&self, current: &RecordingSettings) -> bool {
        self.started_with.as_ref().is_some_and(|started| {
            started.height != current.height
                || started.frames_per_second != current.frames_per_second
                || started.video_kbps != current.video_kbps
                || started.shape != current.shape
                || started.framing != current.framing
                || started.replay_seconds != current.replay_seconds
                || started.post_roll_seconds != current.post_roll_seconds
                || started.rate_control != current.rate_control
                || started.encoder_preset != current.encoder_preset
                || started.keyframe_seconds != current.keyframe_seconds
                || started.capture_target != current.capture_target
                || started.source != current.source
                || started.video_encoder != current.video_encoder
                || started.force_window_capture != current.force_window_capture
                || started.hdr_compatibility != current.hdr_compatibility
                || started.capture_cursor != current.capture_cursor
                || started.capture_game_audio != current.capture_game_audio
                || started.capture_microphone != current.capture_microphone
                || started.microphone_device != current.microphone_device
                || started.output_device != current.output_device
                || started.separate_audio_tracks != current.separate_audio_tracks
                || started.push_to_talk != current.push_to_talk
                || started.microphone_mono != current.microphone_mono
                || started.audio_sample_rate != current.audio_sample_rate
                || started.audio_channels != current.audio_channels
                || started.audio_kbps != current.audio_kbps
        })
    }

    /// Reads the monitors and windows the provider offers right now.
    pub fn refresh_targets(&mut self) {
        self.targets = clipcraft_platform::list_targets().unwrap_or_default();
        self.targets_loaded = true;
    }

    pub fn targets_loaded(&self) -> bool {
        self.targets_loaded
    }
}

fn notice(text: impl Into<String>, error: bool, path: Option<PathBuf>) -> Option<Notice> {
    Some(Notice {
        text: text.into(),
        error,
        at: Instant::now(),
        path,
    })
}

impl ClipCraftApp {
    /// Runs once per frame: hotkeys, status, and a save that finished.
    pub(crate) fn capture_frame(&mut self, context: &egui::Context) {
        if !self.capture.launched {
            self.capture.launched = true;
            self.recover_after_crash();
            if self.recording.record_on_launch {
                self.start_capture();
            }
        }
        self.poll_gpu_probe();
        self.sync_hotkeys();
        let mut actions = Vec::new();
        let mut talking = None;
        if let Some(listener) = self.capture.listener.as_ref() {
            while let Some(event) = listener.try_next() {
                if event.id == TALK_ID {
                    talking = Some(event.pressed);
                } else if event.pressed {
                    match event.id {
                        SAVE_ID => actions.push(HotkeyAction::SaveClip),
                        TOGGLE_ID => actions.push(HotkeyAction::ToggleRecording),
                        MICROPHONE_ID => actions.push(HotkeyAction::ToggleMicrophone),
                        _ => {}
                    }
                }
            }
        }
        if let Some(held) = talking {
            self.capture.session.set_talking(held);
        }
        if self.capture.is_active() {
            self.capture.session.set_levels(&self.recording);
        }
        // Shortcuts pressed while ClipCraft has the focus, when no text box is being typed in.
        let typing = context.memory(|memory| memory.focused().is_some());
        if !typing
            && self.settings_state.capturing.is_none()
            && let Some(pressed) = context.input(crate::settings::pressed_shortcut_in)
        {
            for action in [
                HotkeyAction::SaveClip,
                HotkeyAction::ToggleRecording,
                HotkeyAction::ToggleMicrophone,
            ] {
                if self.recording.hotkeys.get(action) == pressed {
                    actions.push(action);
                }
            }
        }
        for action in actions {
            // A key seen by both the global listener and the window counts once.
            let repeated = self.capture.last_hotkey.is_some_and(|(last, at)| {
                last == action && at.elapsed() < Duration::from_millis(300)
            });
            self.capture.last_hotkey = Some((action, Instant::now()));
            if repeated {
                continue;
            }
            match action {
                HotkeyAction::SaveClip => self.save_clip(),
                HotkeyAction::ToggleRecording => self.toggle_capture(),
                HotkeyAction::ToggleMicrophone if self.capture.session.has_microphone() => {
                    self.capture.session.toggle_microphone();
                }
                _ => {}
            }
        }

        self.capture.status = self.capture.session.status();
        if let Some(RecordStatus {
            state: RecordState::Failed(error),
            ..
        }) = self.capture.status.as_ref()
            && self
                .capture
                .notice
                .as_ref()
                .is_none_or(|current| !current.error)
        {
            self.capture.notice = notice(
                format!("Clipping stopped: {}", RecordErrorText(*error)),
                true,
                None,
            );
        }
        self.poll_save();
        // While the post-roll runs, count down in the notice.
        if self.capture.saving.is_some()
            && let Some(at) = self.capture.saving_at
        {
            let left = at
                .saturating_duration_since(Instant::now())
                .as_secs_f32()
                .ceil() as u32;
            let text = if left > 0 {
                format!("Saving in {left} s, still clipping\u{2026}")
            } else {
                "Saving clip\u{2026}".to_owned()
            };
            self.capture.notice = notice(text, false, None);
        }
        if self.capture.is_active() || self.capture.saving.is_some() {
            let tick = if self.capture.saving_at.is_some() {
                200
            } else {
                500
            };
            context.request_repaint_after(Duration::from_millis(tick));
        }
        if self
            .capture
            .notice
            .as_ref()
            .is_some_and(|notice| !notice.error && notice.at.elapsed() > Duration::from_secs(8))
        {
            self.capture.notice = None;
        }
    }

    /// After a crash or power cut, clears the unfinished clip files it left in the capture folder
    /// and the library folder, and says so.
    fn recover_after_crash(&mut self) {
        let capture_folder = if self.recording.capture_folder.is_empty() {
            default_capture_folder()
        } else {
            Some(PathBuf::from(&self.recording.capture_folder))
        };
        let library_folder = Some(PathBuf::from(self.directory_text.trim()))
            .filter(|folder| !folder.as_os_str().is_empty());
        let mut recovery = clipcraft_engine::Recovery::default();
        let mut seen = Vec::new();
        for folder in capture_folder.into_iter().chain(library_folder) {
            if seen.contains(&folder) {
                continue;
            }
            let found = clipcraft_engine::recover_interrupted(&folder);
            recovery.removed.extend(found.removed);
            recovery.salvaged.extend(found.salvaged);
            seen.push(folder);
        }
        if let Some(message) = recovery.message() {
            self.capture.notice = notice(message, false, None);
            if !recovery.salvaged.is_empty() {
                self.capture.saved_into = seen.first().cloned();
            }
        }
    }

    /// Finds out once, off the interface thread, whether the graphics card can encode video.
    fn poll_gpu_probe(&mut self) {
        if self.capture.gpu == GpuProbe::Unknown {
            self.capture.gpu = GpuProbe::Checking;
            let (sender, receiver) = mpsc::channel();
            self.capture.gpu_receiver = Some(receiver);
            std::thread::spawn(move || {
                let _ = sender.send(clipcraft_platform::probe_hardware_encoder());
            });
        }
        if let Some(receiver) = self.capture.gpu_receiver.as_ref() {
            match receiver.try_recv() {
                Ok(result) => {
                    self.capture.gpu = match result {
                        Ok(name) => GpuProbe::Ready(name),
                        Err(why) => GpuProbe::Unavailable(why),
                    };
                    self.capture.gpu_receiver = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.capture.gpu = GpuProbe::Unavailable("the check did not finish".to_owned());
                    self.capture.gpu_receiver = None;
                }
            }
        }
    }

    /// Keeps the global hotkeys in step with the settings.
    fn sync_hotkeys(&mut self) {
        let wanted: Vec<(u32, String)> = [
            (SAVE_ID, HotkeyAction::SaveClip),
            (TOGGLE_ID, HotkeyAction::ToggleRecording),
            (MICROPHONE_ID, HotkeyAction::ToggleMicrophone),
            (TALK_ID, HotkeyAction::PushToTalk),
        ]
        .into_iter()
        .filter_map(|(id, action)| {
            let key = self.recording.hotkeys.get(action);
            (!key.is_empty()).then(|| (id, key.to_owned()))
        })
        .collect();
        if wanted == self.capture.listened_for {
            return;
        }
        self.capture.listened_for = wanted.clone();
        self.capture.listener = None;
        if wanted.is_empty() {
            return;
        }
        let bindings = wanted
            .into_iter()
            .map(|(id, shortcut)| HotkeyBinding { id, shortcut })
            .collect();
        // Without a provider (or when another program owns a key) the shortcuts still work while
        // ClipCraft has the focus.
        if let Ok(listener) = HotkeyListener::start(bindings) {
            if !listener.skipped().is_empty() {
                self.capture.notice = notice(
                    format!(
                        "{} is used by another program, so it only works while ClipCraft is open in front.",
                        listener.skipped().join(", ")
                    ),
                    false,
                    None,
                );
            }
            self.capture.listener = Some(listener);
        }
    }

    /// Picks what to record from the settings.
    pub(crate) fn capture_target_id(&mut self) -> Result<String, String> {
        if !self.capture.targets_loaded() {
            self.capture.refresh_targets();
        }
        let chosen = self.recording.capture_target.clone();
        let want_window = self.recording.source == clipcraft_engine::CaptureSource::Game;
        // Forcing window capture follows the window in front, unless a game window was chosen.
        if self.recording.force_window_capture && (!want_window || chosen.is_empty()) {
            return match clipcraft_platform::active_window() {
                Ok(Some(window)) if !is_own_window(&window) => Ok(window.id),
                _ => Err(
                    "Force window capture records the window in front. Start clipping with its hotkey while your game has the focus, or choose the window in Settings \u{203a} Capture."
                        .to_owned(),
                ),
            };
        }
        let kind = if want_window {
            TargetKind::Window
        } else {
            TargetKind::Display
        };
        let listed = self
            .capture
            .targets
            .iter()
            .filter(|target| target.kind == kind);
        if !chosen.is_empty() {
            let known = listed.clone().any(|target| target.id == chosen);
            // A monitor id stays valid even when the list was read before it was plugged in.
            if known || (!want_window && chosen.starts_with("monitor:")) {
                return Ok(chosen);
            }
            if want_window {
                return Err(format!(
                    "The window \u{201c}{}\u{201d} is not open. Choose a game window in Settings \u{203a} Capture.",
                    self.recording.capture_target_name
                ));
            }
        }
        match listed.map(|target| target.id.clone()).next() {
            Some(id) => Ok(id),
            None if want_window => Err(
                "Choose the game window to record in Settings \u{203a} Capture, or switch to Desktop."
                    .to_owned(),
            ),
            None => Err("No screen was found to record.".to_owned()),
        }
    }

    pub(crate) fn start_capture(&mut self) {
        if self.capture.is_active() {
            return;
        }
        self.capture.notice = None;
        self.capture.refresh_targets();
        let id = match self.capture_target_id() {
            Ok(id) => id,
            Err(message) => {
                self.capture.notice = notice(message, true, None);
                return;
            }
        };
        let options = CaptureOptions {
            show_cursor: self.recording.capture_cursor,
            hdr_to_sdr: self.recording.hdr_compatibility,
        };
        let source: Box<dyn clipcraft_capture::CaptureSource> =
            match clipcraft_platform::open(&id, &options) {
                Ok(source) => source,
                Err(error) => {
                    self.capture.notice = notice(error.to_string(), true, None);
                    return;
                }
            };
        let hardware = (self.recording.video_encoder == clipcraft_engine::VideoEncoder::Gpu)
            .then(clipcraft_platform::hardware_provider)
            .flatten();
        match self.capture.session.start_with_audio(
            &self.recording,
            source,
            hardware,
            Some(audio_provider()),
        ) {
            Ok(()) => {
                self.capture.started_with = Some(self.recording.clone());
                self.capture.status = self.capture.session.status();
                let warnings = self.capture.session.warnings().join(" ");
                if !warnings.is_empty() {
                    self.capture.notice = notice(
                        format!("Clipping without some sound. {warnings}"),
                        true,
                        None,
                    );
                }
            }
            Err(error) => self.capture.notice = notice(error.to_string(), true, None),
        }
    }

    pub(crate) fn stop_capture(&mut self) {
        self.capture.session.stop();
        self.capture.started_with = None;
        self.capture.status = None;
    }

    fn toggle_capture(&mut self) {
        if self.capture.is_active() {
            self.stop_capture();
        } else {
            self.start_capture();
        }
    }

    /// Saves the replay buffer as a clip on another thread.
    pub(crate) fn save_clip(&mut self) {
        if self.capture.saving.is_some() {
            return;
        }
        let folder = if self.recording.capture_folder.is_empty() {
            default_capture_folder()
        } else {
            Some(PathBuf::from(&self.recording.capture_folder))
        };
        let Some(folder) = folder else {
            self.capture.notice = notice("Choose a capture folder in Settings first.", true, None);
            return;
        };
        let job = match self.capture.session.prepare_save(&self.recording, &folder) {
            Ok(job) => job,
            Err(error) => {
                self.capture.notice = notice(error.to_string(), true, None);
                return;
            }
        };
        let wait = job.post_roll();
        self.capture.saving_at = (!wait.is_zero()).then(|| Instant::now() + wait);
        self.capture.notice = notice("Saving clip\u{2026}", false, None);
        let (sender, receiver) = mpsc::channel();
        self.capture.saving = Some(receiver);
        self.capture.saved_into = Some(folder);
        std::thread::spawn(move || {
            let _ = sender.send(job.run().map_err(|error| error.to_string()));
        });
    }

    fn poll_save(&mut self) {
        let Some(receiver) = self.capture.saving.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("Saving the clip stopped.".to_owned()),
        };
        self.capture.saving = None;
        self.capture.saving_at = None;
        match result {
            Ok(report) => {
                let name = report
                    .clip
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let removed = if report.deleted.is_empty() {
                    String::new()
                } else {
                    format!(
                        " ({} older clips removed to stay under the limit)",
                        report.deleted.len()
                    )
                };
                self.capture.notice = notice(
                    format!("Saved {name}{removed}"),
                    false,
                    Some(report.clip.path.clone()),
                );
                // A saved clip belongs in the library when its folder is the one being browsed.
                let in_library = self.capture.saved_into.as_ref().is_some_and(|folder| {
                    folder.as_path() == std::path::Path::new(self.directory_text.trim())
                });
                if in_library {
                    self.scan_library();
                }
            }
            Err(message) => self.capture.notice = notice(message, true, None),
        }
    }

    /// The recording controls in the top bar.
    pub(crate) fn capture_controls(&mut self, ui: &mut egui::Ui) {
        let active = self.capture.is_active();
        let (dot, text) = match self.capture.status.as_ref() {
            Some(status) => match status.state {
                RecordState::Running => {
                    let encoder = status.encoder.split([' ', '(']).next().unwrap_or("");
                    let tag = status
                        .output
                        .map(|(_, height)| {
                            format!(
                                " \u{00b7} {height}p{} \u{00b7} {encoder}",
                                status.target_fps
                            )
                        })
                        .unwrap_or_default();
                    (LIME, format!("Clipping{tag}"))
                }
                RecordState::Failed(_) => (DANGER, "Capture stopped".to_owned()),
                RecordState::Stopped => (FAINT, "Not clipping".to_owned()),
            },
            None => (FAINT, "Not clipping".to_owned()),
        };
        kit::status_pill(ui, dot, &text);
        ui.add_space(8.0);

        let (icon, label) = if active {
            (Icon::Stop, "Stop clipping")
        } else {
            (Icon::Record, "Start clipping")
        };
        if kit::labelled_button(ui, icon, label, !active, true)
            .on_hover_text(if active {
                "Stop clipping and forget the replay buffer"
            } else {
                "Start clipping: keep the last moments in memory so you can save them as a clip. Nothing is written to disk until you save one."
            })
            .clicked()
        {
            self.toggle_capture();
        }
        ui.add_space(4.0);
        let saving = self.capture.saving.is_some();
        let seconds = self.recording.replay_seconds;
        let length = if seconds >= 60 {
            format!("{}m", seconds / 60)
        } else {
            format!("{seconds}s")
        };
        let key = self.recording.hotkeys.save_clip.replace('+', " + ");
        if kit::labelled_button(
            ui,
            Icon::Import,
            &format!("Save last {length}"),
            false,
            active && !saving,
        )
        .on_hover_text(if key.is_empty() {
            "Save a clip".to_owned()
        } else {
            format!("Save a clip ({key})")
        })
        .clicked()
        {
            self.save_clip();
        }
        if !key.is_empty() {
            kit::keycap(ui, &key);
        }

        if let Some(status) = self
            .capture
            .status
            .as_ref()
            .filter(|s| s.state == RecordState::Running)
        {
            ui.add_space(10.0);
            let dropped = if status.frames_captured == 0 {
                0.0
            } else {
                status.frames_dropped as f32 * 100.0 / status.frames_captured as f32
            };
            let behind = dropped > 5.0;
            ui.label(
                RichText::new(format!("{:.0}s buffered", status.buffered_seconds))
                    .color(MUTED)
                    .size(12.5),
            )
            .on_hover_text(format!("Encoder: {}", status.encoder));
            let lost: Vec<&str> = status
                .audio
                .iter()
                .filter(|input| input.lost)
                .map(|input| input.name.as_str())
                .collect();
            if !lost.is_empty() {
                ui.label(
                    RichText::new(format!("\u{00b7} sound lost: {}", lost.join(", ")))
                        .color(DANGER)
                        .size(12.5),
                )
                .on_hover_text("ClipCraft keeps trying to bring the device back.");
            } else if self.capture.session.has_microphone() {
                let muted = self.capture.session.microphone_muted();
                ui.label(
                    RichText::new(if muted {
                        "\u{00b7} mic off"
                    } else {
                        "\u{00b7} mic on"
                    })
                    .color(if muted { FAINT } else { MUTED })
                    .size(12.5),
                )
                .on_hover_text("Mute or unmute with the microphone shortcut in Settings.");
            }
            if behind {
                ui.label(
                    RichText::new(format!("\u{00b7} dropping {dropped:.0}% of frames"))
                        .color(DANGER)
                        .size(12.5),
                )
                .on_hover_text(
                    "This computer cannot encode at this quality in real time. Lower the resolution or frame rate in Settings \u{203a} Quality, or pick the Fast encoder in Advanced.",
                );
            }
        }
        let shown = self
            .capture
            .notice
            .as_ref()
            .map(|current| (current.text.clone(), current.error, current.path.clone()));
        if let Some((text, error, path)) = shown {
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .color(if error { DANGER } else { TEXT })
                        .size(13.0),
                )
                .truncate(),
            );
            if let Some(path) = path
                && kit::labelled_button(ui, Icon::Folder, "Show", false, true).clicked()
            {
                self.reveal_media(path);
            }
        }
    }
}

/// Wraps a record error so a notice can print it.
struct RecordErrorText(RecordError);

impl std::fmt::Display for RecordErrorText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, formatter)
    }
}

/// ClipCraft's own window is never the game, even when it has the focus.
fn is_own_window(window: &CaptureTarget) -> bool {
    window.name.trim() == "ClipCraft"
}
