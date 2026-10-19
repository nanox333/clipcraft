//! In-app clip playback: turns `clipcraft-player` events into a texture and a full set of controls.
//!
//! Controls: play/pause, a scrub bar with a hover time, skip 10 s, frame step, volume and mute,
//! playback speed, loop, an audio-track picker, and fullscreen with controls that hide when the
//! pointer rests. Every control also has a key (see [`Playback::handle_keys`]).

use clipcraft_audio_out::{AudioOutput, AudioOutputError};
use clipcraft_player::{ClipInfoSummary, Player, PlayerEvent};
use clipcraft_time::Tick;
use eframe::egui;
use egui::{Align2, Color32, FontId, Painter, Pos2, RichText, Sense, Stroke, Vec2};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MUTED: Color32 = Color32::from_rgb(157, 157, 164);
const TEXT: Color32 = Color32::from_rgb(242, 242, 244);
const LIME: Color32 = Color32::from_rgb(190, 255, 43);
const TRACK: Color32 = Color32::from_rgb(58, 58, 64);
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
    info: Option<ClipInfoSummary>,
    position: Tick,
    playing: bool,
    error: Option<String>,
    /// The sound device, while one is running.
    audio: Option<AudioOutput>,
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
}

impl Playback {
    /// Opens `path` and starts playing it.
    pub fn start(path: &str, context: &egui::Context, prefs: Prefs) -> Self {
        let wake = context.clone();
        let player = Player::start(PathBuf::from(path), move || wake.request_repaint());
        player.set_loop(prefs.looping);
        player.play();
        Self {
            path: path.to_owned(),
            player,
            texture: None,
            info: None,
            position: Tick::ZERO,
            playing: true,
            error: None,
            audio: None,
            audio_note: None,
            prefs,
            speed_index: NORMAL_SPEED_INDEX,
            audio_track: 0,
            fullscreen: false,
            resume_after_scrub: false,
            last_activity: Instant::now(),
            audio_retry_at: None,
            audio_retries: 0,
        }
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
        for event in self.player.poll() {
            self.apply(event, context);
        }
        self.watch_audio(context);
    }

    // ---- sound device ---------------------------------------------------------------------

    /// Opens the sound device for the clip's audio, if it has any.
    fn start_audio(&mut self) {
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
                let image = egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba);
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
        if context.wants_keyboard_input() || self.error.is_some() {
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
        for (tenth, key) in DIGITS.iter().enumerate() {
            if pressed(*key) {
                self.seek_to(self.duration().seconds() * tenth as f64 / 10.0);
            }
        }
    }

    // ---- drawing --------------------------------------------------------------------------

    /// Draws the picture area and fills it. Returns the response for clicks.
    fn draw_picture(&mut self, ui: &mut egui::Ui, size: Vec2) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
        let message = match (&self.texture, &self.error) {
            (Some(texture), _) => {
                let fitted = fit(texture.size_vec2(), rect);
                ui.painter().image(
                    texture.id(),
                    fitted,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                None
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
        if response.clicked() && self.error.is_none() {
            self.toggle();
        }
        if response.double_clicked() {
            self.toggle_fullscreen();
        }
        response
    }

    /// The picture with its controls underneath, for the detail view.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.handle_keys(ui.ctx());
        self.draw_picture(ui, Vec2::new(ui.available_width(), 420.0));
        ui.add_space(8.0);
        self.controls(ui);
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
        let bar_height = 92.0;
        let picture = self.draw_picture(ui, size);
        let rect = picture.rect;

        let near_bar = ui
            .input(|input| input.pointer.hover_pos())
            .is_some_and(|pointer| pointer.y > rect.bottom() - bar_height);
        let idle = self.last_activity.elapsed();
        let visible = !self.playing || near_bar || idle < CONTROLS_HIDE_AFTER;
        if visible {
            let bar = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - bar_height),
                rect.max,
            );
            ui.painter()
                .rect_filled(bar, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 170));
            let mut inner =
                ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(Vec2::new(18.0, 8.0))));
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
        let scrub = seek_bar(ui, width, self.position.seconds(), duration, usable);
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

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let enabled = usable;
            if icon_button(ui, Icon::SkipBack, false, enabled, "Back 10 s (J)").clicked() {
                self.skip(-BIG_SKIP_SECONDS);
            }
            if icon_button(ui, Icon::StepBack, false, enabled, "Previous frame (,)").clicked() {
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
            if icon_button(ui, Icon::StepForward, false, enabled, "Next frame (.)").clicked() {
                self.step(1);
            }
            if icon_button(ui, Icon::SkipForward, false, enabled, "Forward 10 s (L)").clicked() {
                self.skip(BIG_SKIP_SECONDS);
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!(
                    "{} / {}",
                    format_clock(self.position),
                    format_clock(self.duration())
                ))
                .color(TEXT)
                .monospace(),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
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
                if icon_button(ui, fullscreen, self.fullscreen, true, tooltip).clicked() {
                    self.toggle_fullscreen();
                }
                if icon_button(ui, Icon::Loop, self.prefs.looping, true, "Loop (R)").clicked() {
                    self.toggle_loop();
                }
                self.speed_menu(ui);
                self.track_menu(ui);
                if self.info.as_ref().is_some_and(|info| info.has_audio) {
                    self.volume_controls(ui);
                }
            });
        });
        if let Some(note) = &self.audio_note {
            ui.add_space(2.0);
            ui.label(RichText::new(note).color(MUTED).small());
        }
    }

    fn speed_menu(&mut self, ui: &mut egui::Ui) {
        let label = format!("{}×", speed_label(self.speed()));
        let mut choice = None;
        ui.menu_button(RichText::new(label).color(TEXT).monospace(), |ui| {
            for (index, speed) in SPEEDS.iter().enumerate() {
                let selected = index == self.speed_index;
                let text = format!(
                    "{}{}×",
                    if selected { "• " } else { "   " },
                    speed_label(*speed)
                );
                if ui.button(text).clicked() {
                    choice = Some(index);
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Speed ([ and ])");
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
        ui.menu_button(
            RichText::new(format!("Audio {}", self.audio_track + 1)).color(TEXT),
            |ui| {
                for (index, track) in tracks.iter().enumerate() {
                    let selected = index == self.audio_track;
                    if ui
                        .button(format!("{}{track}", if selected { "• " } else { "   " }))
                        .clicked()
                    {
                        choice = Some(index);
                        ui.close();
                    }
                }
            },
        )
        .response
        .on_hover_text("Audio track");
        if let Some(index) = choice {
            self.select_audio(index);
        }
    }

    fn volume_controls(&mut self, ui: &mut egui::Ui) {
        // Right-to-left layout: the slider is added first so the speaker ends up on its left.
        let mut volume = if self.prefs.muted {
            0.0
        } else {
            self.prefs.volume
        };
        ui.spacing_mut().slider_width = 80.0;
        let slider = ui.add(egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false));
        if slider.changed() {
            self.set_volume(volume);
        }
        slider.on_hover_text(format!("Volume {}% (↑ ↓)", (volume * 100.0).round() as u32));
        let off = self.prefs.muted || self.prefs.volume == 0.0 || self.audio.is_none();
        if speaker_button(ui, off).clicked() {
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
fn seek_bar(ui: &mut egui::Ui, width: f32, position: f64, duration: f64, enabled: bool) -> Scrub {
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 20.0), sense);
    let track = egui::Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 5.0));
    let fraction = if duration > 0.0 {
        (position / duration).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let active = response.hovered() || response.dragged();
    let painter = ui.painter();
    painter.rect_filled(track, 2.5, TRACK);
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Icon {
    Play,
    Pause,
    StepBack,
    StepForward,
    SkipBack,
    SkipForward,
    Loop,
    Fullscreen,
    ExitFullscreen,
}

fn icon_color(enabled: bool, hovered: bool, active: bool) -> Color32 {
    if !enabled {
        Color32::from_rgb(90, 90, 96)
    } else if hovered || active {
        LIME
    } else {
        TEXT
    }
}

fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    active: bool,
    enabled: bool,
    tooltip: &str,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), sense);
    draw_icon(
        ui.painter(),
        rect.center(),
        icon,
        icon_color(enabled, response.hovered(), active),
    );
    response.on_hover_text(tooltip)
}

/// The round primary button.
fn play_button(ui: &mut egui::Ui, icon: Icon, enabled: bool, tooltip: &str) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(38.0), sense);
    let hovered = response.hovered() && enabled;
    ui.painter().circle_filled(
        rect.center(),
        18.0,
        if hovered {
            LIME
        } else {
            Color32::from_rgb(52, 52, 56)
        },
    );
    draw_icon(
        ui.painter(),
        rect.center(),
        icon,
        if hovered {
            Color32::BLACK
        } else {
            icon_color(enabled, false, false)
        },
    );
    response.on_hover_text(tooltip)
}

/// Draws `icon` centered on `c`, about 18 px across.
fn draw_icon(painter: &Painter, c: Pos2, icon: Icon, color: Color32) {
    let stroke = Stroke::new(1.8_f32, color);
    let fill = |points: Vec<Pos2>| {
        painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
    };
    let at = |x: f32, y: f32| c + Vec2::new(x, y);
    match icon {
        Icon::Play => fill(vec![at(-4.0, -7.0), at(7.0, 0.0), at(-4.0, 7.0)]),
        Icon::Pause => {
            for x in [-4.0_f32, 4.0] {
                painter.rect_filled(
                    egui::Rect::from_center_size(at(x, 0.0), Vec2::new(3.5, 12.0)),
                    1.0,
                    color,
                );
            }
        }
        Icon::StepForward => {
            fill(vec![at(-6.0, -6.0), at(3.0, 0.0), at(-6.0, 6.0)]);
            painter.rect_filled(
                egui::Rect::from_center_size(at(6.0, 0.0), Vec2::new(2.5, 12.0)),
                0.5,
                color,
            );
        }
        Icon::StepBack => {
            fill(vec![at(6.0, -6.0), at(-3.0, 0.0), at(6.0, 6.0)]);
            painter.rect_filled(
                egui::Rect::from_center_size(at(-6.0, 0.0), Vec2::new(2.5, 12.0)),
                0.5,
                color,
            );
        }
        Icon::SkipBack | Icon::SkipForward => {
            // An open circle with an arrowhead where it starts, and "10" inside.
            let direction: f32 = if icon == Icon::SkipForward { 1.0 } else { -1.0 };
            let radius = 9.0_f32;
            let start = -std::f32::consts::FRAC_PI_2 + direction * 0.45;
            let points: Vec<Pos2> = (0..=22)
                .map(|step| {
                    let angle = start - direction * (step as f32) * 0.2;
                    c + Vec2::new(angle.cos(), angle.sin()) * radius
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
            let tip = c + Vec2::new(start.cos(), start.sin()) * radius;
            let tangent = Vec2::new(-start.sin(), start.cos()) * direction;
            let normal = Vec2::new(tangent.y, -tangent.x);
            fill(vec![
                tip + tangent * 4.5,
                tip - tangent * 1.5 + normal * 3.5,
                tip - tangent * 1.5 - normal * 3.5,
            ]);
            painter.text(
                c,
                Align2::CENTER_CENTER,
                "10",
                FontId::proportional(8.5),
                color,
            );
        }
        Icon::Loop => {
            let line = |points: [Pos2; 3]| {
                painter.add(egui::Shape::line(points.to_vec(), stroke));
            };
            line([at(-9.0, 2.0), at(-9.0, -3.0), at(6.0, -3.0)]);
            fill(vec![at(5.0, -7.0), at(10.0, -3.0), at(5.0, 1.0)]);
            line([at(9.0, -2.0), at(9.0, 3.0), at(-6.0, 3.0)]);
            fill(vec![at(-5.0, -1.0), at(-10.0, 3.0), at(-5.0, 7.0)]);
        }
        Icon::Fullscreen => {
            for (sx, sy) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = at(sx * 8.0, sy * 8.0);
                painter.add(egui::Shape::line(
                    vec![
                        corner + Vec2::new(-sx * 5.0, 0.0),
                        corner,
                        corner + Vec2::new(0.0, -sy * 5.0),
                    ],
                    stroke,
                ));
            }
        }
        Icon::ExitFullscreen => {
            for (sx, sy) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = at(sx * 3.5, sy * 3.5);
                painter.add(egui::Shape::line(
                    vec![
                        corner + Vec2::new(sx * 5.0, 0.0),
                        corner,
                        corner + Vec2::new(0.0, sy * 5.0),
                    ],
                    stroke,
                ));
            }
        }
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

/// A speaker icon: with a slash when sound is off.
fn speaker_button(ui: &mut egui::Ui, off: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
    let painter = ui.painter();
    let color = if response.hovered() { LIME } else { TEXT };
    let center = rect.center();
    // Speaker body: a small box and a cone.
    painter.rect_filled(
        egui::Rect::from_center_size(center + Vec2::new(-5.0, 0.0), Vec2::new(4.0, 6.0)),
        0.0,
        color,
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            center + Vec2::new(-3.0, -3.0),
            center + Vec2::new(2.0, -7.0),
            center + Vec2::new(2.0, 7.0),
            center + Vec2::new(-3.0, 3.0),
        ],
        color,
        Stroke::NONE,
    ));
    if off {
        painter.line_segment(
            [center + Vec2::new(5.0, -5.0), center + Vec2::new(11.0, 5.0)],
            Stroke::new(2.0_f32, color),
        );
        painter.line_segment(
            [center + Vec2::new(11.0, -5.0), center + Vec2::new(5.0, 5.0)],
            Stroke::new(2.0_f32, color),
        );
    } else {
        for radius in [5.0_f32, 9.0] {
            let points: Vec<Pos2> = (-4..=4)
                .map(|step| {
                    let angle = step as f32 * 0.19;
                    center + Vec2::new(3.0 + radius * angle.cos() * 0.9, radius * angle.sin())
                })
                .collect();
            painter.add(egui::Shape::line(points, Stroke::new(1.6_f32, color)));
        }
    }
    response.on_hover_text(if off { "Unmute (M)" } else { "Mute (M)" })
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
mod tests {
    use super::*;
    use clipcraft_player::{DecodedFrame, PlayerError};

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
}
