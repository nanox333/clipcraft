use clipcraft_engine::{
    Command, CommandOutput, EngineError, LibraryScanResult, MediaEntry, MediaProbe,
    default_index_path, default_thumbnail_dir, execute,
};
mod kit;
mod playback;
mod thumbs;

use eframe::egui;
use egui::{Align, Color32, Layout, RichText, Stroke, Vec2};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;

use kit::{
    BORDER, CANVAS, CARD, CARD_HOVER, DANGER, FAINT, Icon, LIME, MUTED, PANEL, SIDEBAR, TEXT,
};

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
    /// Volume, mute, and loop, carried from clip to clip.
    playback_prefs: playback::Prefs,
    /// Whether the window is currently fullscreen because of the open clip.
    window_fullscreen: bool,
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
        kit::apply_theme(context);
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
        self.draw_screen(context);
    }
}

impl ClipCraftApp {
    /// Draws the current screen: the library, the player, or settings, with their chrome.
    fn draw_screen(&mut self, context: &egui::Context) {
        // Fullscreen playback hides all chrome: the picture owns the whole window.
        let fullscreen = self
            .playback
            .as_ref()
            .is_some_and(playback::Playback::fullscreen);
        if fullscreen {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(Color32::BLACK))
                .show(context, |ui| {
                    if let Some(open) = self.playback.as_mut() {
                        open.show_fullscreen(ui);
                    }
                });
            self.thumbs.end_frame();
            return;
        }

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

    /// Keeps the volume, mute, and loop choices of the clip being closed for the next one.
    fn remember_playback_prefs(&mut self) {
        if let Some(open) = self.playback.take() {
            self.playback_prefs = open.prefs();
        }
    }

    /// Keeps exactly one player alive for the clip open in the detail view.
    fn sync_playback(&mut self, context: &egui::Context) {
        let wanted_path = self.selected.as_ref().map(|entry| entry.path.clone());
        match wanted_path {
            Some(path)
                if self
                    .playback
                    .as_ref()
                    .is_some_and(|open| open.path() == path) => {}
            Some(path) => {
                self.remember_playback_prefs();
                self.playback = Some(playback::Playback::start(
                    &path,
                    context,
                    self.playback_prefs,
                ));
            }
            None => self.remember_playback_prefs(),
        }
        // The window follows the player: fullscreen turns on and off with the open clip, including
        // when the clip is closed while fullscreen.
        let wanted = self
            .playback
            .as_ref()
            .is_some_and(playback::Playback::fullscreen);
        if wanted != self.window_fullscreen {
            self.window_fullscreen = wanted;
            context.send_viewport_cmd(egui::ViewportCommand::Fullscreen(wanted));
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
            .exact_height(52.0)
            .frame(
                egui::Frame::new()
                    .fill(SIDEBAR)
                    .stroke(Stroke::new(1.0_f32, BORDER)),
            )
            .show(context, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(20.0);
                    let (logo, _) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::hover());
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
                    ui.add_space(2.0);
                    ui.label(RichText::new("ClipCraft").color(TEXT).size(17.0).strong());
                    ui.add_space(22.0);

                    // Capture state: honest about the fact that capture is not built yet.
                    kit::status_pill(ui, FAINT, "Capture unavailable");
                    ui.add_space(6.0);
                    kit::keycap(ui, "F8");
                    ui.label(RichText::new("Clip 2m").color(TEXT).size(13.0));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Long recording").color(MUTED).size(13.0));
                    kit::keycap(ui, "Off");

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(20.0);
                        ui.label(RichText::new("1440p  ·  60 FPS").color(MUTED).size(13.0));
                    });
                });
            });
    }

    fn navigation(&mut self, context: &egui::Context) {
        egui::SidePanel::left("navigation")
            .exact_width(68.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(SIDEBAR)
                    .stroke(Stroke::new(1.0_f32, BORDER)),
            )
            .show(context, |ui| {
                ui.add_space(18.0);
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 10.0;
                    if nav_button(ui, Icon::Library, "Clips", self.page == Page::Library) {
                        self.page = Page::Library;
                        self.selected = None;
                    }
                    if nav_button(ui, Icon::Gear, "Settings", self.page == Page::Settings) {
                        self.page = Page::Settings;
                        self.selected = None;
                    }
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

        // Title row.
        ui.horizontal(|ui| {
            ui.label(RichText::new("Clips").color(TEXT).size(26.0).strong());
            if self.library.is_some() {
                let count = if self.search_text.trim().is_empty() {
                    format!("{total_count}")
                } else {
                    format!("{visible_count} of {total_count}")
                };
                kit::chip(ui, &count);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if kit::labelled_button(ui, Icon::Import, "Import folder", true, true).clicked() {
                    self.choose_library_folder();
                }
            });
        });
        ui.add_space(4.0);

        // Search, sort, and folder.
        ui.horizontal(|ui| {
            egui::Frame::new()
                .fill(CARD)
                .corner_radius(egui::CornerRadius::same(18))
                .inner_margin(egui::Margin::symmetric(12, 5))
                .show(ui, |ui| {
                    let (icon, _) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                    kit::draw_icon(ui.painter(), icon.center(), Icon::Search, MUTED, 0.7);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search_text)
                            .frame(false)
                            .hint_text("Search clips")
                            .desired_width(230.0),
                    );
                });
            self.sort_menu(ui);

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                egui::Frame::new()
                    .fill(CARD)
                    .corner_radius(egui::CornerRadius::same(18))
                    .inner_margin(egui::Margin::symmetric(12, 5))
                    .show(ui, |ui| {
                        // A fixed width, with the field filling it, so the whole pill is clickable.
                        ui.set_width(340.0);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let (icon, _) =
                                ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                            kit::draw_icon(ui.painter(), icon.center(), Icon::Folder, MUTED, 0.75);
                            let field = ui.add(
                                egui::TextEdit::singleline(&mut self.directory_text)
                                    .frame(false)
                                    .hint_text("Clips folder")
                                    .desired_width(
                                        (ui.available_width() - 20.0).clamp(150.0, 340.0),
                                    ),
                            );
                            if field.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter))
                            {
                                self.scan_library();
                            }
                        });
                    });
            });
        });
        ui.add_space(10.0);

        if let Some(error) = self.error.as_deref() {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(255, 112, 112, 24))
                .stroke(Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(255, 112, 112, 90),
                ))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(error).color(DANGER));
                });
            ui.add_space(10.0);
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

    fn sort_menu(&mut self, ui: &mut egui::Ui) {
        let label = if self.sort_newest {
            "Newest first"
        } else {
            "Name A–Z"
        };
        kit::pill_menu(ui, label, "Sort clips", |ui| {
            if ui.selectable_label(!self.sort_newest, "Name A–Z").clicked() {
                self.sort_newest = false;
                ui.close();
            }
            if ui
                .selectable_label(self.sort_newest, "Newest first")
                .clicked()
            {
                self.sort_newest = true;
                ui.close();
            }
        });
    }

    fn first_scan_state(&mut self, ui: &mut egui::Ui) {
        if empty_panel(
            ui,
            Icon::Library,
            "Your clips, on this device",
            "Choose a folder and ClipCraft finds the videos in it. Nothing is uploaded, ever.",
            Some("Choose folder"),
        ) {
            self.choose_library_folder();
        }
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        if empty_panel(
            ui,
            Icon::Folder,
            "No clips in this folder",
            "Pick another folder, or come back once capture can save recordings here.",
            Some("Choose another folder"),
        ) {
            self.choose_library_folder();
        }
    }

    fn no_search_results(&self, ui: &mut egui::Ui) {
        empty_panel(
            ui,
            Icon::Search,
            "No clips match this search",
            "Try another title or clear the search field.",
            None,
        );
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
        let mut back = false;
        ui.horizontal(|ui| {
            if kit::labelled_button(ui, Icon::ChevronLeft, "Clips", false, true).clicked() {
                back = true;
            }
            ui.add_space(6.0);
            ui.add(
                egui::Label::new(RichText::new(&entry.title).color(TEXT).size(22.0).strong())
                    .truncate(),
            );
        });
        if back {
            self.selected = None;
            return;
        }

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            kit::chip(ui, &entry.extension.to_uppercase());
            if let Some(probe) = entry.probe.as_ref() {
                if let (Some(width), Some(height)) = (probe.width, probe.height) {
                    kit::chip(ui, &format!("{width}×{height}"));
                }
                if let Some(rate) = probe.frame_rate {
                    kit::chip(ui, &format!("{} fps", rate.label()));
                }
                if let Some(codec) = probe.video_codec.as_deref() {
                    kit::chip(ui, codec);
                }
                kit::chip(ui, &format_duration(Some(probe)));
            }
            kit::chip(ui, &format_bytes(entry.size_bytes));
        });
        ui.add_space(6.0);

        let side = 340.0_f32;
        let gap = 22.0_f32;
        let main = (ui.available_width() - side - gap).max(360.0);
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(
                Vec2::new(main, height),
                Layout::top_down(Align::Min),
                |ui| {
                    if let Some(open) = self.playback.as_mut() {
                        open.show(ui);
                    }
                },
            );
            ui.allocate_ui_with_layout(
                Vec2::new(side, height),
                Layout::top_down(Align::Min),
                |ui| self.details_card(ui, entry, side),
            );
        });
    }

    fn details_card(&mut self, ui: &mut egui::Ui, entry: &MediaEntry, width: f32) {
        egui::Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(egui::CornerRadius::same(14))
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| {
                ui.set_width((width - 42.0).max(0.0));
                ui.label(RichText::new("Details").color(TEXT).size(16.0).strong());
                ui.add_space(6.0);
                detail_row(ui, "Source", "Local video");
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
                    detail_row(
                        ui,
                        "Audio",
                        &match probe.audio_tracks {
                            0 => "None".to_owned(),
                            1 => "1 track".to_owned(),
                            count => format!("{count} tracks"),
                        },
                    );
                }
                detail_row(ui, "File size", &format_bytes(entry.size_bytes));

                ui.add_space(4.0);
                let (line, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(line, 0.0, BORDER);
                ui.add_space(6.0);
                ui.label(RichText::new("Location").color(FAINT).size(12.0));
                ui.add(
                    egui::Label::new(RichText::new(&entry.path).color(MUTED).size(12.5))
                        .wrap_mode(egui::TextWrapMode::Wrap),
                );
                ui.add_space(8.0);
                let full = ui.available_width();
                if kit::labelled_button_sized(ui, Icon::Copy, "Copy file path", false, true, full)
                    .clicked()
                {
                    self.copy_media_path(ui.ctx(), PathBuf::from(&entry.path));
                }
                if kit::labelled_button_sized(
                    ui,
                    Icon::Folder,
                    open_in_file_manager_label(),
                    false,
                    true,
                    full,
                )
                .clicked()
                {
                    self.reveal_media(PathBuf::from(&entry.path));
                }
                let editor =
                    kit::labelled_button_sized(ui, Icon::Play, "Open in editor", true, false, full);
                editor.on_hover_text("The basic editor is not built yet");
            });
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            let column = 820.0_f32.min(ui.available_width());
            let margin = ((ui.available_width() - column) / 2.0).max(0.0);
            ui.horizontal_top(|ui| {
            ui.add_space(margin);
            ui.vertical(|ui| {
            ui.set_width(column.max(0.0));
            settings_header(
                ui,
                Icon::Folder,
                "Storage",
                "Choose where ClipCraft looks for your clips. Everything stays on this device.",
            );
            let folder = if self.directory_text.trim().is_empty() {
                "No folder chosen yet".to_owned()
            } else {
                self.directory_text.clone()
            };
            let mut browse = false;
            setting_panel(ui, "Clips folder", |ui| {
                ui.label(
                    RichText::new("ClipCraft scans this folder for videos. Nothing is moved or changed.")
                        .color(MUTED),
                );
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(13, 13, 15))
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(36.0);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let (icon, _) =
                                ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::hover());
                            kit::draw_icon(ui.painter(), icon.center(), Icon::Folder, MUTED, 0.8);
                            ui.add(
                                egui::Label::new(RichText::new(folder).color(TEXT))
                                    .truncate(),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                browse = kit::labelled_button(ui, Icon::Folder, "Browse", false, true)
                                    .clicked();
                            });
                        });
                    });
            });
            if browse {
                self.choose_library_folder();
            }

            settings_header(
                ui,
                Icon::Speaker,
                "Audio",
                "Playback uses your system's default sound device. Capture audio arrives with capture.",
            );
            setting_panel(ui, "Playback", |ui| {
                ui.label(
                    RichText::new(
                        "Volume, mute, and loop are remembered between clips. Hover any player control to see its key.",
                    )
                    .color(MUTED),
                );
            });
            setting_panel(ui, "Recording", |ui| {
                ui.label(
                    RichText::new("Capture is not implemented yet. No recording is running.")
                        .color(MUTED),
                );
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "All clips remain on this device. There is no cloud sync or hosted service.",
                )
                .color(FAINT)
                .size(12.5),
            );
            });
            });
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

/// A sidebar icon button with an active plate and a lime marker. True when clicked.
fn nav_button(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(68.0, 48.0), egui::Sense::click());
    let plate = egui::Rect::from_center_size(rect.center(), Vec2::splat(42.0));
    if selected {
        ui.painter().rect_filled(
            plate,
            12.0,
            Color32::from_rgba_unmultiplied(190, 255, 43, 26),
        );
        ui.painter().rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 1.5, rect.center().y),
                Vec2::new(3.0, 22.0),
            ),
            1.5,
            LIME,
        );
    } else if response.hovered() {
        ui.painter()
            .rect_filled(plate, 12.0, Color32::from_white_alpha(12));
    }
    let color = if selected {
        LIME
    } else if response.hovered() {
        TEXT
    } else {
        MUTED
    };
    kit::draw_icon(ui.painter(), rect.center(), icon, color, 1.0);
    response.on_hover_text(label).clicked()
}

/// A centered illustration, headline, help text, and an optional button. True when it is clicked.
fn empty_panel(
    ui: &mut egui::Ui,
    icon: Icon,
    headline: &str,
    help: &str,
    button: Option<&str>,
) -> bool {
    let mut clicked = false;
    ui.add_space((ui.available_height() * 0.16).clamp(24.0, 120.0));
    ui.vertical_centered(|ui| {
        let (circle, _) = ui.allocate_exact_size(Vec2::splat(92.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(circle.center(), 46.0, Color32::from_rgb(28, 28, 33));
        ui.painter().circle_stroke(
            circle.center(),
            46.0,
            Stroke::new(1.0_f32, Color32::from_rgb(48, 48, 55)),
        );
        kit::draw_icon(ui.painter(), circle.center(), icon, MUTED, 1.9);
        ui.add_space(10.0);
        ui.label(RichText::new(headline).color(TEXT).size(20.0).strong());
        ui.label(RichText::new(help).color(MUTED));
        if let Some(button) = button {
            ui.add_space(8.0);
            clicked = kit::labelled_button(ui, Icon::Folder, button, true, true).clicked();
        }
    });
    clicked
}

fn settings_header(ui: &mut egui::Ui, icon: Icon, title: &str, description: &str) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        let (circle, _) = ui.allocate_exact_size(Vec2::splat(46.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(circle.center(), 23.0, Color32::from_rgb(28, 28, 33));
        kit::draw_icon(ui.painter(), circle.center(), icon, LIME, 1.1);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(title).color(TEXT).size(21.0).strong());
            ui.label(RichText::new(description).color(MUTED));
        });
    });
    ui.add_space(8.0);
}

fn setting_panel(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::same(22))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).color(TEXT).size(16.0).strong());
            ui.add_space(4.0);
            body(ui);
        });
    ui.add_space(14.0);
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(value).color(TEXT)).truncate());
        });
    });
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

fn clip_card(
    ui: &mut egui::Ui,
    entry: &MediaEntry,
    width: f32,
    thumbs: &mut thumbs::Thumbnails,
) -> Option<ClipCardAction> {
    const RADIUS: u8 = 12;
    const BODY_HEIGHT: f32 = 66.0;
    const FOOTER_HEIGHT: f32 = 42.0;
    let thumb_height = (width * 0.5625).clamp(120.0, 240.0);
    let (card, response) = ui.allocate_exact_size(
        Vec2::new(width, thumb_height + BODY_HEIGHT + FOOTER_HEIGHT),
        egui::Sense::click(),
    );
    let thumbnail = egui::Rect::from_min_size(card.min, Vec2::new(width, thumb_height));
    let footer = egui::Rect::from_min_max(
        egui::pos2(card.left(), card.bottom() - FOOTER_HEIGHT),
        card.max,
    );
    let middle = footer.center().x;
    let copy_area = egui::Rect::from_min_max(footer.min, egui::pos2(middle, footer.bottom()));
    let reveal_area = egui::Rect::from_min_max(egui::pos2(middle, footer.top()), footer.max);
    // Added after the card so they sit on top of it and take their own clicks.
    let copy = ui.interact(copy_area, response.id.with("copy"), egui::Sense::click());
    let reveal = ui.interact(
        reveal_area,
        response.id.with("reveal"),
        egui::Sense::click(),
    );
    let hovered = response.hovered() || copy.hovered() || reveal.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let painter = ui.painter().clone();
    painter.rect(
        card,
        egui::CornerRadius::same(RADIUS),
        if hovered { CARD_HOVER } else { CARD },
        Stroke::new(
            if hovered { 1.5_f32 } else { 1.0_f32 },
            if hovered {
                Color32::from_rgba_unmultiplied(190, 255, 43, 150)
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );

    // Picture, then the hover play button over it.
    let visible = ui.is_rect_visible(thumbnail);
    let texture = thumbs.texture(ui.ctx(), &entry.path, visible);
    paint_video_placeholder(ui, thumbnail.shrink(1.0), entry, texture, RADIUS - 1);
    if hovered {
        painter.circle_filled(thumbnail.center(), 26.0, Color32::from_black_alpha(165));
        kit::draw_icon(
            &painter,
            thumbnail.center() + Vec2::new(1.0, 0.0),
            Icon::Play,
            LIME,
            1.15,
        );
    }

    // Title (one line, ellipsized) and details.
    let text_left = card.left() + 14.0;
    let mut title = egui::text::LayoutJob::simple_singleline(
        entry.title.clone(),
        egui::FontId::proportional(15.0),
        TEXT,
    );
    title.wrap.max_width = width - 28.0;
    title.wrap.max_rows = 1;
    title.wrap.break_anywhere = true;
    title.wrap.overflow_character = Some('…');
    let title = painter.layout_job(title);
    painter.galley(
        egui::pos2(text_left, thumbnail.bottom() + 12.0),
        title,
        TEXT,
    );
    let mut details = format!(
        "{}  ·  {}",
        format_bytes(entry.size_bytes),
        format_date(entry.modified_unix_seconds)
    );
    if entry.offline {
        details = format!("Offline  ·  {details}");
    }
    painter.text(
        egui::pos2(text_left, thumbnail.bottom() + 38.0),
        egui::Align2::LEFT_TOP,
        details,
        egui::FontId::proportional(12.5),
        if entry.offline { DANGER } else { MUTED },
    );

    // Footer actions.
    painter.line_segment(
        [footer.left_top(), footer.right_top()],
        Stroke::new(1.0_f32, BORDER),
    );
    let bottom_left = egui::CornerRadius {
        nw: 0,
        ne: 0,
        sw: RADIUS,
        se: 0,
    };
    let bottom_right = egui::CornerRadius {
        nw: 0,
        ne: 0,
        sw: 0,
        se: RADIUS,
    };
    for (area, rounding, response, icon, label) in [
        (copy_area, bottom_left, &copy, Icon::Copy, "Copy path"),
        (
            reveal_area,
            bottom_right,
            &reveal,
            Icon::Folder,
            open_in_file_manager_label(),
        ),
    ] {
        let hot = response.hovered();
        if hot {
            painter.rect_filled(
                area.shrink2(Vec2::new(1.0, 1.0)),
                rounding,
                Color32::from_white_alpha(14),
            );
        }
        let color = if hot { LIME } else { MUTED };
        kit::draw_icon(
            &painter,
            egui::pos2(area.left() + 24.0, area.center().y),
            icon,
            color,
            0.72,
        );
        painter.text(
            egui::pos2(area.left() + 42.0, area.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            if hot { TEXT } else { MUTED },
        );
    }
    painter.line_segment(
        [
            egui::pos2(middle, footer.top() + 10.0),
            egui::pos2(middle, footer.bottom() - 10.0),
        ],
        Stroke::new(1.0_f32, BORDER),
    );

    if copy.clicked() {
        Some(ClipCardAction::CopyPath(entry.path.clone()))
    } else if reveal.clicked() {
        Some(ClipCardAction::Reveal(entry.path.clone()))
    } else if response.clicked() {
        Some(ClipCardAction::Open(Box::new(entry.clone())))
    } else {
        None
    }
}

fn paint_video_placeholder(
    ui: &egui::Ui,
    rect: egui::Rect,
    entry: &MediaEntry,
    texture: Option<&egui::TextureHandle>,
    radius: u8,
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
        nw: radius,
        ne: radius,
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
        // No picture yet (or none possible): a soft tinted tile with a film-frame mark.
        painter.rect_filled(rect, top_corners, background);
        painter.circle_filled(
            egui::pos2(
                rect.right() - rect.width() * 0.17,
                rect.top() + rect.height() * 0.3,
            ),
            rect.height() * 0.1,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 90),
        );
        let horizon = rect.center().y + rect.height() * 0.18;
        painter.line_segment(
            [
                egui::pos2(rect.left() + 16.0, horizon),
                egui::pos2(rect.right() - 16.0, horizon),
            ],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 30)),
        );
    }
    // A fade at the bottom keeps the duration badge readable on bright pictures.
    kit::bottom_fade(
        painter,
        egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 54.0), rect.max),
        150,
    );

    let badge = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(10.0, 10.0),
        Vec2::new(44.0, 22.0),
    );
    painter.rect_filled(
        badge,
        egui::CornerRadius::same(6),
        Color32::from_black_alpha(170),
    );
    painter.text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        entry.extension.to_uppercase(),
        egui::FontId::proportional(10.5),
        TEXT,
    );
    let duration = format_duration(entry.probe.as_ref());
    let duration_badge = egui::Rect::from_min_size(
        rect.right_bottom() - Vec2::new(58.0, 32.0),
        Vec2::new(48.0, 22.0),
    );
    painter.rect_filled(
        duration_badge,
        egui::CornerRadius::same(6),
        Color32::from_black_alpha(200),
    );
    painter.text(
        duration_badge.center(),
        egui::Align2::CENTER_CENTER,
        duration,
        egui::FontId::proportional(12.0),
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
        ClipCraftApp, EngineError, MediaProbe, Page, ProbeMessage, format_date, format_duration,
        kit, probe_paths, visible_entries,
    };
    use clipcraft_engine::{LibraryScanResult, MediaEntry};
    use eframe::egui;
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

    /// Runs `frames` frames of the whole UI at `size` points.
    fn draw(app: &mut ClipCraftApp, context: &egui::Context, size: (f32, f32), frames: usize) {
        for _ in 0..frames {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(size.0, size.1),
                )),
                ..Default::default()
            };
            let _ = context.run(input, |context| {
                kit::apply_theme(context);
                app.thumbs.begin_frame();
                app.draw_screen(context);
            });
        }
    }

    fn awkward_entries() -> Vec<MediaEntry> {
        let mut entries = vec![
            media_entry("short.mp4", Some(10)),
            media_entry(&"a very long clip title ".repeat(40), Some(20)),
            media_entry("", None),
            media_entry("日本語のタイトル 🎮 emoji.mp4", Some(30)),
            media_entry("offline.mp4", Some(40)),
        ];
        entries[4].offline = true;
        entries[0].probe = Some(probe_of(95_000));
        entries
    }

    #[test]
    fn every_screen_draws_at_any_window_size_without_panicking() {
        let context = egui::Context::default();
        let sizes = [
            (1.0, 1.0),
            (120.0, 80.0),
            (320.0, 200.0),
            (640.0, 480.0),
            (1440.0, 900.0),
            (5000.0, 3000.0),
        ];
        for size in sizes {
            // First launch: nothing chosen yet.
            let mut app = ClipCraftApp::default();
            draw(&mut app, &context, size, 2);

            // A scanned folder with awkward titles, plus a search that matches nothing.
            app.library = Some(LibraryScanResult {
                directory: "/clips".to_owned(),
                entries: awkward_entries(),
                persisted: false,
            });
            app.error = Some("A scan problem".to_owned());
            draw(&mut app, &context, size, 3);
            app.search_text = "no clip has this title".to_owned();
            draw(&mut app, &context, size, 2);
            app.search_text.clear();
            app.sort_newest = true;
            draw(&mut app, &context, size, 2);

            // An empty folder.
            app.library = Some(LibraryScanResult {
                directory: "/clips".to_owned(),
                entries: Vec::new(),
                persisted: false,
            });
            draw(&mut app, &context, size, 2);

            // Clip details, with and without a probe, and the player area that follows them.
            app.library = Some(LibraryScanResult {
                directory: "/clips".to_owned(),
                entries: awkward_entries(),
                persisted: false,
            });
            for entry in awkward_entries() {
                app.selected = Some(entry);
                draw(&mut app, &context, size, 2);
            }
            app.selected = None;

            // Settings, empty and with a folder.
            app.page = Page::Settings;
            draw(&mut app, &context, size, 2);
            app.directory_text = "/a/very/long/path/".repeat(30);
            draw(&mut app, &context, size, 2);
        }
    }
}
