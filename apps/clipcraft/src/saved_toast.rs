//! The pop-up that appears when a clip is saved: a small card in the corner of the screen with the
//! clip's picture, what it is, and buttons to play it, find it or delete it. It is a window of its
//! own, so it shows even while ClipCraft is minimized to the tray and a game is in front, and it
//! never takes the keyboard focus from that game.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2, ViewportBuilder,
    ViewportCommand, ViewportId,
};

use crate::kit::{self, DANGER, Icon, LIME, MUTED, PANEL, TEXT, medium, semibold};
use crate::notify::{self, Kind, ease_in, ease_out};

const WIDTH: f32 = 408.0;
const HEIGHT: f32 = 116.0;
/// Gap to the edges of the free part of the screen.
const MARGIN: f32 = 18.0;
/// How far the card travels when it slides in and out. It is a window of its own, so it slides
/// in from the screen's edge instead of rising and fading like the in-window card; the timing and
/// easing are the shared ones from `notify`.
const SLIDE: f32 = 80.0;
const SLIDE_IN: f32 = notify::ENTER_SECONDS;
const SLIDE_OUT: f32 = notify::EXIT_SECONDS;
/// The window's title, which is how the system is asked not to give it the focus.
const TITLE: &str = "ClipCraft saved clip";

/// What the person asked for on the card.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToastAction {
    Play,
    Show,
    Delete,
}

pub(crate) struct SavedToast {
    pub path: PathBuf,
    title: String,
    meta: String,
    created: Instant,
    last_frame: Instant,
    remaining: f32,
    closing: Option<Instant>,
    frames: u32,
    texture: Option<egui::TextureHandle>,
    picture: Option<mpsc::Receiver<Option<egui::ColorImage>>>,
    sure_to_delete: Option<Instant>,
    deleted: bool,
}

impl SavedToast {
    /// `title` is the game's name or "Clip"; `meta` the line under it.
    pub(crate) fn new(
        path: PathBuf,
        title: String,
        meta: String,
        cache_dir: Option<PathBuf>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let source = path.clone();
        std::thread::spawn(move || {
            let _ = sender.send(cache_dir.and_then(|dir| load_picture(&source, dir)));
        });
        Self {
            path,
            title,
            meta,
            created: Instant::now(),
            last_frame: Instant::now(),
            remaining: Kind::Success.stay(),
            closing: None,
            frames: 0,
            texture: None,
            picture: Some(receiver),
            sure_to_delete: None,
            deleted: false,
        }
    }

    /// The clip was moved to the Recycle Bin from the card: say so, then leave soon.
    pub(crate) fn mark_deleted(&mut self) {
        self.deleted = true;
        self.sure_to_delete = None;
        self.remaining = self.remaining.min(2.2);
    }

    /// Draws the card. Returns what was asked for, and whether the card is still on screen.
    pub(crate) fn show(
        &mut self,
        context: &egui::Context,
        free: Option<Rect>,
    ) -> (Option<ToastAction>, bool) {
        if let Some(receiver) = &self.picture
            && let Ok(image) = receiver.try_recv()
        {
            self.picture = None;
            if let Some(image) = image {
                self.texture =
                    Some(context.load_texture("saved-clip-picture", image, Default::default()));
            }
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        // Where the card rests: the bottom right of the free part of the screen, in points.
        let free = free.unwrap_or_else(|| {
            let screen = context
                .input(|input| input.viewport().monitor_size)
                .unwrap_or(Vec2::new(1920.0, 1040.0));
            Rect::from_min_size(Pos2::ZERO, screen - Vec2::new(0.0, 48.0))
        });
        let rest = Pos2::new(
            free.right() - WIDTH - MARGIN,
            free.bottom() - HEIGHT - MARGIN,
        );
        let age = self.created.elapsed().as_secs_f32();
        let arriving = ease_out((age / SLIDE_IN).clamp(0.0, 1.0));
        let leaving = self.closing.map_or(0.0, |at| {
            ease_in((at.elapsed().as_secs_f32() / SLIDE_OUT).clamp(0.0, 1.0))
        });
        let offset = (1.0 - arriving + leaving) * SLIDE;
        let position = Pos2::new(rest.x + offset, rest.y);

        let builder = ViewportBuilder::default()
            .with_title(TITLE)
            .with_decorations(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_visible(false)
            .with_active(false)
            .with_position(position)
            .with_inner_size([WIDTH, HEIGHT]);
        let mut action = None;
        let mut hovered = false;
        let mut dismiss = false;
        context.show_viewport_immediate(
            ViewportId::from_hash_of("clipcraft-saved-toast"),
            builder,
            |context, _class| {
                self.frames += 1;
                // The window system rewrites a window's style when it is shown, so the "never take
                // the focus" marking is renewed every frame; it is cheap and does nothing when
                // the marking is already there.
                if self.frames >= 2 {
                    clipcraft_platform::make_window_passive(TITLE);
                }
                if self.frames == 3 {
                    context.send_viewport_cmd(ViewportCommand::Visible(true));
                }
                context.send_viewport_cmd(ViewportCommand::OuterPosition(position));
                hovered = context.input(|input| input.pointer.hover_pos().is_some());
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::new()
                            .fill(PANEL)
                            .stroke(Stroke::new(1.0_f32, Kind::Success.hairline())),
                    )
                    .show(context, |ui| {
                        let rect = ui.max_rect();
                        self.paint(ui, rect, &mut action, &mut dismiss);
                    });
                context.request_repaint_after(Duration::from_millis(16));
            },
        );
        if !hovered && self.closing.is_none() {
            self.remaining -= dt;
        }
        if (self.remaining <= 0.0 || dismiss) && self.closing.is_none() {
            self.closing = Some(now);
        }
        if self
            .sure_to_delete
            .is_some_and(|at| at.elapsed() > Duration::from_secs(3))
        {
            self.sure_to_delete = None;
        }
        let alive = leaving < 1.0;
        (action, alive)
    }

    fn paint(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        action: &mut Option<ToastAction>,
        dismiss: &mut bool,
    ) {
        let painter = ui.painter().clone();
        // The time left, as along the bottom of every floating notification.
        if !self.deleted {
            notify::paint_time_left(
                &painter,
                rect,
                self.remaining / Kind::Success.stay(),
                Kind::Success,
            );
        }

        // The clip's picture.
        let picture = Rect::from_min_size(
            Pos2::new(rect.left() + 20.0, rect.center().y - 36.0),
            Vec2::new(128.0, 72.0),
        );
        match &self.texture {
            Some(texture) => {
                ui.put(
                    picture,
                    egui::Image::new((texture.id(), picture.size()))
                        .corner_radius(crate::kit::radius::MD),
                );
            }
            None => {
                painter.rect_filled(
                    picture,
                    f32::from(crate::kit::radius::MD),
                    Color32::from_rgb(20, 20, 24),
                );
                kit::draw_icon(&painter, picture.center(), Icon::Play, MUTED, 1.0);
            }
        }
        painter.rect_stroke(
            picture,
            8.0,
            Stroke::new(1.0_f32, Color32::from_white_alpha(18)),
            egui::StrokeKind::Inside,
        );

        // What it is: the same round badge as every notification, then what happened.
        let left = picture.right() + 16.0;
        let kind = if self.deleted {
            Kind::Info
        } else {
            Kind::Success
        };
        notify::paint_badge(&painter, Pos2::new(left + 10.0, rect.top() + 24.0), kind);
        painter.text(
            Pos2::new(left + 30.0, rect.top() + 24.0),
            Align2::LEFT_CENTER,
            if self.deleted {
                "MOVED TO THE RECYCLE BIN"
            } else {
                "CLIP SAVED"
            },
            FontId::new(crate::kit::text_size::CAPTION, semibold()),
            kind.accent(),
        );
        let room = rect.right() - left - 34.0;
        painter.text(
            Pos2::new(left, rect.top() + 46.0),
            Align2::LEFT_CENTER,
            fit(
                &painter,
                &self.title,
                FontId::new(crate::kit::text_size::LEAD, semibold()),
                room,
            ),
            FontId::new(crate::kit::text_size::LEAD, semibold()),
            TEXT,
        );
        painter.text(
            Pos2::new(left, rect.top() + 66.0),
            Align2::LEFT_CENTER,
            fit(
                &painter,
                &self.meta,
                FontId::proportional(crate::kit::text_size::CAPTION),
                room,
            ),
            FontId::proportional(crate::kit::text_size::CAPTION),
            MUTED,
        );

        // The close button.
        let close = Rect::from_center_size(
            Pos2::new(rect.right() - 20.0, rect.top() + 20.0),
            Vec2::splat(24.0),
        );
        let response = ui.interact(close, ui.id().with("close"), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Dismiss notification")
        });
        notify::paint_close(&painter, close, response.hovered());
        if response.clicked() {
            *dismiss = true;
        }

        if self.deleted {
            return;
        }
        // The buttons: play, show in folder, delete.
        let top = rect.bottom() - 40.0;
        let play = Rect::from_min_size(Pos2::new(left, top), Vec2::new(72.0, 28.0));
        let response = ui.interact(play, ui.id().with("play"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Play"));
        painter.rect_filled(
            play,
            14.0,
            if response.hovered() {
                Color32::from_rgb(210, 255, 90)
            } else {
                LIME
            },
        );
        kit::draw_icon(
            &painter,
            Pos2::new(play.left() + 18.0, play.center().y),
            Icon::Play,
            Color32::BLACK,
            0.6,
        );
        painter.text(
            Pos2::new(play.left() + 31.0, play.center().y),
            Align2::LEFT_CENTER,
            "Play",
            FontId::new(crate::kit::text_size::SMALL, medium()),
            Color32::BLACK,
        );
        if response.clicked() {
            *action = Some(ToastAction::Play);
        }
        let folder = Rect::from_min_size(Pos2::new(play.right() + 8.0, top), Vec2::splat(28.0));
        if round_button(ui, folder, "folder", Icon::Folder, "Show in folder", false) {
            *action = Some(ToastAction::Show);
        }
        let trash =
            Rect::from_min_size(Pos2::new(folder.right() + 6.0, top), Vec2::new(28.0, 28.0));
        if let Some(asked) = self.sure_to_delete {
            // Second step: a red button with the words, so a stray click does nothing.
            let sure = Rect::from_min_size(trash.min, Vec2::new(74.0, 28.0));
            let response = ui.interact(sure, ui.id().with("sure"), Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Confirm delete")
            });
            painter.rect_filled(
                sure,
                14.0,
                if response.hovered() {
                    Color32::from_rgb(255, 132, 132)
                } else {
                    DANGER
                },
            );
            painter.text(
                sure.center(),
                Align2::CENTER_CENTER,
                "Delete?",
                FontId::new(crate::kit::text_size::SMALL, medium()),
                Color32::BLACK,
            );
            let _ = asked;
            if response.clicked() {
                *action = Some(ToastAction::Delete);
            }
        } else if round_button(ui, trash, "trash", Icon::Trash, "Delete clip", true) {
            self.sure_to_delete = Some(Instant::now());
        }
    }
}

/// A round icon button. True when clicked.
fn round_button(
    ui: &mut egui::Ui,
    rect: Rect,
    id: &str,
    icon: Icon,
    label: &str,
    danger: bool,
) -> bool {
    let response = ui.interact(rect, ui.id().with(id), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let hovered = response.hovered();
    ui.painter().rect_filled(
        rect,
        14.0,
        if hovered {
            kit::CONTROL_HOVER
        } else {
            kit::CONTROL
        },
    );
    let ink = if hovered && danger {
        DANGER
    } else if hovered {
        LIME
    } else {
        TEXT
    };
    kit::draw_icon(ui.painter(), rect.center(), icon, ink, 0.8);
    response.clicked()
}

/// `text` cut short with an ellipsis so it fits in `room` points.
fn fit(painter: &egui::Painter, text: &str, font: FontId, room: f32) -> String {
    let width = |value: &str| {
        painter
            .layout_no_wrap(value.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    if width(text) <= room {
        return text.to_owned();
    }
    let mut kept: Vec<char> = text.chars().collect();
    while !kept.is_empty() {
        kept.pop();
        let candidate: String = kept.iter().collect::<String>() + "…";
        if width(&candidate) <= room {
            return candidate;
        }
    }
    String::new()
}

/// The clip's picture, from the thumbnail cache.
fn load_picture(path: &std::path::Path, cache_dir: PathBuf) -> Option<egui::ColorImage> {
    use clipcraft_engine::{Command, CommandOutput, execute};
    let CommandOutput::Thumbnail(file) = execute(Command::EnsureThumbnail {
        path: path.to_path_buf(),
        cache_dir,
    })
    .ok()?
    else {
        return None;
    };
    let image = image::open(file.file).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(
        size,
        image.as_raw(),
    ))
}
