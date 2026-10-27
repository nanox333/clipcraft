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

impl ClipCraftApp {
    pub(crate) fn show_setup(&mut self, context: &egui::Context) {
        egui::TopBottomPanel::top("setup-title")
            .frame(egui::Frame::new().fill(kit::PANEL).inner_margin(12.0))
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("ClipCraft");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            context.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
            });
        let Some(setup) = self.setup.as_mut() else {
            return;
        };
        let mut finish = false;
        setup.controls.begin_frame();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(kit::CANVAS).inner_margin(32.0)).show(context, |ui| {
            let width = ui.available_width().min(900.0);
            ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.set_max_width(width);
            egui::Frame::new().fill(kit::PANEL).corner_radius(16.0).inner_margin(28.0).show(ui, |ui| {
            ui.set_width((width - 56.0).max(240.0));
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                for (index, name) in ["Welcome", "Storage", "Capture & audio", "Shortcuts"].into_iter().enumerate() {
                    ui.label(egui::RichText::new(format!("{}  {name}", index + 1)).color(if index == setup.step { kit::LIME } else { kit::MUTED }).size(12.0));
                    if index < 3 { ui.label(egui::RichText::new(" / ").color(kit::FAINT)); }
                }
            });
            ui.add_space(24.0);
            if setup.step == 0 {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 64.0), egui::Sense::hover());
                kit::brand_mark(ui, rect);
                ui.add_space(16.0);
            }
            ui.label(egui::RichText::new("WELCOME TO CLIPCRAFT").color(kit::LIME).size(13.0));
            ui.label(egui::RichText::new(["Your moments, on your computer", "Make room for your clips", "Choose what to capture", "Make the shortcut yours"][setup.step]).size(28.0).family(kit::semibold()).color(kit::TEXT));
            ui.label(format!("Step {} of 4 · Everything here can be changed in Settings", setup.step + 1));
            ui.add_space(20.0);
            egui::ScrollArea::vertical().id_salt(("first-run-body",setup.step)).max_height((ui.available_height()-110.0).max(100.0)).show(ui, |ui| {
                match setup.step {
                    0 => {
                        ui.label("Save the last moments of a game with a shortcut, then play, trim and export them locally.");
                        ui.add_space(16.0);
                        ui.label("No account, uploads or internet connection are needed. Discord presence is optional and off by default.");
                        ui.add_space(16.0);
                        ui.label("This setup does not start recording. Use Start clipping when you are ready.");
                    }
                    1 => {
                        ui.label("Clips folder");
                        ui.horizontal(|ui| {
                            let width = (ui.available_width() - 110.0).max(160.0);
                            ui.add(egui::TextEdit::singleline(&mut setup.draft.capture_folder).desired_width(width));
                            if ui.button("Browse…").clicked()
                                && let Some(path) = rfd::FileDialog::new().set_title("Choose your clips folder").pick_folder() {
                                setup.draft.capture_folder = path.to_string_lossy().into_owned();
                            }
                        });
                        ui.label("The folder is created when you first save a clip. Existing files are kept.");
                        ui.add_space(16.0);
                        ui.add(egui::Slider::new(&mut setup.draft.replay_seconds, 5..=120).text("seconds kept in replay"));
                        ui.add(egui::Slider::new(&mut setup.draft.storage_limit_gb, 0..=500).text("GB storage limit (0 = no limit)"));
                        ui.checkbox(&mut setup.draft.delete_oldest_when_full, "Delete oldest clips when the limit is reached");
                    }
                    2 => {
                        ui.checkbox(&mut setup.draft.game_only_capture, "Only arm clipping when a recognized game is running");
                        ui.label("Turn this off to capture your chosen screen or window. Capture source and quality are available in Settings.");
                        ui.add_space(16.0);
                        ui.checkbox(&mut setup.draft.capture_game_audio, "Include game / system sound");
                        ui.checkbox(&mut setup.draft.capture_microphone, "Include my microphone");
                        ui.add_enabled_ui(setup.draft.capture_microphone, |ui| {
                            ui.checkbox(&mut setup.draft.microphone_noise_suppression, "Reduce microphone background noise");
                            ui.checkbox(&mut setup.draft.microphone_mono, "Convert microphone to mono");
                        });
                        ui.label("Audio uses your default devices. Choose another device and listen with Mic Test in Settings → Audio.");
                    }
                    _ => settings::hotkeys_tab(ui, &mut setup.draft, &mut setup.controls),
                }
            });
            ui.add_space(12.0);
            if let Some(error) = setup.error.as_ref() { ui.colored_label(kit::DANGER, error); }
            ui.horizontal(|ui| {
                if ui.add_enabled(setup.step > 0, egui::Button::new("Back").min_size(egui::vec2(70.0,36.0))).clicked() { setup.step = setup.step.saturating_sub(1); setup.controls.capturing = None; }
                if ui.button("Skip setup").on_hover_text("Save the settings currently shown and leave setup.").clicked() { finish = true; }
                if ui.add(egui::Button::new(egui::RichText::new(if setup.step == 3 { "Finish setup" } else { "Continue" }).color(egui::Color32::BLACK)).fill(kit::LIME).min_size(egui::vec2(140.0,36.0))).clicked() {
                    if setup.step == 3 { finish = true; } else { setup.step += 1; setup.controls.capturing = None; }
                }
            });
            });
            });
            });
        });
        setup.controls.end_frame(&setup.draft);
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
