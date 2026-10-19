use clipcraft_engine::{Command, CommandOutput, LibraryScanResult, MediaEntry, execute};
use eframe::egui;
use egui::{Align, Color32, FontId, Layout, RichText, Stroke, Vec2};
use std::path::PathBuf;

const CANVAS: Color32 = Color32::from_rgb(14, 14, 15);
const PANEL: Color32 = Color32::from_rgb(24, 24, 26);
const CARD: Color32 = Color32::from_rgb(29, 29, 31);
const MUTED: Color32 = Color32::from_rgb(157, 157, 164);
const TEXT: Color32 = Color32::from_rgb(242, 242, 244);
const LIME: Color32 = Color32::from_rgb(190, 255, 43);

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ClipCraft")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ClipCraft",
        options,
        Box::new(|_| Ok(Box::<ClipCraftApp>::default())),
    )
}

#[derive(Default)]
struct ClipCraftApp {
    directory_text: String,
    library: Option<LibraryScanResult>,
    selected: Option<MediaEntry>,
    error: Option<String>,
    search_text: String,
    sort_newest: bool,
    page: Page,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Page {
    #[default]
    Library,
    Settings,
}

impl eframe::App for ClipCraftApp {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        apply_theme(context);
        self.top_bar(context);
        self.navigation(context);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(CANVAS)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(context, |ui| {
                if let Some(selected) = self.selected.clone() {
                    self.clip_details(ui, &selected);
                } else if self.page == Page::Settings {
                    self.settings(ui);
                } else {
                    self.library_view(ui);
                }
            });
    }
}

impl ClipCraftApp {
    fn top_bar(&self, context: &egui::Context) {
        egui::TopBottomPanel::top("capture-strip")
            .exact_height(48.0)
            .frame(egui::Frame::new().fill(Color32::from_rgb(19, 19, 20)))
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(16.0);
                    ui.label(RichText::new("◈").color(LIME).size(25.0).strong());
                    ui.label(RichText::new("CLIPCRAFT").color(TEXT).strong());
                    ui.add_space(16.0);
                    ui.separator();
                    ui.label(RichText::new("Capture unavailable").color(MUTED));
                    ui.add_space(4.0);
                    ui.label(RichText::new("F8").color(TEXT).strong());
                    ui.label(RichText::new("Clip 2m").color(TEXT));
                    ui.label(RichText::new("Off").color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(16.0);
                        ui.label(RichText::new("1440p  ·  60 FPS").color(MUTED));
                        ui.label(
                            RichText::new("Capture provider not implemented")
                                .color(MUTED)
                                .small(),
                        );
                    });
                });
            });
    }

    fn navigation(&mut self, context: &egui::Context) {
        egui::SidePanel::left("navigation")
            .exact_width(68.0)
            .frame(egui::Frame::new().fill(Color32::from_rgb(19, 19, 20)))
            .show(context, |ui| {
                ui.add_space(16.0);
                nav_button(ui, "⌂", "Library", self.page == Page::Library, || {
                    self.page = Page::Library;
                    self.selected = None;
                });
                nav_button(ui, "⚙", "Settings", self.page == Page::Settings, || {
                    self.page = Page::Settings;
                    self.selected = None;
                });
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    ui.label(RichText::new("LOCAL").color(MUTED).size(9.0));
                    ui.add_space(12.0);
                });
            });
    }

    fn library_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Clips").color(TEXT).size(23.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("Stored on this device").color(MUTED).small());
            });
        });
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("Folder").color(MUTED));
            let path_field = ui.add_sized(
                [(ui.available_width() - 190.0).max(180.0), 34.0],
                egui::TextEdit::singleline(&mut self.directory_text)
                    .hint_text("Choose a local folder containing video clips"),
            );
            if path_field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                self.scan_library();
            }
            if ui
                .add_sized([82.0, 34.0], egui::Button::new("Browse…"))
                .clicked()
            {
                self.choose_library_folder();
            }
            if ui
                .add_sized(
                    [88.0, 34.0],
                    egui::Button::new(RichText::new("Scan").strong()).fill(LIME),
                )
                .clicked()
            {
                self.scan_library();
            }
        });

        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("All Clips").color(TEXT).strong());
            let sort_label = if self.sort_newest {
                "Sort: Newest"
            } else {
                "Sort: Name"
            };
            if ui.small_button(sort_label).clicked() {
                self.sort_newest = !self.sort_newest;
            }
            ui.add_space(8.0);
            ui.add_sized(
                [240.0, 28.0],
                egui::TextEdit::singleline(&mut self.search_text).hint_text("Search clips"),
            );
            let (visible_count, total_count) = self.library.as_ref().map_or((0, 0), |result| {
                (
                    visible_entries(&result.entries, &self.search_text, self.sort_newest).len(),
                    result.entries.len(),
                )
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let count_label = if self.search_text.trim().is_empty() {
                    format!("{total_count} clips")
                } else {
                    format!("{visible_count} of {total_count} clips")
                };
                ui.label(RichText::new(count_label).color(MUTED).small());
            });
        });
        ui.add_space(14.0);

        if let Some(error) = self.error.as_deref() {
            ui.label(RichText::new(error).color(Color32::from_rgb(255, 110, 110)));
            ui.add_space(8.0);
        }

        let mut card_action = None;
        match self.library.as_ref() {
            Some(result) if result.entries.is_empty() => self.empty_state(ui),
            Some(result) => {
                let entries = visible_entries(&result.entries, &self.search_text, self.sort_newest);
                if entries.is_empty() {
                    self.no_search_results(ui);
                } else {
                    card_action = Self::clip_grid(ui, &entries);
                }
            }
            None => self.first_scan_state(ui),
        }
        if let Some(action) = card_action {
            match action {
                ClipCardAction::Open(entry) => self.selected = Some(entry),
                ClipCardAction::CopyPath(path) => {
                    self.copy_media_path(ui.ctx(), PathBuf::from(path));
                }
            }
        }
    }

    fn first_scan_state(&self, ui: &mut egui::Ui) {
        ui.add_space(70.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("▣").color(MUTED).size(42.0));
            ui.add_space(8.0);
            ui.label(
                RichText::new("Your clips, on this device")
                    .color(TEXT)
                    .size(19.0)
                    .strong(),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("Enter a local folder above to find and organize video files.")
                    .color(MUTED),
            );
            ui.label(
                RichText::new("ClipCraft never uploads your recordings.")
                    .color(MUTED)
                    .small(),
            );
        });
    }

    fn empty_state(&self, ui: &mut egui::Ui) {
        ui.add_space(70.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("No clips found in this folder")
                    .color(TEXT)
                    .size(18.0)
                    .strong(),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Choose another folder or save a recording after capture support is available.",
                )
                .color(MUTED),
            );
        });
    }

    fn no_search_results(&self, ui: &mut egui::Ui) {
        ui.add_space(56.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("No clips match this search")
                    .color(TEXT)
                    .size(18.0)
                    .strong(),
            );
            ui.add_space(6.0);
            ui.label(RichText::new("Try another title or clear the search field.").color(MUTED));
        });
    }

    fn clip_grid(ui: &mut egui::Ui, entries: &[&MediaEntry]) -> Option<ClipCardAction> {
        let available_width = ui.available_width();
        let columns: usize = if available_width < 560.0 {
            1
        } else if available_width < 850.0 {
            2
        } else if available_width < 1_140.0 {
            3
        } else if available_width < 1_430.0 {
            4
        } else if available_width < 1_720.0 {
            5
        } else {
            6
        };
        let card_width = ((available_width - (columns.saturating_sub(1) as f32 * 14.0))
            / columns as f32)
            .max(180.0);
        let mut action = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.columns(columns, |column_uis| {
                for (index, entry) in entries.iter().enumerate() {
                    let Some(column) = column_uis.get_mut(index % columns) else {
                        continue;
                    };
                    if let Some(clicked) = clip_card(column, entry, card_width) {
                        action = Some(clicked);
                    }
                    column.add_space(14.0);
                }
            });
        });

        action
    }

    fn clip_details(&mut self, ui: &mut egui::Ui, entry: &MediaEntry) {
        if ui.button("←  Back to clips").clicked() {
            self.selected = None;
            return;
        }
        ui.add_space(12.0);
        ui.columns(2, |columns| {
            columns[0].vertical_centered(|ui| {
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 420.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Local video preview is not implemented yet",
                    FontId::proportional(16.0),
                    MUTED,
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("▶").color(TEXT));
                    let mut position = 0.0_f32;
                    let _ = ui.add_enabled(
                        false,
                        egui::Slider::new(&mut position, 0.0_f32..=1.0_f32).show_value(false),
                    );
                    ui.label(RichText::new("--:--").color(MUTED));
                });
                ui.add_space(10.0);
                let _ = ui.add_enabled(false, egui::Button::new("Open in basic editor"));
            });
            columns[1].vertical(|ui| {
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::same(18))
                    .show(ui, |ui| {
                        ui.heading(RichText::new(&entry.title).color(TEXT).strong());
                        ui.add_space(16.0);
                        detail_row(ui, "Game / source", "Local video");
                        detail_row(ui, "File size", &format_bytes(entry.size_bytes));
                        detail_row(ui, "Format", &entry.extension.to_uppercase());
                        detail_row(ui, "Modified", &format_date(entry.modified_unix_seconds));
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
                        ui.label(RichText::new("File location").color(MUTED).small());
                        ui.add_space(4.0);
                        ui.label(RichText::new(&entry.path).color(TEXT).small());
                        ui.add_space(16.0);
                        if ui.button("Copy File Path").clicked() {
                            self.copy_media_path(ui.ctx(), PathBuf::from(&entry.path));
                        }
                    });
            });
        });
    }

    fn settings(&self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Settings").color(TEXT).size(23.0).strong());
        ui.add_space(18.0);
        setting_panel(ui, "PC Storage", "Local video folders and storage controls");
        setting_panel(
            ui,
            "Recording",
            "Capture support is not implemented yet. No recording is running.",
        );
        setting_panel(
            ui,
            "Audio",
            "Game audio and microphone capture will be configured here.",
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(
                "All clips remain on this device. There is no cloud sync or hosted service.",
            )
            .color(MUTED),
        );
    }

    fn scan_library(&mut self) {
        self.error = None;
        let directory = PathBuf::from(self.directory_text.trim());
        match execute(Command::ScanLibrary { directory }) {
            Ok(CommandOutput::LibraryScan(result)) => {
                self.directory_text = result.directory.clone();
                self.library = Some(result);
            }
            Ok(CommandOutput::LocalMediaPath { .. }) => {
                self.error = Some("The library scan returned an unexpected result.".to_owned());
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn copy_media_path(&mut self, context: &egui::Context, path: PathBuf) {
        match execute(Command::LocalMediaPath { path }) {
            Ok(CommandOutput::LocalMediaPath { path }) => context.copy_text(path),
            Ok(CommandOutput::LibraryScan(_)) => {
                self.error = Some("The path command returned an unexpected result.".to_owned());
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn choose_library_folder(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose your clips folder");
        let current_directory = self.directory_text.trim();
        if !current_directory.is_empty() && PathBuf::from(current_directory).is_dir() {
            dialog = dialog.set_directory(current_directory);
        }
        self.set_library_folder(dialog.pick_folder());
    }

    fn set_library_folder(&mut self, directory: Option<PathBuf>) {
        let Some(directory) = directory else {
            return;
        };
        self.directory_text = directory.to_string_lossy().into_owned();
        self.scan_library();
    }
}

enum ClipCardAction {
    Open(MediaEntry),
    CopyPath(String),
}

fn apply_theme(context: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = CANVAS;
    visuals.window_fill = PANEL;
    visuals.faint_bg_color = CARD;
    visuals.extreme_bg_color = Color32::from_rgb(12, 12, 13);
    visuals.selection.bg_fill = Color32::from_rgb(72, 102, 23);
    visuals.selection.stroke = Stroke::new(1.0_f32, LIME);
    visuals.widgets.noninteractive.fg_stroke.color = TEXT;
    visuals.widgets.inactive.bg_fill = CARD;
    visuals.widgets.inactive.fg_stroke.color = TEXT;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(47, 47, 49);
    visuals.widgets.active.bg_fill = Color32::from_rgb(58, 73, 34);
    context.set_visuals(visuals);
}

fn nav_button(ui: &mut egui::Ui, icon: &str, label: &str, selected: bool, action: impl FnOnce()) {
    let color = if selected { LIME } else { MUTED };
    let response = ui.add_sized(
        [60.0, 58.0],
        egui::Button::new(
            RichText::new(format!("{icon}\n{label}"))
                .color(color)
                .size(12.0),
        )
        .fill(if selected {
            Color32::from_rgb(37, 40, 32)
        } else {
            Color32::TRANSPARENT
        }),
    );
    if response.clicked() {
        action();
    }
    ui.add_space(6.0);
}

fn clip_card(ui: &mut egui::Ui, entry: &MediaEntry, width: f32) -> Option<ClipCardAction> {
    let mut action = None;
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(38, 38, 40)))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(0))
        .show(ui, |ui| {
            ui.set_width(width);
            let (thumbnail, response) = ui.allocate_exact_size(
                Vec2::new(width, (width * 0.5625).clamp(120.0, 220.0)),
                egui::Sense::click(),
            );
            paint_video_placeholder(ui, thumbnail, entry);
            if response.clicked() {
                action = Some(ClipCardAction::Open(entry.clone()));
            }

            ui.add_space(9.0);
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.add(
                        egui::Label::new(
                            RichText::new(&entry.title).color(TEXT).strong().size(14.0),
                        )
                        .truncate()
                        .halign(Align::Min),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(format!(
                            "On Device  ·  {}  ·  {}",
                            format_bytes(entry.size_bytes),
                            format_date(entry.modified_unix_seconds)
                        ))
                        .color(MUTED)
                        .size(11.0),
                    );
                });
            });
            ui.add_space(10.0);
            ui.separator();
            let (action_rect, action_response) =
                ui.allocate_exact_size(Vec2::new(width, 38.0), egui::Sense::click());
            ui.painter()
                .rect_filled(action_rect, 0.0, Color32::from_rgb(31, 31, 33));
            ui.painter().text(
                action_rect.left_center() + Vec2::new(11.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "⧉  Copy File Path",
                FontId::proportional(13.0),
                TEXT,
            );
            if action_response.clicked() {
                action = Some(ClipCardAction::CopyPath(entry.path.clone()));
            }
        });
    action
}

fn paint_video_placeholder(ui: &egui::Ui, rect: egui::Rect, entry: &MediaEntry) {
    let palette = [
        (
            Color32::from_rgb(29, 46, 42),
            Color32::from_rgb(108, 166, 103),
        ),
        (
            Color32::from_rgb(47, 38, 34),
            Color32::from_rgb(190, 125, 70),
        ),
        (
            Color32::from_rgb(34, 40, 55),
            Color32::from_rgb(104, 140, 204),
        ),
        (
            Color32::from_rgb(44, 35, 52),
            Color32::from_rgb(170, 115, 196),
        ),
    ];
    let seed = entry.title.bytes().fold(0_u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    let (background, accent) = palette[(seed as usize) % palette.len()];
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, background);

    let horizon = rect.center().y + rect.height() * 0.18;
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(rect.left(), horizon),
            egui::pos2(rect.left() + rect.width() * 0.24, rect.top()),
            egui::pos2(rect.left() + rect.width() * 0.68, rect.top()),
            egui::pos2(rect.right(), horizon),
            egui::pos2(rect.right(), rect.bottom()),
            egui::pos2(rect.left(), rect.bottom()),
        ],
        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 36),
        Stroke::NONE,
    ));
    painter.circle_filled(
        egui::pos2(
            rect.right() - rect.width() * 0.17,
            rect.top() + rect.height() * 0.27,
        ),
        rect.height() * 0.1,
        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 100),
    );
    painter.line_segment(
        [
            egui::pos2(rect.left() + 12.0, horizon),
            egui::pos2(rect.right() - 12.0, horizon),
        ],
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 36)),
    );

    let play_center = rect.center();
    painter.circle_filled(
        play_center,
        23.0,
        Color32::from_rgba_unmultiplied(0, 0, 0, 150),
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            play_center + Vec2::new(-5.0, -8.0),
            play_center + Vec2::new(8.0, 0.0),
            play_center + Vec2::new(-5.0, 8.0),
        ],
        TEXT,
        Stroke::NONE,
    ));

    let badge = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(10.0, 10.0),
        Vec2::new(54.0, 22.0),
    );
    painter.rect_filled(
        badge,
        egui::CornerRadius::same(5),
        Color32::from_rgba_unmultiplied(0, 0, 0, 165),
    );
    painter.text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        entry.extension.to_uppercase(),
        FontId::proportional(10.0),
        TEXT,
    );
    let duration_badge = egui::Rect::from_min_size(
        rect.right_bottom() - Vec2::new(53.0, 30.0),
        Vec2::new(43.0, 22.0),
    );
    painter.rect_filled(
        duration_badge,
        egui::CornerRadius::same(5),
        Color32::from_rgba_unmultiplied(0, 0, 0, 180),
    );
    painter.text(
        duration_badge.center(),
        egui::Align2::CENTER_CENTER,
        "--:--",
        FontId::proportional(11.0),
        TEXT,
    );
}

fn visible_entries<'a>(
    entries: &'a [MediaEntry],
    query: &str,
    sort_newest: bool,
) -> Vec<&'a MediaEntry> {
    let query = query.trim().to_lowercase();
    let mut visible: Vec<_> = entries
        .iter()
        .filter(|entry| query.is_empty() || entry.title.to_lowercase().contains(&query))
        .collect();
    if sort_newest {
        visible.sort_by(|left, right| {
            right
                .modified_unix_seconds
                .cmp(&left.modified_unix_seconds)
                .then_with(|| compare_media_names(left, right))
        });
    } else {
        visible.sort_by(|left, right| compare_media_names(left, right));
    }
    visible
}

fn compare_media_names(left: &MediaEntry, right: &MediaEntry) -> std::cmp::Ordering {
    left.title
        .to_lowercase()
        .cmp(&right.title.to_lowercase())
        .then_with(|| left.path.cmp(&right.path))
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(value).color(TEXT));
        });
    });
    ui.add_space(10.0);
}

fn setting_panel(ui: &mut egui::Ui, title: &str, description: &str) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(39, 39, 41)))
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).color(TEXT).strong());
            ui.add_space(7.0);
            ui.label(RichText::new(description).color(MUTED));
        });
    ui.add_space(12.0);
}

fn format_bytes(size: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut scaled = size as f64;
    let mut unit = 0usize;
    while scaled >= 1024.0 && unit < UNITS.len() - 1 {
        scaled /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size} B")
    } else {
        format!("{scaled:.1} {}", UNITS[unit])
    }
}

fn format_date(seconds: Option<u64>) -> String {
    seconds.map_or_else(
        || "Unknown date".to_owned(),
        |value| {
            let Ok(days) = i64::try_from(value / 86_400) else {
                return "Unknown date".to_owned();
            };
            let (year, month, day) = civil_from_days(days);
            format!("{year:04}-{month:02}-{day:02}")
        },
    )
}

fn civil_from_days(unix_days: i64) -> (i64, u32, u32) {
    let shifted_days = unix_days + 719_468;
    let era = if shifted_days >= 0 {
        shifted_days
    } else {
        shifted_days - 146_096
    } / 146_097;
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::{ClipCraftApp, format_date, visible_entries};
    use clipcraft_engine::MediaEntry;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn media_entry(title: &str, modified_unix_seconds: Option<u64>) -> MediaEntry {
        MediaEntry {
            path: format!("/clips/{title}"),
            title: title.to_owned(),
            size_bytes: 100,
            modified_unix_seconds,
            extension: "mp4".to_owned(),
        }
    }

    fn temporary_directory() -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "clipcraft-ui-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("create temporary test directory");
        directory
    }

    #[test]
    fn formats_local_file_dates_as_calendar_dates() {
        assert_eq!(format_date(Some(0)), "1970-01-01");
        assert_eq!(format_date(Some(946_684_800)), "2000-01-01");
        assert_eq!(format_date(None), "Unknown date");
    }

    #[test]
    fn search_matches_clip_titles_without_case_sensitivity() {
        let entries = vec![
            media_entry("Valorant - clutch.mp4", Some(20)),
            media_entry("Apex - final circle.mp4", Some(30)),
        ];

        let visible = visible_entries(&entries, "VALORANT", false);

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].title, "Valorant - clutch.mp4");
    }

    #[test]
    fn sort_newest_puts_recent_clips_first_and_stabilizes_ties_by_name() {
        let entries = vec![
            media_entry("second.mp4", Some(20)),
            media_entry("first.mp4", Some(20)),
            media_entry("older.mp4", Some(10)),
        ];

        let visible = visible_entries(&entries, "", true);

        assert_eq!(
            visible
                .iter()
                .map(|entry| entry.title.as_str())
                .collect::<Vec<_>>(),
            ["first.mp4", "second.mp4", "older.mp4"]
        );
    }

    #[test]
    fn sort_by_name_ignores_case_and_uses_path_to_break_ties() {
        let entries = vec![
            media_entry("zeta.mp4", None),
            media_entry("Alpha.mp4", None),
            media_entry("alpha.mp4", None),
        ];

        let visible = visible_entries(&entries, "", false);

        assert_eq!(
            visible
                .iter()
                .map(|entry| entry.title.as_str())
                .collect::<Vec<_>>(),
            ["Alpha.mp4", "alpha.mp4", "zeta.mp4"]
        );
    }

    #[test]
    fn choosing_a_folder_scans_it_and_cancel_preserves_the_current_selection() {
        let directory = temporary_directory();
        fs::write(directory.join("local-clip.mp4"), b"fixture").expect("write clip fixture");

        let mut app = ClipCraftApp::default();
        app.set_library_folder(Some(directory.clone()));
        assert_eq!(app.directory_text, directory.to_string_lossy());
        assert_eq!(
            app.library.as_ref().map(|result| result.entries.len()),
            Some(1)
        );
        assert_eq!(app.error, None);

        app.error = Some("prior message".to_owned());
        app.set_library_folder(None);
        assert_eq!(app.directory_text, directory.to_string_lossy());
        assert_eq!(
            app.library.as_ref().map(|result| result.entries.len()),
            Some(1)
        );
        assert_eq!(app.error.as_deref(), Some("prior message"));

        fs::remove_dir_all(directory).expect("remove temporary test directory");
    }
}
