//! Recording from the app: starting and stopping the replay buffer, saving a clip, the hotkeys
//! that do both, and the controls in the top bar.

use clipcraft_audio_out::Cue;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clipcraft_capture::{CaptureTarget, TargetKind};
use clipcraft_engine::{
    CaptureSession, DesktopCaptureMode, GameArmingDecision, GameArmingState, GameProcessMatch,
    HotkeyAction, RecordingSettings, SavedClipReport,
};
use clipcraft_platform::{CaptureOptions, HotkeyBinding, HotkeyListener};
use clipcraft_record::{RecordError, RecordState, RecordStatus};
use eframe::egui;
use egui::RichText;
use egui::{Color32, Stroke, Vec2};

use crate::ClipCraftApp;
use crate::kit::{self, DANGER, FAINT, Icon, LIME, MUTED, TEXT};
use crate::settings::GpuProbe;

/// Opens microphones and the computer's own sound for the recorder.
fn receive_hotkey(
    held: &mut std::collections::HashSet<u32>,
    event: clipcraft_platform::HotkeyEvent,
    assigning: bool,
) -> Option<HotkeyAction> {
    let action = clipcraft_engine::Hotkeys::action_for_id(event.id)?;
    if action == HotkeyAction::PushToTalk {
        if event.pressed {
            held.insert(event.id);
        } else {
            held.remove(&event.id);
        }
        None
    } else {
        (event.pressed && !assigning).then_some(action)
    }
}

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
    queued_save: Option<(clipcraft_engine::SaveJob, PathBuf)>,
    saving_path: Option<PathBuf>,
    /// A screenshot being taken and written, and the file it made.
    shooting: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    /// A screen share being set up while the desktop's own chooser is open.
    opening: Option<mpsc::Receiver<Result<Box<dyn clipcraft_capture::CaptureSource>, String>>>,
    /// The game that caused a pending desktop share to open.
    opening_game: Option<GameProcessMatch>,
    /// When the clip being saved will be written, if it is waiting for the post-roll.
    saving_at: Option<Instant>,
    listener: Option<HotkeyListener>,
    hotkey_settings: Option<clipcraft_engine::Hotkeys>,
    talk_keys: std::collections::HashSet<u32>,
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
    automatic_epoch: Option<Instant>,
    #[cfg(test)]
    test_targets: Option<Vec<CaptureTarget>>,
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

    /// How the running recording is doing, if one is running.
    pub fn record_status(&self) -> Option<clipcraft_record::RecordStatus> {
        self.session.status()
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
                || started.microphone_noise_suppression != current.microphone_noise_suppression
                || started.audio_sample_rate != current.audio_sample_rate
                || started.audio_channels != current.audio_channels
                || started.audio_kbps != current.audio_kbps
        })
    }

    /// Reads the monitors and windows the provider offers right now.
    pub fn refresh_targets(&mut self) {
        #[cfg(test)]
        if let Some(targets) = &self.test_targets {
            self.targets = targets.clone();
            self.targets_loaded = true;
            return;
        }
        self.targets = clipcraft_platform::list_targets().unwrap_or_default();
        self.targets_loaded = true;
    }

    pub fn targets_loaded(&self) -> bool {
        self.targets_loaded
    }
}

fn automatic_desktop_target(target: &CaptureTarget) -> bool {
    #[cfg(target_os = "linux")]
    if target.id == clipcraft_platform::PORTAL_TARGET {
        return false;
    }
    target.kind == TargetKind::Display
}

pub(crate) fn notice(
    text: impl Into<String>,
    error: bool,
    path: Option<PathBuf>,
) -> Option<Notice> {
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
        self.check_desktop_timer();
        self.poll_game_arming(context);
        if self.recording.game_only_capture {
            context.request_repaint_after(Duration::from_secs(1));
        }
        self.yield_to_the_game();
        self.sync_hotkeys();
        let mut actions = Vec::new();
        if let Some(listener) = self.capture.listener.as_ref() {
            while let Some(event) = listener.try_next() {
                if let Some(action) = receive_hotkey(
                    &mut self.capture.talk_keys,
                    event,
                    self.settings_state.capturing.is_some(),
                ) {
                    actions.push(action);
                }
            }
        }
        self.capture.session.set_talking(
            !self.capture.talk_keys.is_empty() && self.settings_state.capturing.is_none(),
        );
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
                if self
                    .recording
                    .hotkeys
                    .bindings(action)
                    .any(|key| key == pressed)
                {
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
        if self.capture.hotkey_settings.as_ref() == Some(&self.recording.hotkeys) {
            return;
        }
        self.capture.hotkey_settings = Some(self.recording.hotkeys.clone());
        let wanted: Vec<(u32, String)> = self
            .recording
            .hotkeys
            .registrations()
            .map(|(id, _, key)| (id, key.to_owned()))
            .collect();
        self.capture.listener = None;
        self.capture.talk_keys.clear();
        self.capture.session.set_talking(false);
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
        if self.capture.desktop_fallback
            && self.recording.desktop_capture_mode == DesktopCaptureMode::Never
        {
            self.capture.pending_transition = Some(GameArmingDecision::Stop);
            self.advance_game_transition();
        }
        if let Some(job) = &self.capture.game_scan {
            let result = match job.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty)
                    if self
                        .capture
                        .game_worker
                        .as_ref()
                        .is_some_and(|worker| worker.is_finished()) =>
                {
                    Some(Err(
                        "Game discovery stopped unexpectedly. Refresh to try again.".to_owned(),
                    ))
                }
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
        if !self.recording.has_game_rules()
            && self.recording.desktop_capture_mode != DesktopCaptureMode::Always
            && !self.capture.manual_game_refresh
        {
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
                if self.capture.game_session.is_some() || self.capture.desktop_fallback {
                    self.stop_capture();
                }
                self.settings_state.set_game_processes(Err(error.clone()));
                self.capture.notice = notice(error, true, None);
                self.capture.game_worker = None;
                self.capture.game_scan_due = Some(Instant::now() + Duration::from_secs(10));
            }
        }
    }

    fn automatic_now(&mut self) -> Duration {
        self.capture
            .automatic_epoch
            .get_or_insert_with(Instant::now)
            .elapsed()
    }

    fn check_desktop_timer(&mut self) {
        if !self.recording.game_only_capture || self.capture.pending_transition.is_some() {
            return;
        }
        let now = self.automatic_now();
        if let Some(decision) = self.capture.game_arming.desktop_timeout(
            now,
            self.recording.desktop_capture_mode,
            self.recording.desktop_after_game_seconds,
        ) {
            self.capture.session.halt_for_transition();
            self.capture.pending_transition = Some(decision);
            self.advance_game_transition();
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
        let decision = self.capture.game_arming.observe_automatic(
            &found.matches,
            self.recording.desktop_capture_mode,
            exited,
        );
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
            Some(GameArmingDecision::StartDesktop)
                if self
                    .capture
                    .game_retry_after
                    .is_none_or(|at| Instant::now() >= at) =>
            {
                self.capture.game_retry_after = Some(Instant::now() + Duration::from_secs(10));
                self.capture.pending_transition = Some(GameArmingDecision::StartDesktop)
            }
            Some(GameArmingDecision::Stop)
                if self.capture.game_session.is_some() || self.capture.desktop_fallback =>
            {
                self.capture.pending_transition = Some(GameArmingDecision::Stop);
            }
            _ => {}
        }
        self.advance_game_transition();
    }

    fn advance_game_transition(&mut self) {
        let Some(transition) = self.capture.pending_transition.take() else {
            return;
        };
        if !self.recording.game_only_capture || self.capture.game_arming.is_paused() {
            return;
        }
        let transition = match transition {
            GameArmingDecision::StartDesktop
                if self.recording.desktop_capture_mode == DesktopCaptureMode::Never =>
            {
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
                    // The requested game may have exited while both save slots were occupied.
                    // Resolve the latest destination, then preserve the retained replay below.
                    self.capture
                        .game_arming
                        .observed()
                        .cloned()
                        .map(GameArmingDecision::Start)
                        .unwrap_or_else(|| {
                            if self.recording.desktop_capture_mode == DesktopCaptureMode::Always {
                                GameArmingDecision::StartDesktop
                            } else {
                                GameArmingDecision::Stop
                            }
                        })
                } else {
                    GameArmingDecision::Start(game)
                }
            }
            other => other,
        };
        if self.capture.desktop_fallback
            || (self.capture.game_session.is_some()
                && self.recording.desktop_capture_mode != DesktopCaptureMode::Never)
        {
            // The SaveJob owns the old bounded recorder, so stopping/changing source cannot
            // overwrite its game replay. Skip post-roll because the source has disappeared.
            if self.capture.saving.is_some() && self.capture.queued_save.is_some() {
                self.capture.session.halt_for_transition();
                self.capture.pending_transition = Some(transition);
                self.capture.notice = notice(
                    "Recording paused while the previous clips finish saving.",
                    false,
                    None,
                );
                return;
            }
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
        let reserved: Vec<_> = self
            .capture
            .saving_path
            .iter()
            .cloned()
            .chain(
                self.capture
                    .queued_save
                    .iter()
                    .map(|(job, _)| job.path().to_path_buf()),
            )
            .collect();
        let job = match self
            .capture
            .session
            .prepare_save(&settings, &folder)
            .and_then(|job| job.avoid_paths(&reserved))
        {
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
        if self.capture.saving.is_some() {
            self.capture.queued_save = Some((job, folder));
        } else {
            self.launch_save(job, folder);
        }
        true
    }

    fn start_desktop_fallback(&mut self) {
        self.capture.refresh_targets();
        let chosen = &self.recording.capture_target;
        let target = self
            .capture
            .targets
            .iter()
            .filter(|target| automatic_desktop_target(target))
            .find(|target| target.id == *chosen)
            .or_else(|| {
                self.capture
                    .targets
                    .iter()
                    .find(|target| automatic_desktop_target(target))
            });
        // This start is automatic, so a failure is retried quietly (see `game_retry_after`)
        // instead of announcing an error the person did not cause.
        let Some(target) = target else {
            return;
        };
        let id = target.id.clone();
        let options = CaptureOptions {
            show_cursor: self.recording.capture_cursor,
            hdr_to_sdr: self.recording.hdr_compatibility,
            requested_fps: self.recording.frames_per_second,
        };
        if let Ok(source) = self.open_capture_source(&id, &options)
            && self.begin_session(source)
        {
            self.capture.desktop_fallback = true;
            self.capture.game_retry_after = None;
            let now = self.automatic_now();
            self.capture.game_arming.desktop_started_at(now);
            self.capture.notice = notice(
                "Recording the desktop. A recognized game will switch capture to its window.",
                false,
                None,
            );
        }
    }

    fn open_capture_source(
        &self,
        id: &str,
        options: &CaptureOptions,
    ) -> Result<Box<dyn clipcraft_capture::CaptureSource>, clipcraft_capture::CaptureError> {
        #[cfg(test)]
        if self
            .capture
            .test_targets
            .as_ref()
            .is_some_and(|targets| targets.iter().any(|target| target.id == id))
        {
            return Ok(Box::new(clipcraft_capture::SyntheticSource::new(
                160, 90, 30,
            )));
        }
        clipcraft_platform::open(id, options)
    }

    fn game_window_for_process(
        &self,
        process_id: u32,
    ) -> Result<Option<CaptureTarget>, clipcraft_capture::CaptureError> {
        #[cfg(test)]
        if let Some(targets) = &self.capture.test_targets {
            return Ok(targets
                .iter()
                .find(|target| target.id == format!("test-game:{process_id}"))
                .cloned());
        }
        clipcraft_platform::window_for_process(process_id)
    }

    /// Picks what to record from the settings.
    pub(crate) fn capture_target_id(
        &mut self,
        game_process: Option<&GameProcessMatch>,
    ) -> Result<String, String> {
        let game_window = if let Some(game) = game_process {
            match self.game_window_for_process(game.process_id) {
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
        if let Some(window) = game_window {
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
        self.capture.game_arming.resume();
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
                None if self.recording.desktop_capture_mode == DesktopCaptureMode::Always => {
                    self.start_desktop_fallback();
                    return;
                }
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
            requested_fps: self.recording.frames_per_second,
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
        match self.open_capture_source(&id, &options) {
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
        // Recording and playing a clip both wait on short timers; Windows rounds those up to
        // 15.6 ms unless asked for a finer clock, which halved playback to 30 pictures a second.
        clipcraft_platform::set_fine_timer(clipping || self.playback.is_some() || self.editor.open);
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
        self.capture.saving_path = Some(job.path().to_path_buf());
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
        self.capture.saving_path = None;
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
                        metadata_warning = self
                            .index_store
                            .submit(crate::library_store::Write::Saved {
                                index: index_path,
                                directory: directory.to_path_buf(),
                                path: report.clip.path.clone(),
                                game: game.clone(),
                            })
                            .is_err();
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
                if self.recording.show_save_notification {
                    self.saved_toast = Some(crate::saved_toast::SavedToast::new(
                        report.clip.path.clone(),
                        report
                            .recorded_game
                            .clone()
                            .unwrap_or_else(|| "Clip".to_owned()),
                        saved_clip_facts(&report, &self.recording),
                        self.thumbnail_dir.clone(),
                    ));
                }
                // The card says it all; the top bar only speaks up when there is more to tell.
                if !self.recording.show_save_notification
                    || metadata_warning
                    || !report.deleted.is_empty()
                {
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
                }
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
        if let Some((job, folder)) = self.capture.queued_save.take() {
            self.launch_save(job, folder);
        }
    }

    /// The card for a clip that was just saved: what to do with it. Called every frame.
    pub(crate) fn saved_toast_frame(&mut self, context: &egui::Context) {
        let Some(mut toast) = self.saved_toast.take() else {
            return;
        };
        let ppp = context.pixels_per_point().max(0.5);
        let free = clipcraft_platform::work_area().map(|(left, top, right, bottom)| {
            egui::Rect::from_min_max(
                egui::pos2(left as f32 / ppp, top as f32 / ppp),
                egui::pos2(right as f32 / ppp, bottom as f32 / ppp),
            )
        });
        let (action, alive) = toast.show(context, free);
        let path = toast.path.clone();
        match action {
            Some(crate::saved_toast::ToastAction::Play) => {
                self.pending_open = Some(path.clone());
                crate::tray::show(context);
                self.page = crate::Page::Library;
            }
            Some(crate::saved_toast::ToastAction::Show) => self.reveal_media(path.clone()),
            Some(crate::saved_toast::ToastAction::Delete) => {
                let title = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.start_trash(path.to_string_lossy().into_owned(), title);
                toast.mark_deleted();
            }
            None => {}
        }
        if alive {
            self.saved_toast = Some(toast);
            context.request_repaint_after(std::time::Duration::from_millis(16));
        }
        // A clip the card was asked to play opens once the library lists it.
        if let Some(wanted) = self.pending_open.clone() {
            let found = self.library.as_ref().and_then(|library| {
                library
                    .entries
                    .iter()
                    .find(|entry| std::path::Path::new(&entry.path) == wanted)
                    .cloned()
            });
            if let Some(entry) = found {
                self.selected = Some(entry);
                self.pending_open = None;
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
        let now = self.automatic_now();
        let desktop_label = self
            .capture
            .game_arming
            .desktop_remaining(
                now,
                self.recording.desktop_capture_mode,
                self.recording.desktop_after_game_seconds,
            )
            .map(|remaining| {
                let seconds = remaining
                    .as_secs()
                    .saturating_add(u64::from(remaining.subsec_nanos() > 0));
                format!("Desktop · {}:{:02} left", seconds / 60, seconds % 60)
            })
            .unwrap_or_else(|| "Clipping · Desktop".to_owned());
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
                                    desktop_label.clone()
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
                (FAINT, "Automatic capture paused".to_owned())
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
                .on_hover_text({
                    let mut message = status
                        .audio
                        .iter()
                        .filter_map(|input| input.error.as_deref())
                        .take(8)
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !message.is_empty() {
                        message.push('\n');
                    }
                    message.push_str("ClipCraft keeps trying to bring the device back.");
                    message
                });
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

/// The line under the title on the card: how long the clip is, how big, and at what size.
fn saved_clip_facts(
    report: &clipcraft_engine::SavedClipReport,
    settings: &clipcraft_engine::RecordingSettings,
) -> String {
    let seconds = report.clip.duration_frames / u64::from(settings.frames_per_second.max(1));
    let length = if seconds >= 60 {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    } else {
        format!("{seconds} s")
    };
    format!(
        "{length} · {} · {}p",
        clipcraft_engine::format_bytes(report.clip.bytes),
        settings.height
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn talking_stays_open_until_every_alternative_is_released() {
        let mut keys = clipcraft_engine::Hotkeys::default();
        keys.set_binding(HotkeyAction::PushToTalk, 0, "F1").unwrap();
        keys.set_binding(HotkeyAction::PushToTalk, 1, "F2").unwrap();
        let ids: Vec<_> = keys
            .registrations()
            .filter(|(_, action, _)| *action == HotkeyAction::PushToTalk)
            .map(|(id, _, _)| id)
            .collect();
        let mut held = std::collections::HashSet::new();
        for (id, pressed, count) in [
            (ids[0], true, 1),
            (ids[1], true, 2),
            (ids[0], false, 1),
            (ids[1], false, 0),
        ] {
            assert_eq!(
                receive_hotkey(
                    &mut held,
                    clipcraft_platform::HotkeyEvent { id, pressed },
                    false
                ),
                None
            );
            assert_eq!(held.len(), count);
        }
        assert_eq!(
            receive_hotkey(
                &mut held,
                clipcraft_platform::HotkeyEvent {
                    id: 1,
                    pressed: true
                },
                true
            ),
            None
        );
        assert_eq!(
            receive_hotkey(
                &mut held,
                clipcraft_platform::HotkeyEvent {
                    id: 1,
                    pressed: true
                },
                false
            ),
            Some(HotkeyAction::SaveClip)
        );
    }

    #[test]
    fn changing_noise_suppression_requires_a_recording_restart() {
        let started = RecordingSettings::default();
        let capture = CaptureState {
            started_with: Some(started.clone()),
            ..Default::default()
        };
        assert!(!capture.is_stale(&started));
        assert!(capture.is_stale(&RecordingSettings {
            microphone_noise_suppression: true,
            ..started
        }));
    }

    fn observation(game: Option<u32>) -> crate::game_scan::ResultSet {
        crate::game_scan::ResultSet {
            processes: game
                .map(|id| clipcraft_platform::RunningProcess {
                    process_id: id,
                    executable: "test-game".into(),
                    steam_app_id: None,
                })
                .into_iter()
                .collect(),
            matches: game
                .map(|id| GameProcessMatch {
                    process_id: id,
                    game_name: "Test game".into(),
                })
                .into_iter()
                .collect(),
            steam: Default::default(),
            steam_warning: None,
        }
    }
    /// Removes a test's scratch folder. A capture worker can still be finishing a file as the app
    /// is dropped, so retry briefly; leftovers in a temp folder never fail a test.
    fn remove_scratch(root: PathBuf) {
        for _ in 0..50 {
            if std::fs::remove_dir_all(&root).is_ok() || !root.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn automatic_app(mode: DesktopCaptureMode) -> (ClipCraftApp, PathBuf) {
        let root = crate::tests::temporary_directory();
        let mut app = ClipCraftApp {
            directory_text: root.to_str().unwrap().into(),
            ..Default::default()
        };
        app.recording = RecordingSettings {
            desktop_capture_mode: mode,
            desktop_after_game_seconds: 15,
            quality: clipcraft_engine::QualityMode::Custom,
            height: 90,
            frames_per_second: 30,
            video_encoder: clipcraft_engine::VideoEncoder::Cpu,
            video_kbps: 500,
            replay_seconds: 15,
            capture_game_audio: false,
            capture_microphone: false,
            sound_effects: false,
            ..Default::default()
        };
        app.capture.test_targets = Some(vec![
            CaptureTarget {
                id: "test-display".into(),
                name: "Desktop".into(),
                kind: TargetKind::Display,
                width: 160,
                height: 90,
            },
            CaptureTarget {
                id: "test-game:42".into(),
                name: "Game".into(),
                kind: TargetKind::Window,
                width: 160,
                height: 90,
            },
            CaptureTarget {
                id: "test-game:43".into(),
                name: "Reopened game".into(),
                kind: TargetKind::Window,
                width: 160,
                height: 90,
            },
        ]);
        (app, root)
    }
    fn wait_for_frames(app: &ClipCraftApp) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app
            .capture
            .session
            .status()
            .is_none_or(|status| status.frames_encoded < 3)
        {
            assert!(Instant::now() < deadline, "recorder did not produce frames");
            std::thread::yield_now();
        }
    }
    fn finish_saves(app: &mut ClipCraftApp) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.capture.saving.is_some() || app.capture.queued_save.is_some() {
            assert!(Instant::now() < deadline, "save did not finish");
            app.poll_save();
            std::thread::yield_now();
        }
        assert!(
            app.capture
                .notice
                .as_ref()
                .is_none_or(|notice| !notice.error),
            "{}",
            app.capture
                .notice
                .as_ref()
                .map(|notice| notice.text.as_str())
                .unwrap_or("")
        );
    }
    #[test]
    fn an_obsolete_blocked_process_snapshot_cannot_start_capture_after_settings_change() {
        let (mut app, root) = automatic_app(DesktopCaptureMode::AfterGame);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        app.capture.game_worker = Some(crate::game_scan::Worker::fake(move |_, _| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(observation(Some(42)))
        }));
        let context = egui::Context::default();
        app.poll_game_arming(&context);
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        app.recording.detect_built_in_games = false;
        app.poll_game_arming(&context); // receiver is invalidated while the worker is blocked
        assert!(app.capture.game_scan.is_none());
        release_tx.send(()).unwrap();
        app.poll_game_arming(&context);
        assert!(!app.capture.is_active());
        drop(app);
        remove_scratch(root);
    }

    #[test]
    fn generated_game_desktop_reopen_exit_and_timer_save_keep_media_and_game_labels() {
        let (mut app, root) = automatic_app(DesktopCaptureMode::AfterGame);
        let original = root.join("original.mp4");
        std::fs::write(&original, b"original untouched").unwrap();
        app.apply_game_observation(observation(None));
        assert!(!app.capture.is_active());
        app.apply_game_observation(observation(Some(42)));
        wait_for_frames(&app);
        assert_eq!(app.capture.game_session.as_ref().unwrap().process_id, 42);
        app.apply_game_observation(observation(None));
        assert!(app.capture.desktop_fallback);
        assert!(app.capture.game_session.is_none());
        wait_for_frames(&app);
        // Reopen while the first game replay may still be saving. Transition saving is bounded
        // to one running and one queued job, with distinct paths even within one timestamp.
        app.apply_game_observation(observation(Some(43)));
        assert!(!app.capture.desktop_fallback);
        assert_eq!(app.capture.game_session.as_ref().unwrap().process_id, 43);
        wait_for_frames(&app);
        finish_saves(&mut app);
        app.apply_game_observation(observation(None));
        assert!(app.capture.desktop_fallback);
        wait_for_frames(&app);
        finish_saves(&mut app);
        app.capture.automatic_epoch = Instant::now().checked_sub(Duration::from_secs(100));
        app.check_desktop_timer();
        finish_saves(&mut app);
        app.advance_game_transition();
        assert!(!app.capture.is_active());
        assert!(!app.capture.desktop_fallback);
        assert_eq!(std::fs::read(&original).unwrap(), b"original untouched");
        let clips: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("ClipCraft")
            })
            .collect();
        assert_eq!(clips.len(), 4);
        assert_eq!(app.saved_games.len(), 2); // game replays tagged; desktop replays untagged
        for path in clips {
            assert!(matches!(
                clipcraft_engine::execute(clipcraft_engine::Command::ProbeClip { path }),
                Ok(clipcraft_engine::CommandOutput::ClipProbe(_))
            ));
        }
        drop(app);
        remove_scratch(root);
    }
    #[test]
    fn always_mode_starts_on_desktop_and_manual_stop_does_not_restart() {
        let (mut app, root) = automatic_app(DesktopCaptureMode::Always);
        app.apply_game_observation(observation(None));
        assert!(app.capture.desktop_fallback);
        wait_for_frames(&app);
        app.toggle_capture();
        app.apply_game_observation(observation(None));
        assert!(!app.capture.is_active());
        app.apply_game_observation(observation(Some(42)));
        assert!(!app.capture.is_active());
        app.start_capture();
        assert!(app.capture.game_session.is_some());
        wait_for_frames(&app);
        app.stop_capture();
        drop(app);
        remove_scratch(root);
    }
    #[test]
    fn a_game_exiting_while_saves_are_busy_cannot_discard_the_retained_desktop_replay() {
        let (mut app, root) = automatic_app(DesktopCaptureMode::AfterGame);
        app.apply_game_observation(observation(Some(42)));
        wait_for_frames(&app);
        // Hold a save open deterministically so the first transition fills the queued slot.
        let (sender, receiver) = mpsc::channel();
        app.capture.saving = Some(receiver);
        app.capture.saving_path = Some(root.join("in-flight.mp4"));
        app.apply_game_observation(observation(None));
        assert!(app.capture.desktop_fallback);
        assert!(app.capture.queued_save.is_some());
        wait_for_frames(&app);
        app.apply_game_observation(observation(Some(43)));
        assert!(matches!(
            app.capture.pending_transition,
            Some(GameArmingDecision::Start(_))
        ));
        app.apply_game_observation(observation(None)); // new game exits before it can open
        app.advance_game_transition();
        assert!(app.capture.pending_transition.is_some());
        assert!(app.capture.session.status().unwrap().frames_encoded > 0);
        sender
            .send(Err("generated slow-save failure".into()))
            .unwrap();
        app.poll_save();
        app.advance_game_transition();
        finish_saves(&mut app);
        assert!(!app.capture.is_active());
        assert_eq!(app.saved_games.len(), 1);
        let clips: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "mp4"))
            .collect();
        assert_eq!(clips.len(), 2); // both the game replay and the retained desktop replay survive
        for path in clips {
            assert!(matches!(
                clipcraft_engine::execute(clipcraft_engine::Command::ProbeClip { path }),
                Ok(clipcraft_engine::CommandOutput::ClipProbe(_))
            ));
        }
        drop(app);
        remove_scratch(root);
    }
    #[test]
    fn failed_game_open_and_rule_removal_cannot_start_temporary_desktop() {
        let (mut app, root) = automatic_app(DesktopCaptureMode::AfterGame);
        app.apply_game_observation(observation(Some(99)));
        assert!(!app.capture.is_active());
        app.apply_game_observation(observation(None));
        assert!(!app.capture.desktop_fallback);
        app.capture.game_retry_after = None;
        app.apply_game_observation(observation(Some(42)));
        wait_for_frames(&app);
        let mut removed_rule = observation(Some(42));
        removed_rule.matches.clear();
        app.apply_game_observation(removed_rule);
        assert!(!app.capture.desktop_fallback);
        assert!(!app.capture.is_active());
        drop(app);
        remove_scratch(root);
    }
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
            let deadline = Instant::now() + Duration::from_secs(5);
            while app.scanning.is_some()
                || (app.index_store.pending() > 0 && app.index_store.error.is_none())
                || app.library.is_none()
            {
                assert!(Instant::now() < deadline);
                app.poll_index_store(&egui::Context::default());
                app.poll_library_scan(&egui::Context::default());
                std::thread::yield_now();
            }
            assert_eq!(app.index_store.error.is_some(), !persist);
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
            app.index_store.finish();
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
