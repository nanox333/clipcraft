use clipcraft_engine::{
    Command, CommandOutput, EngineError, LibraryScanResult, MediaEntry, MediaProbe,
    default_index_path, default_thumbnail_dir, execute,
};
mod playback;
mod thumbs;

use eframe::egui;
use egui::{Align, Color32, FontId, Layout, RichText, Stroke, Vec2};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;

const CANVAS: Color32 = Color32::from_rgb(18, 18, 19);
const SIDEBAR: Color32 = Color32::from_rgb(24, 24, 26);
const PANEL: Color32 = Color32::from_rgb(26, 26, 29);
const CARD: Color32 = Color32::from_rgb(33, 33, 36);
const BORDER: Color32 = Color32::from_rgb(36, 36, 40);
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
        Box::new(|_| Ok(Box::new(ClipCraftApp::restored()))),
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
    /// Where the rebuildable library index lives; `None` keeps scans in memory only.
    index_path: Option<PathBuf>,
    /// Background probe worker for the current folder, if one is running.
    probe_receiver: Option<mpsc::Receiver<ProbeMessage>>,
    /// Incremented per worker so results from a replaced folder are ignored.
    probe_generation: u64,
    needs_probe: bool,
    /// Where thumbnail JPEGs are cached; `None` disables thumbnails.
    thumbnail_dir: Option<PathBuf>,
    /// Background thumbnail worker for the current folder, if one is running.
    thumbnail_receiver: Option<mpsc::Receiver<ThumbnailMessage>>,
    needs_thumbnails: bool,
    /// Paths already tried this session, so unsupported clips are not retried.
    thumbnail_attempted: HashSet<String>,
    thumbs: thumbs::Thumbnails,
    /// The clip open in the detail view, if any.
    playback: Option<playback::Playback>,
    /// Probe results from this session not yet saved to the index.
    unsaved_probes: Vec<(String, MediaProbe)>,
    /// Paths already probed or tried this session, so unsupported files are not retried.
    probe_attempted: HashSet<String>,
}

/// A thumbnail the worker finished: the clip and its cached JPEG.
struct ThumbnailMessage {
    path: String,
    file: PathBuf,
}

enum ProbeMessage {
    Result {
        generation: u64,
        path: String,
        probe: Result<MediaProbe, EngineError>,
    },
    Done {
        generation: u64,
    },
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
        if self.needs_probe {
            self.needs_probe = false;
            self.start_probing(context);
        }
        self.poll_probes();
        if self.needs_thumbnails {
            self.needs_thumbnails = false;
            self.start_thumbnails(context);
        }
        self.poll_thumbnails();
        self.thumbs.begin_frame();
        self.sync_playback(context);
        self.top_bar(context);
        self.navigation(context);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(CANVAS)
                    .corner_radius(egui::CornerRadius {
                        nw: 10,
                        ne: 0,
                        sw: 0,
                        se: 0,
                    })
                    .inner_margin(egui::Margin::same(16)),
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
        self.thumbs.end_frame();
    }
}

impl ClipCraftApp {
    /// Starts with the last scanned folder restored from the local index, if there is one.
    fn restored() -> Self {
        Self::restored_from(default_index_path())
    }

    fn restored_from(index_path: Option<PathBuf>) -> Self {
        let mut app = Self {
            index_path,
            thumbnail_dir: default_thumbnail_dir(),
            ..Self::default()
        };
        let Some(index_path) = app.index_path.clone() else {
            return app;
        };
        match execute(Command::LoadLibrary { index_path }) {
            Ok(CommandOutput::LibraryScan(result)) => {
                app.directory_text = result.directory.clone();
                app.library = Some(result);
                app.refresh_restored_library();
            }
            Ok(_) | Err(EngineError::IndexMissing) => {}
            Err(error) => app.error = Some(error.to_string()),
        }
        app
    }

    /// Generates missing thumbnails on a worker thread. A new worker replaces the old one, which
    /// notices its closed channel and stops.
    fn start_thumbnails(&mut self, context: &egui::Context) {
        let (Some(cache_dir), Some(library)) = (self.thumbnail_dir.clone(), self.library.as_ref())
        else {
            return;
        };
        let paths: Vec<String> = library
            .entries
            .iter()
            .filter(|entry| !entry.offline && !self.thumbnail_attempted.contains(&entry.path))
            .map(|entry| entry.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        self.thumbnail_attempted.extend(paths.iter().cloned());
        let (sender, receiver) = mpsc::channel();
        self.thumbnail_receiver = Some(receiver);
        let context = context.clone();
        std::thread::spawn(move || {
            thumbnail_paths(paths, &cache_dir, &sender, || context.request_repaint());
        });
    }

    fn poll_thumbnails(&mut self) {
        let Some(receiver) = self.thumbnail_receiver.as_ref() else {
            return;
        };
        let mut disconnected = false;
        loop {
            match receiver.try_recv() {
                Ok(message) => self.thumbs.set_file(message.path, message.file),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected {
            self.thumbnail_receiver = None;
        }
    }

    /// Keeps exactly one player alive for the clip open in the detail view.
    fn sync_playback(&mut self, context: &egui::Context) {
        match self.selected.as_ref().map(|entry| entry.path.as_str()) {
            Some(path)
                if self
                    .playback
                    .as_ref()
                    .is_some_and(|open| open.path() == path) => {}
            Some(path) => self.playback = Some(playback::Playback::start(path, context)),
            None => self.playback = None,
        }
        if let Some(open) = self.playback.as_mut() {
            open.poll(context);
        }
    }

    /// Probes clips that have no saved facts on a worker thread so the window stays responsive.
    fn start_probing(&mut self, context: &egui::Context) {
        let Some(library) = self.library.as_ref() else {
            return;
        };
        let paths: Vec<String> = library
            .entries
            .iter()
            .filter(|entry| {
                entry.probe.is_none()
                    && !entry.offline
                    && !self.probe_attempted.contains(&entry.path)
            })
            .map(|entry| entry.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        self.probe_attempted.extend(paths.iter().cloned());
        self.probe_generation = self.probe_generation.wrapping_add(1);
        let generation = self.probe_generation;
        let (sender, receiver) = mpsc::channel();
        self.probe_receiver = Some(receiver);
        let context = context.clone();
        std::thread::spawn(move || {
            probe_paths(paths, generation, &sender, || context.request_repaint());
        });
    }

    /// Applies finished probes and saves them to the index once the worker is done.
    fn poll_probes(&mut self) {
        let Some(receiver) = self.probe_receiver.as_ref() else {
            return;
        };
        let mut messages = Vec::new();
        let mut disconnected = false;
        loop {
            match receiver.try_recv() {
                Ok(message) => messages.push(message),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        for message in messages {
            self.handle_probe_message(message);
        }
        if disconnected {
            self.probe_receiver = None;
        }
    }

    fn handle_probe_message(&mut self, message: ProbeMessage) {
        match message {
            ProbeMessage::Result {
                generation,
                path,
                probe,
            } if generation == self.probe_generation => {
                let Ok(probe) = probe else {
                    return;
                };
                if let Some(entry) = self
                    .library
                    .as_mut()
                    .and_then(|library| library.entries.iter_mut().find(|entry| entry.path == path))
                {
                    entry.probe = Some(probe.clone());
                    self.unsaved_probes.push((path, probe));
                }
            }
            ProbeMessage::Done { generation } if generation == self.probe_generation => {
                self.save_probes();
            }
            ProbeMessage::Result { .. } | ProbeMessage::Done { .. } => {}
        }
    }

    fn save_probes(&mut self) {
        let (Some(index_path), Some(library)) = (self.index_path.clone(), self.library.as_ref())
        else {
            self.unsaved_probes.clear();
            return;
        };
        if self.unsaved_probes.is_empty() {
            return;
        }
        // Probe facts are a cache: failing to save them only means probing again next launch.
        let _ = execute(Command::StoreProbes {
            index_path,
            directory: library.directory.clone(),
            probes: std::mem::take(&mut self.unsaved_probes),
        });
    }

    /// Rescans the restored folder so clips deleted since last launch show as offline. If the
    /// folder cannot be read (for example an unplugged drive), every remembered clip is offline.
    fn refresh_restored_library(&mut self) {
        self.scan_library();
        if self.error.is_none() {
            return;
        }
        if let Some(library) = self.library.as_mut() {
            for entry in &mut library.entries {
                entry.offline = true;
            }
        }
    }

    fn top_bar(&mut self, context: &egui::Context) {
        egui::TopBottomPanel::top("capture-strip")
            .exact_height(44.0)
            .frame(egui::Frame::new().fill(SIDEBAR))
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(16.0);
                    let (logo, _) = ui.allocate_exact_size(Vec2::splat(24.0), egui::Sense::hover());
                    ui.painter().add(egui::Shape::convex_polygon(
                        vec![
                            logo.center_top(),
                            logo.right_center(),
                            logo.center_bottom(),
                            logo.left_center(),
                        ],
                        LIME,
                        Stroke::NONE,
                    ));
                    ui.add_space(14.0);
                    ui.label(RichText::new("<").color(MUTED).size(18.0));
                    ui.label(
                        RichText::new(">")
                            .color(Color32::from_rgb(90, 90, 96))
                            .size(18.0),
                    );
                    ui.add_space(14.0);
                    ui.label(RichText::new("🎮").color(MUTED));
                    ui.label(RichText::new("Capture unavailable").color(MUTED));
                    ui.add_space(8.0);
                    key_chip(ui, "F8");
                    ui.label(RichText::new("Clip 2m").color(TEXT));
                    ui.add_space(8.0);
                    key_chip(ui, "Off");
                    ui.label(RichText::new("Long Recording").color(TEXT));
                    ui.add_space(8.0);
                    ui.label(RichText::new("⚙").color(MUTED));
                    ui.label(RichText::new("1440p 60 FPS").color(TEXT));
                });
            });
    }

    fn navigation(&mut self, context: &egui::Context) {
        egui::SidePanel::left("navigation")
            .exact_width(60.0)
            .frame(egui::Frame::new().fill(SIDEBAR))
            .show(context, |ui| {
                ui.add_space(24.0);
                nav_button(ui, "▶", "Clips", self.page == Page::Library, || {
                    self.page = Page::Library;
                    self.selected = None;
                });
                nav_button(ui, "⚙", "Settings", self.page == Page::Settings, || {
                    self.page = Page::Settings;
                    self.selected = None;
                });
            });
    }

    fn library_view(&mut self, ui: &mut egui::Ui) {
        let (visible_count, total_count) = self.library.as_ref().map_or((0, 0), |result| {
            (
                visible_entries(&result.entries, &self.search_text, self.sort_newest).len(),
                result.entries.len(),
            )
        });
        ui.horizontal(|ui| {
            if toolbar_button(ui, "+  Import").clicked() {
                self.choose_library_folder();
            }
            let _ = ui.add_enabled(false, toolbar_widget("▣  New Edit"));
            ui.add_space(6.0);
            ui.label(RichText::new("Filters").color(MUTED).size(12.0));
            ui.add_space(4.0);
            ui.add_sized(
                [220.0, 28.0],
                egui::TextEdit::singleline(&mut self.search_text).hint_text("Search clips"),
            );
            let sort_label = if self.sort_newest {
                "Sort: Newest"
            } else {
                "Sort: Name"
            };
            if toolbar_button(ui, sort_label).clicked() {
                self.sort_newest = !self.sort_newest;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let count_label = if self.search_text.trim().is_empty() {
                    format!("{total_count} clips")
                } else {
                    format!("{visible_count} of {total_count} clips")
                };
                ui.label(RichText::new(count_label).color(MUTED).small());
                let path_field = ui.add_sized(
                    [(ui.available_width() - 80.0).clamp(140.0, 300.0), 28.0],
                    egui::TextEdit::singleline(&mut self.directory_text)
                        .hint_text("Local clips folder"),
                );
                if path_field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))
                {
                    self.scan_library();
                }
            });
        });
        ui.add_space(16.0);

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
                    card_action = Self::clip_grid(ui, &entries, &mut self.thumbs);
                }
            }
            None => self.first_scan_state(ui),
        }
        if let Some(action) = card_action {
            match action {
                ClipCardAction::Open(entry) => self.selected = Some(*entry),
                ClipCardAction::CopyPath(path) => {
                    self.copy_media_path(ui.ctx(), PathBuf::from(path));
                }
                ClipCardAction::Reveal(path) => self.reveal_media(PathBuf::from(path)),
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

    fn clip_grid(
        ui: &mut egui::Ui,
        entries: &[&MediaEntry],
        thumbs: &mut thumbs::Thumbnails,
    ) -> Option<ClipCardAction> {
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
        let card_width = ((available_width - (columns.saturating_sub(1) as f32 * 16.0))
            / columns as f32)
            .max(180.0);
        let mut action = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.columns(columns, |column_uis| {
                for (index, entry) in entries.iter().enumerate() {
                    let Some(column) = column_uis.get_mut(index % columns) else {
                        continue;
                    };
                    if let Some(clicked) = clip_card(column, entry, card_width, thumbs) {
                        action = Some(clicked);
                    }
                    column.add_space(16.0);
                }
            });
        });

        action
    }

    fn clip_details(&mut self, ui: &mut egui::Ui, entry: &MediaEntry) {
        if ui.button("<  Back to clips").clicked() {
            self.selected = None;
            return;
        }
        ui.add_space(12.0);
        ui.columns(2, |columns| {
            columns[0].vertical_centered(|ui| {
                if let Some(open) = self.playback.as_mut() {
                    open.show(ui);
                }
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
                        if let Some(probe) = entry.probe.as_ref() {
                            detail_row(ui, "Duration", &format_duration(Some(probe)));
                            if let (Some(width), Some(height)) = (probe.width, probe.height) {
                                detail_row(ui, "Resolution", &format!("{width}×{height}"));
                            }
                            if let Some(rate) = probe.frame_rate {
                                detail_row(ui, "Frame rate", &format!("{} fps", rate.label()));
                            }
                            if let Some(codec) = probe.video_codec.as_deref() {
                                detail_row(ui, "Video codec", codec);
                            }
                        }
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
                        ui.label(RichText::new("File location").color(MUTED).small());
                        ui.add_space(4.0);
                        ui.label(RichText::new(&entry.path).color(TEXT).small());
                        ui.add_space(16.0);
                        ui.horizontal(|ui| {
                            if ui.button("Copy File Path").clicked() {
                                self.copy_media_path(ui.ctx(), PathBuf::from(&entry.path));
                            }
                            if ui.button(open_in_file_manager_label()).clicked() {
                                self.reveal_media(PathBuf::from(&entry.path));
                            }
                        });
                    });
            });
        });
    }

    fn settings(&self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            settings_header(
                ui,
                "🖵",
                "PC Storage",
                "Choose where ClipCraft stores the gameplay it captures. Everything stays on this device.",
            );
            let folder = if self.directory_text.trim().is_empty() {
                "No folder chosen yet"
            } else {
                self.directory_text.as_str()
            };
            setting_panel(ui, "Storage Settings", |ui| {
                ui.label(RichText::new("Capture Folder Location").color(TEXT).strong());
                ui.label(
                    RichText::new("Choose where you want to save your clips and videos:")
                        .color(MUTED),
                );
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(30, 30, 32))
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("Capture folder").color(TEXT).strong());
                        ui.add_space(4.0);
                        ui.label(RichText::new(folder).color(MUTED));
                    });
            });
            settings_header(
                ui,
                "🔊",
                "Audio",
                "Game audio and microphone capture will be configured here once capture exists.",
            );
            setting_panel(ui, "Recording Audio", |ui| {
                ui.label(
                    RichText::new("Capture support is not implemented yet. No recording is running.")
                        .color(MUTED),
                );
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "All clips remain on this device. There is no cloud sync or hosted service.",
                )
                .color(MUTED)
                .small(),
            );
        });
    }

    fn scan_library(&mut self) {
        self.error = None;
        let directory = PathBuf::from(self.directory_text.trim());
        let command = match self.index_path.clone() {
            Some(index_path) => Command::RefreshLibrary {
                directory,
                index_path,
            },
            None => Command::ScanLibrary { directory },
        };
        match execute(command) {
            Ok(CommandOutput::LibraryScan(result)) => {
                if self.index_path.is_some() && !result.persisted {
                    self.error = Some(EngineError::IndexSaveFailed.to_string());
                }
                if self.directory_text != result.directory {
                    self.probe_attempted.clear();
                }
                self.directory_text = result.directory.clone();
                self.library = Some(result);
                self.needs_probe = true;
                self.needs_thumbnails = true;
            }
            Ok(_) => {
                self.error = Some("The library scan returned an unexpected result.".to_owned());
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn copy_media_path(&mut self, context: &egui::Context, path: PathBuf) {
        match execute(Command::LocalMediaPath { path }) {
            Ok(CommandOutput::LocalMediaPath { path }) => context.copy_text(path),
            Ok(_) => {
                self.error = Some("The path command returned an unexpected result.".to_owned());
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn reveal_media(&mut self, path: PathBuf) {
        self.error = None;
        match execute(Command::RevealInFileManager { path }) {
            Ok(CommandOutput::Revealed { .. }) => {}
            Ok(_) => {
                self.error =
                    Some("The file manager command returned an unexpected result.".to_owned());
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
    Open(Box<MediaEntry>),
    CopyPath(String),
    Reveal(String),
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
    let radius = egui::CornerRadius::same(8);
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    context.set_visuals(visuals);
}

fn nav_button(ui: &mut egui::Ui, icon: &str, label: &str, selected: bool, action: impl FnOnce()) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(60.0, 46.0), egui::Sense::click());
    let fill = if selected {
        Color32::from_rgb(52, 52, 56)
    } else if response.hovered() {
        Color32::from_rgb(34, 34, 37)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().circle_filled(rect.center(), 21.0, fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::proportional(20.0),
        if selected { TEXT } else { MUTED },
    );
    if response.on_hover_text(label).clicked() {
        action();
    }
    ui.add_space(8.0);
}

fn open_in_file_manager_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "Open in Explorer"
    } else if cfg!(target_os = "macos") {
        "Show in Finder"
    } else {
        "Show in Folder"
    }
}

/// Makes sure each clip has a cached thumbnail, reporting every one that is ready. Stops when the
/// receiving side goes away, which is how a replaced worker is cancelled.
fn thumbnail_paths(
    paths: Vec<String>,
    cache_dir: &std::path::Path,
    sender: &mpsc::Sender<ThumbnailMessage>,
    notify: impl Fn(),
) {
    for path in paths {
        let Ok(CommandOutput::Thumbnail(thumbnail)) = execute(Command::EnsureThumbnail {
            path: PathBuf::from(&path),
            cache_dir: cache_dir.to_path_buf(),
        }) else {
            continue;
        };
        let message = ThumbnailMessage {
            path,
            file: PathBuf::from(thumbnail.file),
        };
        if sender.send(message).is_err() {
            return;
        }
        notify();
    }
}

/// Probes each path in turn, reporting every result and a final `Done`.
fn probe_paths(
    paths: Vec<String>,
    generation: u64,
    sender: &mpsc::Sender<ProbeMessage>,
    notify: impl Fn(),
) {
    for path in paths {
        let probe = match execute(Command::ProbeClip {
            path: PathBuf::from(&path),
        }) {
            Ok(CommandOutput::ClipProbe(probe)) => Ok(probe),
            Ok(_) => Err(EngineError::MediaUnsupported),
            Err(error) => Err(error),
        };
        if sender
            .send(ProbeMessage::Result {
                generation,
                path,
                probe,
            })
            .is_err()
        {
            return;
        }
        notify();
    }
    let _ = sender.send(ProbeMessage::Done { generation });
    notify();
}

/// `m:ss`, or `h:mm:ss` from one hour up. Display only.
fn format_duration(probe: Option<&MediaProbe>) -> String {
    let Some(duration) = probe.and_then(|probe| probe.duration) else {
        return "--:--".to_owned();
    };
    let total = duration.seconds().max(0.0).floor() as u64;
    let (hours, minutes, seconds) = (total / 3600, total % 3600 / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn key_chip(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(TEXT).strong().size(12.0));
        });
}

fn toolbar_widget(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(TEXT).strong().size(12.0))
        .fill(Color32::TRANSPARENT)
        .min_size(Vec2::new(0.0, 28.0))
}

fn toolbar_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(toolbar_widget(label))
}

fn settings_header(ui: &mut egui::Ui, icon: &str, title: &str, description: &str) {
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(icon).color(MUTED).size(40.0));
        ui.label(RichText::new(title).color(TEXT).size(20.0).strong());
        ui.add_space(4.0);
        ui.add_sized(
            [480.0_f32.min(ui.available_width()), 40.0],
            egui::Label::new(RichText::new(description).color(MUTED)).wrap(),
        );
    });
    ui.add_space(14.0);
}

fn setting_panel(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(PANEL)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).color(TEXT).size(16.0).strong());
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            body(ui);
        });
    ui.add_space(14.0);
}

fn clip_card(
    ui: &mut egui::Ui,
    entry: &MediaEntry,
    width: f32,
    thumbs: &mut thumbs::Thumbnails,
) -> Option<ClipCardAction> {
    let mut action = None;
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(egui::CornerRadius::same(8))
        .show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            let (thumbnail, response) = ui.allocate_exact_size(
                Vec2::new(width, (width * 0.5625).clamp(120.0, 220.0)),
                egui::Sense::click(),
            );
            let visible = ui.is_rect_visible(thumbnail);
            let texture = thumbs.texture(ui.ctx(), &entry.path, visible);
            paint_video_placeholder(ui, thumbnail, entry, texture);
            if response.hovered() {
                ui.painter().rect_filled(
                    thumbnail,
                    egui::CornerRadius {
                        nw: 8,
                        ne: 8,
                        sw: 0,
                        se: 0,
                    },
                    Color32::from_rgba_unmultiplied(255, 255, 255, 14),
                );
            }
            if response.clicked() {
                action = Some(ClipCardAction::Open(Box::new(entry.clone())));
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.add(
                    egui::Label::new(RichText::new(&entry.title).color(TEXT).strong().size(14.0))
                        .truncate(),
                );
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(
                    RichText::new(format!(
                        "{}  •  {}  •  {}",
                        if entry.offline {
                            "Offline"
                        } else {
                            "On Device"
                        },
                        format_bytes(entry.size_bytes),
                        format_date(entry.modified_unix_seconds)
                    ))
                    .color(MUTED)
                    .size(12.0),
                );
            });
            ui.add_space(14.0);

            let (footer, _) = ui.allocate_exact_size(Vec2::new(width, 48.0), egui::Sense::hover());
            let (copy_rect, reveal_rect) = {
                let split = footer.left() + width * 0.5;
                (
                    egui::Rect::from_min_max(footer.min, egui::pos2(split, footer.bottom())),
                    egui::Rect::from_min_max(egui::pos2(split, footer.top()), footer.max),
                )
            };
            let copy_response =
                ui.interact(copy_rect, response.id.with("copy"), egui::Sense::click());
            let reveal_response = ui.interact(
                reveal_rect,
                response.id.with("reveal"),
                egui::Sense::click(),
            );
            let idle = Color32::from_rgb(28, 28, 31);
            let hover = Color32::from_rgb(40, 40, 44);
            ui.painter().rect_filled(
                copy_rect,
                egui::CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: 8,
                    se: 0,
                },
                if copy_response.hovered() { hover } else { idle },
            );
            ui.painter().rect_filled(
                reveal_rect,
                egui::CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: 0,
                    se: 8,
                },
                if reveal_response.hovered() {
                    hover
                } else {
                    idle
                },
            );
            ui.painter().text(
                copy_rect.left_center() + Vec2::new(14.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "📋  Copy Path",
                FontId::proportional(13.0),
                TEXT,
            );
            ui.painter().text(
                reveal_rect.left_center() + Vec2::new(14.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("📁  {}", open_in_file_manager_label()),
                FontId::proportional(13.0),
                TEXT,
            );
            if copy_response.clicked() {
                action = Some(ClipCardAction::CopyPath(entry.path.clone()));
            }
            if reveal_response.clicked() {
                action = Some(ClipCardAction::Reveal(entry.path.clone()));
            }
        });
    action
}

fn paint_video_placeholder(
    ui: &egui::Ui,
    rect: egui::Rect,
    entry: &MediaEntry,
    texture: Option<&egui::TextureHandle>,
) {
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
    let top_corners = egui::CornerRadius {
        nw: 8,
        ne: 8,
        sw: 0,
        se: 0,
    };
    let painter = ui.painter();
    if let Some(texture) = texture {
        // A real thumbnail, cropped to fill the card without stretching.
        egui::Image::from_texture(egui::load::SizedTexture::from_handle(texture))
            .uv(thumbs::cover_uv(texture.size_vec2(), rect.size()))
            .corner_radius(top_corners)
            .paint_at(ui, rect);
    } else {
        painter.rect_filled(rect, top_corners, background);
        painter.circle_filled(
            egui::pos2(
                rect.right() - rect.width() * 0.17,
                rect.top() + rect.height() * 0.3,
            ),
            rect.height() * 0.1,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 100),
        );
        let horizon = rect.center().y + rect.height() * 0.18;
        painter.line_segment(
            [
                egui::pos2(rect.left() + 16.0, horizon),
                egui::pos2(rect.right() - 16.0, horizon),
            ],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 36)),
        );
    }

    let badge = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(10.0, 10.0),
        Vec2::new(46.0, 22.0),
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
        rect.right_bottom() - Vec2::new(55.0, 32.0),
        Vec2::new(45.0, 24.0),
    );
    painter.rect_filled(
        duration_badge,
        egui::CornerRadius::same(6),
        Color32::from_rgba_unmultiplied(0, 0, 0, 200),
    );
    painter.text(
        duration_badge.center(),
        egui::Align2::CENTER_CENTER,
        format_duration(entry.probe.as_ref()),
        FontId::proportional(12.0),
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
    use super::{
        ClipCraftApp, EngineError, MediaProbe, ProbeMessage, format_date, format_duration,
        probe_paths, visible_entries,
    };
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
            offline: false,
            favorite: false,
            tags: Vec::new(),
            probe: None,
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

    #[test]
    fn a_new_launch_restores_the_last_library_and_flags_files_removed_since() {
        let directory = temporary_directory();
        let index_path = directory.join("data").join("library-index.json");
        let clips = directory.join("clips");
        fs::create_dir(&clips).expect("clips folder");
        fs::write(clips.join("keep.mp4"), b"x").expect("keep clip");
        fs::write(clips.join("gone.mp4"), b"x").expect("gone clip");

        let mut first = ClipCraftApp::restored_from(Some(index_path.clone()));
        assert!(first.library.is_none());
        first.set_library_folder(Some(clips.clone()));
        assert_eq!(first.error, None);

        fs::remove_file(clips.join("gone.mp4")).expect("remove clip");
        let second = ClipCraftApp::restored_from(Some(index_path.clone()));
        assert_eq!(second.directory_text, clips.to_string_lossy());
        assert_eq!(second.library.as_ref().map(|l| l.entries.len()), Some(2));
        let entries = &second.library.as_ref().expect("library").entries;
        assert_eq!(entries.iter().filter(|entry| entry.offline).count(), 1);
        assert!(entries.iter().any(|e| e.title == "gone.mp4" && e.offline));

        // An unreadable folder marks every remembered clip offline instead of hiding them.
        fs::remove_dir_all(&clips).expect("remove clips folder");
        let unplugged = ClipCraftApp::restored_from(Some(index_path.clone()));
        let entries = &unplugged.library.as_ref().expect("library").entries;
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.offline));

        fs::write(&index_path, b"damaged").expect("damage index");
        let third = ClipCraftApp::restored_from(Some(index_path));
        assert!(third.library.is_none());
        assert!(third.error.is_some());

        fs::remove_dir_all(directory).expect("remove temporary test directory");
    }

    fn probe_of(milliseconds: i64) -> MediaProbe {
        MediaProbe {
            container: "mp4".to_owned(),
            duration: Some(clipcraft_time::Tick::from_rational(milliseconds, 1, 1000)),
            width: Some(1280),
            height: Some(720),
            frame_rate: Some(clipcraft_time::FrameRate::FPS_30),
            video_codec: Some("avc1".to_owned()),
            audio_tracks: 1,
        }
    }

    #[test]
    fn durations_are_formatted_for_cards() {
        assert_eq!(format_duration(None), "--:--");
        assert_eq!(format_duration(Some(&probe_of(5_000))), "0:05");
        assert_eq!(format_duration(Some(&probe_of(125_400))), "2:05");
        assert_eq!(format_duration(Some(&probe_of(7_500))), "0:07");
        assert_eq!(format_duration(Some(&probe_of(3_725_000))), "1:02:05");
        let mut unknown = probe_of(0);
        unknown.duration = None;
        assert_eq!(format_duration(Some(&unknown)), "--:--");
    }

    #[test]
    fn probe_results_apply_only_to_the_current_folder_and_save_when_done() {
        let directory = temporary_directory();
        let index_path = directory.join("index.json");
        let clips = directory.join("clips");
        fs::create_dir(&clips).expect("clips folder");
        let clip = clips.join("a.mp4");
        fs::write(&clip, b"x").expect("clip");

        let mut app = ClipCraftApp::restored_from(Some(index_path.clone()));
        app.set_library_folder(Some(clips));
        let path = clip.to_string_lossy().into_owned();
        app.probe_generation = 7;

        // A result from a replaced folder's worker is ignored.
        app.handle_probe_message(ProbeMessage::Result {
            generation: 6,
            path: path.clone(),
            probe: Ok(probe_of(1_000)),
        });
        assert!(
            app.library.as_ref().expect("library").entries[0]
                .probe
                .is_none()
        );

        // A failed probe leaves the entry alone; a good one is applied.
        app.handle_probe_message(ProbeMessage::Result {
            generation: 7,
            path: path.clone(),
            probe: Err(EngineError::MediaUnsupported),
        });
        assert!(app.unsaved_probes.is_empty());
        app.handle_probe_message(ProbeMessage::Result {
            generation: 7,
            path,
            probe: Ok(probe_of(9_000)),
        });
        assert_eq!(app.unsaved_probes.len(), 1);

        // `Done` persists, so a fresh launch shows the duration without probing again.
        app.handle_probe_message(ProbeMessage::Done { generation: 7 });
        assert!(app.unsaved_probes.is_empty());
        let relaunched = ClipCraftApp::restored_from(Some(index_path));
        let entry = &relaunched.library.as_ref().expect("library").entries[0];
        assert_eq!(entry.probe, Some(probe_of(9_000)));

        fs::remove_dir_all(directory).expect("remove temporary test directory");
    }

    #[test]
    fn the_probe_worker_reports_every_path_then_done_even_for_bad_files() {
        let directory = temporary_directory();
        let fake = directory.join("fake.mp4");
        fs::write(&fake, b"not a video").expect("fake clip");
        let paths = vec![
            fake.to_string_lossy().into_owned(),
            directory.join("missing.mp4").to_string_lossy().into_owned(),
        ];

        let (sender, receiver) = std::sync::mpsc::channel();
        probe_paths(paths, 3, &sender, || {});
        let messages: Vec<_> = receiver.try_iter().collect();

        assert_eq!(messages.len(), 3);
        assert!(matches!(
            &messages[0],
            ProbeMessage::Result {
                probe: Err(EngineError::MediaUnsupported),
                ..
            }
        ));
        assert!(matches!(
            &messages[1],
            ProbeMessage::Result {
                probe: Err(EngineError::FileUnavailable),
                ..
            }
        ));
        assert!(matches!(messages[2], ProbeMessage::Done { generation: 3 }));

        fs::remove_dir_all(directory).expect("remove temporary test directory");
    }

    #[test]
    fn the_thumbnail_worker_skips_bad_clips_and_stops_when_replaced() {
        let directory = temporary_directory();
        let fake = directory.join("fake.mp4");
        fs::write(&fake, b"not a video").expect("fake clip");
        let paths = vec![
            fake.to_string_lossy().into_owned(),
            directory.join("missing.mp4").to_string_lossy().into_owned(),
        ];
        let cache = directory.join("cache");

        // Bad clips produce no thumbnails and the worker still finishes.
        let (sender, receiver) = std::sync::mpsc::channel();
        super::thumbnail_paths(paths.clone(), &cache, &sender, || {});
        assert!(receiver.try_iter().next().is_none());
        assert!(!cache.exists() || fs::read_dir(&cache).map_or(0, Iterator::count) == 0);

        // A dropped receiver (a replaced worker) is not an error.
        let (sender, receiver) = std::sync::mpsc::channel();
        drop(receiver);
        super::thumbnail_paths(paths, &cache, &sender, || {});

        fs::remove_dir_all(directory).expect("remove temporary test directory");
    }
}
