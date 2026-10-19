//! In-app clip playback: turns `clipcraft-player` events into a texture and simple controls.

use clipcraft_player::{ClipInfoSummary, Player, PlayerEvent};
use clipcraft_time::Tick;
use eframe::egui;
use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, Vec2};
use std::path::PathBuf;

const MUTED: Color32 = Color32::from_rgb(157, 157, 164);
const TEXT: Color32 = Color32::from_rgb(242, 242, 244);
const LIME: Color32 = Color32::from_rgb(190, 255, 43);
/// Seconds skipped by the arrow keys.
const KEY_SEEK_SECONDS: f64 = 5.0;

/// A clip open in the detail view.
pub struct Playback {
    path: String,
    player: Player,
    texture: Option<egui::TextureHandle>,
    info: Option<ClipInfoSummary>,
    position: Tick,
    playing: bool,
    error: Option<String>,
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
    }

    fn apply(&mut self, event: PlayerEvent, context: &egui::Context) {
        match event {
            PlayerEvent::Opened(info) => self.info = Some(info),
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
        });
    }

    fn handle_keys(&mut self, context: &egui::Context) {
        if context.wants_keyboard_input() || self.error.is_some() {
            return;
        }
        let (space, left, right) = context.input(|input| {
            (
                input.key_pressed(egui::Key::Space),
                input.key_pressed(egui::Key::ArrowLeft),
                input.key_pressed(egui::Key::ArrowRight),
            )
        });
        if space {
            self.toggle();
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
