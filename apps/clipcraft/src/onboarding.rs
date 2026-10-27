//! First launch uses a draft; nothing captures or publishes while choices are being made.
use crate::{ClipCraftApp, kit, settings};
use clipcraft_engine::{Command, CommandOutput, RecordingSettings, execute};
use eframe::egui;

pub(crate) struct Setup {
    step: usize,
    draft: RecordingSettings,
    controls: settings::SettingsState,
    error: Option<String>,
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

const STEPS: [&str; 4] = ["Welcome", "Storage", "Capture & sound", "Shortcuts"];
const COLUMN: f32 = 720.0;

/// A rounded text button: lime when it is the way forward, quiet otherwise.
fn pill_button(ui: &mut egui::Ui, label: &str, primary: bool) -> egui::Response {
    let font = egui::FontId::new(14.5, kit::medium());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, egui::Color32::PLACEHOLDER);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(galley.size().x + 44.0, 40.0),
        egui::Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let (fill, ink) = match (primary, response.hovered()) {
        (true, false) => (kit::LIME, egui::Color32::BLACK),
        (true, true) => (egui::Color32::from_rgb(210, 255, 90), egui::Color32::BLACK),
        (false, false) => (kit::CONTROL, kit::TEXT),
        (false, true) => (kit::CONTROL_HOVER, kit::TEXT),
    };
    ui.painter().rect_filled(rect, 12.0, fill);
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

/// The four steps as numbered dots joined by a line, the finished ones filled.
fn stepper(ui: &mut egui::Ui, current: usize) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 62.0), egui::Sense::hover());
    let painter = ui.painter();
    let count = STEPS.len();
    let slot = width / count as f32;
    let centre =
        |index: usize| egui::pos2(rect.left() + slot * (index as f32 + 0.5), rect.top() + 16.0);
    for index in 0..count - 1 {
        let (from, to) = (centre(index), centre(index + 1));
        painter.line_segment(
            [from + egui::vec2(18.0, 0.0), to - egui::vec2(18.0, 0.0)],
            egui::Stroke::new(
                2.0_f32,
                if index < current {
                    kit::LIME
                } else {
                    kit::BORDER
                },
            ),
        );
    }
    for (index, name) in STEPS.into_iter().enumerate() {
        let at = centre(index);
        let (fill, ring, ink) = if index < current {
            (kit::LIME, kit::LIME, egui::Color32::BLACK)
        } else if index == current {
            (kit::PANEL, kit::LIME, kit::LIME)
        } else {
            (kit::PANEL, kit::BORDER, kit::FAINT)
        };
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

/// A soft lime glow: stacked, fading circles.
fn glow(painter: &egui::Painter, centre: egui::Pos2, radius: f32, strength: f32) {
    const RINGS: usize = 64;
    let alpha = (strength * 255.0 / RINGS as f32 * 2.2).clamp(1.0, 255.0) as u8;
    for ring in 0..RINGS {
        let t = ring as f32 / RINGS as f32;
        painter.circle_filled(
            centre,
            radius * (1.0 - t),
            egui::Color32::from_rgba_unmultiplied(190, 255, 43, alpha),
        );
    }
}

/// A small card with an icon, a heading and a sentence, for the welcome page.
fn tile(ui: &mut egui::Ui, icon: kit::Icon, heading: &str, body: &str) {
    egui::Frame::new()
        .fill(kit::CARD)
        .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
        .corner_radius(16.0)
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(150.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                ui.painter().circle_filled(
                    rect.center(),
                    20.0,
                    egui::Color32::from_rgba_unmultiplied(190, 255, 43, 30),
                );
                kit::draw_icon(ui.painter(), rect.center(), icon, kit::LIME, 0.85);
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(heading)
                        .color(kit::TEXT)
                        .size(15.5)
                        .family(kit::semibold()),
                );
                ui.add_space(4.0);
                ui.label(egui::RichText::new(body).color(kit::MUTED).size(13.0));
            });
        });
}

/// The first page: a glowing mark, a big hello and three reasons to carry on.
fn welcome(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(150.0, 130.0), egui::Sense::hover());
        glow(ui.painter(), rect.center(), 110.0, 0.3);
        kit::brand_mark(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(92.0, 92.0)),
        );
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Welcome to ClipCraft")
                .color(kit::TEXT)
                .size(40.0)
                .family(kit::semibold()),
        );
        ui.add_space(8.0);
        ui.scope(|ui| {
            ui.set_max_width(540.0);
            ui.label(
                egui::RichText::new(
                    "Instant game clips, saved on your own computer. Setup takes about a minute, and everything can be changed later in Settings.",
                )
                .color(kit::MUTED)
                .size(15.0),
            );
        });
    });
    ui.add_space(30.0);
    ui.columns(3, |columns| {
        tile(
            &mut columns[0],
            kit::Icon::Record,
            "Always ready",
            "A replay buffer keeps the last moments, so you never have to start recording first.",
        );
        tile(
            &mut columns[1],
            kit::Icon::Folder,
            "Private",
            "No account, no upload, no internet needed. Clips never leave this computer.",
        );
        tile(
            &mut columns[2],
            kit::Icon::Scissors,
            "Yours to edit",
            "Play, trim and export clips, or cut them together in the editor.",
        );
    });
}

impl ClipCraftApp {
    pub(crate) fn show_setup(&mut self, context: &egui::Context) {
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
        let (mut finish, mut back, mut next) = (false, false, false);
        egui::TopBottomPanel::bottom("setup-footer")
            .exact_height(76.0)
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
                        egui::vec2(width, 40.0),
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
                                        _ if last => "Finish setup",
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
        setup.controls.begin_frame();
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(kit::CANVAS)
                    .inner_margin(egui::Margin::symmetric(24, 0)),
            )
            .show(context, |ui| {
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
                                setup_body(ui, setup);
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

/// The heading, stepper and choices for the current step.
fn setup_body(ui: &mut egui::Ui, setup: &mut Setup) {
    use crate::rows::{Meta, Rows};
    ui.add_space(28.0);
    stepper(ui, setup.step);
    ui.add_space(26.0);
    let (title, lead) = [
        (
            "Welcome to ClipCraft",
            "Instant game clips, saved on your own computer. Setup takes about a minute, and everything can be changed later in Settings.",
        ),
        (
            "Where should clips go?",
            "ClipCraft keeps the last moments in memory and writes a clip only when you save one.",
        ),
        (
            "What should it capture?",
            "Pick what ends up in your clips. Nothing starts recording until you press Start clipping.",
        ),
        (
            "Choose your shortcuts",
            "Press the save shortcut during a game and the last moments become a clip. Add up to five per action.",
        ),
    ][setup.step];
    if setup.step == 0 {
        welcome(ui);
    } else {
        ui.label(
            egui::RichText::new(title)
                .color(kit::TEXT)
                .size(30.0)
                .family(kit::semibold()),
        );
        ui.add_space(4.0);
        ui.label(egui::RichText::new(lead).color(kit::MUTED).size(14.5));
        ui.add_space(22.0);
    }
    match setup.step {
        0 => {}
        1 => {
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
            rows.section(ui, "Replay and space");
            rows.range(
                ui,
                &mut setup.draft,
                Meta::new(
                    "Replay length",
                    "How far back a saved clip reaches.",
                    "replay seconds",
                ),
                |s| &mut s.replay_seconds,
                5..=120,
                5,
                |v| format!("{v} s"),
            );
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
        }
        2 => {
            let mut rows = Rows::new("");
            rows.section(ui, "Video");
            rows.toggle(
                ui,
                &mut setup.draft,
                Meta::new(
                    "Only clip when a game is running",
                    "ClipCraft waits for a game it recognizes. Turn off to record your screen or a window you choose in Settings.",
                    "game only",
                ),
                |s| &mut s.game_only_capture,
            );
            rows.section(ui, "Sound");
            rows.toggle(
                ui,
                &mut setup.draft,
                Meta::new(
                    "Game and system sound",
                    "Everything you hear.",
                    "game audio",
                ),
                |s| &mut s.capture_game_audio,
            );
            rows.toggle(
                ui,
                &mut setup.draft,
                Meta::new(
                    "My microphone",
                    "Your voice goes into the clip. Pick the device and test it in Settings.",
                    "microphone",
                ),
                |s| &mut s.capture_microphone,
            );
            rows.toggle(
                ui,
                &mut setup.draft,
                Meta::new(
                    "Reduce microphone noise",
                    "Quiets background noise in your voice, offline.",
                    "noise suppression",
                ),
                |s| &mut s.microphone_noise_suppression,
            );
            rows.finish(ui);
        }
        _ => settings::hotkeys_tab(ui, &mut setup.draft, &mut setup.controls),
    }
    if let Some(error) = setup.error.as_ref() {
        ui.add_space(12.0);
        ui.colored_label(kit::DANGER, error);
    }
    ui.add_space(28.0);
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
