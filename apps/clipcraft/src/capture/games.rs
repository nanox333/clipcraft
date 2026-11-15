//! Game-only recording: finding a recognized game's window, following it, and moving capture
//! between the game, the desktop and idle without losing the replay buffer.

use super::*;

/// For each recognized game process, the processes that may own its window: the ones it started
/// (and theirs), plus every other process recognized as the same game and their children.
pub(super) fn game_families(
    found: &crate::game_scan::ResultSet,
) -> std::collections::HashMap<u32, Vec<u32>> {
    let mut families = std::collections::HashMap::new();
    for game in &found.matches {
        let mut family = Vec::new();
        for same in found
            .matches
            .iter()
            .filter(|other| other.game_name == game.game_name)
        {
            for id in clipcraft_platform::process_family(&found.processes, same.process_id) {
                if !family.contains(&id) {
                    family.push(id);
                }
            }
        }
        // The recognized process itself is looked at first.
        family.retain(|id| *id != game.process_id);
        family.insert(0, game.process_id);
        families.insert(game.process_id, family);
    }
    families
}

pub(super) fn automatic_desktop_target(target: &CaptureTarget) -> bool {
    #[cfg(target_os = "linux")]
    if target.id == clipcraft_platform::PORTAL_TARGET {
        return false;
    }
    target.kind == TargetKind::Display
}

impl ClipCraftApp {
    pub(crate) fn request_game_refresh(&mut self) {
        self.capture.manual_game_refresh = true;
        self.capture.game_scan_due = None;
    }

    pub(crate) fn poll_game_arming(&mut self, context: &egui::Context) {
        let config = crate::game_scan::DetectionConfig::from(&self.recording);
        if self.capture.game_config.as_ref() != Some(&config) {
            self.capture.game_scan = None; // obsolete receivers cannot apply stale settings
            self.capture.game_config = Some(config.clone());
            self.capture.game_scan_due = None;
        }
        if !self.recording.game_only_capture {
            if self.capture.game_only_was_enabled {
                if self.capture.automatic_session() {
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
                            if self.capture.automatic_session() {
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
            if self.capture.automatic_session() {
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
                if self.capture.automatic_session() {
                    self.stop_capture();
                }
                self.settings_state.set_game_processes(Err(error.clone()));
                self.capture.notice = notice(error, true, None);
                self.capture.game_worker = None;
                self.capture.game_scan_due = Some(Instant::now() + Duration::from_secs(10));
            }
        }
    }

    pub(crate) fn automatic_now(&mut self) -> Duration {
        self.capture
            .automatic_epoch
            .get_or_insert_with(Instant::now)
            .elapsed()
    }

    pub(crate) fn check_desktop_timer(&mut self) {
        if !self.recording.game_only_capture || self.capture.pending_transition.is_some() {
            return;
        }
        let now = self.automatic_now();
        if let Some(decision) = self.capture.game_arming.desktop_timeout(
            now,
            self.recording.desktop_capture_mode,
            self.recording.desktop_after_game_seconds,
        ) {
            self.capture.pending_transition = Some(decision);
            self.advance_game_transition();
        }
    }

    pub(crate) fn apply_game_observation(&mut self, found: crate::game_scan::ResultSet) {
        let exited = self.capture.game_session.as_ref().is_some_and(|game| {
            !found
                .processes
                .iter()
                .any(|process| process.process_id == game.process_id)
        });
        // "Found <game> running, but it has no window" is about a game that is no longer there, so
        // it must not stay on screen after the game closes.
        if found.matches.is_empty()
            && self
                .capture
                .notice
                .as_ref()
                .is_some_and(|notice| notice.error && notice.text.starts_with("ClipCraft found "))
        {
            self.capture.notice = None;
        }
        self.capture.game_families = game_families(&found);
        self.capture.process_names = found
            .processes
            .iter()
            .map(|process| (process.process_id, process.executable.clone()))
            .collect();
        self.settings_state.steam_games = found.steam;
        self.settings_state.steam_warning = found.steam_warning;
        self.settings_state.set_game_processes(Ok(found.processes));
        if !self.recording.game_only_capture {
            return;
        }
        let mut restart_after_loss = false;
        // The game's window closed while the game kept running: a launcher or loading window was
        // replaced by the game's own (FiveM, Rockstar titles, Unreal bootstraps), or the game
        // recreated its window when switching display mode. Start again on the new window
        // instead of staying stopped.
        if let Some(game) = self.capture.game_session.clone()
            && found.matches.contains(&game)
            && self.capture.session.status().is_some_and(|status| {
                status.source_lost
                    || matches!(
                        status.state,
                        RecordState::Failed(clipcraft_record::RecordError::Capture(_))
                    )
            })
            && self
                .capture
                .game_retry_after
                .is_none_or(|at| Instant::now() >= at)
        {
            self.capture.game_arming.capture_stopped_automatically();
            // A window that keeps failing (protected content, say) is retried every ten seconds,
            // not on every scan.
            restart_after_loss = true;
        }
        // Recording the screen for want of the game's window: move to the window once one shows.
        if self.capture.game_on_screen
            && let Some(game) = self.capture.game_session.clone()
            && found.matches.contains(&game)
            && self.capture.pending_transition.is_none()
            && matches!(self.usable_game_window(game.process_id), Ok(Some(_)))
        {
            self.capture.game_arming.capture_stopped_automatically();
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
                    if restart_after_loss {
                        self.capture.game_retry_after =
                            Some(Instant::now() + Duration::from_secs(10));
                    }
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
            Some(GameArmingDecision::Stop) if self.capture.automatic_session() => {
                self.capture.pending_transition = Some(GameArmingDecision::Stop);
            }
            _ => {}
        }
        self.advance_game_transition();
    }

    pub(crate) fn advance_game_transition(&mut self) {
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
        // Between two sources the same recording carries on, so the replay buffer is not lost.
        if self.switch_source_in_place(&transition) {
            return;
        }
        let had_replay = self
            .capture
            .session
            .status()
            .is_some_and(|status| status.buffered_frames > 0);
        // A game closing never saves a clip by itself: clips are saved only when you ask.
        self.stop_capture();
        match transition {
            GameArmingDecision::Start(game) => self.start_capture_for_game(Some(game)),
            GameArmingDecision::StartDesktop => self.start_desktop_fallback(),
            GameArmingDecision::Stop => {}
        }
        if had_replay
            && self.capture.saving.is_none()
            && self
                .capture
                .notice
                .as_ref()
                .is_none_or(|notice| !notice.error)
        {
            let message = if self.capture.is_active() {
                "Capture source changed. Previous buffered footage was cleared; the new buffer is filling."
            } else {
                "Capture stopped. Previous buffered footage was cleared."
            };
            self.capture.notice = notice(message, false, None);
        }
    }

    /// Moves a running recording to its next source (the desktop when a game window closes, a
    /// game's window when one opens) inside the same recording, so the replay buffer and the sound
    /// carry on and a clip saved afterwards covers the switch. Returns false when that is not
    /// possible (nothing recording, a stopped recorder, the desktop's chooser, a source that will
    /// not open), and the caller restarts as before.
    pub(crate) fn switch_source_in_place(&mut self, transition: &GameArmingDecision) -> bool {
        let healthy = self.capture.session.status().is_some_and(|status| {
            status.source_lost || matches!(status.state, RecordState::Running)
        });
        if !healthy || self.capture.opening.is_some() {
            return false;
        }
        let options = self.capture_options();
        let id = match transition {
            GameArmingDecision::Start(game) => {
                self.capture.game_on_screen = false;
                match self.capture_target_id(Some(game)) {
                    Ok(id) => id,
                    Err(_) => return false,
                }
            }
            GameArmingDecision::StartDesktop => match self.desktop_target_id() {
                Some(id) => id,
                None => return false,
            },
            GameArmingDecision::Stop => return false,
        };
        #[cfg(target_os = "linux")]
        if id == clipcraft_platform::PORTAL_TARGET {
            return false;
        }
        let Ok(source) = self.open_capture_source(&id, &options) else {
            return false;
        };
        if !self.capture.session.replace_source(source) {
            return false;
        }
        self.capture.game_retry_after = None;
        match transition {
            GameArmingDecision::Start(game) => {
                self.capture.desktop_fallback = false;
                self.capture.game_arming.capture_started(game.clone());
                self.capture.game_session = Some(game.clone());
            }
            _ => {
                self.capture.game_session = None;
                self.capture.game_on_screen = false;
                self.capture.desktop_fallback = true;
                let now = self.automatic_now();
                self.capture.game_arming.desktop_started_at(now);
                self.capture.notice = notice(
                    "Recording the desktop. A recognized game will switch capture to its window.",
                    false,
                    None,
                );
            }
        }
        true
    }

    /// The game's window, unless it recently gave only black pictures (then it counts as missing
    /// for a while, so capture does not keep going back to a window that shows nothing).
    fn usable_game_window(
        &self,
        process_id: u32,
    ) -> Result<Option<CaptureTarget>, clipcraft_capture::CaptureError> {
        match self.game_window_for_process(process_id) {
            Ok(Some(window))
                if self
                    .capture
                    .black_windows
                    .get(&window.id)
                    .is_some_and(|until| Instant::now() < *until) =>
            {
                Ok(None)
            }
            other => other,
        }
    }

    /// A game window that delivers only black pictures (some games and protected windows do) is
    /// swapped for the screen, in the same recording, so the clip is not black. The window is
    /// tried again after a minute, since a black loading screen passes.
    pub(crate) fn rescue_black_window(&mut self) {
        if !self.recording.game_only_capture
            || !self.recording.record_screen_when_game_window_missing
            || self.capture.game_on_screen
            || self.capture.pending_transition.is_some()
            || !self
                .capture
                .status
                .as_ref()
                .is_some_and(|status| status.source_black)
        {
            return;
        }
        let (Some(game), Some(window)) = (
            self.capture.game_session.clone(),
            self.capture.game_window_id.clone(),
        ) else {
            return;
        };
        self.capture
            .black_windows
            .insert(window, Instant::now() + Duration::from_secs(60));
        self.capture.pending_transition = Some(GameArmingDecision::Start(game.clone()));
        self.advance_game_transition();
        if self.capture.game_on_screen {
            self.capture.notice = notice(
                format!(
                    "{}'s window gave only a black picture, so ClipCraft is recording the screen instead. It tries the window again in a minute.",
                    game.game_name
                ),
                false,
                None,
            );
        }
    }

    /// While clipping, watches how much of the computer ClipCraft itself is using once a second.
    /// Using nearly all of the processor for a long time first lightens the recording, and then
    /// stops it; far too much memory stops it at once. Never while an export is running (that is
    /// heavy on purpose).
    pub(crate) fn watch_resources(&mut self) {
        let watching = self.recording.protect_computer
            && self.capture.session.is_active()
            && !clipcraft_engine::HeavyWork::running();
        if !watching {
            self.capture.watchdog.reset();
            self.capture.meter_read_at = None;
            return;
        }
        if self
            .capture
            .meter_read_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.capture.meter_read_at = Some(Instant::now());
        if self.capture.meter.is_none() {
            self.capture.meter = clipcraft_platform::SelfMeter::new();
        }
        let Some(usage) = self.capture.meter.as_mut().and_then(|meter| meter.sample()) else {
            return;
        };
        if let Some(verdict) = self
            .capture
            .watchdog
            .observe(usage.cpu_share, usage.memory_bytes)
        {
            self.apply_safety_verdict(verdict);
        }
    }

    /// Does what the watchdog asks.
    pub(crate) fn apply_safety_verdict(&mut self, verdict: clipcraft_engine::Verdict) {
        use clipcraft_engine::{StopReason, Verdict};
        match verdict {
            Verdict::Lighten => {
                self.capture.session.lighten_now();
                self.capture.notice = notice(
                    "ClipCraft is using almost all of your processor, so it is recording fewer pictures a second to take the load off. Try the graphics card encoder or a lower quality in Settings if this keeps happening.",
                    true,
                    None,
                );
            }
            Verdict::Recovered => {
                self.capture.notice = notice(
                    "ClipCraft is back to using a normal amount of your processor.",
                    false,
                    None,
                );
            }
            Verdict::Stop(reason) => {
                let why = match reason {
                    StopReason::Processor => {
                        "ClipCraft stopped clipping because it was using almost all of your processor even after lowering the quality, which can make the whole computer unresponsive. Switch to the graphics card encoder or a lower quality in Settings, then start clipping again."
                    }
                    StopReason::Memory => {
                        "ClipCraft stopped clipping because it was using far more memory than a replay needs. Lower the replay length or the quality in Settings, then start clipping again."
                    }
                };
                // Stopped for safety: automatic game capture must not start it again at once.
                if self.recording.game_only_capture {
                    self.capture.game_arming.capture_stopped_by_user();
                }
                self.stop_capture();
                self.capture.notice = notice(why, true, None);
            }
        }
    }

    /// Says so when automatic quality changes the picture rate, and shows the rate in the bar.
    pub(crate) fn report_quality_change(&mut self) {
        let Some(status) = self.capture.status.as_ref() else {
            self.capture.last_effective_fps = None;
            return;
        };
        let (now, target) = (status.effective_fps, status.target_fps);
        let squeezed = status.squeezed;
        let before = self.capture.last_effective_fps.replace(now);
        let Some(before) = before.filter(|before| *before != now) else {
            return;
        };
        if self.capture.saving.is_some()
            || self
                .capture
                .notice
                .as_ref()
                .is_some_and(|notice| notice.error)
        {
            return;
        }
        self.capture.notice = notice(
            if now < before && squeezed {
                format!(
                    "Your game started using more of your computer, so ClipCraft is recording {now} pictures a second instead of {target} to stay out of its way. Your replay is kept, and it goes back up when there is room."
                )
            } else if now < before {
                format!(
                    "Your computer is struggling to keep up, so ClipCraft is recording {now} pictures a second instead of {target}. Your replay is kept, and it goes back up when there is room."
                )
            } else if now == target {
                format!("There is room again: back to {target} pictures a second.")
            } else {
                format!("Recording {now} pictures a second now.")
            },
            false,
            None,
        );
    }

    /// How the screen is read, from the settings.
    pub(crate) fn capture_options(&self) -> CaptureOptions {
        CaptureOptions {
            show_cursor: self.recording.capture_cursor,
            hdr_to_sdr: self.recording.hdr_compatibility,
            requested_fps: self.recording.frames_per_second,
        }
    }

    /// The screen to record when no game is on: the chosen one, else the first.
    pub(crate) fn desktop_target_id(&mut self) -> Option<String> {
        self.capture.refresh_targets();
        let chosen = &self.recording.capture_target;
        self.capture
            .targets
            .iter()
            .filter(|target| automatic_desktop_target(target))
            .find(|target| target.id == *chosen)
            .or_else(|| {
                self.capture
                    .targets
                    .iter()
                    .find(|target| automatic_desktop_target(target))
            })
            .map(|target| target.id.clone())
    }

    pub(crate) fn start_desktop_fallback(&mut self) {
        // This start is automatic, so a failure is retried quietly (see `game_retry_after`)
        // instead of announcing an error the person did not cause.
        let Some(id) = self.desktop_target_id() else {
            return;
        };
        let options = self.capture_options();
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

    pub(crate) fn open_capture_source(
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
            let source = clipcraft_capture::SyntheticSource::new(160, 90, 30);
            if self.capture.test_black.iter().any(|t| t == id) {
                return Ok(Box::new(source.as_window().black_after(5)));
            }
            return Ok(Box::new(
                if self.capture.test_closing.iter().any(|t| t == id) {
                    source.failing_after(10)
                } else {
                    source
                },
            ));
        }
        clipcraft_platform::open(id, options)
    }

    pub(crate) fn game_window_for_process(
        &self,
        process_id: u32,
    ) -> Result<Option<CaptureTarget>, clipcraft_capture::CaptureError> {
        let family = self
            .capture
            .game_families
            .get(&process_id)
            .cloned()
            .unwrap_or_else(|| vec![process_id]);
        #[cfg(test)]
        if let Some(targets) = &self.capture.test_targets {
            let by_family = targets
                .iter()
                .find(|target| {
                    family
                        .iter()
                        .any(|id| target.id == format!("test-game:{id}"))
                })
                .cloned();
            return Ok(by_family.or_else(|| {
                self.game_window_by_class(process_id, &self.capture.test_owned_windows)
            }));
        }
        if let Some(window) = clipcraft_platform::window_for_processes(&family)? {
            return Ok(Some(window));
        }
        let windows = clipcraft_platform::owned_windows()?;
        Ok(self.game_window_by_class(process_id, &windows))
    }

    /// A second chance, as Vice does: a window whose own process name or window class (a Proton
    /// game's `steam_app_<id>`, say) is recognized as the same game, even though its process is
    /// not part of the recognized one's family. The largest such window wins.
    pub(crate) fn game_window_by_class(
        &self,
        process_id: u32,
        windows: &[clipcraft_platform::OwnedWindow],
    ) -> Option<CaptureTarget> {
        let game = self
            .capture
            .game_arming
            .observed()
            .filter(|game| game.process_id == process_id)
            .or(self.capture.game_session.as_ref())
            .map(|game| game.game_name.clone())?;
        let steam = &self.settings_state.steam_games.games;
        windows
            .iter()
            .filter(|window| {
                let app_id = clipcraft_engine::steam_app_id_in_window_class(&window.class);
                let executable = self.capture.process_names.get(&window.process_id);
                let by_process = executable.and_then(|name| {
                    self.recording
                        .match_game_with_steam(name, app_id.as_deref(), steam)
                });
                let by_class = window.class.split_whitespace().find_map(|part| {
                    self.recording
                        .match_game_with_steam(part, app_id.as_deref(), steam)
                });
                by_process.or(by_class) == Some(game.as_str())
            })
            .max_by_key(|window| u64::from(window.target.width) * u64::from(window.target.height))
            .map(|window| window.target.clone())
    }

    /// The screen to record while a recognized game has no window: the chosen one, else the first.
    /// The target list is read again when it is empty or has no screen, since it may have been
    /// read before a display was ready.
    pub(crate) fn fallback_screen(&mut self) -> Option<String> {
        for attempt in 0..2 {
            if attempt == 1 || !self.capture.targets_loaded() {
                self.capture.refresh_targets();
            }
            let chosen = &self.recording.capture_target;
            let screen = self
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
                })
                .map(|target| target.id.clone());
            if screen.is_some() {
                return screen;
            }
        }
        None
    }

    /// Picks what to record from the settings.
    pub(crate) fn capture_target_id(
        &mut self,
        game_process: Option<&GameProcessMatch>,
    ) -> Result<String, String> {
        let game_window = if let Some(game) = game_process {
            match self.usable_game_window(game.process_id) {
                Ok(Some(window)) => Some(window),
                Ok(None) => {
                    let fallback = self.recording.record_screen_when_game_window_missing;
                    if fallback && let Some(screen) = self.fallback_screen() {
                        self.capture.game_on_screen = true;
                        self.capture.notice = notice(
                            format!(
                                "ClipCraft found {} running but not its window, so it is recording the screen. It switches to the game's window when one appears.",
                                game.game_name
                            ),
                            false,
                            None,
                        );
                        return Ok(screen);
                    }
                    let advice = if fallback {
                        "No screen was available to record instead (on Wayland, choose the screen yourself in Settings \u{2192} Capture)."
                    } else {
                        "Turn on \u{201c}Record the screen when the game window is missing\u{201d} in Settings \u{2192} Games to record the screen until it appears."
                    };
                    return Err(format!(
                        "ClipCraft found {} running, but this desktop did not expose its game window. Capture is still idle. {advice}",
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
        self.capture.game_window_id = game_window.as_ref().map(|window| window.id.clone());
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
}
