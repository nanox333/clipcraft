//! Local match-data settings and lifecycle; no game data affects capture or leaves the device.

use clipcraft_engine::game_state::{GameIntegration, GameIntegrationSettings};
use clipcraft_engine::{Command, CommandOutput, execute};
use clipcraft_platform::game_state::{GameStateWorker, IntegrationStatuses};
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

#[derive(Default)]
pub(crate) struct State {
    worker: Option<GameStateWorker>,
    applied: Option<GameIntegrationSettings>,
    pub(crate) statuses: IntegrationStatuses,
    key_path: Option<PathBuf>,
    setup: Option<(GameIntegration, mpsc::Receiver<Result<(), String>>)>,
    message: Option<(GameIntegration, String)>,
}

impl State {
    pub(crate) fn sync(
        &mut self,
        settings: &GameIntegrationSettings,
        settings_path: Option<&Path>,
        context: &egui::Context,
    ) {
        self.key_path = settings_path.map(|p| p.with_file_name("game-state-key"));
        if let Some((game, receiver)) = &self.setup {
            match receiver.try_recv() {
                Ok(result) => {
                    self.message = Some(match result {
                        Ok(()) => (*game, "Config ready. Restart the game to connect.".into()),
                        Err(e) => (*game, e),
                    });
                    self.setup = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = Some((*game, "Setup worker stopped. Please try again.".into()));
                    self.setup = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.applied.as_ref() != Some(settings) {
            if let Some(worker) = &self.worker {
                worker.stop();
                self.statuses = IntegrationStatuses::default();
                if !worker.is_finished() {
                    context.request_repaint_after(std::time::Duration::from_millis(100));
                    return;
                }
            }
            self.worker = None;
            self.applied = Some(settings.clone());
            self.statuses = IntegrationStatuses::default();
            if GameIntegration::ALL
                .into_iter()
                .any(|g| settings.enabled(g))
            {
                if let Some(key_path) = &self.key_path {
                    match GameStateWorker::start(settings.clone(), key_path.clone()) {
                        Ok(worker) => self.worker = Some(worker),
                        Err(e) => self.set_error(settings, &e.to_string()),
                    }
                } else {
                    self.set_error(settings, "Game data needs a local settings directory.");
                }
            }
        }
        if let Some(worker) = &self.worker {
            match worker.statuses() {
                Ok(statuses) => self.statuses = statuses,
                Err(e) => {
                    self.statuses = IntegrationStatuses::default();
                    self.set_error(settings, &e.to_string());
                }
            }
            context.request_repaint_after(std::time::Duration::from_millis(500));
        }
        if self.setup.is_some() {
            context.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui, settings: &mut GameIntegrationSettings) {
        crate::settings::card(ui, "Live game data", |ui| {
            ui.label(RichText::new("Optional match details, read locally from the game. Off by default. Nothing is published to Discord or uploaded, and recording works without this.").color(crate::kit::MUTED).size(12.5));
            ui.add_space(12.0);
            for game in GameIntegration::ALL {
                ui.push_id(game.port(), |ui| {
                    let enabled = match game {
                        GameIntegration::Cs2 => &mut settings.cs2,
                        GameIntegration::Dota2 => &mut settings.dota2,
                        GameIntegration::League => &mut settings.league,
                    };
                    let supported =
                        game != GameIntegration::League || cfg!(any(windows, target_os = "macos"));
                    let help = match game {
                        GameIntegration::Cs2 => {
                            "Map, mode, round, score, your health, weapon and K/D/A."
                        }
                        GameIntegration::Dota2 => {
                            "Match phase, clock, your hero, health, mana, gold and K/D/A."
                        }
                        GameIntegration::League => {
                            "Map, mode, clock, your champion, health, resource, gold and K/D/A. Windows/macOS only."
                        }
                    };
                    let switch_label = match game {
                        GameIntegration::Cs2 => "CS2 match details",
                        GameIntegration::Dota2 => "Dota 2 match details",
                        GameIntegration::League => "League of Legends match details",
                    };
                    // A saved enabled integration can still be turned off on an unsupported OS.
                    ui.add_enabled_ui(supported || *enabled, |ui| {
                        crate::settings::toggle_row(ui, switch_label, help, enabled, true);
                    });
                    if *enabled {
                        ui.label(
                            RichText::new(self.statuses.get(game).label())
                                .size(12.5)
                                .color(crate::kit::MUTED),
                        );
                        if let clipcraft_platform::game_state::IntegrationStatus::Live(snapshot) =
                            self.statuses.get(game)
                        {
                            ui.push_id(game.port(), |ui| {
                                ui.collapsing("Match details", |ui| {
                                    egui::Grid::new("game-data-fields")
                                        .num_columns(2)
                                        .spacing([24.0, 6.0])
                                        .show(ui, |ui| {
                                            for (name, value) in [
                                                ("Round", snapshot.round),
                                                ("Health", snapshot.health),
                                                ("Armour", snapshot.armor),
                                                ("Mana / resource", snapshot.resource),
                                                ("Money / gold", snapshot.money),
                                                ("Ammunition", snapshot.ammo),
                                                ("Match time (seconds)", snapshot.game_seconds),
                                            ] {
                                                if let Some(value) = value {
                                                    ui.label(
                                                        RichText::new(name).color(crate::kit::MUTED),
                                                    );
                                                    ui.label(value.to_string());
                                                    ui.end_row();
                                                }
                                            }
                                            if let Some(weapon) = &snapshot.weapon {
                                                ui.label(
                                                    RichText::new("Weapon").color(crate::kit::MUTED),
                                                );
                                                ui.label(weapon);
                                                ui.end_row();
                                            }
                                        });
                                });
                            });
                        }
                        if game != GameIntegration::League {
                            let label = format!("Set up {}…", game.label());
                            if crate::kit::labelled_button(
                                ui,
                                crate::kit::Icon::Folder,
                                &label,
                                false,
                                self.setup.is_none() && self.key_path.is_some(),
                            )
                            .clicked()
                            {
                                self.setup(game);
                            }
                            ui.label(RichText::new(match game {
                                GameIntegration::Cs2 => "Choose Counter-Strike Global Offensive/game/csgo/cfg. Restart CS2 after setup.",
                                _ => "Choose dota 2 beta/game/dota/cfg/gamestate_integration. Add -gamestateintegration to Dota's Steam launch options, then restart Dota.",
                            }).size(12.0).color(crate::kit::FAINT));
                        } else {
                            ui.label(RichText::new("No config file or API key needed. Start a match; the client must expose its HTTPS API on 127.0.0.1:2999.").size(12.0).color(crate::kit::FAINT));
                        }
                    }
                    if let Some((which, message)) = &self.message
                        && *which == game
                    {
                        ui.label(RichText::new(message).size(12.5).color(crate::kit::MUTED));
                    }
                    ui.add_space(8.0);
                });
            }
        });
    }

    fn setup(&mut self, game: GameIntegration) {
        let Some(key_path) = self.key_path.clone() else {
            return;
        };
        let Some(directory) = rfd::FileDialog::new()
            .set_title("Choose the game's GSI config folder")
            .pick_folder()
        else {
            return;
        };
        let (tx, rx) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("clipcraft-game-setup".into())
            .spawn(move || {
                let result = match execute(Command::InstallGameIntegration {
                    game,
                    directory,
                    key_path,
                }) {
                    Ok(CommandOutput::GameIntegrationInstalled(_)) => Ok(()),
                    Ok(_) => Err("Unexpected setup result.".into()),
                    Err(e) => Err(e.to_string()),
                };
                let _ = tx.send(result);
            }) {
            Ok(_) => {
                self.setup = Some((game, rx));
                self.message = None;
            }
            Err(_) => {
                self.message = Some((game, "Could not start setup. Please try again.".into()))
            }
        }
    }

    fn set_error(&mut self, settings: &GameIntegrationSettings, message: &str) {
        for (game, status) in GameIntegration::ALL.into_iter().zip(&mut self.statuses.0) {
            if settings.enabled(game) {
                *status =
                    clipcraft_platform::game_state::IntegrationStatus::Error(message.to_owned());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_data_switches_are_keyboard_accessible_without_starting_a_worker() {
        for target in ["CS2 match details", "Dota 2 match details"] {
            let context = crate::kit::test_context();
            context.enable_accesskit();
            let mut state = State::default();
            let mut settings = GameIntegrationSettings::default();
            let mut names = std::collections::HashMap::new();
            let mut reached = false;
            for frame in 0..30 {
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
                let output = context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(960.0, 900.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        crate::kit::apply_theme(ctx);
                        egui::CentralPanel::default().show(ctx, |ui| state.show(ui, &mut settings));
                    },
                );
                if reached {
                    break;
                }
                if let Some(update) = output.platform_output.accesskit_update {
                    for (id, node) in update.nodes {
                        names.insert(id, node.label().unwrap_or_default().to_owned());
                    }
                    reached = names.get(&update.focus).is_some_and(|name| name == target);
                }
            }
            assert!(reached, "{target} unreachable with Tab");
            assert_eq!(settings.cs2, target == "CS2 match details");
            assert_eq!(settings.dota2, target == "Dota 2 match details");
            assert!(state.worker.is_none());
        }
    }

    #[test]
    fn turning_integrations_off_clears_live_status_and_reaps_before_replacement() {
        let context = crate::kit::test_context();
        let key = std::env::temp_dir().join("unused-clipcraft-ui-game-key");
        let mut state = State {
            worker: Some(GameStateWorker::start(GameIntegrationSettings::default(), key).unwrap()),
            applied: Some(GameIntegrationSettings {
                cs2: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        state.statuses.0[0] =
            clipcraft_platform::game_state::IntegrationStatus::Error("old".into());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            state.sync(&GameIntegrationSettings::default(), None, &context);
            if state.worker.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(state.worker.is_none());
        assert!(state.statuses.0.iter().all(|s| matches!(
            s,
            clipcraft_platform::game_state::IntegrationStatus::Disabled
        )));
    }

    #[test]
    fn keyboard_focus_survives_another_games_live_details_disappearing() {
        let context = crate::kit::test_context();
        context.enable_accesskit();
        let mut state = State::default();
        state.statuses.0[0] = clipcraft_platform::game_state::IntegrationStatus::Live(
            clipcraft_engine::game_state::parse_game_state(
                GameIntegration::Cs2,
                br#"{"provider":{"appid":730}}"#,
            )
            .unwrap(),
        );
        let mut settings = GameIntegrationSettings {
            cs2: true,
            ..Default::default()
        };
        let mut names = std::collections::HashMap::new();
        let mut reached = false;
        for frame in 0..40 {
            let key = if reached {
                egui::Key::Enter
            } else {
                egui::Key::Tab
            };
            if reached {
                state.statuses.0[0] = clipcraft_platform::game_state::IntegrationStatus::Waiting;
            }
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
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960.0, 1200.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| state.show(ui, &mut settings));
                },
            );
            if reached {
                break;
            }
            if let Some(update) = output.platform_output.accesskit_update {
                for (id, node) in update.nodes {
                    names.insert(id, node.label().unwrap_or_default().to_owned());
                }
                reached = names
                    .get(&update.focus)
                    .is_some_and(|name| name == "Dota 2 match details");
            }
        }
        assert!(reached);
        assert!(settings.dota2);
    }
}
