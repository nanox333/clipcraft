// Release builds on Windows are window programs: without this Windows opens a terminal beside
// the app. Debug builds keep the console so development output stays visible.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use clipcraft_engine::{
    Command, CommandOutput, EngineError, LibraryScanResult, MediaEntry, MediaProbe, MetaEdit,
    RecordingSettings, apply_edit, default_capture_folder, default_index_path,
    default_thumbnail_dir, execute, game_of,
};
mod bug_report;
mod capture;
mod clip_card;
mod clip_delete;
mod clip_page;
mod clip_rename;
mod editor;
mod editor_media;
mod editor_recovery;
mod game_data;
mod game_scan;
mod kit;
mod library_grid;
mod library_scan;
mod library_store;
mod library_view;
mod onboarding;
mod organize;
mod playback;
mod presence;
mod rows;
mod saved_toast;
mod settings;
mod settings_page;
mod svg;
mod thumbs;
mod tray;
mod tray_menu;
mod trim;
mod updater;
mod window_chrome;

use eframe::egui;
use egui::{Align, Color32, Layout, RichText, Stroke, Vec2};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc;

use clip_card::{ClipCardAction, clip_card};
use kit::{
    BORDER, CANVAS, CARD, CARD_HOVER, DANGER, FAINT, Icon, LIME, MUTED, PANEL, SIDEBAR, TEXT,
};
use organize::{CollectionEdit, LibraryFilter, RailAction};
use settings::{card, setting_row};
use window_chrome::{WindowButton, nav_button, resize_from_the_edges, window_button};

fn main() -> eframe::Result<()> {
    clipcraft_engine::set_trash_handler(clipcraft_platform::move_to_trash);
    clipcraft_engine::update::set_http_fetcher(clipcraft_platform::https_get);
    let launch_settings = clipcraft_engine::default_settings_path()
        .and_then(
            |path| match execute(Command::LoadRecordingSettings { path }) {
                Ok(CommandOutput::RecordingSettings(settings)) => Some(settings),
                _ => None,
            },
        )
        .unwrap_or_default();
    let minimized = launch_settings.setup_completed
        && (launch_settings.start_minimized
            || std::env::args().any(|argument| argument == "--minimized"));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ClipCraft")
            // Names the window to the desktop (X11 class, Wayland app id), which matches it to
            // clipcraft.desktop and its icon in the taskbar or dock.
            .with_app_id("clipcraft")
            // The title bar is drawn by the app itself (see `top_bar`).
            .with_decorations(false)
            .with_icon(kit::window_icon_data())
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ClipCraft",
        options,
        Box::new(|creation| {
            // Fonts must be installed before the first frame uses a named family.
            kit::install_fonts(&creation.egui_ctx);
            kit::apply_theme(&creation.egui_ctx);
            if minimized {
                creation
                    .egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            let mut app = ClipCraftApp::restored();
            app.tray = tray::Tray::new(&creation.egui_ctx);
            if let Some(path) = clipcraft_engine::default_editor_draft_path() {
                app.editor.enable_recovery(path, &creation.egui_ctx);
            }
            Ok(Box::new(app))
        }),
    )
}

#[derive(Default)]
struct ClipCraftApp {
    /// The window icon was set again after the window opened.
    icon_sent: bool,
    directory_text: String,
    library: Option<LibraryScanResult>,
    library_view: library_view::ViewCache,
    grid: library_grid::GridState,
    scanning: Option<library_scan::ScanJob>,
    scan_worker: Option<library_scan::ScanWorker>,
    index_store: library_store::State,
    saved_games: std::collections::HashMap<String, String>,
    selected: Option<MediaEntry>,
    error: Option<String>,
    /// A search for a moved clip that is running.
    finding: Option<FindJob>,
    /// The keyboard moved the focus last, so the focus ring shows.
    keyboard_focus: bool,
    /// When the diagnostics report was last copied, to confirm it on the button.
    diagnostics_copied: Option<std::time::Instant>,
    bug_report: Option<bug_report::Draft>,
    search_text: String,
    sort: library_view::Sort,
    page: Page,
    /// Where the rebuildable library index lives; `None` keeps scans in memory only.
    index_path: Option<PathBuf>,
    /// Background probe worker for the current folder, if one is running.
    probe_receiver: Option<mpsc::Receiver<ProbeMessage>>,
    /// Incremented per worker so results from a replaced folder are ignored.
    probe_generation: u64,
    probe_positions: std::collections::HashMap<String, usize>,
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
    /// How clips are recorded: container, source, and framing.
    recording: RecordingSettings,
    setup: Option<onboarding::Setup>,
    /// Where the recording settings are saved; `None` keeps them in memory only.
    settings_path: Option<PathBuf>,
    settings_tab: SettingsTab,
    /// What is typed in the settings search box.
    settings_search: String,
    /// The system-tray icon, when the platform has one.
    tray: Option<tray::Tray>,
    /// The taskbar button is hidden because the window is minimized to the tray.
    taskbar_hidden: bool,
    updater: updater::Updater,
    update_checked_on_launch: bool,
    /// The version whose banner was dismissed with Later.
    update_dismissed: Option<String>,
    /// The settings as they were when the Settings page was opened, for Cancel changes.
    settings_snapshot: Option<RecordingSettings>,
    /// The tray menu, while it is open.
    tray_menu: Option<tray_menu::TrayMenu>,
    /// The pop-up for the clip just saved.
    saved_toast: Option<saved_toast::SavedToast>,
    /// A clip to open as soon as the library lists it.
    pending_open: Option<PathBuf>,
    /// The clip the person is being asked about deleting.
    confirm_delete: Option<MediaEntry>,
    /// A clip being moved to the Recycle Bin.
    trashing: Option<clip_delete::Trashing>,
    /// The rename box, while it is open.
    rename_draft: Option<clip_rename::RenameDraft>,
    /// A clip being renamed.
    renaming: Option<clip_rename::Renaming>,
    settings_state: settings::SettingsState,
    game_data: game_data::State,
    presence: presence::State,
    /// Which clips the library shows: all, favorites, a collection, or a game.
    filter: LibraryFilter,
    /// A collection being created or renamed in the sidebar.
    collection_edit: Option<CollectionEdit>,
    /// The game name being typed for the open clip: its path and the text.
    game_draft: Option<(String, String)>,
    /// Recording: the replay buffer, hotkeys, and the clip being saved.
    capture: capture::CaptureState,
    editor: editor::EditorState,
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
enum SettingsTab {
    #[default]
    Overview,
    General,
    Storage,
    Hotkeys,
    Games,
    Capture,
    Video,
    Audio,
    Replay,
    About,
}

impl SettingsTab {
    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::General => "General",
            Self::Storage => "Storage",
            Self::Hotkeys => "Hotkeys",
            Self::Games => "Games",
            Self::Capture => "Capture",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Replay => "Replay",
            Self::About => "About",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::Overview => "Your setup at a glance, what to fix, and one-click profiles.",
            Self::General => {
                "Where your clips live, which settings are shown, and when ClipCraft starts."
            }
            Self::Storage => "Where clips are saved, how much space they use, and a limit.",
            Self::Hotkeys => "Keyboard shortcuts for recording.",
            Self::Games => "Choose which running games can arm replay capture.",
            Self::Capture => "What gets recorded: a screen, a window, or your game.",
            Self::Video => {
                "How sharp and smooth your clips are, which chip encodes them, and how big they get."
            }
            Self::Audio => "What sound is recorded and how well.",
            Self::Replay => {
                "How the replay buffer works: how far back a clip reaches and what happens when you save."
            }
            Self::About => {
                "What ClipCraft is, which version you have, and where to find the project."
            }
        }
    }
}

/// Settings that are not rows (folders, lists, forms), so the search box can still point at them.
const SETTINGS_INDEX: &[(&str, &str, SettingsTab)] = &[
    (
        "Library folder",
        "clips folder where videos are watched scan",
        SettingsTab::General,
    ),
    (
        "Discord activity",
        "rich presence status share profile",
        SettingsTab::General,
    ),
    (
        "Diagnostics report",
        "help support copy problem bug",
        SettingsTab::General,
    ),
    (
        "Clips folder",
        "save location directory path move",
        SettingsTab::Storage,
    ),
    (
        "Storage used",
        "disk space size usage",
        SettingsTab::Storage,
    ),
    (
        "Save clip hotkey",
        "shortcut key keybind F8",
        SettingsTab::Hotkeys,
    ),
    (
        "Record hotkey",
        "shortcut key toggle recording F9 start stop",
        SettingsTab::Hotkeys,
    ),
    (
        "Screenshot hotkey",
        "shortcut key picture F11",
        SettingsTab::Hotkeys,
    ),
    (
        "Microphone hotkey",
        "mute push to talk shortcut F10",
        SettingsTab::Hotkeys,
    ),
    (
        "Games only capture",
        "game only recognized automatic",
        SettingsTab::Games,
    ),
    (
        "Desktop replay",
        "between games after game always desktop continuation",
        SettingsTab::Games,
    ),
    (
        "Known games",
        "list supported counter-strike dota league detection",
        SettingsTab::Games,
    ),
    (
        "Steam games",
        "installed library detect",
        SettingsTab::Games,
    ),
    (
        "Custom games",
        "add game executable rule override",
        SettingsTab::Games,
    ),
    (
        "Game integrations",
        "cs2 dota 2 league match details score",
        SettingsTab::Games,
    ),
    (
        "About ClipCraft",
        "version license project bug report feedback help",
        SettingsTab::About,
    ),
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Page {
    #[default]
    Library,
    Editor,
    Settings,
}

impl eframe::App for ClipCraftApp {
    /// While the tray menu is open its window is cleared to the menu colour, so it never flashes
    /// a different colour before the first frame is painted.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if self.tray_menu.is_some() {
            kit::PANEL.to_normalized_gamma_f32()
        } else {
            Color32::from_rgba_unmultiplied(12, 12, 12, 180).to_normalized_gamma_f32()
        }
    }

    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        // Some desktops end up with an empty icon from the window's first creation (the taskbar
        // then shows a blank or generic icon), so the icon is set again once the window exists.
        if !self.icon_sent {
            self.icon_sent = true;
            context.send_viewport_cmd(egui::ViewportCommand::Icon(Some(std::sync::Arc::new(
                kit::window_icon_data(),
            ))));
        }
        if self.setup.is_some() {
            self.show_setup(context);
            resize_from_the_edges(context);
            return;
        }
        self.index_store.wake = context.clone();
        self.poll_index_store(context);
        if self.needs_probe {
            self.needs_probe = false;
            self.start_probing(context);
        }
        self.poll_library_scan(context);
        self.poll_probes();
        if self.needs_thumbnails {
            self.needs_thumbnails = false;
            self.start_thumbnails(context);
        }
        self.poll_thumbnails();
        self.thumbs.begin_frame();
        self.capture_frame(context);
        self.game_data.sync(
            &self.recording.game_integrations,
            self.settings_path.as_deref(),
            context,
        );
        self.sync_presence(context);
        self.sync_playback(context);
        self.editor.poll(context);
        if self.editor.take_recovered() {
            // Stay where the person is (the library, on launch) and say the draft is waiting;
            // the Editor tab reopens it as it was.
            self.editor.open = false;
            self.capture.notice = Some(capture::Notice {
                text: "Your editor draft was restored. Open the Editor to continue.".into(),
                error: false,
                at: std::time::Instant::now(),
                path: None,
            });
        }
        if self.editor.take_saved().is_some() {
            self.scan_library();
        }
        self.poll_find(context);
        self.draw_screen(context);
        self.draw_focus_ring(context);
        self.hide_to_tray(context);
        self.sync_taskbar_button(context);
        self.poll_update(context);
        if !matches!(self.page, Page::Settings) {
            self.settings_snapshot = None;
        }
        self.poll_tray(context);
        self.saved_toast_frame(context);
        self.index_save_status(context);
        resize_from_the_edges(context);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.presence.shutdown();
        self.editor.flush_recovery();
        self.index_store.finish();
    }
}

impl ClipCraftApp {
    /// Runs what the tray asked for and draws its menu while one is open.
    fn poll_tray(&mut self, context: &egui::Context) {
        let recording = self.capture.is_active();
        let status = tray_menu::Status {
            recording,
            text: match (recording, self.capture.game_recording_name()) {
                (true, Some(game)) => format!("Recording {game}"),
                (true, None) => "Recording".to_owned(),
                (false, _) if self.capture.game_capture_paused() => {
                    "Game capture paused".to_owned()
                }
                (false, _) => "Not recording".to_owned(),
            },
        };
        let mut actions = Vec::new();
        if let Some(tray) = self.tray.as_ref() {
            while let Some(action) = tray.next_action() {
                actions.push(action);
            }
        }
        if let Some(menu) = self.tray_menu.as_mut() {
            let (choice, open) = menu.show(context, &status);
            actions.extend(choice);
            if !open {
                self.tray_menu = None;
            }
        }
        for action in actions {
            match action {
                tray::Action::Menu { x, y } => {
                    self.tray_menu = Some(tray_menu::TrayMenu::at(context, x, y));
                }
                tray::Action::Open => tray::show(context),
                tray::Action::ToggleRecording => {
                    if recording {
                        self.stop_capture();
                    } else {
                        self.start_capture();
                    }
                }
                tray::Action::SaveClip => self.save_clip(),
                tray::Action::Screenshot => self.take_screenshot(),
                tray::Action::OpenClipsFolder => {
                    if let Some(folder) = self.clips_folder() {
                        self.reveal_media(folder);
                    }
                }
                tray::Action::OpenLibrary => {
                    self.page = Page::Library;
                    tray::show(context);
                }
                tray::Action::OpenSettings => {
                    self.page = Page::Settings;
                    tray::show(context);
                }
                tray::Action::Quit => tray::Tray::quit(context),
            }
        }
    }

    /// Turns a close into a minimize while the tray icon is there to bring the window back.
    /// Minimized, not hidden: a hidden window stops getting frames, which would stall capture.
    fn hide_to_tray(&mut self, context: &egui::Context) {
        if self.tray.is_some()
            && self.recording.close_to_tray
            && !self.index_store.closing
            && !tray::Tray::quitting()
            && context.input(|i| i.viewport().close_requested())
        {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    }

    /// While the window is minimized to the tray, the tray icon is the way back, so the taskbar
    /// button goes away. Anything else brings it back.
    fn sync_taskbar_button(&mut self, context: &egui::Context) {
        let hidden = self.tray.is_some()
            && self.recording.close_to_tray
            && clipcraft_platform::window_minimized("ClipCraft");
        if !hidden && !self.taskbar_hidden {
            return;
        }
        if hidden != self.taskbar_hidden
            && clipcraft_platform::set_taskbar_button("ClipCraft", !hidden)
        {
            self.taskbar_hidden = hidden;
        }
        // A minimized window draws no frames of its own, so look again soon.
        if hidden != self.taskbar_hidden || hidden {
            context.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn sync_presence(&mut self, context: &egui::Context) {
        use clipcraft_engine::presence::{PresenceContext, PresenceMode, integration_for_name};
        let mode = if self.editor.open {
            PresenceMode::Editing
        } else if self.capture.is_active() {
            PresenceMode::Clipping
        } else {
            PresenceMode::Browsing
        };
        let game_name = if mode == PresenceMode::Clipping {
            self.capture.game_recording_name().map(str::to_owned)
        } else {
            None
        };
        let snapshot = game_name
            .as_deref()
            .and_then(integration_for_name)
            .and_then(|game| {
                if let clipcraft_platform::game_state::IntegrationStatus::Live(snapshot) =
                    self.game_data.statuses.get(game)
                {
                    Some(Box::new(snapshot.clone()))
                } else {
                    None
                }
            });
        self.presence.sync(
            &self.recording.discord_presence,
            PresenceContext {
                mode,
                game_name,
                snapshot,
            },
            context,
        );
    }
    /// Shows which control the keyboard is on: a ring around the focused widget, drawn on top of
    /// everything, only after the keyboard (not the mouse) moved the focus.
    fn draw_focus_ring(&mut self, context: &egui::Context) {
        let (navigated, clicked) = context.input(|input| {
            (
                [
                    egui::Key::Tab,
                    egui::Key::ArrowUp,
                    egui::Key::ArrowDown,
                    egui::Key::Home,
                    egui::Key::End,
                    egui::Key::PageUp,
                    egui::Key::PageDown,
                ]
                .into_iter()
                .any(|key| input.key_pressed(key)),
                input.pointer.any_pressed(),
            )
        });
        if navigated {
            self.keyboard_focus = true;
        } else if clicked {
            self.keyboard_focus = false;
        }
        if !self.keyboard_focus {
            return;
        }
        let Some(id) = context.memory(|memory| memory.focused()) else {
            return;
        };
        let Some(response) = context.read_response(id) else {
            return;
        };
        let painter = context.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("focus-ring"),
        ));
        let rect = response.rect.expand(3.0);
        let rounding = egui::CornerRadius::same(10);
        // A dark halo under a bright ring, so it shows on both light and dark controls.
        painter.rect_stroke(
            rect.expand(1.5),
            rounding,
            Stroke::new(1.5_f32, Color32::from_black_alpha(200)),
            egui::StrokeKind::Outside,
        );
        painter.rect_stroke(
            rect,
            rounding,
            Stroke::new(2.0_f32, Color32::WHITE),
            egui::StrokeKind::Outside,
        );
    }

    /// Draws the current screen: the library, the player, or settings, with their chrome.
    fn draw_screen(&mut self, context: &egui::Context) {
        self.settings_state.begin_frame();
        // Fullscreen playback hides all chrome: the picture owns the whole window.
        let fullscreen = self
            .playback
            .as_ref()
            .is_some_and(playback::Playback::fullscreen);
        if fullscreen {
            self.settings_state.end_frame(&self.recording);
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
        self.notice_toast(context);
        self.poll_trash(context);
        self.poll_rename(context);

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
                self.update_banner(ui);
                if self.editor.open {
                    self.editor.show(ui, &mut self.thumbs);
                    self.editor.clip_picker(ui.ctx(), self.library.as_ref());
                    if !self.editor.open {
                        self.selected = None;
                    }
                } else if let Some(selected) = self.selected.clone() {
                    // Delete asks first and F2 renames, unless the person is typing somewhere.
                    let typing = ui.ctx().wants_keyboard_input()
                        || self.rename_draft.is_some()
                        || self.confirm_delete.is_some();
                    if !typing && ui.input(|input| input.key_pressed(egui::Key::Delete)) {
                        self.ask_to_delete(selected.clone());
                    }
                    if !typing && ui.input(|input| input.key_pressed(egui::Key::F2)) {
                        self.ask_to_rename(selected.clone());
                    }
                    self.clip_details(ui, &selected);
                } else if self.page == Page::Settings {
                    self.settings(ui);
                } else {
                    self.library_view(ui);
                }
            });
        self.delete_dialog(context);
        self.rename_dialog(context);
        self.bug_report_dialog(context);
        self.thumbs.end_frame();
        self.settings_state.end_frame(&self.recording);
    }

    /// Starts with the last scanned folder restored from the local index, if there is one.
    fn restored() -> Self {
        Self::restored_from(default_index_path())
    }

    fn restored_from(index_path: Option<PathBuf>) -> Self {
        let settings_path = index_path
            .as_ref()
            .map(|path| path.with_file_name("recording-settings.json"));
        let mut app = Self {
            settings_path,
            index_path,
            thumbnail_dir: default_thumbnail_dir(),
            ..Self::default()
        };
        if let Some(path) = app.settings_path.clone() {
            match execute(Command::LoadRecordingSettings { path }) {
                Ok(CommandOutput::RecordingSettings(settings)) => app.recording = *settings,
                Ok(_) => {}
                Err(error) => app.error = Some(error.to_string()),
            }
        }
        let Some(index_path) = app.index_path.clone() else {
            if !app.recording.setup_completed {
                app.setup = Some(onboarding::Setup::new(app.recording.clone()));
            }
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
        let restore_error = app.error.take();
        app.reconcile_clips_folder();
        if app.error.is_none() {
            app.error = restore_error;
        }
        if !app.recording.setup_completed {
            app.setup = Some(onboarding::Setup::new(app.recording.clone()));
        }
        app
    }

    /// The one folder clips live in: the library folder, which is also where new clips are saved.
    /// Before a library folder is chosen it is the folder set in the saved settings, and before
    /// that the default one.
    fn clips_folder(&self) -> Option<PathBuf> {
        let library = self.directory_text.trim();
        if !library.is_empty() {
            return Some(PathBuf::from(library));
        }
        if !self.recording.capture_folder.is_empty() {
            return Some(PathBuf::from(&self.recording.capture_folder));
        }
        default_capture_folder()
    }

    /// Makes the library folder and the saved "where clips are saved" setting the same folder.
    /// Older versions kept them apart: the library folder wins, and a folder that was only a save
    /// location becomes the library folder.
    fn reconcile_clips_folder(&mut self) {
        let library = self.directory_text.trim().to_owned();
        if library.is_empty() {
            if self.recording.capture_folder.is_empty() {
                return;
            }
            self.directory_text = self.recording.capture_folder.clone();
            self.scan_library();
        } else if self.recording.capture_folder != library {
            self.recording.capture_folder = library;
            self.save_recording_settings();
        }
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
        if self.editor.open {
            self.remember_playback_prefs();
            return;
        }
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
        let mut trimmed = false;
        if let Some(open) = self.playback.as_mut() {
            open.poll(context);
            trimmed = open.take_trimmed().is_some();
        }
        // A trimmed clip lands beside the original, so the library lists it.
        if trimmed {
            self.scan_library();
        }
    }

    /// Probes clips that have no saved facts on a worker thread so the window stays responsive.
    fn start_probing(&mut self, context: &egui::Context) {
        self.probe_positions = self
            .library
            .as_ref()
            .map(|l| {
                l.entries
                    .iter()
                    .enumerate()
                    .map(|(i, e)| (e.path.clone(), i))
                    .collect()
            })
            .unwrap_or_default();
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
                if let Some(entry) = self.library.as_mut().and_then(|library| {
                    if let Some(&index) = self.probe_positions.get(&path) {
                        library.entries.get_mut(index).filter(|e| e.path == path)
                    } else {
                        library.entries.iter_mut().find(|entry| entry.path == path)
                    }
                }) {
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
        let probes = std::mem::take(&mut self.unsaved_probes);
        if let Err(error) = self.index_store.submit(library_store::Write::Probes {
            index: index_path,
            directory: library.directory.clone(),
            probes: probes.clone(),
        }) {
            self.unsaved_probes = probes;
            self.error = Some(error);
        }
    }

    /// Refresh asynchronously; an unavailable tree retains the previous indexed view.
    fn refresh_restored_library(&mut self) {
        self.scan_library();
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
                // The whole strip drags the window and a double click maximises it; the buttons
                // drawn after this sit on top and take their own clicks.
                let strip = ui.max_rect();
                let drag = ui.interact(
                    strip,
                    ui.id().with("title-drag"),
                    egui::Sense::click_and_drag(),
                );
                if drag.drag_started_by(egui::PointerButton::Primary) {
                    context.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let maximised = context.input(|input| input.viewport().maximized);
                    context.send_viewport_cmd(egui::ViewportCommand::Maximized(
                        !maximised.unwrap_or(false),
                    ));
                }
                ui.horizontal_centered(|ui| {
                    ui.add_space(20.0);
                    let (logo, _) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::hover());
                    kit::brand_mark(ui, logo);
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("ClipCraft")
                            .color(TEXT)
                            .size(17.0)
                            .family(kit::semibold()),
                    );
                    ui.add_space(22.0);

                    // Leave the right of the bar for the size label and window buttons, so a long
                    // message is cut short instead of running underneath them.
                    let room = (ui.available_width() - 300.0).max(220.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(room, ui.available_height()),
                        Layout::left_to_right(Align::Center),
                        |ui| self.capture_controls(ui),
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        // Window buttons at the very right edge, rightmost first.
                        let maximised = context
                            .input(|input| input.viewport().maximized)
                            .unwrap_or(false);
                        if window_button(ui, WindowButton::Close) {
                            context.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        let toggle = if maximised {
                            WindowButton::Restore
                        } else {
                            WindowButton::Maximize
                        };
                        if window_button(ui, toggle) {
                            context.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximised));
                        }
                        if window_button(ui, WindowButton::Minimize) {
                            context.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        ui.add_space(18.0);
                        ui.label(
                            RichText::new(format!(
                                "{}p  ·  {} FPS",
                                self.recording.height, self.recording.frames_per_second
                            ))
                            .color(MUTED)
                            .size(13.0),
                        );
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
                        self.editor.close();
                        self.page = Page::Library;
                        self.selected = None;
                    }
                    if nav_button(ui, Icon::Scissors, "Editor", self.page == Page::Editor) {
                        self.page = Page::Editor;
                        self.selected = None;
                        self.editor.open = true;
                    }
                });
                // Settings sits at the bottom of the rail.
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    ui.add_space(18.0);
                    if nav_button(ui, Icon::Gear, "Settings", self.page == Page::Settings) {
                        self.editor.close();
                        self.page = Page::Settings;
                        self.selected = None;
                    }
                });
            });
    }

    fn library_view(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = &self.library {
            self.library_view
                .prepare(result, &self.filter, &self.search_text, self.sort);
        }
        let show_rail = self.library.is_some() && ui.available_width() >= 720.0;
        let mut rail_actions = Vec::new();
        if show_rail {
            let height = ui.available_height().max(0.0);
            let rail_width = 210.0_f32.min(ui.available_width() * 0.28);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.allocate_ui_with_layout(
                    Vec2::new(rail_width, height),
                    Layout::top_down(Align::Min),
                    |ui| {
                        if let Some(result) = self.library.as_ref() {
                            rail_actions = organize::rail(
                                ui,
                                result,
                                &self.library_view,
                                &self.filter,
                                &mut self.collection_edit,
                            );
                        }
                    },
                );
                ui.add_space(16.0);
                let width = ui.available_width().max(0.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(width, height),
                    Layout::top_down(Align::Min),
                    |ui| self.library_content(ui),
                );
            });
        } else {
            self.library_content(ui);
        }
        self.handle_rail(rail_actions);
    }

    fn handle_rail(&mut self, actions: Vec<RailAction>) {
        for action in actions {
            match action {
                RailAction::Select(filter) => self.filter = filter,
                RailAction::BeginCreate => self.collection_edit = Some(CollectionEdit::default()),
                RailAction::BeginRename(name) => {
                    self.collection_edit = Some(CollectionEdit {
                        original: Some(name.clone()),
                        text: name,
                        focused: false,
                    });
                }
                RailAction::Delete(name) => {
                    if self.filter == LibraryFilter::Collection(name.clone()) {
                        self.filter = LibraryFilter::All;
                    }
                    self.edit_library(MetaEdit::DeleteCollection { name });
                }
                RailAction::Cancel => self.collection_edit = None,
                RailAction::Commit => {
                    let Some(edit) = self.collection_edit.take() else {
                        continue;
                    };
                    if edit.text.trim().is_empty() {
                        continue;
                    }
                    match edit.original {
                        None => self.edit_library(MetaEdit::CreateCollection { name: edit.text }),
                        Some(from) => {
                            let to = edit.text.trim().to_owned();
                            if from != to {
                                let renamed =
                                    self.filter == LibraryFilter::Collection(from.clone());
                                self.edit_library(MetaEdit::RenameCollection {
                                    from,
                                    to: to.clone(),
                                });
                                if renamed && self.error.is_none() {
                                    self.filter = LibraryFilter::Collection(to);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Applies a favorite, game, or collection change on screen and saves it in the index.
    /// Asks for a folder and looks in it, and below it, for a clip that was moved.
    fn start_find(&mut self, from: String, name: String, size: u64) {
        if self.finding.is_some() {
            return;
        }
        let mut dialog = rfd::FileDialog::new().set_title(format!("Where might {name} be now?"));
        let library = self.directory_text.trim();
        if !library.is_empty() && PathBuf::from(library).is_dir() {
            dialog = dialog.set_directory(library);
        }
        let Some(folder) = dialog.pick_folder() else {
            return;
        };
        self.error = None;
        let (sender, receiver) = std::sync::mpsc::channel();
        let wanted = name.clone();
        std::thread::spawn(move || {
            let _ = sender.send(clipcraft_engine::find_moved_file(&wanted, size, &folder));
        });
        self.finding = Some(FindJob {
            from,
            name,
            receiver,
        });
    }

    /// Collects a finished search: relinks the clip when it was found, says so when it was not.
    fn poll_find(&mut self, context: &egui::Context) {
        let Some(job) = self.finding.as_ref() else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                context.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.finding = None;
                return;
            }
        };
        let Some(job) = self.finding.take() else {
            return;
        };
        match result.found {
            Some(path) => {
                self.edit_library(MetaEdit::RelinkFile {
                    from: job.from,
                    to: path.to_string_lossy().into_owned(),
                });
                // The clip view of a missing file now has a file to open.
                self.selected = None;
            }
            None => {
                self.error = Some(format!(
                    "Could not find {} in that folder (looked in {} folders). Choose another folder to try again.",
                    job.name, result.folders_searched
                ));
            }
        }
    }

    fn edit_library(&mut self, edit: MetaEdit) {
        if self.index_store.closing {
            return;
        }
        self.error = None;
        let Some(mut result) = self.library.clone() else {
            return;
        };
        if let Err(error) = apply_edit(&mut result.entries, &mut result.collections, &edit) {
            self.error = Some(error.to_string());
            return;
        }
        if let Some(index) = self.index_path.clone()
            && let Err(error) = self.index_store.submit(library_store::Write::Edit {
                index,
                directory: result.directory.clone(),
                edit,
            })
        {
            self.error = Some(error);
            return;
        }
        if let Some(open) = self.selected.as_mut()
            && let Some(fresh) = result.entries.iter().find(|entry| entry.path == open.path)
        {
            *open = fresh.clone();
        }
        self.probe_positions = result
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.path.clone(), i))
            .collect();
        self.library = Some(result);
        self.library_view.invalidate();
    }

    fn poll_index_store(&mut self, context: &egui::Context) {
        for event in self.index_store.poll() {
            if let library_store::Event::Fallback { generation, scan } = event {
                if generation == self.index_store.generation
                    && scan.directory == self.directory_text.trim()
                {
                    self.apply_library_scan(scan);
                }
                continue;
            }
            if let library_store::Event::Done {
                id,
                generation,
                scan: Some(mut scan),
                ..
            } = event
            {
                if generation == Some(self.index_store.generation)
                    && scan.directory == self.directory_text.trim()
                {
                    match self.index_store.rebase(id, &mut scan) {
                        Ok(()) => {
                            // Probe facts arriving after this write remain current for unchanged files.
                            if let Some(current) = self
                                .library
                                .as_ref()
                                .filter(|l| l.directory == scan.directory)
                            {
                                let by_path: std::collections::HashMap<_, _> =
                                    current.entries.iter().map(|e| (&e.path, e)).collect();
                                for entry in &mut scan.entries {
                                    if let Some(old) = by_path.get(&entry.path)
                                        && old.size_bytes == entry.size_bytes
                                        && old.modified_unix_seconds == entry.modified_unix_seconds
                                        && old.probe.is_some()
                                    {
                                        entry.probe = old.probe.clone();
                                    }
                                }
                            }
                            self.apply_library_scan(scan);
                        }
                        Err(error) => {
                            self.error = Some(format!(
                                "The refresh could not preserve pending changes: {error} Refresh again after saving."
                            ))
                        }
                    }
                }
                self.index_store.completed(id);
            }
        }
        if self.index_store.pending() == 0
            && !self.unsaved_probes.is_empty()
            && self.probe_receiver.is_none()
        {
            self.save_probes();
        }
        if self.index_store.pending() > 0 && self.index_store.error.is_none() {
            context.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }
    fn index_save_status(&mut self, context: &egui::Context) {
        if context.input(|i| i.viewport().close_requested()) && self.index_store.pending() > 0 {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.index_store.closing = true;
        }
        if self.index_store.closing && self.index_store.pending() == 0 {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.index_store.visible() {
            return;
        }
        egui::Window::new("Library changes")
            .id(egui::Id::new("library-save-status"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -18.0])
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                if let Some(error) = &self.index_store.error {
                    ui.label(error);
                    ui.horizontal(|ui| {
                        if ui.button("Retry saving").clicked() {
                            self.index_store.retry();
                        }
                        if self.index_store.closing
                            && ui.button("Close without saving pending changes").clicked()
                        {
                            self.index_store.discard();
                        }
                    });
                } else {
                    ui.label(if self.index_store.closing {
                        "Saving library changes before closing…"
                    } else {
                        "Saving library changes…"
                    });
                    ui.spinner();
                }
            });
    }

    fn library_content(&mut self, ui: &mut egui::Ui) {
        // Title row.
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(self.filter.title())
                    .color(TEXT)
                    .size(26.0)
                    .family(kit::semibold()),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if kit::labelled_button(ui, Icon::Folder, "Open folder", true, true)
                    .on_hover_text("Show your clips folder in the file manager")
                    .clicked()
                {
                    let folder = self.directory_text.trim();
                    if !folder.is_empty() {
                        self.reveal_media(PathBuf::from(folder));
                    }
                }
            });
        });
        ui.add_space(4.0);

        // Search, sort, and folder.
        // In a narrow window the folder pill and the search box both give way so they never touch.
        let row = ui.available_width();
        let folder_width = (row * 0.4).clamp(200.0, 340.0);
        let search_width = (row - folder_width - 190.0).clamp(90.0, 230.0);
        ui.horizontal(|ui| {
            egui::Frame::new()
                .fill(CARD)
                .corner_radius(egui::CornerRadius::same(18))
                .inner_margin(egui::Margin::symmetric(12, 5))
                .show(ui, |ui| {
                    let (icon, _) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                    kit::draw_icon(ui.painter(), icon.center(), Icon::Search, MUTED, 0.7);
                    let search = ui.add(
                        egui::TextEdit::singleline(&mut self.search_text)
                            .frame(false)
                            .hint_text("Search clips")
                            .desired_width(search_width),
                    );
                    kit::name_text_box(&search, "Search clips", &self.search_text);
                });
            self.sort_menu(ui);

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                egui::Frame::new()
                    .fill(CARD)
                    .corner_radius(egui::CornerRadius::same(18))
                    .inner_margin(egui::Margin::symmetric(12, 5))
                    .show(ui, |ui| {
                        // A fixed width, with the field filling it, so the whole pill is clickable.
                        ui.set_width(folder_width);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let (icon, _) =
                                ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                            kit::draw_icon(ui.painter(), icon.center(), Icon::Folder, MUTED, 0.75);
                            // The folder is chosen here, not typed: new clips are saved in it.
                            let shown = if self.directory_text.trim().is_empty() {
                                "Choose where clips are saved".to_owned()
                            } else {
                                self.directory_text.clone()
                            };
                            let button = ui
                                .add(
                                    egui::Label::new(RichText::new(shown).color(TEXT).size(13.0))
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_text("Where new clips are saved. Click to change it.")
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            button.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    "Change clips folder",
                                )
                            });
                            if button.clicked() {
                                self.choose_library_folder();
                            }
                        });
                    });
            });
        });
        ui.add_space(10.0);

        if self.scanning.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Scanning clips and subfolders…").color(MUTED));
                if ui.button("Cancel scan").clicked() {
                    self.scanning = None;
                }
            });
            ui.add_space(8.0);
        }
        if let Some(job) = self.finding.as_ref() {
            ui.label(
                RichText::new(format!("Looking for {}\u{2026}", job.name))
                    .color(MUTED)
                    .size(13.0),
            );
            ui.add_space(8.0);
        }
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
                // Search/sort controls above may have changed this frame.
                self.library_view
                    .prepare(result, &self.filter, &self.search_text, self.sort);
                let entries = &self.library_view.indices;
                if entries.is_empty() && self.search_text.trim().is_empty() {
                    let (headline, help) = self.filter.empty_message();
                    empty_panel(ui, Icon::Library, headline, help, None);
                } else if entries.is_empty() {
                    self.no_search_results(ui);
                } else {
                    card_action = self
                        .grid
                        .show(ui, result, &self.library_view, &mut self.thumbs);
                }
            }
            None => self.first_scan_state(ui),
        }
        if let Some(action) = card_action {
            match action {
                ClipCardAction::Open(entry) => self.selected = Some(*entry),
                ClipCardAction::SendToEditor(entry) => self.send_to_editor(*entry),
                ClipCardAction::CopyPath(path) => {
                    self.copy_media_path(ui.ctx(), PathBuf::from(path));
                }
                ClipCardAction::Reveal(path) => self.reveal_media(PathBuf::from(path)),
                ClipCardAction::SetFavorite { path, favorite } => {
                    self.edit_library(MetaEdit::SetFavorite { path, favorite });
                }
                ClipCardAction::FindFile(path, name, size) => self.start_find(path, name, size),
                ClipCardAction::RemoveFromLibrary(path) => {
                    self.edit_library(MetaEdit::RemoveFromLibrary { path });
                }
                ClipCardAction::Delete(entry) => self.ask_to_delete(*entry),
                ClipCardAction::Rename(entry) => self.ask_to_rename(*entry),
                ClipCardAction::SetCollection { path, name, add } => {
                    self.edit_library(if add {
                        MetaEdit::AddToCollection { path, name }
                    } else {
                        MetaEdit::RemoveFromCollection { path, name }
                    });
                }
            }
        }
    }

    fn sort_menu(&mut self, ui: &mut egui::Ui) {
        let label = self.sort.label();
        kit::pill_menu(ui, label, "Sort clips", |ui| {
            for sort in library_view::Sort::ALL {
                if ui
                    .selectable_label(self.sort == sort, sort.label())
                    .clicked()
                {
                    self.sort = sort;
                    ui.close();
                }
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

    fn send_to_editor(&mut self, mut entry: MediaEntry) {
        // Library duration may include audio padding. Resolve editor bounds on its import worker,
        // using the same reader as preview/export, before this reference can be appended.
        entry.probe = None;
        self.editor.receive(entry);
        self.editor.open = true;
        self.page = Page::Editor;
        self.selected = None;
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
        // New clips are saved in the folder the library shows.
        let before = self.recording.clone();
        let mut working = before.clone();
        working.capture_folder = self.directory_text.clone();
        self.apply_recording(before, working);
    }
}

/// Looking for a clip that was moved, on another thread.
struct FindJob {
    from: String,
    name: String,
    receiver: std::sync::mpsc::Receiver<clipcraft_engine::SearchResult>,
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
        ui.label(
            RichText::new(headline)
                .color(TEXT)
                .size(20.0)
                .family(kit::semibold()),
        );
        ui.label(RichText::new(help).color(MUTED));
        if let Some(button) = button {
            ui.add_space(8.0);
            clicked = kit::labelled_button(ui, Icon::Folder, button, true, true).clicked();
        }
    });
    clicked
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

/// The clips of the current view, searched and sorted.
#[cfg(test)]
fn filtered_entries<'a>(
    entries: &'a [MediaEntry],
    filter: &LibraryFilter,
    directory: &str,
    query: &str,
    sort_newest: bool,
) -> Vec<&'a MediaEntry> {
    let mut visible = visible_entries(entries, query, sort_newest);
    visible.retain(|entry| filter.matches(entry, directory));
    visible
}

#[cfg(test)]
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

#[cfg(test)]
fn compare_media_names(left: &MediaEntry, right: &MediaEntry) -> std::cmp::Ordering {
    left.title
        .to_lowercase()
        .cmp(&right.title.to_lowercase())
        .then_with(|| left.path.cmp(&right.path))
}

pub(crate) fn format_bytes(size: u64) -> String {
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
        ClipCraftApp, EngineError, LibraryFilter, MediaProbe, Page, ProbeMessage,
        RecordingSettings, SettingsTab, format_date, format_duration, kit, probe_paths,
        visible_entries,
    };
    use clipcraft_engine::{Container, Framing, LibraryScanResult, MediaEntry, OutputShape};
    use eframe::egui;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub(crate) fn media_entry(title: &str, modified_unix_seconds: Option<u64>) -> MediaEntry {
        MediaEntry {
            path: format!("/clips/{title}"),
            title: title.to_owned(),
            size_bytes: 100,
            modified_unix_seconds,
            extension: "mp4".to_owned(),
            offline: false,
            favorite: false,
            tags: Vec::new(),
            game: None,
            recorded_game: None,
            collections: Vec::new(),
            probe: None,
        }
    }

    pub(crate) fn finish_scan(app: &mut ClipCraftApp) {
        let context = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.scanning.is_some() || app.index_store.pending() > 0 {
            app.poll_index_store(&context);
            assert!(
                std::time::Instant::now() < deadline,
                "scan worker timed out"
            );
            app.poll_library_scan(&context);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    pub(crate) fn temporary_directory() -> PathBuf {
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
    fn static_keyboard_focus_ring_does_not_schedule_continuous_repaints() {
        let context = crate::kit::test_context();
        let mut app = ClipCraftApp {
            keyboard_focus: true,
            ..Default::default()
        };
        let mut delay = std::time::Duration::ZERO;
        for _ in 0..8 {
            let output = context.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let button = ui.button("Focused control");
                    if ui.memory(|m| m.focused()).is_none() {
                        button.request_focus();
                    }
                });
                app.draw_focus_ring(ctx);
            });
            delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        }
        assert!(app.keyboard_focus);
        assert!(context.memory(|m| m.focused()).is_some());
        assert!(
            delay > std::time::Duration::from_secs(1),
            "static ring requested repaint after {delay:?}"
        );
    }

    #[test]
    fn favorites_and_collections_change_the_view_and_survive_a_relaunch() {
        use super::filtered_entries;
        use clipcraft_engine::MetaEdit;
        let directory = temporary_directory();
        let clips = directory.join("clips");
        fs::create_dir(&clips).unwrap();
        for name in ["Hades 2024.01.01.mp4", "Celeste 2024.01.02.mp4"] {
            fs::write(clips.join(name), b"not a real video").unwrap();
        }
        let index_path = directory.join("library-index.json");
        let mut app = ClipCraftApp::restored_from(Some(index_path.clone()));
        app.directory_text = clips.to_string_lossy().into_owned();
        app.scan_library();
        finish_scan(&mut app);
        let hades = clips
            .join("Hades 2024.01.01.mp4")
            .to_string_lossy()
            .into_owned();

        app.edit_library(MetaEdit::SetFavorite {
            path: hades.clone(),
            favorite: true,
        });
        app.edit_library(MetaEdit::CreateCollection {
            name: "Best".to_owned(),
        });
        app.edit_library(MetaEdit::AddToCollection {
            path: hades.clone(),
            name: "Best".to_owned(),
        });
        assert!(app.error.is_none(), "{:?}", app.error);

        let result = app.library.as_ref().unwrap();
        let count = |filter: &LibraryFilter| {
            filtered_entries(&result.entries, filter, &result.directory, "", false).len()
        };
        assert_eq!(count(&LibraryFilter::All), 2);
        assert_eq!(count(&LibraryFilter::Favorites), 1);
        assert_eq!(count(&LibraryFilter::Collection("Best".to_owned())), 1);
        assert_eq!(count(&LibraryFilter::Game("hades".to_owned())), 1);
        assert_eq!(count(&LibraryFilter::Game("Nope".to_owned())), 0);

        // A refused edit shows a message and changes nothing.
        app.edit_library(MetaEdit::CreateCollection {
            name: "best".to_owned(),
        });
        assert!(app.error.is_some());
        app.error = None;

        finish_scan(&mut app);
        let relaunched = ClipCraftApp::restored_from(Some(index_path));
        let restored = relaunched.library.as_ref().unwrap();
        assert_eq!(restored.collections, ["Best"]);
        assert!(
            restored
                .entries
                .iter()
                .any(|entry| entry.favorite && entry.collections == ["Best"])
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn the_library_folder_is_the_folder_new_clips_are_saved_in() {
        let directory = temporary_directory();
        let mut app = ClipCraftApp::default();
        // Nothing chosen: the default folder is both.
        assert_eq!(
            app.clips_folder(),
            clipcraft_engine::default_capture_folder()
        );
        // Choosing a library folder also chooses where clips are saved.
        app.set_library_folder(Some(directory.clone()));
        finish_scan(&mut app);
        assert_eq!(app.clips_folder(), Some(directory.clone()));
        assert_eq!(
            PathBuf::from(&app.recording.capture_folder),
            PathBuf::from(&app.directory_text)
        );
        // An older settings file that names another save folder yields to the library folder.
        app.recording.capture_folder = "/somewhere/else".to_owned();
        app.reconcile_clips_folder();
        assert_eq!(app.recording.capture_folder, app.directory_text);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_folder_that_was_only_a_save_location_becomes_the_library_folder() {
        let directory = temporary_directory();
        let mut app = ClipCraftApp::default();
        app.recording.capture_folder = directory.to_string_lossy().into_owned();
        assert!(app.directory_text.is_empty());
        app.reconcile_clips_folder();
        assert_eq!(PathBuf::from(&app.directory_text), directory);
        assert_eq!(app.clips_folder(), Some(directory.clone()));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn forcing_window_capture_without_a_game_in_front_says_how_to_use_it() {
        let mut app = ClipCraftApp::default();
        app.recording.force_window_capture = true;
        app.recording.source = clipcraft_engine::CaptureSource::Desktop;
        // With no window in front the person is told what to do; a machine that has one (a CI
        // runner's desktop) records it, which is what the option is for.
        match app.capture_target_id(None) {
            Err(message) => assert!(
                message.contains("hotkey") && message.contains("focus"),
                "{message}"
            ),
            Ok(id) => assert!(id.starts_with("window:"), "{id}"),
        }
        // Without the option, the desktop is simply the first screen (or nothing, with no display).
        app.recording.force_window_capture = false;
        if let Err(message) = app.capture_target_id(None) {
            assert!(!message.contains("hotkey"), "{message}");
        }
    }

    #[test]
    fn recording_choices_are_saved_and_restored_on_the_next_launch() {
        let directory = temporary_directory();
        let index_path = directory.join("library-index.json");
        let mut app = ClipCraftApp::restored_from(Some(index_path.clone()));
        assert_eq!(app.recording, RecordingSettings::default());
        app.recording.container = Container::Mkv;
        app.recording.shape = OutputShape::Wide16x9;
        app.recording.framing = Framing::Letterbox;
        app.save_recording_settings();
        finish_scan(&mut app);
        let relaunched = ClipCraftApp::restored_from(Some(index_path));
        assert_eq!(relaunched.recording.container, Container::Mkv);
        assert_eq!(relaunched.recording.framing, Framing::Letterbox);
        let _ = fs::remove_dir_all(directory);
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
        finish_scan(&mut app);
        assert_eq!(app.directory_text, directory.to_string_lossy());
        assert_eq!(
            app.library.as_ref().map(|result| result.entries.len()),
            Some(1)
        );
        assert_eq!(app.error, None);

        app.error = Some("prior message".to_owned());
        app.set_library_folder(None);
        finish_scan(&mut app);
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
        finish_scan(&mut first);
        assert_eq!(first.error, None);

        fs::remove_file(clips.join("gone.mp4")).expect("remove clip");
        let mut second = ClipCraftApp::restored_from(Some(index_path.clone()));
        finish_scan(&mut second);
        assert_eq!(second.directory_text, clips.to_string_lossy());
        assert_eq!(second.library.as_ref().map(|l| l.entries.len()), Some(2));
        let entries = &second.library.as_ref().expect("library").entries;
        assert_eq!(entries.iter().filter(|entry| entry.offline).count(), 1);
        assert!(entries.iter().any(|e| e.title == "gone.mp4" && e.offline));

        // An unreadable folder marks every remembered clip offline instead of hiding them.
        fs::remove_dir_all(&clips).expect("remove clips folder");
        let mut unplugged = ClipCraftApp::restored_from(Some(index_path.clone()));
        finish_scan(&mut unplugged);
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
        finish_scan(&mut app);
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
        finish_scan(&mut app);
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
        let context = kit::test_context();
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
                collections: vec!["Clutches".to_owned()],
            });
            app.error = Some("A scan problem".to_owned());
            draw(&mut app, &context, size, 3);
            app.search_text = "no clip has this title".to_owned();
            draw(&mut app, &context, size, 2);
            app.search_text.clear();
            app.sort = crate::library_view::Sort::Newest;
            draw(&mut app, &context, size, 2);

            // An empty folder.
            app.library = Some(LibraryScanResult {
                directory: "/clips".to_owned(),
                entries: Vec::new(),
                persisted: false,
                collections: vec!["Clutches".to_owned()],
            });
            draw(&mut app, &context, size, 2);

            // Clip details, with and without a probe, and the player area that follows them.
            app.library = Some(LibraryScanResult {
                directory: "/clips".to_owned(),
                entries: awkward_entries(),
                persisted: false,
                collections: vec!["Clutches".to_owned()],
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

            // Every library view, with a collection being named.
            app.page = Page::Library;
            app.library = Some(LibraryScanResult {
                directory: "/clips".to_owned(),
                entries: awkward_entries(),
                persisted: false,
                collections: vec!["Clutches".to_owned(), "x".repeat(200)],
            });
            app.collection_edit = Some(crate::organize::CollectionEdit::default());
            for filter in [
                LibraryFilter::Favorites,
                LibraryFilter::Collection("Clutches".to_owned()),
                LibraryFilter::Game("short.mp4".to_owned()),
                LibraryFilter::All,
            ] {
                app.filter = filter;
                draw(&mut app, &context, size, 2);
            }
            app.collection_edit = None;
            app.page = Page::Settings;

            // Every settings tab, with its advanced controls on and custom quality shown.
            app.recording.game_rules = vec![clipcraft_engine::GameMatchRule::new(
                "A custom game",
                ["custom-game.exe"],
            )];
            app.recording.quality = clipcraft_engine::QualityMode::Custom;
            app.recording.push_to_talk = true;
            app.recording.hotkeys.save_clip.clear();
            for tab in [
                SettingsTab::Overview,
                SettingsTab::General,
                SettingsTab::Storage,
                SettingsTab::Hotkeys,
                SettingsTab::Games,
                SettingsTab::Capture,
                SettingsTab::Video,
                SettingsTab::Audio,
                SettingsTab::Replay,
                SettingsTab::About,
            ] {
                app.settings_tab = tab;
                draw(&mut app, &context, size, 3);
            }
        }
    }

    /// Runs frames of the whole UI with the accessibility tree switched on and returns the
    /// interactive controls a screen reader would find: role and name.
    fn controls(
        app: &mut ClipCraftApp,
        context: &egui::Context,
    ) -> Vec<(egui::accesskit::Role, String)> {
        use egui::accesskit::Role;
        context.enable_accesskit();
        let mut found = Vec::new();
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(1440.0, 900.0),
                )),
                ..Default::default()
            };
            let output = context.run(input, |context| {
                kit::apply_theme(context);
                app.thumbs.begin_frame();
                app.draw_screen(context);
            });
            if let Some(update) = output.platform_output.accesskit_update {
                found.clear();
                for (_, node) in update.nodes {
                    if matches!(
                        node.role(),
                        Role::Button
                            | Role::CheckBox
                            | Role::RadioButton
                            | Role::Slider
                            | Role::ComboBox
                            | Role::TextInput
                            | Role::SpinButton
                            | Role::Switch
                    ) {
                        let name = node
                            .label()
                            .or_else(|| node.value())
                            .unwrap_or_default()
                            .to_owned();
                        found.push((node.role(), name));
                    }
                }
            }
        }
        found
    }

    #[test]
    fn every_control_on_every_screen_has_a_name_a_screen_reader_can_say() {
        let context = kit::test_context();
        let mut app = ClipCraftApp {
            library: Some(LibraryScanResult {
                directory: "/clips".to_owned(),
                entries: awkward_entries(),
                persisted: false,
                collections: vec!["Clutches".to_owned()],
            }),
            ..ClipCraftApp::default()
        };
        let mut unnamed = Vec::new();
        let mut total = 0;
        let mut check = |label: &str, controls: Vec<(egui::accesskit::Role, String)>| {
            total += controls.len();
            for (role, name) in controls {
                if name.trim().is_empty() {
                    unnamed.push(format!("{label}: an unnamed {role:?}"));
                }
            }
        };
        check("library", controls(&mut app, &context));
        app.selected = Some(awkward_entries().remove(0));
        check("clip", controls(&mut app, &context));
        app.selected = None;
        app.page = Page::Settings;
        app.recording.game_rules = vec![clipcraft_engine::GameMatchRule::new(
            "A custom game",
            ["custom-game.exe"],
        )];
        app.recording.quality = clipcraft_engine::QualityMode::Custom;
        for tab in [
            SettingsTab::Overview,
            SettingsTab::General,
            SettingsTab::Storage,
            SettingsTab::Hotkeys,
            SettingsTab::Games,
            SettingsTab::Capture,
            SettingsTab::Video,
            SettingsTab::Audio,
            SettingsTab::Replay,
            SettingsTab::About,
        ] {
            app.settings_tab = tab;
            check(&format!("settings {tab:?}"), controls(&mut app, &context));
        }
        assert!(
            total > 40,
            "only {total} controls were found; the tree may be empty"
        );
        assert!(
            unnamed.is_empty(),
            "controls with no name:\n{}",
            unnamed.join("\n")
        );
    }
}
