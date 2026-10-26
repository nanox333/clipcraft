//! ClipCraft's own tray menu: a small borderless window in the app's look, opened by a right
//! click on the tray icon and closed by a choice, Escape, or a click elsewhere.

use crate::kit::{
    BORDER, CARD_HOVER, DANGER, FAINT, Icon, LIME, MUTED, PANEL, TEXT, draw_icon, medium, semibold,
};
use crate::tray::Action;
use eframe::egui::{self, Align2, FontId, Pos2, Sense, Stroke, Vec2, ViewportBuilder, ViewportId};

const WIDTH: f32 = 252.0;
const ROW: f32 = 34.0;
const MARGIN: f32 = 6.0;
const HEADER: f32 = 44.0;
const SEPARATOR: f32 = 9.0;
const SHOW_FRAME: u32 = 3;

/// An open menu.
pub struct TrayMenu {
    /// The click position in points; the menu opens above it, centred on it.
    anchor: Pos2,
    /// The menu has had the focus once, so losing it afterwards means "click elsewhere".
    focused: bool,
    opened: std::time::Instant,
    /// Frames drawn so far. The window starts hidden and is shown on the third, once the system
    /// has moved and sized it, so it never appears at a default place and jumps.
    frames: u32,
}

/// What the menu shows about the recording.
pub struct Status {
    pub recording: bool,
    pub text: String,
}

struct Item {
    icon: Icon,
    label: &'static str,
    action: Action,
    enabled: bool,
}

impl TrayMenu {
    /// `x` and `y` are the click position in physical pixels.
    pub fn at(context: &egui::Context, x: f32, y: f32) -> Self {
        let scale = context.pixels_per_point().max(0.5);
        Self {
            anchor: Pos2::new(x / scale, y / scale),
            focused: false,
            opened: std::time::Instant::now(),
            frames: 0,
        }
    }

    fn groups(recording: bool) -> Vec<Vec<Item>> {
        vec![
            vec![Item {
                icon: Icon::Library,
                label: "Open ClipCraft",
                action: Action::Open,
                enabled: true,
            }],
            vec![
                Item {
                    icon: if recording { Icon::Stop } else { Icon::Record },
                    label: if recording {
                        "Stop recording"
                    } else {
                        "Start recording"
                    },
                    action: Action::ToggleRecording,
                    enabled: true,
                },
                Item {
                    icon: Icon::Scissors,
                    label: "Save clip",
                    action: Action::SaveClip,
                    enabled: recording,
                },
                Item {
                    icon: Icon::Fullscreen,
                    label: "Take screenshot",
                    action: Action::Screenshot,
                    enabled: recording,
                },
            ],
            vec![
                Item {
                    icon: Icon::Folder,
                    label: "Open clips folder",
                    action: Action::OpenClipsFolder,
                    enabled: true,
                },
                Item {
                    icon: Icon::Library,
                    label: "Library",
                    action: Action::OpenLibrary,
                    enabled: true,
                },
                Item {
                    icon: Icon::Gear,
                    label: "Settings",
                    action: Action::OpenSettings,
                    enabled: true,
                },
            ],
            vec![Item {
                icon: Icon::Trash,
                label: "Quit ClipCraft",
                action: Action::Quit,
                enabled: true,
            }],
        ]
    }

    /// Draws the menu. Returns the choice made, if any, and whether the menu stays open.
    pub fn show(&mut self, context: &egui::Context, status: &Status) -> (Option<Action>, bool) {
        let groups = Self::groups(status.recording);
        let rows: usize = groups.iter().map(Vec::len).sum();
        let height =
            MARGIN * 2.0 + HEADER + rows as f32 * ROW + (groups.len() - 1) as f32 * SEPARATOR;
        let position = Pos2::new(
            (self.anchor.x - WIDTH / 2.0).max(4.0),
            (self.anchor.y - height - 10.0).max(4.0),
        );
        let builder = ViewportBuilder::default()
            .with_title("ClipCraft menu")
            .with_decorations(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_visible(false)
            .with_position(position)
            .with_inner_size([WIDTH, height]);
        let mut choice = None;
        let mut open = true;
        let mut focused = self.focused;
        let opened = self.opened;
        let mut frames = self.frames;
        context.show_viewport_immediate(
            ViewportId::from_hash_of("clipcraft-tray-menu"),
            builder,
            |context, _class| {
                let (has_focus, escape, close) = context.input(|input| {
                    (
                        input.viewport().focused,
                        input.key_pressed(egui::Key::Escape),
                        input.viewport().close_requested(),
                    )
                });
                if has_focus == Some(true) {
                    focused = true;
                }
                if escape
                    || close
                    || (focused && has_focus == Some(false))
                    // Never got the focus (another window kept it): give up after a moment.
                    || (!focused && opened.elapsed() > std::time::Duration::from_secs(2))
                {
                    open = false;
                    return;
                }
                frames += 1;
                if frames == SHOW_FRAME {
                    context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    context.send_viewport_cmd(egui::ViewportCommand::Focus);
                } else if frames < SHOW_FRAME {
                    context.request_repaint();
                }
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::new()
                            .fill(PANEL)
                            .stroke(Stroke::new(1.0_f32, BORDER))
                            .inner_margin(egui::Margin::same(MARGIN as i8)),
                    )
                    .show(context, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        header(ui, status);
                        for (index, group) in groups.iter().enumerate() {
                            if index > 0 {
                                separator(ui);
                            }
                            for item in group {
                                if row(ui, item) {
                                    choice = Some(item.action);
                                }
                            }
                        }
                    });
                context.request_repaint_after(std::time::Duration::from_millis(250));
            },
        );
        self.focused = focused;
        self.frames = frames;
        if choice.is_some() {
            open = false;
        }
        (choice, open)
    }
}

fn header(ui: &mut egui::Ui, status: &Status) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HEADER), Sense::hover());
    let painter = ui.painter();
    let dot = if status.recording { DANGER } else { FAINT };
    painter.circle_filled(Pos2::new(rect.left() + 16.0, rect.center().y), 4.5, dot);
    painter.text(
        Pos2::new(rect.left() + 30.0, rect.center().y - 7.0),
        Align2::LEFT_CENTER,
        "ClipCraft",
        FontId::new(14.0, semibold()),
        TEXT,
    );
    painter.text(
        Pos2::new(rect.left() + 30.0, rect.center().y + 9.0),
        Align2::LEFT_CENTER,
        &status.text,
        FontId::proportional(12.0),
        MUTED,
    );
}

fn separator(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), SEPARATOR), Sense::hover());
    ui.painter().hline(
        rect.x_range().shrink(8.0),
        rect.center().y,
        Stroke::new(1.0_f32, BORDER),
    );
}

fn row(ui: &mut egui::Ui, item: &Item) -> bool {
    let sense = if item.enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), sense);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, item.enabled, item.label)
    });
    let hovered = item.enabled && response.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 8.0, CARD_HOVER);
    }
    let ink = if !item.enabled {
        FAINT
    } else if hovered && item.action == Action::Quit {
        DANGER
    } else if hovered {
        LIME
    } else {
        TEXT
    };
    draw_icon(
        painter,
        Pos2::new(rect.left() + 20.0, rect.center().y),
        item.icon,
        ink,
        0.9,
    );
    painter.text(
        Pos2::new(rect.left() + 40.0, rect.center().y),
        Align2::LEFT_CENTER,
        item.label,
        FontId::new(13.5, medium()),
        if item.enabled { TEXT } else { FAINT },
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}
