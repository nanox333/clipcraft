//! In-app clip playback: turns `clipcraft-player` events into a texture and a full set of controls.
//!
//! Controls: play/pause, a scrub bar with a hover time, skip 10 s, frame step, volume and mute,
//! playback speed, loop, an audio-track picker, and fullscreen with controls that hide when the
//! pointer rests. Every control also has a key (see [`Playback::handle_keys`]).

use crate::kit::{self, Icon, LIME, MUTED, PANEL, TEXT};
use clipcraft_audio_out::{AudioOutput, AudioOutputError};
use clipcraft_player::{ClipInfoSummary, Player, PlayerEvent};
use clipcraft_time::Tick;
use eframe::egui;
use egui::{Align2, Color32, FontId, Pos2, RichText, Sense, Vec2};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const TRACK: Color32 = Color32::from_rgb(62, 62, 69);
/// Seconds skipped by the arrow keys and by the skip buttons.
const SMALL_SKIP_SECONDS: f64 = 5.0;
const BIG_SKIP_SECONDS: f64 = 10.0;
/// Volume change per key press.
const VOLUME_STEP: f32 = 0.05;
/// How long to wait before restarting a sound device that reported an error, and how often to try.
const AUDIO_RETRY_DELAY: Duration = Duration::from_secs(1);
const AUDIO_RETRY_LIMIT: u32 = 5;
/// Fullscreen controls hide after the pointer has rested this long while playing.
const CONTROLS_HIDE_AFTER: Duration = Duration::from_millis(2500);
/// Height of the control card (seek bar and buttons), used to size the picture above it.
const CONTROLS_HEIGHT: f32 = 100.0;
/// Extra room the trim panel takes under the controls.
const TRIM_HEIGHT: f32 = 276.0;
/// Speeds offered by the menu and the `[` / `]` keys.
pub const SPEEDS: [f64; 10] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0, 4.0];
const NORMAL_SPEED_INDEX: usize = 3;

/// Settings that carry over from one clip to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    /// Output volume, 0.0 to 1.0.
    pub volume: f32,
    /// Whether the sound is muted.
    pub muted: bool,
    /// Whether clips loop.
    pub looping: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            volume: 1.0,
            muted: false,
            looping: false,
        }
    }
}

/// A clip open in the detail view.
pub struct Playback {
    path: String,
    player: Player,
    texture: Option<egui::TextureHandle>,
    effect: clipcraft_engine::Effect,
    info: Option<ClipInfoSummary>,
    position: Tick,
    /// The last decoded position, distinct from a seek that has only been requested.
    frame_pts: Option<Tick>,
    playing: bool,
    error: Option<String>,
    /// The sound device, while one is running.
    audio: Option<AudioOutput>,
    /// Prepared editor sources never open an output device until promoted.
    audio_enabled: bool,
    /// Why there is no sound, shown under the controls. The picture keeps playing without it.
    audio_note: Option<String>,
    prefs: Prefs,
    speed_index: usize,
    audio_track: usize,
    fullscreen: bool,
    /// Whether playback was running when a scrub drag began, so it can resume after.
    resume_after_scrub: bool,
    last_activity: Instant,
    audio_retry_at: Option<Instant>,
    audio_retries: u32,
    /// Trimming the clip into a new file.
    trim: crate::trim::TrimState,
}

impl Playback {
    /// Sets the colour look for later frames; true when it changed (the picture needs a refresh).
    pub(crate) fn editor_effect(&mut self, effect: clipcraft_engine::Effect) -> bool {
        std::mem::replace(&mut self.effect, effect) != effect
    }
    pub(crate) fn editor_ready(&self) -> bool {
        self.error.is_none()
            && self.info.is_some()
            && self.frame_pts.is_some()
            && self.texture.is_some()
    }
    pub(crate) fn editor_target_ready(&self, target: Tick) -> bool {
        self.info.as_ref().is_some_and(|info| {
            let last = (info.duration - info.frame_rate().frame_duration()).max(Tick::ZERO);
            self.editor_ready() && self.frame_pts == Some(target.clamp(Tick::ZERO, last))
        })
    }
    pub(crate) fn editor_activate(&mut self) {
        if !self.audio_enabled {
            self.audio_enabled = true;
            if self.info.as_ref().is_some_and(|info| info.has_audio) {
                self.start_audio();
            }
        }
    }
    pub(crate) fn editor_retire(&mut self) {
        self.audio_enabled = false;
        self.audio_retry_at = None;
        if let Some(feed) = self.player.audio_feed() {
            feed.set_playing(false);
        }
        self.audio = None;
        self.playing = false;
        self.player.request_stop();
    }
    pub(crate) fn editor_finished(&self) -> bool {
        self.player.worker_finished()
    }
    #[cfg(test)]
    pub(crate) fn editor_texture_id(&self) -> Option<egui::TextureId> {
        self.texture.as_ref().map(egui::TextureHandle::id)
    }
    pub(crate) fn editor_error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub(crate) fn editor_audio_clock(&self) -> Option<Tick> {
        self.player
            .audio_feed()
            .filter(|feed| feed.driving())
            .map(|feed| feed.position())
    }
    pub(crate) fn editor_transport(&mut self, position: Option<Tick>, playing: bool, volume: f32) {
        self.prefs.volume = volume.clamp(0.0, 1.0);
        self.prefs.muted = volume <= 0.0;
        self.prefs.looping = false;
        self.player.set_loop(false);
        self.apply_volume();
        if let Some(position) = position {
            self.frame_pts = None;
            self.position = position;
            self.player.seek(position);
        }
        if self.playing != playing {
            if playing {
                self.player.play_from_current();
            } else {
                if let Some(feed) = self.player.audio_feed() {
                    feed.set_playing(false);
                }
                self.player.pause();
            }
            self.playing = playing;
        }
    }
    pub(crate) fn editor_picture(&mut self, ui: &mut egui::Ui, size: Vec2, canvas: Vec2) {
        self.picture(ui, size, true, false, Some(canvas));
    }
    /// Opens `path` and starts playing it.
    pub fn start(path: &str, context: &egui::Context, prefs: Prefs) -> Self {
        let mut playback = Self::open(path, context, prefs, true);
        playback.player.play();
        playback.playing = true;
        playback
    }

    /// Opens and decodes an editor source on its worker, paused and without an audio device.
    pub(crate) fn prepare_editor(
        path: &str,
        start: Tick,
        effect: clipcraft_engine::Effect,
        context: &egui::Context,
    ) -> Self {
        let mut playback = Self::open(
            path,
            context,
            Prefs {
                volume: 0.0,
                muted: true,
                looping: false,
            },
            false,
        );
        playback.effect = effect;
        playback.position = start;
        playback.player.seek(start);
        playback
    }

    fn open(path: &str, context: &egui::Context, prefs: Prefs, audio_enabled: bool) -> Self {
        let wake = context.clone();
        let player = Player::start(PathBuf::from(path), move || wake.request_repaint());
        player.set_loop(prefs.looping);
        Self {
            path: path.to_owned(),
            player,
            texture: None,
            effect: clipcraft_engine::Effect::None,
            info: None,
            position: Tick::ZERO,
            frame_pts: None,
            playing: false,
            error: None,
            audio: None,
            audio_enabled,
            audio_note: None,
            prefs,
            speed_index: NORMAL_SPEED_INDEX,
            audio_track: 0,
            fullscreen: false,
            resume_after_scrub: false,
            last_activity: Instant::now(),
            audio_retry_at: None,
            audio_retries: 0,
            trim: crate::trim::TrimState::default(),
        }
    }

    /// A trimmed clip saved since the last call, so the library can show it.
    pub fn take_trimmed(&mut self) -> Option<std::path::PathBuf> {
        self.trim.take_saved()
    }

    /// True when the clip failed to open and its file is no longer on disk.
    pub fn file_missing(&self) -> bool {
        self.error.is_some() && !std::path::Path::new(&self.path).exists()
    }

    /// The file being played.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The settings to carry to the next clip.
    pub fn prefs(&self) -> Prefs {
        self.prefs
    }

    /// Whether the clip is shown fullscreen.
    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// Applies everything the worker reported since the last frame.
    pub fn poll(&mut self, context: &egui::Context) {
        self.trim.poll();
        for event in self.player.poll() {
            self.apply(event, context);
        }
        self.watch_audio(context);
    }

    // ---- sound device ---------------------------------------------------------------------

    /// Opens the sound device for the clip's audio, if it has any.
    fn start_audio(&mut self) {
        if !self.audio_enabled || self.audio.is_some() {
            return;
        }
        let Some(feed) = self.player.audio_feed() else {
            return;
        };
        feed.set_muted(self.prefs.muted);
        feed.set_volume(self.prefs.volume);
        match AudioOutput::start(feed) {
            Ok(output) => {
                self.audio = Some(output);
                self.audio_note = None;
            }
            Err(error) => {
                self.audio_note = Some(error.to_string());
                // A missing or unusable device will not appear by itself; a failed start might.
                if error == AudioOutputError::StartFailed {
                    self.audio_retry_at = Some(Instant::now() + AUDIO_RETRY_DELAY);
                }
            }
        }
    }

    /// Restarts a sound device that stopped (unplugged, driver error) a few times, then gives up.
    fn watch_audio(&mut self, context: &egui::Context) {
        if !self.audio_enabled {
            return;
        }
        if self.audio.as_ref().is_some_and(AudioOutput::failed) {
            // Dropping the output hands the clock back to the wall clock, so the picture goes on.
            self.audio = None;
            self.audio_retry_at = Some(Instant::now() + AUDIO_RETRY_DELAY);
            self.audio_note = Some("The sound device stopped. Trying to reconnect…".to_owned());
        }
        let Some(retry_at) = self.audio_retry_at else {
            return;
        };
        if Instant::now() < retry_at {
            context.request_repaint_after(retry_at.saturating_duration_since(Instant::now()));
            return;
        }
        self.audio_retry_at = None;
        if self.audio_retries >= AUDIO_RETRY_LIMIT {
            self.audio_note =
                Some("The sound device is unavailable. The video plays without sound.".to_owned());
            return;
        }
        self.audio_retries += 1;
        self.start_audio();
    }

    // ---- events ---------------------------------------------------------------------------

    fn apply(&mut self, event: PlayerEvent, context: &egui::Context) {
        match event {
            PlayerEvent::Opened(info) => {
                let has_audio = info.has_audio;
                self.info = Some(info);
                if !self.audio_enabled {
                    self.apply_volume();
                }
                if has_audio {
                    self.start_audio();
                }
            }
            PlayerEvent::Frame(frame) => {
                let size = [frame.width as usize, frame.height as usize];
                // `from_rgba_unmultiplied` asserts on a size mismatch; refuse instead of panicking.
                if size[0]
                    .checked_mul(size[1])
                    .and_then(|pixels| pixels.checked_mul(4))
                    != Some(frame.rgba.len())
                {
                    self.error = Some("The video produced a damaged picture.".to_owned());
                    self.playing = false;
                    return;
                }
                let image = if self.effect == clipcraft_engine::Effect::None {
                    egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba)
                } else {
                    let mut recolored = frame.rgba.to_vec();
                    self.effect.apply_rgba(&mut recolored);
                    egui::ColorImage::from_rgba_unmultiplied(size, &recolored)
                };
                match self.texture.as_mut() {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        self.texture = Some(context.load_texture(
                            "clip-frame",
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
                self.position = frame.pts;
                self.frame_pts = Some(frame.pts);
            }
            PlayerEvent::Ended => self.playing = false,
            PlayerEvent::Failed(error) => {
                self.error = Some(error.to_string());
                self.playing = false;
            }
        }
    }

    // ---- actions --------------------------------------------------------------------------

    fn duration(&self) -> Tick {
        self.info.as_ref().map_or(Tick::ZERO, |info| info.duration)
    }

    fn toggle(&mut self) {
        if self.playing {
            self.player.pause();
        } else {
            self.player.play();
        }
        self.playing = !self.playing;
    }

    fn seek_to(&mut self, seconds: f64) {
        let target = Tick::from_seconds_f64(seconds.max(0.0)).clamp(Tick::ZERO, self.duration());
        self.position = target;
        self.player.seek(target);
    }

    fn skip(&mut self, seconds: f64) {
        self.seek_to(self.position.seconds() + seconds);
    }

    fn step(&mut self, frames: i64) {
        self.playing = false;
        self.player.step(frames);
    }

    fn speed(&self) -> f64 {
        SPEEDS.get(self.speed_index).copied().unwrap_or(1.0)
    }

    fn set_speed_index(&mut self, index: usize) {
        self.speed_index = index.min(SPEEDS.len() - 1);
        self.player.set_speed(self.speed());
    }

    fn set_volume(&mut self, volume: f32) {
        self.prefs.volume = volume.clamp(0.0, 1.0);
        // Moving the volume up from silence or mute means "I want to hear it".
        if self.prefs.volume > 0.0 {
            self.prefs.muted = false;
        }
        self.apply_volume();
    }

    fn toggle_mute(&mut self) {
        self.prefs.muted = !self.prefs.muted;
        if !self.prefs.muted && self.prefs.volume == 0.0 {
            self.prefs.volume = 0.5;
        }
        self.apply_volume();
    }

    fn apply_volume(&self) {
        if let Some(feed) = self.player.audio_feed() {
            feed.set_muted(self.prefs.muted);
            feed.set_volume(self.prefs.volume);
        }
    }

    fn toggle_loop(&mut self) {
        self.prefs.looping = !self.prefs.looping;
        self.player.set_loop(self.prefs.looping);
    }

    fn select_audio(&mut self, index: usize) {
        self.audio_track = index;
        self.player.select_audio(index);
    }

    fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        self.last_activity = Instant::now();
    }

    // ---- keyboard -------------------------------------------------------------------------

    /// Space/K play, ←/→ ∓5 s, J/L ∓10 s, `,`/`.` frame step, ↑/↓ volume, M mute, F fullscreen,
    /// Esc leave fullscreen, `[`/`]` speed, R loop, Home/End, 0–9 jump to 0–90 %.
    pub fn handle_keys(&mut self, context: &egui::Context) {
        if context.wants_keyboard_input()
            || egui::Popup::is_any_open(context)
            || self.error.is_some()
        {
            return;
        }
        let pressed = |key: egui::Key| context.input(|input| input.key_pressed(key));
        let has_audio = self.info.as_ref().is_some_and(|info| info.has_audio);

        if pressed(egui::Key::Space) || pressed(egui::Key::K) {
            self.toggle();
        }
        if pressed(egui::Key::ArrowLeft) {
            self.skip(-SMALL_SKIP_SECONDS);
        }
        if pressed(egui::Key::ArrowRight) {
            self.skip(SMALL_SKIP_SECONDS);
        }
        if pressed(egui::Key::J) {
            self.skip(-BIG_SKIP_SECONDS);
        }
        if pressed(egui::Key::L) {
            self.skip(BIG_SKIP_SECONDS);
        }
        if pressed(egui::Key::T) {
            self.trim.open = !self.trim.open;
        }
        if self.trim.open && !self.trim.busy() {
            let duration = self.duration().seconds();
            if pressed(egui::Key::I) {
                self.trim.set_start(self.position.seconds(), duration);
            }
            if pressed(egui::Key::O) {
                self.trim.set_end(self.position.seconds(), duration);
            }
        }
        if pressed(egui::Key::Comma) {
            self.step(-1);
        }
        if pressed(egui::Key::Period) {
            self.step(1);
        }
        if has_audio {
            if pressed(egui::Key::ArrowUp) {
                self.set_volume(self.prefs.volume + VOLUME_STEP);
            }
            if pressed(egui::Key::ArrowDown) {
                self.set_volume(self.prefs.volume - VOLUME_STEP);
            }
            if pressed(egui::Key::M) {
                self.toggle_mute();
            }
        }
        if pressed(egui::Key::F) {
            self.toggle_fullscreen();
        }
        if pressed(egui::Key::Escape) && self.fullscreen {
            self.fullscreen = false;
        }
        if pressed(egui::Key::CloseBracket) {
            self.set_speed_index(self.speed_index + 1);
        }
        if pressed(egui::Key::OpenBracket) {
            self.set_speed_index(self.speed_index.saturating_sub(1));
        }
        if pressed(egui::Key::R) {
            self.toggle_loop();
        }
        if pressed(egui::Key::Home) {
            self.seek_to(0.0);
        }
        if pressed(egui::Key::End) {
            self.seek_to(self.duration().seconds());
        }
        const DIGITS: [egui::Key; 10] = [
            egui::Key::Num0,
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        // Several key presses can arrive before one UI frame. Respect the user's final
        // request rather than applying every pressed digit in numeric order.
        let tenth = context.input(|input| {
            input.events.iter().rev().find_map(|event| {
                if let egui::Event::Key {
                    key, pressed: true, ..
                } = event
                {
                    DIGITS.iter().position(|digit| digit == key)
                } else {
                    None
                }
            })
        });
        if let Some(tenth) = tenth {
            self.seek_to(self.duration().seconds() * tenth as f64 / 10.0);
        }
    }

    // ---- drawing --------------------------------------------------------------------------

    /// Draws the picture area and fills it. Returns the response for clicks.
    fn draw_picture(&mut self, ui: &mut egui::Ui, size: Vec2, rounded: bool) -> egui::Response {
        self.picture(ui, size, rounded, true, None)
    }

    fn picture(
        &mut self,
        ui: &mut egui::Ui,
        size: Vec2,
        rounded: bool,
        interactive: bool,
        canvas: Option<Vec2>,
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let corner = if rounded { 14 } else { 0 };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(corner), Color32::BLACK);
        let message = match (&self.texture, &self.error) {
            (Some(texture), _) => {
                let canvas = canvas.map_or(rect, |size| fit(size, rect));
                let fitted = fit(texture.size_vec2(), canvas);
                // Round the picture's corners only when it reaches the corners of its area.
                let fills = (fitted.width() - rect.width()).abs() < 1.5
                    && (fitted.height() - rect.height()).abs() < 1.5;
                egui::Image::from_texture(egui::load::SizedTexture::from_handle(texture))
                    .corner_radius(egui::CornerRadius::same(if rounded && fills {
                        corner
                    } else {
                        0
                    }))
                    .paint_at(ui, fitted);
                None
            }
            (None, Some(_)) if self.file_missing() => {
                Some("This file was moved, renamed or deleted.")
            }
            (None, Some(message)) => Some(message.as_str()),
            (None, None) => Some("Loading…"),
        };
        if let Some(message) = message {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                message,
                FontId::proportional(16.0),
                MUTED,
            );
        }
        if interactive && response.clicked() && self.error.is_none() {
            self.toggle();
        }
        if interactive && response.double_clicked() {
            self.toggle_fullscreen();
        }
        response
    }

    /// The picture with its controls underneath, for the detail view. The picture takes the
    /// largest 16:9 area that leaves room for the controls.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.handle_keys(ui.ctx());
        let width = ui.available_width();
        let trim_room = if self.trim.open { TRIM_HEIGHT } else { 0.0 };
        let room = (ui.available_height() - CONTROLS_HEIGHT - trim_room - 12.0).max(180.0);
        let height = (width * 9.0 / 16.0).min(room);
        self.draw_picture(ui, Vec2::new(width, height), true);
        ui.add_space(12.0);
        egui::Frame::new()
            .fill(PANEL)
            .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
            .corner_radius(egui::CornerRadius::same(14))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width((width - 34.0).max(0.0));
                self.controls(ui);
            });
    }

    /// The picture filling the whole window, with controls that fade away when the pointer rests.
    pub fn show_fullscreen(&mut self, ui: &mut egui::Ui) {
        self.handle_keys(ui.ctx());
        let moved = ui.input(|input| {
            input.pointer.delta() != Vec2::ZERO
                || input.pointer.any_pressed()
                || !input.events.is_empty()
        });
        if moved {
            self.last_activity = Instant::now();
        }
        let size = ui.available_size();
        let picture = self.draw_picture(ui, size, false);
        let rect = picture.rect;
        let bar_height = CONTROLS_HEIGHT + 24.0;

        let near_bar = ui
            .input(|input| input.pointer.hover_pos())
            .is_some_and(|pointer| pointer.y > rect.bottom() - bar_height - 40.0);
        let idle = self.last_activity.elapsed();
        let visible = !self.playing || near_bar || idle < CONTROLS_HIDE_AFTER;
        if visible {
            // A soft gradient, not a hard bar, so the picture stays visible behind the controls.
            let fade = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - bar_height - 70.0),
                rect.max,
            );
            kit::bottom_fade(ui.painter(), fade, 215);
            let bar = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - bar_height),
                rect.max,
            );
            let mut inner =
                ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(Vec2::new(28.0, 10.0))));
            self.controls(&mut inner);
            if self.playing && !near_bar {
                ui.ctx()
                    .request_repaint_after(CONTROLS_HIDE_AFTER.saturating_sub(idle));
            }
        } else {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        }
    }

    /// The seek bar and the button row.
    fn controls(&mut self, ui: &mut egui::Ui) {
        let usable = self.info.is_some() && self.error.is_none();
        let duration = self.duration().seconds();

        // Seek bar.
        let width = ui.available_width();
        let marked = self.trim.marked(duration);
        let scrub = seek_bar(ui, width, self.position.seconds(), duration, usable, marked);
        if scrub.started && self.playing {
            self.resume_after_scrub = true;
            self.player.pause();
            self.playing = false;
        }
        if let Some(seconds) = scrub.target {
            self.seek_to(seconds);
        }
        if scrub.stopped && self.resume_after_scrub {
            self.resume_after_scrub = false;
            self.player.play();
            self.playing = true;
        }

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let enabled = usable;
            if kit::icon_button(ui, Icon::SkipBack, 36.0, false, enabled, "Back 10 s (J)").clicked()
            {
                self.skip(-BIG_SKIP_SECONDS);
            }
            if kit::icon_button(
                ui,
                Icon::StepBack,
                36.0,
                false,
                enabled,
                "Previous frame (,)",
            )
            .clicked()
            {
                self.step(-1);
            }
            let play = if self.playing {
                Icon::Pause
            } else {
                Icon::Play
            };
            let tooltip = if self.playing {
                "Pause (Space)"
            } else {
                "Play (Space)"
            };
            if play_button(ui, play, enabled, tooltip).clicked() {
                self.toggle();
            }
            if kit::icon_button(
                ui,
                Icon::StepForward,
                36.0,
                false,
                enabled,
                "Next frame (.)",
            )
            .clicked()
            {
                self.step(1);
            }
            if kit::icon_button(
                ui,
                Icon::SkipForward,
                36.0,
                false,
                enabled,
                "Forward 10 s (L)",
            )
            .clicked()
            {
                self.skip(BIG_SKIP_SECONDS);
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new(format_clock(self.position))
                    .color(TEXT)
                    .monospace()
                    .size(14.0),
            );
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!("/ {}", format_clock(self.duration())))
                    .color(MUTED)
                    .monospace()
                    .size(14.0),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let fullscreen = if self.fullscreen {
                    Icon::ExitFullscreen
                } else {
                    Icon::Fullscreen
                };
                let tooltip = if self.fullscreen {
                    "Exit fullscreen (F, Esc)"
                } else {
                    "Fullscreen (F)"
                };
                if kit::icon_button(ui, fullscreen, 36.0, self.fullscreen, true, tooltip).clicked()
                {
                    self.toggle_fullscreen();
                }
                if kit::icon_button(ui, Icon::Loop, 36.0, self.prefs.looping, true, "Loop (R)")
                    .clicked()
                {
                    self.toggle_loop();
                }
                if kit::icon_button(
                    ui,
                    Icon::Scissors,
                    36.0,
                    self.trim.open,
                    usable,
                    "Trim this clip into a new file (T)",
                )
                .clicked()
                {
                    self.trim.open = !self.trim.open;
                }
                self.speed_menu(ui);
                self.track_menu(ui);
                if self.info.as_ref().is_some_and(|info| info.has_audio) {
                    self.volume_controls(ui);
                }
            });
        });
        if !self.fullscreen {
            let (position, input) = (self.position.seconds(), self.path.clone());
            if let Some(seconds) = self.trim.show(ui, position, duration, &input) {
                self.seek_to(seconds);
            }
        }
        // The note would run off the bottom edge over a fullscreen picture; it shows windowed.
        if let Some(note) = self.audio_note.as_ref().filter(|_| !self.fullscreen) {
            ui.label(RichText::new(note).color(MUTED).size(12.0));
        }
    }

    fn speed_menu(&mut self, ui: &mut egui::Ui) {
        let label = format!("{}×", speed_label(self.speed()));
        let mut choice = None;
        kit::pill_menu(ui, &label, "Speed ([ and ])", |ui| {
            for (index, speed) in SPEEDS.iter().enumerate() {
                let text = format!("{}×", speed_label(*speed));
                if ui
                    .selectable_label(index == self.speed_index, text)
                    .clicked()
                {
                    choice = Some(index);
                    ui.close();
                }
            }
        });
        if let Some(index) = choice {
            self.set_speed_index(index);
        }
    }

    fn track_menu(&mut self, ui: &mut egui::Ui) {
        let Some(tracks) = self
            .info
            .as_ref()
            .map(|info| info.audio_tracks.clone())
            .filter(|tracks| tracks.len() > 1)
        else {
            return;
        };
        let mut choice = None;
        let label = format!("Audio {}", self.audio_track + 1);
        kit::pill_menu(ui, &label, "Audio track", |ui| {
            for (index, track) in tracks.iter().enumerate() {
                if ui
                    .selectable_label(index == self.audio_track, track)
                    .clicked()
                {
                    choice = Some(index);
                    ui.close();
                }
            }
        });
        if let Some(index) = choice {
            self.select_audio(index);
        }
    }

    fn volume_controls(&mut self, ui: &mut egui::Ui) {
        // Right-to-left layout: the slider is added first so the speaker ends up on its left.
        let volume = if self.prefs.muted {
            0.0
        } else {
            self.prefs.volume
        };
        let tooltip = format!("Volume {}% (↑ ↓)", (volume * 100.0).round() as u32);
        if let Some(new) = kit::slim_slider(ui, 96.0, volume, true, &tooltip) {
            self.set_volume(new);
        }
        let off = self.prefs.muted || self.prefs.volume == 0.0 || self.audio.is_none();
        let icon = if off { Icon::SpeakerOff } else { Icon::Speaker };
        let tip = if off { "Unmute (M)" } else { "Mute (M)" };
        if kit::icon_button(ui, icon, 36.0, false, true, tip).clicked() {
            self.toggle_mute();
        }
    }
}

/// What the user did to the seek bar this frame.
struct Scrub {
    /// Seconds to jump to, while dragging or clicking.
    target: Option<f64>,
    started: bool,
    stopped: bool,
}

/// A scrub bar: filled to the position, draggable, with the time under the pointer on hover.
fn seek_bar(
    ui: &mut egui::Ui,
    width: f32,
    position: f64,
    duration: f64,
    enabled: bool,
    marked: Option<(f64, f64)>,
) -> Scrub {
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width.max(0.0), 20.0), sense);
    response.widget_info(|| {
        egui::WidgetInfo::slider(
            enabled,
            position,
            format!(
                "Seek, {} of {}",
                format_clock(Tick::from_seconds_f64(position)),
                format_clock(Tick::from_seconds_f64(duration))
            ),
        )
    });
    let track = egui::Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 5.0));
    let fraction = if duration > 0.0 {
        (position / duration).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let active = response.hovered() || response.dragged();
    let painter = ui.painter();
    painter.rect_filled(track, 2.5, TRACK);
    if let Some((from, to)) = marked.filter(|_| duration > 0.0) {
        let at = |seconds: f64| {
            track.left() + track.width() * (seconds / duration).clamp(0.0, 1.0) as f32
        };
        let band = egui::Rect::from_min_max(
            egui::pos2(at(from), rect.top() + 3.0),
            egui::pos2(at(to).max(at(from) + 2.0), rect.bottom() - 3.0),
        );
        painter.rect_filled(band, 3.0, LIME.gamma_multiply(0.28));
    }
    let filled = egui::Rect::from_min_max(
        track.min,
        egui::pos2(track.left() + track.width() * fraction, track.bottom()),
    );
    painter.rect_filled(filled, 2.5, LIME);
    let handle = Pos2::new(track.left() + track.width() * fraction, rect.center().y);
    painter.circle_filled(handle, if active { 7.0 } else { 5.0 }, LIME);

    let fraction_at = |pointer: Pos2| ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
    if let Some(pointer) = response.hover_pos().filter(|_| enabled && duration > 0.0) {
        let seconds = f64::from(fraction_at(pointer)) * duration;
        response
            .clone()
            .on_hover_text_at_pointer(format_clock(Tick::from_seconds_f64(seconds)));
    }
    let target = if enabled && duration > 0.0 && (response.dragged() || response.clicked()) {
        response
            .interact_pointer_pos()
            .map(|pointer| f64::from(fraction_at(pointer)) * duration)
    } else {
        None
    };
    Scrub {
        target,
        started: response.drag_started(),
        stopped: response.drag_stopped(),
    }
}

/// Largest rectangle with `size`'s aspect ratio that fits in `bounds`, centered.
fn fit(size: Vec2, bounds: egui::Rect) -> egui::Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return bounds;
    }
    let scale = (bounds.width() / size.x).min(bounds.height() / size.y);
    egui::Rect::from_center_size(bounds.center(), size * scale)
}

/// The round primary play/pause button.
fn play_button(ui: &mut egui::Ui, icon: Icon, enabled: bool, tooltip: &str) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(42.0), sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tooltip));
    let hovered = response.hovered() && enabled;
    let fill = if !enabled {
        Color32::from_rgb(44, 44, 50)
    } else if hovered {
        Color32::from_rgb(210, 255, 90)
    } else {
        LIME
    };
    ui.painter().circle_filled(rect.center(), 21.0, fill);
    // The triangle sits a little right of center so it looks centered.
    let offset = if icon == Icon::Play {
        Vec2::new(1.0, 0.0)
    } else {
        Vec2::ZERO
    };
    kit::draw_icon(
        ui.painter(),
        rect.center() + offset,
        icon,
        Color32::BLACK,
        1.1,
    );
    response
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// `1`, `0.5`, `1.25`: a speed without trailing zeros.
pub fn speed_label(speed: f64) -> String {
    let text = format!("{speed:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// `m:ss`, or `h:mm:ss` from one hour up. Display only.
pub fn format_clock(time: Tick) -> String {
    let total = time.seconds().max(0.0).floor() as u64;
    let (hours, minutes, seconds) = (total / 3600, total % 3600 / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
pub(crate) mod editor_fixtures {
    use filmcraft_h264enc::{Encoder, EncoderConfig, PacketFormat, YuvFrame};
    use filmcraft_isobmff::{
        AvcConfig, Brand, Mp4Writer, PcmConfig, SampleEntry, TrackConfig, WriteSample,
        WriterOptions,
    };
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    pub(crate) struct Fixture {
        folder: PathBuf,
        bytes: Vec<u8>,
    }
    impl Fixture {
        pub(crate) fn new(sound: bool) -> Self {
            let mut config = EncoderConfig::new(64, 48, 30, 1);
            config.bframes = 0;
            config.keyint = 10;
            config.format = PacketFormat::LengthPrefixed;
            let mut encoder = Encoder::new(config).unwrap();
            let (sps, pps) = encoder.sps_pps();
            let strip = |nal: &[u8]| {
                nal.strip_prefix(&[0, 0, 0, 1])
                    .or_else(|| nal.strip_prefix(&[0, 0, 1]))
                    .unwrap_or(nal)
                    .to_vec()
            };
            let mut writer =
                Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
            let track = writer
                .add_track(TrackConfig::new(
                    SampleEntry::avc(
                        AvcConfig::new(vec![strip(&sps)], vec![strip(&pps)], 4),
                        64,
                        48,
                    ),
                    30,
                ))
                .unwrap();
            let audio = sound.then(|| {
                writer
                    .add_track(TrackConfig::new(
                        SampleEntry::pcm(PcmConfig {
                            bits: 16,
                            float: false,
                            big_endian: false,
                            signed: true,
                            channels: 1,
                            sample_rate: 48_000.0,
                        }),
                        48_000,
                    ))
                    .unwrap()
            });
            for index in 0..30_u8 {
                let y = vec![24 + index * 7; 64 * 48];
                let uv = vec![128; 64 * 48 / 4];
                let frame = YuvFrame {
                    y: &y,
                    u: &uv,
                    v: &uv,
                    y_stride: 64,
                    uv_stride: 32,
                };
                for packet in encoder.encode(&frame, i64::from(index)).unwrap() {
                    writer
                        .write_sample(
                            track,
                            WriteSample {
                                data: &packet.data,
                                duration: 1,
                                composition_offset: 0,
                                is_sync: packet.keyframe,
                            },
                        )
                        .unwrap();
                }
            }
            for packet in encoder.flush() {
                writer
                    .write_sample(
                        track,
                        WriteSample {
                            data: &packet.data,
                            duration: 1,
                            composition_offset: 0,
                            is_sync: packet.keyframe,
                        },
                    )
                    .unwrap();
            }
            if let Some(audio) = audio {
                let bytes = vec![0_u8; 48_000 * 2];
                writer
                    .write_sample(
                        audio,
                        WriteSample {
                            data: &bytes,
                            duration: 0,
                            composition_offset: 0,
                            is_sync: true,
                        },
                    )
                    .unwrap();
            }
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let folder = std::env::temp_dir().join(format!(
                "clipcraft-editor-preload-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&folder).unwrap();
            Self {
                folder,
                bytes: writer.finish().unwrap().into_inner(),
            }
        }
        pub(crate) fn file(&self, name: &str) -> PathBuf {
            let path = self.folder.join(name);
            std::fs::write(&path, &self.bytes).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipcraft_player::{DecodedFrame, PlayerError};

    #[test]
    fn editor_preparation_decodes_target_without_starting_sound_and_cancels() {
        let fixture = editor_fixtures::Fixture::new(true);
        let path = fixture.file("prepared.mp4");
        let context = egui::Context::default();
        let target = Tick::from_units(1, 3);
        let mut preview = Playback::prepare_editor(
            &path.to_string_lossy(),
            target,
            clipcraft_engine::Effect::None,
            &context,
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !preview.editor_target_ready(target) && Instant::now() < deadline {
            preview.poll(&context);
            assert!(
                preview.editor_error().is_none(),
                "{:?}",
                preview.editor_error()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(preview.editor_target_ready(target));
        assert!(!preview.playing);
        assert!(!preview.audio_enabled);
        assert!(preview.audio.is_none());
        assert!(preview.audio_retry_at.is_none());
        let feed = preview.player.audio_feed().unwrap();
        assert!(
            !feed.driving(),
            "preparation must not create an output device"
        );
        assert!(feed.muted());

        let next = Tick::from_units(2, 3);
        preview.editor_transport(Some(next), false, 0.25);
        assert!(
            !preview.editor_target_ready(next),
            "a queued seek is not a decoded frame"
        );
        while !preview.editor_target_ready(next) && Instant::now() < deadline {
            preview.poll(&context);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(preview.editor_target_ready(next));
        preview.editor_retire();
        while !preview.editor_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(preview.editor_finished());
        assert!(!feed.driving());
        assert!(preview.audio.is_none());
    }

    fn playback() -> Playback {
        Playback::start(
            "/definitely/not/here.mp4",
            &egui::Context::default(),
            Prefs::default(),
        )
    }

    /// A playback that has "opened" a clip of `seconds` with audio, without a real file.
    fn opened(seconds: f64, tracks: usize) -> (Playback, egui::Context) {
        let context = egui::Context::default();
        let mut playback = Playback::start("/definitely/not/here.mp4", &context, Prefs::default());
        // The missing file would fail; ignore that and describe the clip by hand.
        playback.error = None;
        playback.info = Some(ClipInfoSummary {
            duration: Tick::from_seconds_f64(seconds),
            width: 640,
            height: 360,
            rate_numerator: 30,
            rate_denominator: 1,
            has_audio: true,
            audio_tracks: (1..=tracks).map(|n| format!("Track {n}")).collect(),
        });
        (playback, context)
    }

    /// Runs one egui frame in which `keys` are pressed, calling `handle_keys`.
    fn press(playback: &mut Playback, context: &egui::Context, keys: &[egui::Key]) {
        let events = keys
            .iter()
            .map(|key| egui::Event::Key {
                key: *key,
                physical_key: Some(*key),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .collect();
        let _ = context.run(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |context| playback.handle_keys(context),
        );
    }

    /// Presses `key` `times` times, one frame each, like a person tapping it.
    fn tap(playback: &mut Playback, context: &egui::Context, key: egui::Key, times: usize) {
        for _ in 0..times {
            press(playback, context, &[key]);
        }
    }

    fn frame(width: u32, height: u32, bytes: usize, pts: Tick) -> PlayerEvent {
        PlayerEvent::Frame(DecodedFrame {
            pts,
            width,
            height,
            rgba: vec![200; bytes],
        })
    }

    #[test]
    fn rapid_percentage_seeks_use_the_last_event_instead_of_the_highest_digit() {
        use egui::Key::*;
        for (keys, seconds) in [
            (vec![Num1, Num8, Num2, Num9, Num3], 3.0),
            (vec![Num9, Num0], 0.0),
            (vec![Num9, Num3, Num9, Num3], 3.0),
        ] {
            let (mut playback, context) = opened(10.0, 0);
            press(&mut playback, &context, &keys);
            assert_eq!(playback.position, Tick::from_seconds_f64(seconds));
            assert!(playback.playing);
        }
    }

    #[test]
    fn player_shortcuts_do_not_fire_while_an_export_menu_is_open() {
        let context = egui::Context::default();
        let mut player = playback();
        let volume = player.prefs.volume;
        let playing = player.playing;
        egui::Popup::open_id(&context, egui::Id::new("export-menu"));
        press(
            &mut player,
            &context,
            &[egui::Key::ArrowDown, egui::Key::Space, egui::Key::T],
        );
        assert_eq!(player.prefs.volume, volume);
        assert_eq!(player.playing, playing);
        assert!(!player.trim.open);
    }

    #[test]
    fn clock_formats_floor_to_whole_seconds() {
        assert_eq!(format_clock(Tick::ZERO), "0:00");
        assert_eq!(format_clock(Tick::from_seconds_f64(59.9)), "0:59");
        assert_eq!(format_clock(Tick::from_seconds_f64(125.0)), "2:05");
        assert_eq!(format_clock(Tick::from_seconds_f64(3725.0)), "1:02:05");
        assert_eq!(format_clock(Tick(-5)), "0:00");
    }

    #[test]
    fn speeds_are_labelled_without_trailing_zeros() {
        assert_eq!(speed_label(1.0), "1");
        assert_eq!(speed_label(0.5), "0.5");
        assert_eq!(speed_label(1.25), "1.25");
        assert_eq!(speed_label(0.25), "0.25");
        assert_eq!(speed_label(4.0), "4");
        assert_eq!(SPEEDS[NORMAL_SPEED_INDEX], 1.0);
        assert!(
            SPEEDS.windows(2).all(|pair| pair[0] < pair[1]),
            "speeds ascend"
        );
    }

    #[test]
    fn frames_become_a_texture_and_move_the_position() {
        let context = egui::Context::default();
        let mut playback = playback();
        playback.apply(
            frame(4, 2, 4 * 2 * 4, Tick::from_seconds_f64(1.5)),
            &context,
        );
        assert!(playback.texture.is_some());
        assert_eq!(playback.position, Tick::from_seconds_f64(1.5));
        assert!(playback.error.is_none());
    }

    #[test]
    fn a_frame_with_the_wrong_byte_count_is_refused_instead_of_panicking() {
        let context = egui::Context::default();
        let mut playback = playback();
        playback.apply(frame(4, 2, 5, Tick::ZERO), &context);
        assert!(playback.texture.is_none());
        assert!(playback.error.is_some());
        assert!(!playback.playing);
        // A size whose pixel count overflows is refused too.
        playback.apply(frame(u32::MAX, u32::MAX, 16, Tick::ZERO), &context);
        assert!(playback.texture.is_none());
    }

    #[test]
    fn ending_and_failing_stop_playback() {
        let context = egui::Context::default();
        let mut playback = playback();
        assert!(playback.playing);
        playback.apply(PlayerEvent::Ended, &context);
        assert!(!playback.playing && playback.error.is_none());
        playback.apply(PlayerEvent::Failed(PlayerError::Unsupported), &context);
        assert_eq!(
            playback.error.as_deref(),
            Some("ClipCraft cannot play this video format yet.")
        );
    }

    #[test]
    fn the_picture_is_fitted_inside_its_area_without_stretching() {
        let bounds = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(800.0, 400.0));
        let wide = fit(Vec2::new(1920.0, 1080.0), bounds);
        assert!((wide.height() - 400.0).abs() < 0.01);
        assert!((wide.width() / wide.height() - 1920.0 / 1080.0).abs() < 0.01);
        let tall = fit(Vec2::new(100.0, 400.0), bounds);
        assert!(tall.width() < bounds.width());
        assert_eq!(fit(Vec2::ZERO, bounds), bounds);
    }

    #[test]
    fn a_clip_whose_file_is_gone_is_reported_as_missing_only_after_it_failed() {
        let (mut playback, _context) = opened(10.0, 1);
        assert!(
            !playback.file_missing(),
            "a clip that is playing is not missing"
        );
        playback.error = Some("The video could not be read.".to_owned());
        assert!(playback.file_missing(), "the path does not exist");
        playback.path = std::env::temp_dir().to_string_lossy().into_owned();
        assert!(
            !playback.file_missing(),
            "an existing file that fails is damaged, not missing"
        );
    }

    #[test]
    fn t_opens_the_trim_row_and_i_and_o_mark_the_range() {
        let (mut playback, context) = opened(100.0, 1);
        use egui::Key::*;
        // Marks are ignored until the trim row is open.
        press(&mut playback, &context, &[I]);
        assert_eq!(playback.trim.range(100.0), (0.0, 100.0));
        press(&mut playback, &context, &[T]);
        assert!(playback.trim.open);
        press(&mut playback, &context, &[ArrowRight]);
        press(&mut playback, &context, &[I]);
        press(&mut playback, &context, &[L]);
        press(&mut playback, &context, &[O]);
        assert_eq!(playback.trim.range(100.0), (5.0, 15.0));
        press(&mut playback, &context, &[T]);
        assert!(!playback.trim.open);
    }

    #[test]
    fn keys_control_playback_volume_speed_loop_and_fullscreen() {
        let (mut playback, context) = opened(100.0, 1);
        use egui::Key::*;

        assert!(playback.playing);
        press(&mut playback, &context, &[Space]);
        assert!(!playback.playing);
        press(&mut playback, &context, &[K]);
        assert!(playback.playing);

        // Skips move the position and clamp at both ends.
        press(&mut playback, &context, &[ArrowRight]);
        assert_eq!(playback.position, Tick::from_seconds_f64(5.0));
        press(&mut playback, &context, &[L]);
        assert_eq!(playback.position, Tick::from_seconds_f64(15.0));
        tap(&mut playback, &context, J, 2);
        assert_eq!(
            playback.position,
            Tick::ZERO,
            "skipping back stops at the start"
        );
        press(&mut playback, &context, &[End]);
        assert_eq!(playback.position, Tick::from_seconds_f64(100.0));
        press(&mut playback, &context, &[ArrowRight]);
        assert_eq!(
            playback.position,
            Tick::from_seconds_f64(100.0),
            "skipping forward stops at the end"
        );
        press(&mut playback, &context, &[Num5]);
        assert_eq!(playback.position, Tick::from_seconds_f64(50.0));
        press(&mut playback, &context, &[Home]);
        assert_eq!(playback.position, Tick::ZERO);

        // Frame stepping pauses.
        press(&mut playback, &context, &[Period]);
        assert!(!playback.playing);
        press(&mut playback, &context, &[Comma]);
        assert!(!playback.playing);

        // Volume steps, clamps, and mutes.
        tap(&mut playback, &context, ArrowDown, 2);
        assert!((playback.prefs.volume - 0.9).abs() < 1e-6);
        tap(&mut playback, &context, ArrowUp, 3);
        assert_eq!(playback.prefs.volume, 1.0);
        press(&mut playback, &context, &[M]);
        assert!(playback.prefs.muted);
        press(&mut playback, &context, &[ArrowDown]);
        assert!(!playback.prefs.muted, "changing the volume unmutes");

        // Speed walks the menu list and stops at its ends.
        press(&mut playback, &context, &[CloseBracket]);
        assert_eq!(playback.speed(), 1.25);
        for _ in 0..20 {
            press(&mut playback, &context, &[CloseBracket]);
        }
        assert_eq!(playback.speed(), 4.0);
        for _ in 0..20 {
            press(&mut playback, &context, &[OpenBracket]);
        }
        assert_eq!(playback.speed(), 0.25);

        press(&mut playback, &context, &[R]);
        assert!(playback.prefs.looping);
        press(&mut playback, &context, &[F]);
        assert!(playback.fullscreen());
        press(&mut playback, &context, &[Escape]);
        assert!(!playback.fullscreen());
    }

    #[test]
    fn volume_keys_and_mute_are_ignored_for_a_clip_without_sound() {
        let (mut playback, context) = opened(10.0, 0);
        playback.info.as_mut().expect("info").has_audio = false;
        press(
            &mut playback,
            &context,
            &[egui::Key::ArrowDown, egui::Key::M],
        );
        assert_eq!(playback.prefs, Prefs::default());
    }

    #[test]
    fn muting_from_zero_volume_restores_an_audible_level() {
        let (mut playback, _) = opened(10.0, 1);
        playback.set_volume(0.0);
        playback.toggle_mute();
        assert!(playback.prefs.muted);
        playback.toggle_mute();
        assert!(!playback.prefs.muted);
        assert!(
            playback.prefs.volume > 0.0,
            "unmuting must not leave the volume at zero"
        );
        // Hostile values are clamped.
        playback.set_volume(f32::INFINITY);
        assert_eq!(playback.prefs.volume, 1.0);
        playback.set_volume(-4.0);
        assert_eq!(playback.prefs.volume, 0.0);
    }

    #[test]
    fn preferences_carry_to_the_next_clip_but_speed_starts_normal() {
        let (mut playback, _) = opened(10.0, 1);
        playback.set_volume(0.3);
        playback.toggle_loop();
        playback.set_speed_index(7);
        let next = Playback::start(
            "/definitely/not/here.mp4",
            &egui::Context::default(),
            playback.prefs(),
        );
        assert_eq!(next.prefs.volume, 0.3);
        assert!(next.prefs.looping);
        assert_eq!(next.speed(), 1.0);
    }

    #[test]
    fn keys_are_ignored_while_a_text_field_has_focus_or_after_a_failure() {
        let (mut playback, context) = opened(100.0, 1);
        playback.error = Some("broken".to_owned());
        press(
            &mut playback,
            &context,
            &[egui::Key::Space, egui::Key::ArrowRight],
        );
        assert!(
            playback.playing,
            "no key works on a clip that failed to open"
        );
        assert_eq!(playback.position, Tick::ZERO);
    }

    #[test]
    fn the_audio_track_choice_is_remembered_and_bounded() {
        let (mut playback, _) = opened(10.0, 3);
        playback.select_audio(2);
        assert_eq!(playback.audio_track, 2);
    }

    #[test]
    fn the_player_draws_windowed_and_fullscreen_at_any_size_without_panicking() {
        let (mut playback, _) = opened(125.0, 3);
        let context = crate::kit::test_context();
        let context_ref = &context;
        let draw = |playback: &mut Playback, size: (f32, f32), fullscreen: bool| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(size.0, size.1),
                )),
                ..Default::default()
            };
            let _ = context_ref.run(input, |context| {
                crate::kit::apply_theme(context);
                egui::CentralPanel::default().show(context, |ui| {
                    if fullscreen {
                        playback.show_fullscreen(ui);
                    } else {
                        playback.show(ui);
                    }
                });
            });
        };
        let sizes = [
            (1.0, 1.0),
            (100.0, 60.0),
            (400.0, 300.0),
            (1280.0, 720.0),
            (6000.0, 4000.0),
        ];
        for size in sizes {
            // No picture yet, then with one, then paused, with an error, and at the end.
            draw(&mut playback, size, false);
            draw(&mut playback, size, true);
            playback.apply(
                frame(64, 36, 64 * 36 * 4, Tick::from_seconds_f64(3.0)),
                &context,
            );
            draw(&mut playback, size, false);
            draw(&mut playback, size, true);
            playback.playing = false;
            draw(&mut playback, size, true);
            playback.position = Tick::from_seconds_f64(1e9);
            draw(&mut playback, size, false);
            playback.error = Some("A problem".to_owned());
            draw(&mut playback, size, false);
            draw(&mut playback, size, true);
            playback.error = None;
            playback.playing = true;
        }
        // A clip with a zero or tiny duration must not divide by zero in the seek bar.
        playback.info.as_mut().expect("info").duration = Tick::ZERO;
        draw(&mut playback, (800.0, 600.0), false);
    }
}
