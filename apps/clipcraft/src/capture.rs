//! Recording from the app: starting and stopping the replay buffer, saving a clip, the hotkeys
//! that do both, and the controls in the top bar.

use clipcraft_audio_out::Cue;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clipcraft_capture::{CaptureTarget, TargetKind};
use clipcraft_engine::{
    CaptureSession, GameArmingDecision, GameArmingState, GameProcessMatch, HotkeyAction,
    RecordingSettings, SavedClipReport,
};
use clipcraft_platform::{CaptureOptions, HotkeyBinding, HotkeyListener};
use clipcraft_record::{RecordError, RecordState, RecordStatus};
use eframe::egui;
use egui::RichText;
use egui::{Color32, Stroke, Vec2};

use crate::ClipCraftApp;
use crate::kit::{self, DANGER, FAINT, Icon, LIME, MUTED, TEXT};
use crate::settings::GpuProbe;

/// Hotkey ids handed to the platform listener.
const SAVE_ID: u32 = 1;
const TOGGLE_ID: u32 = 2;
const MICROPHONE_ID: u32 = 3;
const TALK_ID: u32 = 4;
const SCREENSHOT_ID: u32 = 5;

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
    /// A screenshot being taken and written, and the file it made.
    shooting: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    /// A screen share being set up while the desktop's own chooser is open.
    opening: Option<mpsc::Receiver<Result<Box<dyn clipcraft_capture::CaptureSource>, String>>>,
    /// The game that caused a pending desktop share to open.
    opening_game: Option<GameProcessMatch>,
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
    /// Whether the program is running at the lower priority used while clipping.
    background_priority: bool,
    last_hotkey: Option<(HotkeyAction, Instant)>,
    /// Set after a clip was saved into the library folder, so the library is rescanned.
    pub saved_into: Option<PathBuf>,
    game_worker: Option<crate::game_scan::Worker>,
    game_scan: Option<crate::game_scan::Job>,
    game_config: Option<crate::game_scan::DetectionConfig>,
    manual_game_refresh: bool,
    pending_transition: Option<GameArmingDecision>,
    desktop_fallback: bool,
    game_arming: GameArmingState,
    game_session: Option<GameProcessMatch>,
    game_scan_due: Option<Instant>,
    game_retry_after: Option<Instant>,
    game_only_was_enabled: bool,
}

impl CaptureState {
    /// True while recording runs (or has failed but can still save what it has).
    pub fn is_active(&self) -> bool {
        self.session.is_active() || self.opening.is_some()
    }

    pub fn game_recording_name(&self) -> Option<&str> {
        self.game_arming
            .recording()
            .map(|game| game.game_name.as_str())
    }

    pub fn game_capture_paused(&self) -> bool {
        self.game_arming.is_paused()
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
        clipcraft_engine::enable_export_hardware(
            self.recording.video_encoder == clipcraft_engine::VideoEncoder::Gpu,
        );
        if !self.capture.launched {
            self.capture.launched = true;
            // Exports may use the same hardware encoder as recording.
            clipcraft_engine::set_export_hardware(clipcraft_platform::hardware_provider());
            clipcraft_platform::register_hardware_decoding();
            self.recover_after_crash();
            if self.recording.record_on_launch && !self.recording.game_only_capture {
                self.start_capture();
            }
        }
        self.poll_gpu_probe();
        self.poll_opening();
        self.poll_game_arming(context);
        if self.recording.game_only_capture {
            context.request_repaint_after(Duration::from_secs(1));
        }
        self.yield_to_the_game();
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
                        SCREENSHOT_ID => actions.push(HotkeyAction::Screenshot),
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
                HotkeyAction::Screenshot,
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
                    let muted = self.capture.session.toggle_microphone();
                    self.cue(if muted {
                        Cue::MicrophoneOff
                    } else {
                        Cue::MicrophoneOn
                    });
                }
                HotkeyAction::Screenshot => self.take_screenshot(),
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
            self.cue(Cue::Problem);
        }
        self.poll_save();
        self.advance_game_transition();
        self.poll_screenshot();
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

    /// After a crash or power cut, clears the unfinished clip files it left in the clips folder,
    /// and says so.
    fn recover_after_crash(&mut self) {
        let Some(folder) = self.clips_folder() else {
            return;
        };
        let recovery = clipcraft_engine::recover_interrupted(&folder);
        if let Some(message) = recovery.message() {
            self.capture.notice = notice(message, false, None);
            if !recovery.salvaged.is_empty() {
                self.capture.saved_into = Some(folder);
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
            (SCREENSHOT_ID, HotkeyAction::Screenshot),
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

    pub(crate) fn request_game_refresh(&mut self) {
        self.capture.manual_game_refresh = true;
        self.capture.game_scan_due = None;
    }

    fn poll_game_arming(&mut self, context: &egui::Context) {
        let config = crate::game_scan::DetectionConfig::from(&self.recording);
        if self.capture.game_config.as_ref() != Some(&config) {
            self.capture.game_scan = None; // obsolete receivers cannot apply stale settings
            self.capture.game_config = Some(config.clone());
            self.capture.game_scan_due = None;
        }
        if !self.recording.game_only_capture {
            if self.capture.game_only_was_enabled {
                if self.capture.game_session.is_some() || self.capture.desktop_fallback {
                    self.stop_capture();
                }
                self.capture.game_arming.reset();
                self.capture.game_only_was_enabled = false;
            }
            if !self.capture.manual_game_refresh {
                self.capture.game_scan = None;
                return;
            }
        } else if !self.capture.game_only_was_enabled {
            self.capture.game_only_was_enabled = true;
            if self.capture.is_active() && self.capture.game_session.is_none() {
                self.stop_capture();
            }
        }
        if self.capture.desktop_fallback && !self.recording.record_desktop_after_game {
            self.stop_capture();
        }
        if let Some(job) = &self.capture.game_scan {
            let result = match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Game discovery stopped unexpectedly. Refresh to try again.".to_owned(),
                )),
            };
            if let Some(result) = result {
                let config_matches = job.config == config;
                self.capture.game_scan = None;
                self.capture.manual_game_refresh = false;
                self.capture.game_scan_due = Some(Instant::now() + Duration::from_secs(2));
                if config_matches {
                    match result {
                        Ok(found) => self.apply_game_observation(found),
                        Err(error) => {
                            if self.capture.game_session.is_some() || self.capture.desktop_fallback
                            {
                                self.stop_capture();
                            }
                            self.settings_state.set_game_processes(Err(error.clone()));
                            self.capture.notice = notice(error, true, None);
                            self.capture.game_worker = None;
                        }
                    }
                }
            }
        }
        if self.capture.game_scan.is_some() {
            context.request_repaint_after(Duration::from_millis(100));
            return;
        }
        if !self.recording.game_only_capture && !self.capture.manual_game_refresh {
            return;
        }
        if !self.recording.has_game_rules() && !self.capture.manual_game_refresh {
            if self.capture.game_session.is_some() || self.capture.desktop_fallback {
                self.stop_capture();
            }
            return;
        }
        if self
            .capture
            .game_scan_due
            .is_some_and(|at| Instant::now() < at)
        {
            return;
        }
        let result = (|| {
            if self.capture.game_worker.is_none() {
                self.capture.game_worker = Some(crate::game_scan::Worker::start()?);
            }
            self.capture
                .game_worker
                .as_ref()
                .ok_or_else(|| "Game discovery is unavailable.".to_owned())?
                .submit(config, self.capture.manual_game_refresh, context)
        })();
        match result {
            Ok(job) => self.capture.game_scan = Some(job),
            Err(error) => {
                self.settings_state.set_game_processes(Err(error.clone()));
                self.capture.notice = notice(error, true, None);
                self.capture.game_worker = None;
                self.capture.game_scan_due = Some(Instant::now() + Duration::from_secs(10));
            }
        }
    }

    fn apply_game_observation(&mut self, found: crate::game_scan::ResultSet) {
        let exited = self.capture.game_session.as_ref().is_some_and(|game| {
            !found
                .processes
                .iter()
                .any(|process| process.process_id == game.process_id)
        });
        self.settings_state.steam_games = found.steam;
        self.settings_state.steam_warning = found.steam_warning;
        self.settings_state.set_game_processes(Ok(found.processes));
        if !self.recording.game_only_capture {
            return;
        }
        let allow_desktop =
            self.recording.record_desktop_after_game && (exited || self.capture.desktop_fallback);
        let decision = self
            .capture
            .game_arming
            .observe_with_desktop(&found.matches, allow_desktop);
        if self.capture.pending_transition.is_some() {
            return;
        }
        match decision {
            Some(GameArmingDecision::Start(game))
                if self
                    .capture
                    .game_retry_after
                    .is_none_or(|at| Instant::now() >= at) =>
            {
                if self.capture.is_active() {
                    self.capture.pending_transition = Some(GameArmingDecision::Start(game));
                } else {
                    self.capture.game_retry_after = Some(Instant::now() + Duration::from_secs(10));
                    self.start_capture_for_game(Some(game));
                }
            }
            Some(GameArmingDecision::StartDesktop) => {
                self.capture.pending_transition = Some(GameArmingDecision::StartDesktop)
            }
            Some(GameArmingDecision::Stop)
                if self.capture.game_session.is_some() || self.capture.desktop_fallback =>
            {
                self.stop_capture()
            }
            _ => {}
        }
        self.advance_game_transition();
    }

    fn advance_game_transition(&mut self) {
        if self.capture.saving.is_some() {
            return;
        }
        let Some(transition) = self.capture.pending_transition.take() else {
            return;
        };
        if !self.recording.game_only_capture || self.capture.game_arming.is_paused() {
            return;
        }
        let transition = match transition {
            GameArmingDecision::StartDesktop if !self.recording.record_desktop_after_game => {
                self.stop_capture();
                return;
            }
            // A game may have started while an earlier save was finishing.
            GameArmingDecision::StartDesktop => self
                .capture
                .game_arming
                .observed()
                .cloned()
                .map(GameArmingDecision::Start)
                .unwrap_or(GameArmingDecision::StartDesktop),
            GameArmingDecision::Start(game) => {
                if self.capture.game_arming.observed() != Some(&game) {
                    self.stop_capture();
                    return;
                }
                GameArmingDecision::Start(game)
            }
            other => other,
        };
        if self.capture.game_session.is_some() && self.recording.record_desktop_after_game {
            // The SaveJob owns the old bounded recorder, so stopping/changing source cannot
            // overwrite its game replay. Skip post-roll because the source has disappeared.
            if !self.save_game_before_transition() {
                return;
            }
        }
        self.stop_capture();
        match transition {
            GameArmingDecision::Start(game) => self.start_capture_for_game(Some(game)),
            GameArmingDecision::StartDesktop => self.start_desktop_fallback(),
            GameArmingDecision::Stop => {}
        }
    }

    fn save_game_before_transition(&mut self) -> bool {
        if self
            .capture
            .session
            .status()
            .is_none_or(|status| status.frames_encoded == 0)
        {
            return true;
        }
        let Some(folder) = self.clips_folder() else {
            self.capture.notice = notice(
                "Choose a capture folder before continuing on the desktop; the game replay is still retained.",
                true,
                None,
            );
            return false;
        };
        let mut settings = self.recording.clone();
        settings.post_roll_seconds = 0;
        let game = self
            .capture
            .game_session
            .as_ref()
            .map(|game| game.game_name.as_str());
        let job = match self.capture.session.prepare_save(&settings, &folder) {
            Ok(job) => job.with_game(game),
            Err(error) => {
                self.capture.notice = notice(
                    format!(
                        "Desktop continuation did not start: {error}. The game replay is still retained."
                    ),
                    true,
                    None,
                );
                return false;
            }
        };
        self.launch_save(job, folder);
        true
    }

    fn start_desktop_fallback(&mut self) {
        self.capture.refresh_targets();
        let chosen = &self.recording.capture_target;
        let target = self
            .capture
            .targets
            .iter()
            .filter(|target| target.kind == TargetKind::Display)
            .find(|target| target.id == *chosen)
            .or_else(|| {
                self.capture
                    .targets
                    .iter()
                    .find(|target| target.kind == TargetKind::Display)
            });
        let Some(target) = target else {
            self.capture.notice = notice(
                "Desktop continuation is unavailable: no monitor can be opened automatically. Start a desktop capture manually if your desktop requires a chooser.",
                true,
                None,
            );
            return;
        };
        let id = target.id.clone();
        let options = CaptureOptions {
            show_cursor: self.recording.capture_cursor,
            hdr_to_sdr: self.recording.hdr_compatibility,
        };
        match clipcraft_platform::open(&id, &options) {
            Ok(source) => {
                if self.begin_session(source) {
                    self.capture.desktop_fallback = true;
                    self.capture.game_arming.desktop_started();
                    self.capture.notice = notice(
                        "Game closed. Recording the desktop; a new recognized game will switch capture back.",
                        false,
                        None,
                    );
                }
            }
            Err(error) => {
                self.capture.notice = notice(
                    format!("Desktop continuation could not start: {error}"),
                    true,
                    None,
                )
            }
        }
    }

    /// Picks what to record from the settings.
    pub(crate) fn capture_target_id(
        &mut self,
        game_process: Option<&GameProcessMatch>,
    ) -> Result<String, String> {
        let game_window = if let Some(game) = game_process {
            match clipcraft_platform::window_for_process(game.process_id) {
                Ok(Some(window)) => Some(window),
                Ok(None) => {
                    return Err(format!(
                        "ClipCraft found {} running, but this desktop did not expose its game window. Capture is still idle.",
                        game.game_name
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "ClipCraft found {} running, but could not read its game window: {error}",
                        game.game_name
                    ));
                }
            }
        } else {
            None
        };
        if self.recording.source == clipcraft_engine::CaptureSource::Game
            && let Some(window) = game_window
        {
            return Ok(window.id);
        }
        if !self.capture.targets_loaded() {
            self.capture.refresh_targets();
        }
        // On Wayland the desktop asks what to share; there is nothing to pick here.
        #[cfg(target_os = "linux")]
        if self
            .capture
            .targets
            .iter()
            .any(|target| target.id == clipcraft_platform::PORTAL_TARGET)
        {
            return Ok(clipcraft_platform::PORTAL_TARGET.to_owned());
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
            None => Err("No monitor was found to record.".to_owned()),
        }
    }

    pub(crate) fn start_capture(&mut self) {
        let game = if self.recording.game_only_capture {
            self.request_game_refresh();
            if self.capture.game_arming.is_paused() {
                self.capture.notice = notice(
                    "Game capture is paused until all selected games have closed.",
                    false,
                    None,
                );
                return;
            }
            match self.capture.game_arming.observed().cloned() {
                Some(game) => Some(game),
                None => {
                    self.capture.notice = notice(
                        if !self.recording.has_game_rules() {
                            "Enable built-in recognition or add a custom game in Settings › Games."
                        } else {
                            "Checking for a recognized game. Capture stays idle until its process is running."
                        },
                        false,
                        None,
                    );
                    return;
                }
            }
        } else {
            None
        };
        self.start_capture_for_game(game);
    }

    fn start_capture_for_game(&mut self, game: Option<GameProcessMatch>) {
        if self.capture.is_active() {
            return;
        }
        self.capture.notice = None;
        if game.is_none() || self.recording.source != clipcraft_engine::CaptureSource::Game {
            self.capture.refresh_targets();
        }
        let id = match self.capture_target_id(game.as_ref()) {
            Ok(id) => id,
            Err(message) => {
                self.capture.notice = notice(message, true, None);
                self.capture.game_arming.capture_start_failed();
                return;
            }
        };
        let options = CaptureOptions {
            show_cursor: self.recording.capture_cursor,
            hdr_to_sdr: self.recording.hdr_compatibility,
        };
        // The desktop's chooser can stay open for minutes, so it is waited for off the window.
        #[cfg(target_os = "linux")]
        if id == clipcraft_platform::PORTAL_TARGET {
            let (sender, receiver) = mpsc::channel();
            self.capture.opening = Some(receiver);
            self.capture.opening_game = game.clone();
            self.capture.game_session = game.clone();
            if let Some(game) = game {
                self.capture.game_arming.capture_started(game);
            }
            self.capture.notice = notice(
                "Choose the monitor or window to clip in the window your desktop shows.",
                false,
                None,
            );
            std::thread::spawn(move || {
                let _ = sender.send(
                    clipcraft_platform::open(&id, &options).map_err(|error| error.to_string()),
                );
            });
            return;
        }
        match clipcraft_platform::open(&id, &options) {
            Ok(source) => {
                if self.begin_session(source) {
                    if let Some(game) = game {
                        self.capture.game_arming.capture_started(game.clone());
                        self.capture.game_session = Some(game);
                        self.capture.game_retry_after = None;
                    }
                } else {
                    self.capture.game_arming.capture_start_failed();
                }
            }
            Err(error) => {
                self.capture.notice = notice(error.to_string(), true, None);
                self.capture.game_arming.capture_start_failed();
            }
        }
    }

    /// Finishes starting once a screen is being captured.
    fn begin_session(&mut self, source: Box<dyn clipcraft_capture::CaptureSource>) -> bool {
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
                self.cue(Cue::ClippingStarted);
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
                true
            }
            Err(error) => {
                self.capture.notice = notice(error.to_string(), true, None);
                false
            }
        }
    }

    /// While clipping the program runs a little below normal priority, so a game that wants the
    /// processor gets it before the encoder does; back to normal when clipping stops.
    fn yield_to_the_game(&mut self) {
        let clipping = self.capture.session.is_active();
        if clipping != self.capture.background_priority {
            clipcraft_platform::set_background_priority(clipping);
            self.capture.background_priority = clipping;
        }
    }

    /// Picks up a screen share the desktop's chooser has finished setting up.
    fn poll_opening(&mut self) {
        let Some(receiver) = &self.capture.opening else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.capture.opening = None;
                self.capture.notice = None;
                match result {
                    Ok(source) => {
                        if !self.begin_session(source) {
                            self.capture.opening_game = None;
                            self.capture.game_session = None;
                            self.capture.game_arming.capture_start_failed();
                        } else if let Some(game) = self.capture.opening_game.take() {
                            self.capture.game_arming.capture_started(game.clone());
                            self.capture.game_session = Some(game);
                        }
                    }
                    Err(message) => {
                        self.capture.notice = notice(message, true, None);
                        self.capture.opening_game = None;
                        self.capture.game_session = None;
                        self.capture.game_arming.capture_start_failed();
                    }
                }
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.capture.opening = None;
                self.capture.opening_game = None;
                self.capture.game_session = None;
                self.capture.game_arming.capture_start_failed();
            }
        }
    }

    pub(crate) fn stop_capture(&mut self) {
        // Cancelling while the chooser is open: the share is dropped when it arrives.
        self.capture.opening = None;
        self.capture.opening_game = None;
        let was_clipping = self.capture.session.is_active();
        self.capture.session.stop();
        if was_clipping {
            self.cue(Cue::ClippingStopped);
        }
        self.capture.started_with = None;
        self.capture.status = None;
        self.capture.game_session = None;
        self.capture.desktop_fallback = false;
        self.capture.pending_transition = None;
        self.capture.game_arming.capture_stopped_automatically();
    }

    fn toggle_capture(&mut self) {
        if self.capture.is_active() {
            if self.recording.game_only_capture {
                self.capture.game_arming.capture_stopped_by_user();
            }
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
        let Some(folder) = self.clips_folder() else {
            self.capture.notice = notice("Choose a capture folder in Settings first.", true, None);
            return;
        };
        let game = self
            .capture
            .game_session
            .as_ref()
            .map(|game| game.game_name.as_str());
        let job = match self
            .capture
            .session
            .prepare_save(&self.recording, &folder)
            .map(|job| job.with_game(game))
        {
            Ok(job) => job,
            Err(error) => {
                self.capture.notice = notice(error.to_string(), true, None);
                return;
            }
        };
        self.launch_save(job, folder);
    }

    fn launch_save(&mut self, job: clipcraft_engine::SaveJob, folder: PathBuf) {
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
                let mut metadata_warning = false;
                if let Some(game) = report.recorded_game.as_ref() {
                    let path = report.clip.path.to_string_lossy().into_owned();
                    if self.saved_games.len() >= clipcraft_engine::MAX_LIBRARY_RESULTS
                        && let Some(oldest) = self.saved_games.keys().next().cloned()
                    {
                        self.saved_games.remove(&oldest);
                    }
                    self.saved_games.insert(path, game.clone());
                    if let (Some(index_path), Some(directory)) =
                        (self.index_path.clone(), report.clip.path.parent())
                    {
                        metadata_warning = !matches!(clipcraft_engine::execute(clipcraft_engine::Command::RegisterSavedClip {
                            index_path, directory: directory.to_path_buf(), path: report.clip.path.clone(), game: game.clone(),
                        }), Ok(clipcraft_engine::CommandOutput::LibraryScan(result)) if result.persisted);
                    }
                }
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
                    format!(
                        "Saved {name}{removed}{}",
                        if metadata_warning {
                            "; game label could not be saved"
                        } else {
                            ""
                        }
                    ),
                    metadata_warning,
                    Some(report.clip.path.clone()),
                );
                self.cue(Cue::ClipSaved);
                // A saved clip belongs in the library when its folder is the one being browsed.
                let in_library = self.capture.saved_into.as_ref().is_some_and(|folder| {
                    folder.as_path() == std::path::Path::new(self.directory_text.trim())
                });
                if in_library {
                    self.scan_library();
                } else if self.directory_text.trim().is_empty()
                    && let Some(folder) = self.capture.saved_into.clone()
                {
                    // No library folder yet: the folder clips were saved in becomes it.
                    self.set_library_folder(Some(folder));
                }
            }
            Err(message) => {
                self.capture.notice = notice(message, true, None);
                self.cue(Cue::Problem);
            }
        }
    }

    /// Plays a sound effect, when they are switched on.
    pub(crate) fn cue(&self, cue: Cue) {
        if self.recording.sound_effects {
            clipcraft_audio_out::play_cue(
                cue,
                u8::try_from(self.recording.sound_effects_percent.min(100)).unwrap_or(100),
            );
        }
    }

    /// Saves a picture of the screen being clipped, on another thread.
    pub(crate) fn take_screenshot(&mut self) {
        if self.capture.shooting.is_some() {
            return;
        }
        let Some(folder) = self.clips_folder() else {
            self.capture.notice = notice("Choose a clips folder in Settings first.", true, None);
            return;
        };
        let job = match self.capture.session.screenshot_job(&folder) {
            Ok(job) => job,
            Err(_) => {
                self.capture.notice =
                    notice("Start clipping first, then take a screenshot.", true, None);
                self.cue(Cue::Problem);
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        self.capture.shooting = Some(receiver);
        std::thread::spawn(move || {
            let _ = sender.send(job.run().map_err(|error| error.to_string()));
        });
    }

    /// Picks up a screenshot that has been written.
    fn poll_screenshot(&mut self) {
        let Some(receiver) = self.capture.shooting.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("The screenshot could not be taken.".to_owned())
            }
        };
        self.capture.shooting = None;
        match result {
            Ok(path) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.capture.notice = notice(format!("Saved {name}"), false, Some(path));
                self.cue(Cue::Screenshot);
            }
            Err(message) => {
                self.capture.notice = notice(message, true, None);
                self.cue(Cue::Problem);
            }
        }
    }

    /// The recording controls in the top bar.
    pub(crate) fn capture_controls(&mut self, ui: &mut egui::Ui) {
        let active = self.capture.is_active();
        let game_name = self.capture.game_recording_name().map(str::to_owned);
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
                    (
                        LIME,
                        game_name
                            .map(|game| format!("Clipping · {game}"))
                            .unwrap_or_else(|| {
                                if self.capture.desktop_fallback {
                                    "Clipping · Desktop (game closed)".to_owned()
                                } else {
                                    format!("Clipping{tag}")
                                }
                            }),
                    )
                }
                RecordState::Failed(_) => (DANGER, "Capture stopped".to_owned()),
                RecordState::Stopped => (FAINT, "Not clipping".to_owned()),
            },
            None if self.recording.game_only_capture && self.capture.game_capture_paused() => {
                (FAINT, "Game capture paused".to_owned())
            }
            None if self.recording.game_only_capture && !active => {
                (FAINT, "Waiting for game".to_owned())
            }
            None if self.recording.game_only_capture => {
                let label = game_name
                    .map(|game| format!("Opening capture · {game}"))
                    .unwrap_or_else(|| "Opening capture".to_owned());
                (MUTED, label)
            }
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
    }

    /// The latest message as a small card under the top bar, so a long one is never squeezed into
    /// the bar itself. A message with a file offers to show it; any can be dismissed.
    pub(crate) fn notice_toast(&mut self, context: &egui::Context) {
        let Some((text, error, path)) = self
            .capture
            .notice
            .as_ref()
            .map(|current| (current.text.clone(), current.error, current.path.clone()))
        else {
            return;
        };
        let mut show = false;
        let mut dismiss = false;
        egui::Area::new(egui::Id::new("notice-toast"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 62.0))
            .order(egui::Order::Foreground)
            .show(context, |ui| {
                egui::Frame::new()
                    .fill(kit::PANEL)
                    .stroke(Stroke::new(
                        1.0_f32,
                        if error {
                            Color32::from_rgba_unmultiplied(255, 112, 112, 150)
                        } else {
                            kit::BORDER
                        },
                    ))
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .shadow(egui::Shadow {
                        offset: [0, 4],
                        blur: 16,
                        spread: 0,
                        color: Color32::from_black_alpha(120),
                    })
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&text)
                                        .color(if error { DANGER } else { TEXT })
                                        .size(13.0),
                                )
                                .wrap(),
                            );
                            ui.add_space(6.0);
                            if path.is_some()
                                && kit::labelled_button(ui, Icon::Folder, "Show", false, true)
                                    .clicked()
                            {
                                show = true;
                            }
                            let (rect, response) =
                                ui.allocate_exact_size(Vec2::splat(24.0), egui::Sense::click());
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    "Dismiss message",
                                )
                            });
                            let ink = if response.hovered() { TEXT } else { MUTED };
                            let stroke = Stroke::new(1.4_f32, ink);
                            let c = rect.center();
                            ui.painter().line_segment(
                                [c + Vec2::new(-4.0, -4.0), c + Vec2::new(4.0, 4.0)],
                                stroke,
                            );
                            ui.painter().line_segment(
                                [c + Vec2::new(4.0, -4.0), c + Vec2::new(-4.0, 4.0)],
                                stroke,
                            );
                            dismiss = response.on_hover_text("Dismiss").clicked();
                        });
                    });
            });
        if show && let Some(path) = path {
            self.reveal_media(path);
        }
        if dismiss {
            self.capture.notice = None;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_completed_save_updates_game_filters_and_metadata_failure_keeps_the_clip() {
        for persist in [true, false] {
            let root = crate::tests::temporary_directory();
            let clips = root.join("clips");
            std::fs::create_dir(&clips).unwrap();
            let path = clips.join("ClipCraft 2026-10-22.mp4");
            std::fs::write(&path, b"saved media").unwrap();
            let index = root.join("index.json");
            if !persist {
                std::fs::create_dir(&index).unwrap();
            }
            let mut app = ClipCraftApp {
                directory_text: clips.to_str().unwrap().into(),
                index_path: Some(index.clone()),
                ..Default::default()
            };
            app.recording.sound_effects = false;
            let (sender, receiver) = mpsc::channel();
            app.capture.saving = Some(receiver);
            app.capture.saved_into = Some(clips);
            sender
                .send(Ok(SavedClipReport {
                    clip: clipcraft_record::SavedClip {
                        path: path.clone(),
                        frames: 30,
                        duration_frames: 30,
                        bytes: 11,
                        audio_tracks: 0,
                        start_frame: 0,
                    },
                    deleted: Vec::new(),
                    recorded_game: Some("Captured Game".into()),
                }))
                .unwrap();
            app.poll_save();
            assert_eq!(app.capture.notice.as_ref().unwrap().error, !persist);
            let deadline = Instant::now() + Duration::from_secs(5);
            while app.scanning.is_some() {
                assert!(Instant::now() < deadline);
                app.poll_library_scan(&egui::Context::default());
                std::thread::yield_now();
            }
            let library = app.library.as_ref().unwrap();
            assert_eq!(
                clipcraft_engine::games(&library.entries, &library.directory),
                [clipcraft_engine::GameGroup {
                    name: "Captured Game".into(),
                    clips: 1
                }]
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"saved media");
            if persist {
                let output = clipcraft_engine::execute(clipcraft_engine::Command::LoadLibrary {
                    index_path: index,
                })
                .unwrap();
                let clipcraft_engine::CommandOutput::LibraryScan(loaded) = output else {
                    panic!("library output")
                };
                assert_eq!(
                    loaded.entries[0].recorded_game.as_deref(),
                    Some("Captured Game")
                );
            }
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
