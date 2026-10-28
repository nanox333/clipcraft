//! First launch uses a draft; nothing captures or publishes while choices are being made.
use crate::{ClipCraftApp, kit, settings};
use clipcraft_engine::{Command, CommandOutput, RecordingSettings, execute};
use eframe::egui;

pub(crate) struct Setup {
    step: usize,
    draft: RecordingSettings,
    controls: settings::SettingsState,
    error: Option<String>,
    /// Which page was last drawn, and when it appeared, for the fade-in.
    shown_step: usize,
    entered: std::time::Instant,
    /// Free space on the clips folder's drive, read once per folder.
    space_for: String,
    free_bytes: Option<u64>,
}

impl Setup {
    pub(crate) fn new(mut draft: RecordingSettings) -> Self {
        if draft.capture_folder.is_empty() {
            draft.capture_folder = clipcraft_engine::default_capture_folder()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        Self {
            step: 0,
            draft,
            controls: Default::default(),
            error: None,
            shown_step: 0,
            entered: std::time::Instant::now(),
            space_for: String::new(),
            free_bytes: None,
        }
    }

    fn finish(&mut self, path: Option<std::path::PathBuf>) -> Result<RecordingSettings, String> {
        let path = path
            .ok_or("Your settings folder is unavailable. Check your user profile and try again.")?;
        if !self.draft.capture_folder.is_empty()
            && !std::path::Path::new(&self.draft.capture_folder).is_absolute()
        {
            return Err("Choose an absolute folder path for your clips.".into());
        }
        let mut settings = self.draft.clone();
        settings.setup_completed = true;
        match execute(Command::SaveRecordingSettings {
            path,
            settings: Box::new(settings),
        }) {
            Ok(CommandOutput::RecordingSettings(settings)) => Ok(*settings),
            Ok(_) => Err("Settings could not be saved. Please try again.".into()),
            Err(error) => Err(error.to_string()),
        }
    }
}

const STEPS: [&str; 7] = [
    "Welcome",
    "Quality",
    "Capture",
    "Sound",
    "Storage",
    "Shortcuts",
    "Ready",
];
const COLUMN: f32 = 820.0;

type Color = egui::Color32;

fn lime_alpha(alpha: u8) -> Color {
    Color::from_rgba_unmultiplied(190, 255, 43, alpha)
}

/// Fast at first, gentle at the end.
fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// A rounded text button: lime when it is the way forward, quiet otherwise.
fn pill_button(ui: &mut egui::Ui, label: &str, primary: bool) -> egui::Response {
    let font = egui::FontId::new(14.5, kit::medium());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color::PLACEHOLDER);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(galley.size().x + 44.0, 42.0),
        egui::Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let (fill, ink) = match (primary, response.hovered()) {
        (true, false) => (kit::LIME, Color::BLACK),
        (true, true) => (Color::from_rgb(210, 255, 90), Color::BLACK),
        (false, false) => (kit::CONTROL, kit::TEXT),
        (false, true) => (kit::CONTROL_HOVER, kit::TEXT),
    };
    ui.painter().rect_filled(rect, 13.0, fill);
    ui.painter().galley(
        egui::pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        ink,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A quiet clickable word.
fn text_link(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Label::new(egui::RichText::new(label).color(kit::MUTED).size(13.5))
            .sense(egui::Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A soft lime glow: stacked, fading circles.
fn glow(painter: &egui::Painter, centre: egui::Pos2, radius: f32, strength: f32) {
    // A radial gradient mesh, so there is no banding: bright in the middle, fading to nothing.
    const SEGMENTS: usize = 48;
    const RINGS: [(f32, f32); 4] = [(0.0, 1.0), (0.35, 0.45), (0.7, 0.12), (1.0, 0.0)];
    let peak = strength * 255.0 * 0.55;
    let colour =
        |fade: f32| egui::Color32::from_rgba_unmultiplied(190, 255, 43, (peak * fade) as u8);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, colour(RINGS[0].1));
    for &(distance, fade) in &RINGS[1..] {
        for step in 0..SEGMENTS {
            let angle = step as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(
                centre + egui::vec2(angle.cos(), angle.sin()) * radius * distance,
                colour(fade),
            );
        }
    }
    let ring = |index: usize, step: usize| (1 + index * SEGMENTS + step % SEGMENTS) as u32;
    for step in 0..SEGMENTS {
        mesh.add_triangle(0, ring(0, step), ring(0, step + 1));
    }
    for index in 0..RINGS.len() - 2 {
        for step in 0..SEGMENTS {
            let (a, b) = (ring(index, step), ring(index, step + 1));
            let (c, d) = (ring(index + 1, step), ring(index + 1, step + 1));
            mesh.add_triangle(a, c, b);
            mesh.add_triangle(b, c, d);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// The seven steps as numbered dots joined by a line, the finished ones filled.
fn stepper(ui: &mut egui::Ui, current: usize, progress: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 62.0), egui::Sense::hover());
    let painter = ui.painter();
    let count = STEPS.len();
    let slot = width / count as f32;
    let centre =
        |index: usize| egui::pos2(rect.left() + slot * (index as f32 + 0.5), rect.top() + 16.0);
    for index in 0..count - 1 {
        let (from, to) = (
            centre(index) + egui::vec2(18.0, 0.0),
            centre(index + 1) - egui::vec2(18.0, 0.0),
        );
        painter.line_segment([from, to], egui::Stroke::new(2.0_f32, kit::BORDER));
        // The line to the current step fills in as the page arrives.
        let fill = if index < current.saturating_sub(1) {
            1.0
        } else if index + 1 == current {
            progress
        } else {
            0.0
        };
        if fill > 0.0 {
            painter.line_segment(
                [from, from + (to - from) * fill],
                egui::Stroke::new(2.0_f32, kit::LIME),
            );
        }
    }
    for (index, name) in STEPS.into_iter().enumerate() {
        let at = centre(index);
        let (fill, ring, ink) = if index < current {
            (kit::LIME, kit::LIME, Color::BLACK)
        } else if index == current {
            (kit::PANEL, kit::LIME, kit::LIME)
        } else {
            (kit::PANEL, kit::BORDER, kit::FAINT)
        };
        if index == current {
            glow(painter, at, 30.0, 0.35);
        }
        painter.circle_filled(at, 15.0, fill);
        painter.circle_stroke(at, 15.0, egui::Stroke::new(2.0_f32, ring));
        painter.text(
            at,
            egui::Align2::CENTER_CENTER,
            if index < current {
                "\u{2713}".to_owned()
            } else {
                (index + 1).to_string()
            },
            egui::FontId::new(13.5, kit::semibold()),
            ink,
        );
        painter.text(
            egui::pos2(at.x, rect.top() + 46.0),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::new(12.5, kit::medium()),
            if index == current {
                kit::TEXT
            } else {
                kit::MUTED
            },
        );
    }
}

/// A big card you click to choose it. `paint` fills the inside.
fn choice_tile(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    selected: bool,
    label: &str,
    paint: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, label)
    });
    let hover = ui
        .ctx()
        .animate_bool_with_time(response.id, response.hovered(), 0.12);
    let base = if selected {
        Color::from_rgb(38, 44, 28)
    } else {
        kit::CARD
    };
    let fill = base.lerp_to_gamma(kit::CARD_HOVER, hover * 0.6);
    ui.painter().rect_filled(rect, 18.0, fill);
    ui.painter().rect_stroke(
        rect,
        18.0,
        if selected {
            egui::Stroke::new(2.0_f32, kit::LIME)
        } else {
            egui::Stroke::new(1.0_f32, kit::BORDER)
        },
        egui::StrokeKind::Inside,
    );
    if selected {
        let at = egui::pos2(rect.right() - 22.0, rect.top() + 22.0);
        ui.painter().circle_filled(at, 11.0, kit::LIME);
        ui.painter().text(
            at,
            egui::Align2::CENTER_CENTER,
            "\u{2713}",
            egui::FontId::new(12.5, kit::semibold()),
            Color::BLACK,
        );
    }
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(20.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    paint(&mut inner);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Three rising bars, `level` of them lit: a picture of "how much quality".
fn level_bars(ui: &mut egui::Ui, level: usize) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(46.0, 30.0), egui::Sense::hover());
    for index in 0..3 {
        let height = 10.0 + index as f32 * 8.0;
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + index as f32 * 16.0, rect.bottom() - height),
            egui::vec2(12.0, height),
        );
        ui.painter().rect_filled(
            bar,
            3.0,
            if index < level {
                kit::LIME
            } else {
                Color::from_rgb(62, 62, 69)
            },
        );
    }
}

/// A monitor drawn with lines: a play button for games, two windows for the whole screen.
fn monitor(ui: &mut egui::Ui, games: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(120.0, 84.0), egui::Sense::hover());
    let screen = egui::Rect::from_min_size(rect.min, egui::vec2(120.0, 66.0));
    let painter = ui.painter();
    painter.rect_filled(screen, 8.0, Color::from_rgb(20, 20, 23));
    painter.rect_stroke(
        screen,
        8.0,
        egui::Stroke::new(2.0_f32, kit::MUTED),
        egui::StrokeKind::Inside,
    );
    let stand = egui::pos2(rect.center().x, screen.bottom());
    painter.line_segment(
        [stand, stand + egui::vec2(0.0, 10.0)],
        egui::Stroke::new(2.0_f32, kit::MUTED),
    );
    painter.line_segment(
        [
            stand + egui::vec2(-16.0, 10.0),
            stand + egui::vec2(16.0, 10.0),
        ],
        egui::Stroke::new(2.0_f32, kit::MUTED),
    );
    if games {
        let c = screen.center();
        painter.circle_filled(c, 17.0, lime_alpha(40));
        painter.add(egui::Shape::convex_polygon(
            vec![
                c + egui::vec2(-5.0, -9.0),
                c + egui::vec2(-5.0, 9.0),
                c + egui::vec2(9.0, 0.0),
            ],
            kit::LIME,
            egui::Stroke::NONE,
        ));
    } else {
        for (offset, size) in [
            (egui::vec2(10.0, 10.0), egui::vec2(52.0, 32.0)),
            (egui::vec2(48.0, 28.0), egui::vec2(58.0, 28.0)),
        ] {
            let window = egui::Rect::from_min_size(screen.min + offset, size);
            painter.rect_filled(window, 4.0, Color::from_rgb(40, 40, 46));
            painter.rect_stroke(
                window,
                4.0,
                egui::Stroke::new(1.5_f32, kit::MUTED),
                egui::StrokeKind::Inside,
            );
        }
    }
}

/// A line of text in the tile style.
fn tile_text(ui: &mut egui::Ui, heading: &str, body: &str) {
    ui.label(
        egui::RichText::new(heading)
            .color(kit::TEXT)
            .size(17.0)
            .family(kit::semibold()),
    );
    ui.add_space(3.0);
    ui.label(egui::RichText::new(body).color(kit::MUTED).size(13.0));
}

/// A little pill with a dot: green when something is ready.
fn chip(ui: &mut egui::Ui, ready: bool, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::new(12.5, kit::medium()),
        Color::PLACEHOLDER,
    );
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(galley.size().x + 38.0, 30.0),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(rect, 15.0, kit::CARD);
    ui.painter().rect_stroke(
        rect,
        15.0,
        egui::Stroke::new(1.0_f32, kit::BORDER),
        egui::StrokeKind::Inside,
    );
    ui.painter().circle_filled(
        egui::pos2(rect.left() + 15.0, rect.center().y),
        4.0,
        if ready { kit::LIME } else { kit::FAINT },
    );
    ui.painter().galley(
        egui::pos2(rect.left() + 27.0, rect.center().y - galley.size().y / 2.0),
        galley,
        kit::TEXT,
    );
}

/// A key on a keyboard: `Ctrl`, `Shift`, `F8`.
fn keycap(ui: &mut egui::Ui, text: &str, active: bool) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::new(15.0, kit::semibold()),
        Color::PLACEHOLDER,
    );
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2((galley.size().x + 28.0).max(44.0), 40.0),
        egui::Sense::hover(),
    );
    let body = egui::Rect::from_min_max(rect.min, rect.max - egui::vec2(0.0, 3.0));
    ui.painter()
        .rect_filled(rect, 9.0, Color::from_rgb(20, 20, 23));
    ui.painter().rect_filled(
        body,
        9.0,
        if active {
            lime_alpha(60)
        } else {
            Color::from_rgb(58, 58, 66)
        },
    );
    ui.painter().rect_stroke(
        body,
        9.0,
        egui::Stroke::new(
            1.0_f32,
            if active {
                kit::LIME
            } else {
                kit::CONTROL_BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        egui::pos2(
            body.center().x - galley.size().x / 2.0,
            body.center().y - galley.size().y / 2.0,
        ),
        galley,
        if active { kit::LIME } else { kit::TEXT },
    );
}

/// A bar that shows how far back a clip reaches.
fn timeline(ui: &mut egui::Ui, seconds: u32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 74.0), egui::Sense::hover());
    let bar = egui::Rect::from_min_size(
        egui::pos2(rect.left(), rect.top() + 26.0),
        egui::vec2(rect.width(), 14.0),
    );
    let painter = ui.painter();
    painter.rect_filled(bar, 7.0, kit::CARD);
    let kept = (seconds as f32 / 120.0).clamp(0.04, 1.0);
    let lit = egui::Rect::from_min_max(
        egui::pos2(bar.right() - bar.width() * kept, bar.top()),
        bar.max,
    );
    painter.rect_filled(lit, 7.0, lime_alpha(200));
    let marker = egui::pos2(bar.right(), bar.center().y);
    painter.circle_filled(marker, 11.0, kit::LIME);
    painter.add(egui::Shape::convex_polygon(
        vec![
            marker + egui::vec2(-3.0, -5.0),
            marker + egui::vec2(-3.0, 5.0),
            marker + egui::vec2(5.0, 0.0),
        ],
        Color::BLACK,
        egui::Stroke::NONE,
    ));
    painter.text(
        egui::pos2(bar.left(), rect.top() + 8.0),
        egui::Align2::LEFT_CENTER,
        "Earlier",
        egui::FontId::new(12.0, kit::medium()),
        kit::FAINT,
    );
    painter.text(
        egui::pos2(bar.right(), rect.top() + 8.0),
        egui::Align2::RIGHT_CENTER,
        "You press the save shortcut",
        egui::FontId::new(12.0, kit::medium()),
        kit::MUTED,
    );
    let near_end = lit.left() > bar.right() - 280.0;
    painter.text(
        egui::pos2(
            if near_end {
                bar.right()
            } else {
                lit.left().max(bar.left() + 4.0)
            },
            bar.bottom() + 16.0,
        ),
        if near_end {
            egui::Align2::RIGHT_CENTER
        } else {
            egui::Align2::LEFT_CENTER
        },
        format!("\u{2190} the last {seconds} seconds become your clip"),
        egui::FontId::new(13.0, kit::semibold()),
        kit::LIME,
    );
}

fn seconds_text(value: u32) -> String {
    match value {
        s if s < 60 => format!("{s} s"),
        s if s % 60 == 0 => format!("{} min", s / 60),
        s => format!("{}:{:02}", s / 60, s % 60),
    }
}

/// "About 1.2 GB" style sizes.
fn size_text(bytes: u64) -> String {
    crate::format_bytes(bytes)
}

impl ClipCraftApp {
    pub(crate) fn show_setup(&mut self, context: &egui::Context) {
        self.poll_gpu_probe();
        let gpu = self.capture.gpu.clone();
        egui::TopBottomPanel::top("setup-title")
            .exact_height(52.0)
            .frame(
                egui::Frame::new()
                    .fill(kit::SIDEBAR)
                    .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
                    .inner_margin(egui::Margin::symmetric(18, 0)),
            )
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                    kit::brand_mark(ui, rect);
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("ClipCraft setup")
                            .color(kit::TEXT)
                            .size(15.0)
                            .family(kit::semibold()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if text_link(ui, "Close").clicked() {
                            context.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
            });
        let Some(setup) = self.setup.as_mut() else {
            return;
        };
        if setup.shown_step != setup.step {
            setup.shown_step = setup.step;
            setup.entered = std::time::Instant::now();
        }
        let arrived = ease(setup.entered.elapsed().as_secs_f32() / 0.35);
        let (mut finish, mut back, mut next) = (false, false, false);
        egui::TopBottomPanel::bottom("setup-footer")
            .exact_height(80.0)
            .frame(
                egui::Frame::new()
                    .fill(kit::SIDEBAR)
                    .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
                    .inner_margin(egui::Margin::symmetric(24, 0)),
            )
            .show(context, |ui| {
                let width = ui.available_width().min(COLUMN);
                let pad = (ui.available_width() - width) / 2.0;
                ui.horizontal_centered(|ui| {
                    ui.add_space(pad);
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, 42.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if setup.step > 0 && pill_button(ui, "Back", false).clicked() {
                                back = true;
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let last = setup.step == STEPS.len() - 1;
                                    let label = match setup.step {
                                        0 => "Get started",
                                        _ if last => "Start using ClipCraft",
                                        _ => "Continue",
                                    };
                                    if pill_button(ui, label, true).clicked() {
                                        if last {
                                            finish = true;
                                        } else {
                                            next = true;
                                        }
                                    }
                                    if !last {
                                        ui.add_space(8.0);
                                        if text_link(ui, "Skip setup")
                                            .on_hover_text(
                                                "Keep the settings shown and leave setup.",
                                            )
                                            .clicked()
                                        {
                                            finish = true;
                                        }
                                    }
                                },
                            );
                        },
                    );
                });
            });
        // Enter goes on, unless a shortcut is being recorded or a box is being typed in.
        if setup.controls.capturing.is_none()
            && !context.wants_keyboard_input()
            && context.input(|input| input.key_pressed(egui::Key::Enter))
        {
            if setup.step == STEPS.len() - 1 {
                finish = true;
            } else {
                next = true;
            }
        }
        setup.controls.begin_frame();
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(kit::CANVAS)
                    .inner_margin(egui::Margin::symmetric(24, 0)),
            )
            .show(context, |ui| {
                // Two slow glows drift behind everything.
                let area = ui.max_rect();
                let time = ui.input(|input| input.time) as f32;
                let clip = ui.painter().with_clip_rect(area);
                glow(
                    &clip,
                    egui::pos2(
                        area.center().x + 280.0 * (time * 0.23).sin(),
                        area.top() + 40.0,
                    ),
                    420.0,
                    0.13,
                );
                glow(
                    &clip,
                    egui::pos2(
                        area.center().x - 300.0 * (time * 0.17).cos(),
                        area.top() + 260.0,
                    ),
                    360.0,
                    0.08,
                );
                context.request_repaint_after(std::time::Duration::from_millis(33));
                let width = ui.available_width().min(COLUMN);
                let pad = (ui.available_width() - width) / 2.0;
                egui::ScrollArea::vertical()
                    .id_salt(("first-run-body", setup.step))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.add_space(pad);
                            ui.vertical(|ui| {
                                ui.set_width(width);
                                ui.add_space(24.0);
                                stepper(ui, setup.step, arrived);
                                ui.add_space(10.0 + (1.0 - arrived) * 22.0);
                                ui.scope(|ui| {
                                    ui.set_opacity(arrived);
                                    setup_body(ui, setup, &gpu);
                                });
                            });
                        });
                    });
            });
        setup.controls.end_frame(&setup.draft);
        if back {
            setup.step = setup.step.saturating_sub(1);
            setup.controls.capturing = None;
        } else if next {
            setup.step = (setup.step + 1).min(STEPS.len() - 1);
            setup.controls.capturing = None;
        }
        if finish {
            match setup.finish(self.settings_path.clone()) {
                Ok(settings) => {
                    self.recording = settings;
                    self.setup = None;
                    self.reconcile_clips_folder();
                }
                Err(error) => setup.error = Some(error),
            }
        }
    }
}

/// A heading and a lead sentence.
fn heading(ui: &mut egui::Ui, title: &str, lead: &str) {
    ui.label(
        egui::RichText::new(title)
            .color(kit::TEXT)
            .size(32.0)
            .family(kit::semibold()),
    );
    ui.add_space(4.0);
    ui.label(egui::RichText::new(lead).color(kit::MUTED).size(15.0));
    ui.add_space(24.0);
}

/// The first page: a glowing mark, a big hello, what was found, and three reasons to carry on.
fn welcome(ui: &mut egui::Ui, gpu: &settings::GpuProbe) {
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(150.0, 120.0), egui::Sense::hover());
        glow(ui.painter(), rect.center(), 130.0, 0.55);
        kit::brand_mark(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(92.0, 92.0)),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Welcome to ClipCraft")
                .color(kit::TEXT)
                .size(40.0)
                .family(kit::semibold()),
        );
        ui.add_space(8.0);
        ui.scope(|ui| {
            ui.set_max_width(560.0);
            ui.label(
                egui::RichText::new(
                    "Instant game clips, saved on your own computer. Setup takes about a minute, and everything can be changed later in Settings.",
                )
                .color(kit::MUTED)
                .size(15.0),
            );
        });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            // Centre the row of chips.
            let chips_width = 330.0;
            ui.add_space(((ui.available_width() - chips_width) / 2.0).max(0.0));
            match gpu {
                settings::GpuProbe::Ready(name) => chip(ui, true, &format!("Graphics card ready: {name}")),
                settings::GpuProbe::Unavailable(_) => chip(ui, false, "Using the processor to encode video"),
                _ => chip(ui, false, "Checking your graphics card\u{2026}"),
            }
        });
    });
    ui.add_space(24.0);
    let tile_width = (ui.available_width() - 28.0) / 3.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        for (icon, title, body) in [
            (
                kit::Icon::Record,
                "Always ready",
                "A replay buffer keeps the last moments, so you never have to start recording first.",
            ),
            (
                kit::Icon::Folder,
                "Private",
                "No account, no upload, no internet needed. Clips never leave this computer.",
            ),
            (
                kit::Icon::Scissors,
                "Yours to edit",
                "Play, trim and export clips, or cut them together in the editor.",
            ),
        ] {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(tile_width, 168.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 18.0, kit::CARD);
            ui.painter().rect_stroke(
                rect,
                18.0,
                egui::Stroke::new(1.0_f32, kit::BORDER),
                egui::StrokeKind::Inside,
            );
            let mut inner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect.shrink(18.0))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            let (badge, _) = inner.allocate_exact_size(egui::vec2(42.0, 42.0), egui::Sense::hover());
            inner.painter().circle_filled(badge.center(), 21.0, lime_alpha(30));
            kit::draw_icon(inner.painter(), badge.center(), icon, kit::LIME, 0.9);
            inner.add_space(12.0);
            tile_text(&mut inner, title, body);
        }
    });
}

/// The heading, and the choices for the current step.
fn setup_body(ui: &mut egui::Ui, setup: &mut Setup, gpu: &settings::GpuProbe) {
    use crate::rows::{Meta, Rows};
    // Recording a shortcut: the next key combination pressed is the new one.
    if setup.step == 5 && setup.controls.capturing.is_some() {
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            setup.controls.capturing = None;
        } else if let (Some((action, index)), Some(shortcut)) = (
            setup.controls.capturing,
            ui.input(settings::pressed_shortcut_in),
        ) {
            setup.controls.hotkey_error = setup
                .draft
                .hotkeys
                .set_binding(action, index, &shortcut)
                .err();
            setup.controls.capturing = None;
        }
    }
    match setup.step {
        0 => welcome(ui, gpu),
        1 => {
            heading(
                ui,
                "How should your clips look?",
                "Pick a starting point. You can fine-tune everything later in Settings \u{203a} Video.",
            );
            let width = (ui.available_width() - 28.0) / 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for (index, profile) in settings::PROFILES.iter().enumerate() {
                    let selected = (profile.is_active)(&setup.draft);
                    let clicked =
                        choice_tile(ui, egui::vec2(width, 232.0), selected, profile.name, |ui| {
                            level_bars(ui, index + 1);
                            ui.add_space(14.0);
                            tile_text(ui, profile.name, profile.detail);
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(profile.tagline)
                                    .color(kit::LIME)
                                    .size(13.0)
                                    .family(kit::semibold()),
                            );
                        })
                        .clicked();
                    if clicked {
                        (profile.apply)(&mut setup.draft);
                    }
                }
            });
            ui.add_space(14.0);
            ui.label(
                egui::RichText::new(
                    "Balanced suits most computers. If your game stutters while clipping, choose Performance.",
                )
                .color(kit::MUTED)
                .size(13.0),
            );
        }
        2 => {
            heading(
                ui,
                "What should it capture?",
                "Nothing records until you press Start clipping. This only decides where it looks.",
            );
            let width = (ui.available_width() - 14.0) / 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for (games, title, body) in [
                    (
                        true,
                        "Games",
                        "Waits until a game ClipCraft recognizes is running, then clips just that game. Recommended.",
                    ),
                    (
                        false,
                        "My screen or a window",
                        "Clips whatever is on your chosen screen or window. Pick which one in Settings \u{203a} Capture.",
                    ),
                ] {
                    let selected = setup.draft.game_only_capture == games;
                    let clicked = choice_tile(ui, egui::vec2(width, 230.0), selected, title, |ui| {
                        monitor(ui, games);
                        ui.add_space(14.0);
                        tile_text(ui, title, body);
                    })
                    .clicked();
                    if clicked {
                        setup.draft.game_only_capture = games;
                    }
                }
            });
            ui.add_space(26.0);
            ui.label(
                egui::RichText::new("How far back should a clip reach?")
                    .color(kit::TEXT)
                    .size(17.0)
                    .family(kit::semibold()),
            );
            ui.add_space(8.0);
            timeline(ui, setup.draft.replay_seconds);
            let mut rows = Rows::new("");
            rows.section(ui, "");
            rows.range(
                ui,
                &mut setup.draft,
                Meta::new(
                    "Replay length",
                    "Kept in memory until you save a clip.",
                    "replay seconds",
                ),
                |s| &mut s.replay_seconds,
                5..=120,
                5,
                seconds_text,
            );
            rows.finish(ui);
        }
        3 => {
            heading(
                ui,
                "What should be in your clips?",
                "Choose the sound that goes into every clip. Pick devices and test your microphone later in Settings \u{203a} Audio.",
            );
            let width = (ui.available_width() - 14.0) / 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                for (microphone, title, body) in [
                    (
                        false,
                        "Game and system sound",
                        "Everything you hear: the game, music, and people you talk to.",
                    ),
                    (
                        true,
                        "My microphone",
                        "Your own voice, so your reactions are in the clip too.",
                    ),
                ] {
                    let on = if microphone {
                        setup.draft.capture_microphone
                    } else {
                        setup.draft.capture_game_audio
                    };
                    let clicked = choice_tile(ui, egui::vec2(width, 190.0), on, title, |ui| {
                        let (badge, _) =
                            ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
                        ui.painter().circle_filled(
                            badge.center(),
                            22.0,
                            lime_alpha(if on { 50 } else { 24 }),
                        );
                        kit::draw_icon(
                            ui.painter(),
                            badge.center(),
                            if microphone {
                                kit::Icon::Record
                            } else {
                                kit::Icon::Speaker
                            },
                            if on { kit::LIME } else { kit::MUTED },
                            0.95,
                        );
                        ui.add_space(12.0);
                        tile_text(ui, title, body);
                    })
                    .clicked();
                    if clicked {
                        if microphone {
                            setup.draft.capture_microphone = !setup.draft.capture_microphone;
                        } else {
                            setup.draft.capture_game_audio = !setup.draft.capture_game_audio;
                        }
                    }
                }
            });
            if setup.draft.capture_microphone {
                ui.add_space(20.0);
                let mut rows = Rows::new("");
                rows.section(ui, "Microphone");
                rows.toggle(
                    ui,
                    &mut setup.draft,
                    Meta::new(
                        "Reduce background noise",
                        "Quiets fans, keyboards and rooms in your voice. Works offline.",
                        "noise suppression",
                    ),
                    |s| &mut s.microphone_noise_suppression,
                );
                rows.finish(ui);
            }
        }
        4 => {
            heading(
                ui,
                "Where should clips go?",
                "ClipCraft writes a clip only when you save one. Games get a folder of their own inside.",
            );
            settings::card(ui, "Clips folder", |ui| {
                settings::setting_row(
                    ui,
                    "Save clips in",
                    "The folder is created when you save your first clip. Files already there are kept.",
                    true,
                    |ui| {
                        let folder = if setup.draft.capture_folder.is_empty() {
                            "Choose a folder".to_owned()
                        } else {
                            setup.draft.capture_folder.clone()
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(folder).color(kit::TEXT).size(13.5),
                            )
                            .truncate(),
                        );
                        if pill_button(ui, "Browse\u{2026}", false).clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .set_title("Choose your clips folder")
                                .pick_folder()
                        {
                            setup.draft.capture_folder = path.to_string_lossy().into_owned();
                        }
                    },
                );
            });
            let mut rows = Rows::new("");
            rows.section(ui, "Space");
            rows.range(
                ui,
                &mut setup.draft,
                Meta::new(
                    "Space limit",
                    "The most disk space your clips may use.",
                    "storage limit gb",
                ),
                |s| &mut s.storage_limit_gb,
                0..=500,
                5,
                |v| {
                    if v == 0 {
                        "No limit".to_owned()
                    } else {
                        format!("{v} GB")
                    }
                },
            );
            rows.toggle(
                ui,
                &mut setup.draft,
                Meta::new(
                    "Delete the oldest clips when full",
                    "Keeps you under the limit without stopping. Only clips ClipCraft saved are removed.",
                    "delete oldest storage",
                ),
                |s| &mut s.delete_oldest_when_full,
            );
            rows.finish(ui);
            // What the numbers mean in practice.
            let seconds = u64::from(setup.draft.replay_seconds + setup.draft.post_roll_seconds);
            let clip_bytes = u64::from(setup.draft.video_kbps) * 1000 / 8 * seconds
                + u64::from(setup.draft.audio_kbps) * 1000 / 8 * seconds;
            let folder = std::path::PathBuf::from(&setup.draft.capture_folder);
            if setup.space_for != setup.draft.capture_folder {
                setup.space_for = setup.draft.capture_folder.clone();
                setup.free_bytes = clipcraft_engine::storage_usage(&folder).free_bytes;
            }
            let mut facts = vec![format!(
                "A {} clip is about {} at these settings.",
                seconds_text(seconds as u32),
                size_text(clip_bytes)
            )];
            if setup.draft.storage_limit_gb > 0 && clip_bytes > 0 {
                let limit = u64::from(setup.draft.storage_limit_gb) * 1_000_000_000;
                facts.push(format!(
                    "Your limit holds roughly {} of them.",
                    limit / clip_bytes
                ));
            }
            if let Some(free) = setup.free_bytes {
                facts.push(format!("{} is free on that drive.", size_text(free)));
            }
            ui.add_space(14.0);
            ui.label(
                egui::RichText::new(facts.join(" "))
                    .color(kit::MUTED)
                    .size(13.5),
            );
        }
        5 => {
            heading(
                ui,
                "Choose your shortcuts",
                "These work anywhere, even while a game has the focus. Click Change, then press the keys you want.",
            );
            use clipcraft_engine::HotkeyAction;
            for action in [
                HotkeyAction::SaveClip,
                HotkeyAction::ToggleRecording,
                HotkeyAction::ToggleMicrophone,
            ] {
                let waiting = setup.controls.capturing.is_some_and(|(a, _)| a == action);
                egui::Frame::new()
                    .fill(kit::CARD)
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        if waiting { kit::LIME } else { kit::BORDER },
                    ))
                    .corner_radius(16.0)
                    .inner_margin(egui::Margin::symmetric(20, 16))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width((ui.available_width() - 330.0).max(160.0));
                                ui.label(
                                    egui::RichText::new(action.label())
                                        .color(kit::TEXT)
                                        .size(16.0)
                                        .family(kit::semibold()),
                                );
                                ui.label(
                                    egui::RichText::new(action.detail())
                                        .color(kit::MUTED)
                                        .size(13.0),
                                );
                                if let Some(other) = setup.draft.hotkeys.conflict(action) {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Also used by \u{201c}{}\u{201d}.",
                                            other.label()
                                        ))
                                        .color(kit::DANGER)
                                        .size(12.5),
                                    );
                                }
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let label = if waiting { "Cancel" } else { "Change" };
                                    if pill_button(ui, label, false).clicked() {
                                        setup.controls.capturing =
                                            if waiting { None } else { Some((action, 0)) };
                                        setup.controls.hotkey_error = None;
                                    }
                                    ui.add_space(6.0);
                                    if waiting {
                                        ui.label(
                                            egui::RichText::new("Press the keys\u{2026}")
                                                .color(kit::LIME)
                                                .size(14.0)
                                                .family(kit::semibold()),
                                        );
                                    } else {
                                        let binding = setup
                                            .draft
                                            .hotkeys
                                            .bindings(action)
                                            .next()
                                            .unwrap_or_default()
                                            .to_owned();
                                        let parts: Vec<&str> = binding
                                            .split('+')
                                            .filter(|part| !part.is_empty())
                                            .collect();
                                        if parts.is_empty() {
                                            ui.label(
                                                egui::RichText::new("Not set")
                                                    .color(kit::FAINT)
                                                    .size(14.0),
                                            );
                                        }
                                        for (index, part) in parts.iter().enumerate().rev() {
                                            keycap(ui, part, false);
                                            if index > 0 {
                                                ui.label(
                                                    egui::RichText::new("+")
                                                        .color(kit::FAINT)
                                                        .size(14.0),
                                                );
                                            }
                                        }
                                    }
                                },
                            );
                        });
                    });
                ui.add_space(10.0);
            }
            if let Some(error) = setup.controls.hotkey_error.as_ref() {
                ui.colored_label(kit::DANGER, error);
            }
            ui.label(
                egui::RichText::new(
                    "Push to talk, screenshots and extra shortcuts are in Settings \u{203a} Hotkeys.",
                )
                .color(kit::MUTED)
                .size(13.0),
            );
        }
        _ => {
            ui.vertical_centered(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(150.0, 120.0), egui::Sense::hover());
                glow(ui.painter(), rect.center(), 130.0, 0.55);
                ui.painter().circle_filled(rect.center(), 36.0, kit::LIME);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "\u{2713}",
                    egui::FontId::new(38.0, kit::semibold()),
                    Color::BLACK,
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("You\u{2019}re all set")
                        .color(kit::TEXT)
                        .size(38.0)
                        .family(kit::semibold()),
                );
                ui.add_space(6.0);
                let save = setup
                    .draft
                    .hotkeys
                    .bindings(clipcraft_engine::HotkeyAction::SaveClip)
                    .next()
                    .unwrap_or("the save shortcut")
                    .replace('+', " + ");
                ui.label(
                    egui::RichText::new(format!(
                        "Start clipping, then press {save} any time to keep the last {}.",
                        seconds_text(setup.draft.replay_seconds)
                    ))
                    .color(kit::MUTED)
                    .size(15.0),
                );
            });
            ui.add_space(22.0);
            let profile = settings::PROFILES
                .iter()
                .find(|profile| (profile.is_active)(&setup.draft))
                .map_or("Custom", |profile| profile.name);
            let sound = match (
                setup.draft.capture_game_audio,
                setup.draft.capture_microphone,
            ) {
                (true, true) => "Game sound and microphone",
                (true, false) => "Game sound",
                (false, true) => "Microphone",
                (false, false) => "No sound",
            };
            settings::card(ui, "Your setup", |ui| {
                settings::info_row(ui, "Quality", profile, false);
                settings::info_row(
                    ui,
                    "Captures",
                    if setup.draft.game_only_capture {
                        "Games"
                    } else {
                        "Your screen or a window"
                    },
                    false,
                );
                settings::info_row(
                    ui,
                    "Replay",
                    &seconds_text(setup.draft.replay_seconds),
                    false,
                );
                settings::info_row(ui, "Sound", sound, false);
                settings::info_row(
                    ui,
                    "Clips go to",
                    &if setup.draft.capture_folder.is_empty() {
                        "The default folder".to_owned()
                    } else {
                        setup.draft.capture_folder.clone()
                    },
                    true,
                );
            });
        }
    }
    if let Some(error) = setup.error.as_ref() {
        ui.add_space(12.0);
        ui.colored_label(kit::DANGER, error);
    }
    ui.add_space(32.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_preferences_skip_setup_and_fresh_preferences_need_it() {
        let old: RecordingSettings = serde_json::from_str(r#"{"microphone_percent":42}"#).unwrap();
        assert!(old.setup_completed);
        assert_eq!(old.microphone_percent, 42);
        assert!(!RecordingSettings::default().setup_completed);
    }

    #[test]
    fn finishing_persists_choices_and_completion_in_one_command() {
        let path = std::env::temp_dir().join(format!(
            "cut-first-run-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut setup = Setup::new(RecordingSettings::default());
        setup.draft.microphone_percent = 42;
        setup
            .draft
            .hotkeys
            .set_binding(clipcraft_engine::HotkeyAction::SaveClip, 1, "Ctrl+F1")
            .unwrap();
        assert!(!path.exists());
        assert!(setup.finish(Some(path.clone())).unwrap().setup_completed);
        let CommandOutput::RecordingSettings(saved) =
            execute(Command::LoadRecordingSettings { path: path.clone() }).unwrap()
        else {
            panic!("settings result")
        };
        assert!(saved.setup_completed);
        assert_eq!(saved.microphone_percent, 42);
        assert_eq!(
            saved
                .hotkeys
                .bindings(clipcraft_engine::HotkeyAction::SaveClip)
                .count(),
            2
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn setup_saves_only_when_finished_and_reports_failure() {
        let mut setup = Setup::new(RecordingSettings::default());
        setup.draft.capture_folder = "relative".into();
        assert!(
            setup
                .finish(Some(std::env::temp_dir().join("never-written-setup.json")))
                .is_err()
        );
        assert!(!setup.draft.setup_completed);
        assert!(setup.finish(None).is_err());
    }
}
