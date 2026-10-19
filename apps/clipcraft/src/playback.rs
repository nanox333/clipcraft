//! In-app clip playback: turns `clipcraft-player` events into a texture and simple controls.

use clipcraft_audio_out::{AudioOutput, AudioOutputError};
use clipcraft_player::{ClipInfoSummary, Player, PlayerEvent};
use clipcraft_time::Tick;
use eframe::egui;
use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, Vec2};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MUTED: Color32 = Color32::from_rgb(157, 157, 164);
const TEXT: Color32 = Color32::from_rgb(242, 242, 244);
const LIME: Color32 = Color32::from_rgb(190, 255, 43);
/// Seconds skipped by the arrow keys.
const KEY_SEEK_SECONDS: f64 = 5.0;
/// How long to wait before restarting a sound device that reported an error, and how often to try.
const AUDIO_RETRY_DELAY: Duration = Duration::from_secs(1);
const AUDIO_RETRY_LIMIT: u32 = 5;

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
    muted: bool,
    audio_retry_at: Option<Instant>,
    audio_retries: u32,
}

impl Playback {
    /// Opens `path` and starts playing it.
    pub fn start(path: &str, context: &egui::Context) -> Self {
        let wake = context.clone();
        let player = Player::start(PathBuf::from(path), move || wake.request_repaint());
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
            muted: false,
            audio_retry_at: None,
            audio_retries: 0,
        }
    }

    /// The file being played.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Applies everything the worker reported since the last frame.
    pub fn poll(&mut self, context: &egui::Context) {
        for event in self.player.poll() {
            self.apply(event, context);
        }
        self.watch_audio(context);
    }

    /// Opens the sound device for the clip's audio, if it has any.
    fn start_audio(&mut self) {
        let Some(feed) = self.player.audio_feed() else {
            return;
        };
        feed.set_muted(self.muted);
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

    fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        if let Some(feed) = self.player.audio_feed() {
            feed.set_muted(self.muted);
        }
    }

    fn apply(&mut self, event: PlayerEvent, context: &egui::Context) {
        match event {
            PlayerEvent::Opened(info) => {
                self.info = Some(info);
                if info.has_audio {
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

    fn toggle(&mut self) {
        if self.playing {
            self.player.pause();
        } else {
            self.player.play();
        }
        self.playing = !self.playing;
    }

    fn seek(&mut self, seconds: f64) {
        let duration = self.info.map_or(Tick::ZERO, |info| info.duration);
        let target = Tick::from_seconds_f64(seconds.max(0.0)).clamp(Tick::ZERO, duration);
        self.position = target;
        self.player.seek(target);
    }

    /// Draws the picture and its controls into `ui`.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.handle_keys(ui.ctx());

        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 420.0), Sense::click());
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
        match (&self.texture, &self.error) {
            (Some(texture), _) => {
                let fitted = fit(texture.size_vec2(), rect);
                ui.painter().image(
                    texture.id(),
                    fitted,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            (None, Some(message)) => {
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    message,
                    FontId::proportional(16.0),
                    MUTED,
                );
            }
            (None, None) => {
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "Loading…",
                    FontId::proportional(16.0),
                    MUTED,
                );
            }
        }
        if response.clicked() && self.error.is_none() {
            self.toggle();
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if play_button(ui, self.playing).clicked() && self.error.is_none() {
                self.toggle();
            }
            let duration = self.info.map_or(0.0, |info| info.duration.seconds());
            let mut seconds = self.position.seconds().min(duration);
            ui.spacing_mut().slider_width = (ui.available_width() - 130.0).max(80.0);
            let slider = ui.add_enabled(
                self.info.is_some() && self.error.is_none(),
                egui::Slider::new(&mut seconds, 0.0..=duration.max(0.001)).show_value(false),
            );
            if slider.changed() {
                self.seek(seconds);
            }
            ui.label(
                RichText::new(format!(
                    "{} / {}",
                    format_clock(self.position),
                    format_clock(self.info.map_or(Tick::ZERO, |info| info.duration))
                ))
                .color(TEXT)
                .monospace(),
            );
            if self.info.is_some_and(|info| info.has_audio)
                && speaker_button(ui, self.muted || self.audio.is_none()).clicked()
            {
                self.toggle_mute();
            }
        });
        if let Some(note) = &self.audio_note {
            ui.add_space(4.0);
            ui.label(RichText::new(note).color(MUTED).small());
        }
    }

    fn handle_keys(&mut self, context: &egui::Context) {
        if context.wants_keyboard_input() || self.error.is_some() {
            return;
        }
        let (space, left, right, mute) = context.input(|input| {
            (
                input.key_pressed(egui::Key::Space),
                input.key_pressed(egui::Key::ArrowLeft),
                input.key_pressed(egui::Key::ArrowRight),
                input.key_pressed(egui::Key::M),
            )
        });
        if space {
            self.toggle();
        }
        if mute {
            self.toggle_mute();
        }
        if left {
            self.seek(self.position.seconds() - KEY_SEEK_SECONDS);
        }
        if right {
            self.seek(self.position.seconds() + KEY_SEEK_SECONDS);
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

fn play_button(ui: &mut egui::Ui, playing: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    let painter = ui.painter();
    let fill = if response.hovered() {
        LIME
    } else {
        Color32::from_rgb(52, 52, 56)
    };
    painter.circle_filled(rect.center(), 15.0, fill);
    let icon = if response.hovered() {
        Color32::BLACK
    } else {
        TEXT
    };
    let center = rect.center();
    if playing {
        for offset in [-3.5, 3.5] {
            painter.rect_filled(
                egui::Rect::from_center_size(center + Vec2::new(offset, 0.0), Vec2::new(3.0, 11.0)),
                1.0,
                icon,
            );
        }
    } else {
        painter.add(egui::Shape::convex_polygon(
            vec![
                center + Vec2::new(-4.0, -6.0),
                center + Vec2::new(6.0, 0.0),
                center + Vec2::new(-4.0, 6.0),
            ],
            icon,
            Stroke::NONE,
        ));
    }
    response
}

/// A speaker icon: with a slash when sound is off.
fn speaker_button(ui: &mut egui::Ui, off: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
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
            let points: Vec<egui::Pos2> = (-4..=4)
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
        Playback::start("/definitely/not/here.mp4", &egui::Context::default())
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
}
