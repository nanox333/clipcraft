//! The Settings screens: widgets in the Discord style and one function per tab.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use clipcraft_audio_out::{DeviceNames, MicTest, list_devices};
use clipcraft_engine::{
    AUDIO_KBPS, CaptureSource, Command, CommandOutput, Container, DesktopCaptureMode,
    EncoderPreset, Framing, GameMatchRule, HEIGHTS, HotkeyAction, OutputShape, QualityMode,
    RateControl, RecordingSettings, StorageUsage, VideoCodec, VideoEncoder, execute, format_bytes,
    normalize_hotkey,
};
use eframe::egui;
use egui::{Color32, RichText, Stroke, Vec2};

mod pages;
mod widgets;
pub(crate) use widgets::*;
mod games;
pub use games::*;
mod hotkeys;
pub use hotkeys::*;
mod storage;
pub use storage::*;
mod capture;
pub use capture::*;
pub(crate) use pages::*;

use crate::kit::{self, BORDER, CARD, CARD_HOVER, DANGER, FAINT, LIME, MUTED, TEXT};

/// Whether the graphics card can encode video, as found by trying it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum GpuProbe {
    /// Not tried yet.
    #[default]
    Unknown,
    /// Being tried.
    Checking,
    /// Works; the encoder's name.
    Ready(String),
    /// Does not work; why.
    Unavailable(String),
}

/// What the Settings screens keep between frames.
#[derive(Default)]
pub struct SettingsState {
    devices: Option<DeviceNames>,
    /// Outputs that can be recorded, as (saved name, name shown): PulseAudio's outputs on Linux
    /// (its monitors are what is recorded), the sound system's own list elsewhere.
    outputs: Vec<(String, String)>,
    mic: Option<MicTest>,
    mic_plan: Option<clipcraft_engine::MicrophoneTestPlan>,
    mic_visible: bool,
    mic_error: Option<String>,
    mic_shown: f32,
    pub(crate) capturing: Option<(HotkeyAction, usize)>,
    /// The clip-length shortcut waiting for its keys.
    pub(crate) capturing_clip_length: Option<usize>,
    pub(crate) hotkey_error: Option<String>,
    usage: Option<StorageUsage>,
    usage_for: Option<PathBuf>,
    usage_receiver: Option<mpsc::Receiver<StorageUsage>>,
    pub(crate) game_processes: Vec<clipcraft_platform::RunningProcess>,
    pub(crate) game_processes_loaded: bool,
    pub(crate) game_process_error: Option<String>,
    selected_game_process: Option<clipcraft_platform::RunningProcess>,
    game_name_draft: String,
    game_process_draft: String,
    game_steam_id_draft: String,
    game_exact_draft: bool,
    builtin_game_search: String,
    game_preset_error: Option<String>,
    pub(crate) steam_games: clipcraft_engine::SteamGames,
    pub(crate) steam_warning: Option<String>,
}

impl SettingsState {
    pub(crate) fn begin_frame(&mut self) {
        self.mic_visible = false;
    }
    pub(crate) fn end_frame(&mut self, settings: &RecordingSettings) {
        if !self.mic_visible {
            self.mic = None;
            self.mic_plan = None;
            self.mic_shown = 0.0;
            return;
        }
        if self.mic.is_none() {
            return;
        }
        if self.mic_plan.as_ref().is_some_and(|plan| {
            plan.device == settings.microphone_device
                && plan.noise_suppression == settings.microphone_noise_suppression
                && plan.mono == settings.microphone_mono
                && plan.gain_percent == settings.microphone_percent.min(100)
                && u32::from(plan.channels) == settings.audio_channels
        }) {
            return;
        }
        let result = execute(Command::PrepareMicrophoneTest {
            settings: Box::new(settings.clone()),
        });
        match result {
            Ok(CommandOutput::MicrophoneTest(plan)) => {
                if self.mic_plan.as_ref() != Some(&plan) {
                    if let Some(test) = &self.mic
                        && let Err(error) = test.update(plan.clone())
                    {
                        self.mic_error = Some(error);
                        self.mic = None;
                    }
                    self.mic_plan = Some(plan);
                }
            }
            _ => {
                self.mic_error = Some("The microphone test settings could not be applied.".into());
                self.mic = None;
            }
        }
    }

    fn select_game_process(&mut self, process: &clipcraft_platform::RunningProcess) {
        self.selected_game_process = Some(process.clone());
        self.game_process_draft = process.executable.clone();
        // Selecting an app without Steam metadata must not retain another game's ID.
        self.game_steam_id_draft = process.steam_app_id.clone().unwrap_or_default();
        if self.game_name_draft.trim().is_empty() {
            self.game_name_draft = display_game_name(&process.executable);
        }
    }

    pub(crate) fn set_game_processes(
        &mut self,
        result: Result<Vec<clipcraft_platform::RunningProcess>, String>,
    ) {
        self.game_processes_loaded = true;
        match result {
            Ok(processes) => {
                self.game_process_error = None;
                self.game_processes = processes;
            }
            Err(error) => {
                self.game_process_error = Some(error);
                self.game_processes.clear();
            }
        }
    }
}

impl SettingsState {
    /// Stops anything running for a screen that is no longer shown.
    pub fn leave(&mut self, keep_mic: bool, keep_capture: bool) {
        if !keep_mic {
            self.mic = None;
            self.mic_shown = 0.0;
        }
        if !keep_capture {
            self.capturing = None;
            self.capturing_clip_length = None;
        }
    }

    /// True while a shortcut button waits for keys: shortcuts must not fire then.
    pub fn assigning_keys(&self) -> bool {
        self.capturing.is_some() || self.capturing_clip_length.is_some()
    }
}

/// The About tab: what ClipCraft is, which version this is, and where to find the project.
pub fn about_tab(ui: &mut egui::Ui) {
    card(ui, "ClipCraft", |ui| {
        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "ClipCraft records your monitor or game window and keeps the last moments ready to save. Press \
                 a hotkey and the last seconds become a clip. Then find, play, trim and keep your \
                 clips from one library.",
            )
            .color(TEXT)
            .size(14.5),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Everything happens on this computer. ClipCraft has no account, no upload, no \
                 sharing service and no tracking, and it works without a network connection.",
            )
            .color(MUTED)
            .size(13.5),
        );
        ui.add_space(14.0);
    });
    card(ui, "Details", |ui| {
        let version = format!(
            "{} ({} {})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        info_row(ui, "Version", &version, false);
        info_row(ui, "License", "MIT or Apache-2.0, open source", true);
    });
    card(ui, "Project", |ui| {
        let url = "https://github.com/nanox333/clipcraft";
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add(egui::Hyperlink::from_label_and_url(
                RichText::new("Source code and issues")
                    .color(kit::LIME)
                    .size(14.5)
                    .family(kit::semibold()),
                url,
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(url).color(MUTED).size(13.0));
            });
        });
        ui.add_space(14.0);
    });
}

/// The start-up switches shown on the General tab.
pub fn startup_card(ui: &mut egui::Ui, settings: &mut RecordingSettings) {
    let mut rows = crate::rows::Rows::new("");
    pages::app_rows(&mut rows, ui, settings);
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn wayland_shortcut_guidance_matches_both_session_indicators() {
        use std::ffi::OsStr;
        assert!(wayland_session(Some(OsStr::new("wayland-0")), None));
        assert!(wayland_session(None, Some("WAYLAND")));
        assert!(!wayland_session(Some(OsStr::new("")), Some("x11")));
        assert!(!wayland_session(None, None));
    }
    fn settings_frame(
        app: &mut crate::ClipCraftApp,
        context: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 2400.0),
                )),
                events,
                ..Default::default()
            },
            |context| {
                crate::kit::apply_theme(context);
                app.draw_screen(context);
            },
        )
    }

    fn category_controls(app: &mut crate::ClipCraftApp) -> Vec<String> {
        let context = crate::kit::test_context();
        context.enable_accesskit();
        let mut controls = Vec::new();
        for _ in 0..3 {
            if let Some(update) = settings_frame(app, &context, Vec::new())
                .platform_output
                .accesskit_update
            {
                controls = update
                    .nodes
                    .into_iter()
                    .map(|(_, node)| {
                        node.label()
                            .or_else(|| node.value())
                            .unwrap_or_default()
                            .to_owned()
                    })
                    .collect();
            }
        }
        controls
    }

    fn keyboard_activate(app: &mut crate::ClipCraftApp, target: &str) {
        keyboard_action(app, target, true);
    }

    fn keyboard_action(app: &mut crate::ClipCraftApp, target: &str, activate: bool) {
        let context = crate::kit::test_context();
        context.enable_accesskit();
        let mut names = std::collections::HashMap::new();
        let mut reached = false;
        for frame in 0..120 {
            let key = if reached {
                egui::Key::Enter
            } else {
                egui::Key::Tab
            };
            let events = if frame < 3 {
                Vec::new()
            } else {
                [true, false]
                    .into_iter()
                    .map(|pressed| egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    })
                    .collect()
            };
            let output = settings_frame(app, &context, events);
            if reached {
                return;
            }
            if let Some(update) = output.platform_output.accesskit_update {
                for (id, node) in update.nodes {
                    names.insert(
                        id,
                        node.label()
                            .or_else(|| node.value())
                            .unwrap_or_default()
                            .to_owned(),
                    );
                }
                reached = names.get(&update.focus).is_some_and(|name| name == target);
                if reached && !activate {
                    return;
                }
            }
        }
        panic!("{target} was not reachable with Tab");
    }

    #[test]
    fn microphone_test_and_optional_mono_are_keyboard_accessible() {
        let mut app = crate::ClipCraftApp {
            page: crate::Page::Settings,
            settings_tab: crate::SettingsTab::Audio,
            settings_state: SettingsState {
                devices: Some(DeviceNames::default()),
                ..Default::default()
            },
            ..Default::default()
        };
        app.recording.microphone_noise_suppression = true;
        keyboard_activate(&mut app, "Microphone is mono");
        assert!(app.recording.microphone_mono && app.recording.microphone_noise_suppression);
        keyboard_action(&mut app, "Mic Test", false);
        assert!(
            app.settings_state.mic.is_none(),
            "focusing the test must not start a microphone"
        );
    }

    #[test]
    fn extra_shortcuts_are_keyboard_assignable_and_removable() {
        let mut app = crate::ClipCraftApp {
            page: crate::Page::Settings,
            settings_tab: crate::SettingsTab::Hotkeys,
            ..Default::default()
        };
        keyboard_activate(&mut app, "Add shortcut for Save clip");
        assert_eq!(
            app.settings_state.capturing,
            Some((HotkeyAction::SaveClip, 1))
        );
        let context = crate::kit::test_context();
        settings_frame(
            &mut app,
            &context,
            vec![egui::Event::Key {
                key: egui::Key::F1,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(
            app.recording
                .hotkeys
                .bindings(HotkeyAction::SaveClip)
                .collect::<Vec<_>>(),
            ["F8", "F1"]
        );
        keyboard_activate(&mut app, "Remove Save clip shortcut 2");
        assert_eq!(
            app.recording
                .hotkeys
                .bindings(HotkeyAction::SaveClip)
                .collect::<Vec<_>>(),
            ["F8"]
        );
    }

    #[test]
    fn a_clip_length_shortcut_is_added_assigned_and_removed_with_the_keyboard() {
        let mut app = crate::ClipCraftApp {
            page: crate::Page::Settings,
            settings_tab: crate::SettingsTab::Hotkeys,
            ..Default::default()
        };
        keyboard_activate(&mut app, "Add a clip-length shortcut");
        assert_eq!(app.settings_state.capturing_clip_length, Some(0));
        assert!(app.settings_state.assigning_keys());
        let context = crate::kit::test_context();
        let press = |key, modifiers| {
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }]
        };
        // The Save clip key is taken: refused, with the reason, and nothing changes.
        settings_frame(
            &mut app,
            &context,
            press(egui::Key::F8, egui::Modifiers::NONE),
        );
        assert!(
            app.settings_state
                .hotkey_error
                .as_deref()
                .is_some_and(|error| error.contains("Save clip"))
        );
        assert_eq!(app.recording.hotkeys.clip_lengths[0].key, "");
        keyboard_activate(&mut app, "Shortcut for saving the last 2 min: Choose a key");
        // The test frame carries no held modifiers, so a free plain key stands in for Shift + F8.
        settings_frame(
            &mut app,
            &context,
            press(egui::Key::F2, egui::Modifiers::NONE),
        );
        assert_eq!(app.recording.hotkeys.clip_lengths[0].key, "F2");
        assert_eq!(app.recording.hotkeys.clip_length_for_key("F2"), Some(120));
        keyboard_activate(&mut app, "Remove the 2 min clip shortcut");
        assert!(app.recording.hotkeys.clip_lengths.is_empty());
    }

    #[test]
    fn microphone_noise_suppression_is_searchable_and_keyboard_accessible() {
        let mut app = crate::ClipCraftApp {
            page: crate::Page::Settings,
            settings_tab: crate::SettingsTab::Audio,
            settings_search: "noise suppression".into(),
            settings_state: SettingsState {
                devices: Some(DeviceNames::default()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!app.recording.microphone_noise_suppression);
        keyboard_activate(&mut app, "Noise suppression");
        assert!(app.recording.microphone_noise_suppression);
        assert!(
            category_controls(&mut app)
                .iter()
                .any(|name| name == "Reset Noise suppression")
        );
        keyboard_activate(&mut app, "Reset Noise suppression");
        assert!(!app.recording.microphone_noise_suppression);
    }

    #[test]
    fn recording_modes_and_duration_are_keyboard_accessible() {
        let mut app = crate::ClipCraftApp {
            page: crate::Page::Settings,
            settings_tab: crate::SettingsTab::Games,
            ..Default::default()
        };
        keyboard_activate(&mut app, "Games, then desktop briefly");
        assert_eq!(
            app.recording.desktop_capture_mode,
            DesktopCaptureMode::AfterGame
        );
        assert!(
            category_controls(&mut app)
                .iter()
                .any(|name| name == "Desktop recording duration in seconds")
        );
        app.recording.desktop_after_game_seconds = 120;
        keyboard_activate(&mut app, "Desktop and games");
        assert_eq!(
            app.recording.desktop_capture_mode,
            DesktopCaptureMode::Always
        );
        assert!(
            !category_controls(&mut app)
                .iter()
                .any(|name| name == "Desktop recording duration in seconds")
        );
        keyboard_activate(&mut app, "Games only");
        assert_eq!(
            app.recording.desktop_capture_mode,
            DesktopCaptureMode::Never
        );
        assert_eq!(app.recording.desktop_after_game_seconds, 120);
    }

    #[test]
    fn custom_game_creation_works_with_the_keyboard_without_any_advanced_switch() {
        let mut app = crate::ClipCraftApp {
            page: crate::Page::Settings,
            settings_tab: crate::SettingsTab::Games,
            ..Default::default()
        };
        assert!(
            category_controls(&mut app)
                .iter()
                .any(|name| name == "Game name")
        );
        app.settings_state.builtin_game_search = "no-known-game-matches-this".to_owned();
        app.settings_state.game_name_draft = "My local game".to_owned();
        app.settings_state.game_process_draft = "local-game.exe".to_owned();
        app.settings_state.game_exact_draft = true;
        keyboard_activate(&mut app, "Add custom game");
        assert_eq!(app.recording.game_rules.len(), 1);
        assert_eq!(
            app.recording.match_game("local-game.exe", None),
            Some("My local game")
        );
        assert_eq!(app.recording.match_game("not-local-game.exe", None), None);
        assert!(app.settings_state.game_name_draft.is_empty());
        assert!(app.settings_state.game_process_draft.is_empty());
    }

    #[test]
    fn choosing_a_non_steam_process_clears_the_previous_games_steam_id() {
        let mut state = SettingsState::default();
        let mut process = clipcraft_platform::RunningProcess {
            process_id: 1,
            executable: "first-game.exe".to_owned(),
            steam_app_id: Some("730".to_owned()),
            parent_id: None,
        };
        state.select_game_process(&process);
        assert_eq!(state.game_steam_id_draft, "730");
        state.game_name_draft = "My chosen name".to_owned();
        process.executable = "second-game.exe".to_owned();
        process.steam_app_id = None;
        state.select_game_process(&process);
        assert!(state.game_steam_id_draft.is_empty());
        assert_eq!(state.game_process_draft, "second-game.exe");
        assert_eq!(state.game_name_draft, "My chosen name");
    }

    #[test]
    fn a_known_game_can_be_disabled_with_tab_and_enter() {
        let context = crate::kit::test_context();
        context.enable_accesskit();
        let mut app = crate::ClipCraftApp {
            page: crate::Page::Settings,
            settings_tab: crate::SettingsTab::Games,
            ..Default::default()
        };
        let mut names = std::collections::HashMap::new();
        let mut reached = false;
        for frame in 0..100 {
            let key = if reached {
                egui::Key::Enter
            } else {
                egui::Key::Tab
            };
            let events = if frame < 3 {
                Vec::new()
            } else {
                vec![
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |context| {
                    crate::kit::apply_theme(context);
                    app.draw_screen(context);
                },
            );
            if reached {
                break;
            }
            if let Some(update) = output.platform_output.accesskit_update {
                for (id, node) in update.nodes {
                    names.insert(
                        id,
                        node.label()
                            .or_else(|| node.value())
                            .unwrap_or_default()
                            .to_owned(),
                    );
                }
                reached = names
                    .get(&update.focus)
                    .is_some_and(|label| label == "Counter-Strike 2");
            }
        }
        assert!(reached, "Known game checkbox was not reachable with Tab");
        assert!(
            app.recording
                .disabled_builtin_games
                .iter()
                .any(|id| id == "cs2")
        );
        assert!(app.recording.game_rules.is_empty());
        assert_eq!(app.recording.match_game("cs2.exe", None), None);
        assert_eq!(
            app.recording.match_game("Wow.exe", None),
            Some("World of Warcraft")
        );
    }

    #[test]
    fn the_settings_action_changes_exclusions_atomically_and_preserves_custom_rules() {
        let mut settings = RecordingSettings::default();
        settings
            .game_rules
            .push(GameMatchRule::new("My CS name", ["cs2.exe"]));
        let before = settings.clone();
        assert!(
            set_game_presets(&mut settings, vec!["cs2".into(), "bad-id".into()], false).is_err()
        );
        assert_eq!(settings, before);
        set_game_presets(&mut settings, vec!["cs2".into()], false).unwrap();
        assert_eq!(settings.game_rules, before.game_rules);
        assert!(settings.disabled_builtin_games.iter().any(|id| id == "cs2"));
        // Excluding a built-in does not suppress an explicit custom override.
        assert_eq!(settings.match_game("cs2.exe", None), Some("My CS name"));
        set_game_presets(&mut settings, vec!["cs2".into()], true).unwrap();
        assert!(settings.disabled_builtin_games.is_empty());
    }
}
