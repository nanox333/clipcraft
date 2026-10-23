//! Discord UI and lifecycle. The engine owns every field allowed out through IPC.
use clipcraft_engine::presence::{
    PresenceActivity, PresenceContext, PresenceSettings, resolve_application_id,
    shared_application_id, valid_application_id,
};
use clipcraft_engine::{Command, CommandOutput, execute};
use clipcraft_platform::presence::{PresenceStatus, PresenceWorker};
use eframe::egui::{self, RichText};

#[derive(Default)]
pub(crate) struct State {
    worker: Option<PresenceWorker>,
    id_draft: Option<String>,
    failed: bool,
    preview: Option<PresenceActivity>,
    badges: std::collections::HashMap<&'static str, egui::TextureHandle>,
    pub(crate) status: PresenceStatus,
}
impl State {
    pub(crate) fn sync(
        &mut self,
        settings: &PresenceSettings,
        context: PresenceContext,
        ui: &egui::Context,
    ) {
        self.preview = match execute(Command::PreviewDiscordCard {
            settings: settings.clone(),
            context: Box::new(context.clone()),
        }) {
            Ok(CommandOutput::DiscordCard(card)) => card,
            _ => None,
        };
        let target = match execute(Command::BuildDiscordPresence {
            settings: settings.clone(),
            context: Box::new(context),
        }) {
            Ok(CommandOutput::DiscordPresence(target)) => target,
            _ => {
                self.status = PresenceStatus::Failed;
                return;
            }
        };
        if !settings.enabled {
            self.failed = false;
            if self
                .worker
                .as_ref()
                .is_some_and(|w| w.status() == PresenceStatus::Failed)
            {
                self.worker = None;
                self.status = PresenceStatus::Disabled;
            }
        }
        if self.worker.is_none() && target.is_some() && !self.failed {
            match PresenceWorker::start() {
                Ok(worker) => self.worker = Some(worker),
                Err(_) => {
                    self.status = PresenceStatus::Failed;
                    self.failed = true;
                }
            }
        }
        if let Some(worker) = &self.worker {
            if worker.publish(target).is_err() {
                self.status = PresenceStatus::Failed;
            } else {
                self.status = worker.status();
            }
            if settings.enabled || self.status != PresenceStatus::Disabled {
                ui.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }
    }
    pub(crate) fn show_advanced(&mut self, ui: &mut egui::Ui, settings: &mut PresenceSettings) {
        crate::settings::advanced_heading(ui);
        crate::settings::card(ui, "Discord developer settings", |ui| {
            ui.add_space(6.0);
            ui.label(
                RichText::new("Discord Application ID")
                    .family(crate::kit::medium())
                    .size(14.0),
            );
            let draft = self
                .id_draft
                .get_or_insert_with(|| settings.application_id.clone());
            let response = ui.add(
                egui::TextEdit::singleline(draft)
                    .char_limit(20)
                    .hint_text("Application ID — not a bot token")
                    .desired_width(ui.available_width().min(360.0)),
            );
            crate::kit::name_text_box(&response, "Discord Application ID", draft);
            if crate::kit::labelled_button(
                ui,
                crate::kit::Icon::Gear,
                "Apply Application ID",
                false,
                valid_application_id(draft.trim()) && draft.trim() != settings.application_id,
            )
            .clicked()
            {
                settings.application_id = draft.trim().to_owned();
            }
            ui.label(RichText::new("Create an application named ClipCraft in Discord's developer portal, then copy its Application ID from General Information. No bot, secret, token or login inside ClipCraft is needed.").size(12.0).color(crate::kit::MUTED));
            ui.hyperlink_to(
                "Discord developer portal",
                "https://discord.com/developers/applications",
            );
            if !settings.application_id.is_empty()
                && ui.button("Use shared ClipCraft identity").clicked()
            {
                settings.application_id.clear();
                self.id_draft = Some(String::new());
            }
            ui.label(RichText::new("This override is for developers. Normal installations use the shared identity included in the build. An Application ID is public; never enter a token or secret.").size(12.0).color(crate::kit::MUTED));
        });
    }
    pub(crate) fn shutdown(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            worker.shutdown();
        }
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, settings: &mut PresenceSettings) {
        crate::settings::card(ui, "Discord Rich Presence", |ui| {
            crate::settings::toggle_row(
                ui,
                "Show ClipCraft activity on Discord",
                "Discord can publish “Clipping”, “Editing clips” or “Browsing clips” to your profile. No clips, file paths or clip titles are sent. Off by default.",
                &mut settings.enabled,
                true,
            );
            if settings.enabled {
                let text = if resolve_application_id(settings, shared_application_id()).is_none() {
                    "Discord presence is not configured in this build yet."
                } else {
                    self.status.label()
                };
                ui.label(RichText::new(text).size(12.5).color(crate::kit::MUTED));
                crate::settings::toggle_row(
                    ui,
                    "Show the captured game's name",
                    "Adds the currently captured game's name while clipping. Editing and browsing never include saved clips' game labels.",
                    &mut settings.show_game,
                    true,
                );
                ui.add_enabled_ui(settings.show_game,|ui| {
                    crate::settings::toggle_row(ui,"Show map, mode, score and hero",
                        "Also publishes available map, game mode, team score and hero/champion from an enabled local game integration. No health, money, K/D/A or account identifiers.",&mut settings.show_match,true);
                });
                if let Some(activity) = &self.preview {
                    ui.add_space(8.0);
                    egui::Frame::new()
                        .fill(egui::Color32::from_rgb(30, 31, 34))
                        .corner_radius(8.0)
                        .inner_margin(16.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width().min(340.0));
                            ui.label(
                                RichText::new("LOCAL CARD PREVIEW")
                                    .size(10.5)
                                    .color(crate::kit::MUTED),
                            );
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                let key = activity.asset_key();
                                for key in [key, "clipcraft"] {
                                    if !self.badges.contains_key(key)
                                        && let Ok(image) =
                                            clipcraft_engine::presence_art::badge(key, 128)
                                    {
                                        let pixels = egui::ColorImage::from_rgba_unmultiplied(
                                            [128, 128],
                                            image.as_raw(),
                                        );
                                        self.badges.insert(
                                            key,
                                            ui.ctx().load_texture(
                                                key,
                                                pixels,
                                                egui::TextureOptions::LINEAR,
                                            ),
                                        );
                                    }
                                }
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(64.0, 64.0),
                                    egui::Sense::hover(),
                                );
                                if let Some(texture) = self.badges.get(key) {
                                    ui.put(
                                        rect,
                                        egui::Image::new(texture)
                                            .fit_to_exact_size(rect.size())
                                            .corner_radius(10.0),
                                    );
                                }
                                if activity.game.is_some() {
                                    let badge = egui::Rect::from_center_size(
                                        rect.right_bottom() - egui::vec2(4.0, 4.0),
                                        egui::vec2(24.0, 24.0),
                                    );
                                    if let Some(texture) = self.badges.get("clipcraft") {
                                        ui.put(
                                            badge,
                                            egui::Image::new(texture)
                                                .fit_to_exact_size(badge.size())
                                                .corner_radius(12.0),
                                        );
                                    }
                                }
                                ui.add_space(12.0);
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new("ClipCraft").family(crate::kit::medium()),
                                    );
                                    let wire = activity.wire();
                                    if let Some(details) = wire["details"].as_str() {
                                        ui.label(details);
                                    }
                                    if let Some(state) = wire["state"].as_str() {
                                        ui.label(RichText::new(state).color(crate::kit::MUTED));
                                    }
                                });
                            });
                        });
                    ui.label(RichText::new("Preview only. Discord controls the application name and renders the uploaded artwork.").size(11.5).color(crate::kit::FAINT));
                }
                ui.label(RichText::new("Requires the Discord desktop app and activity sharing enabled in Discord. Discord controls visibility to other people. Local recording works without it.").size(12.0).color(crate::kit::FAINT));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(
        context: &egui::Context,
        state: &mut State,
        settings: &mut PresenceSettings,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960.0, 1200.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                crate::kit::apply_theme(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    state.show(ui, settings);
                    state.show_advanced(ui, settings);
                });
            },
        )
    }
    fn activate(state: &mut State, settings: &mut PresenceSettings, target: &str) {
        let context = crate::kit::test_context();
        context.enable_accesskit();
        let mut names = std::collections::HashMap::new();
        let mut reached = false;
        for n in 0..40 {
            let key = if reached {
                egui::Key::Enter
            } else {
                egui::Key::Tab
            };
            let events = if n < 3 {
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
            let out = frame(&context, state, settings, events);
            if reached {
                return;
            }
            if let Some(update) = out.platform_output.accesskit_update {
                for (id, node) in update.nodes {
                    names.insert(id, node.label().unwrap_or_default().to_owned());
                }
                reached = names
                    .get(&update.focus)
                    .is_some_and(|label| label == target);
            }
        }
        panic!("{target} unreachable");
    }
    #[test]
    fn keyboard_consent_is_separate_and_application_id_can_be_applied_without_connecting() {
        let mut state = State::default();
        let mut settings = PresenceSettings::default();
        activate(
            &mut state,
            &mut settings,
            "Show ClipCraft activity on Discord",
        );
        assert!(settings.enabled);
        assert!(!settings.show_game && !settings.show_match);
        activate(&mut state, &mut settings, "Show the captured game's name");
        assert!(settings.show_game);
        assert!(!settings.show_match);
        activate(&mut state, &mut settings, "Show map, mode, score and hero");
        assert!(settings.show_match);
        state.id_draft = Some("123456789012345678".into());
        activate(&mut state, &mut settings, "Apply Application ID");
        assert_eq!(settings.application_id, "123456789012345678");
        assert!(state.worker.is_none());
        activate(&mut state, &mut settings, "Use shared ClipCraft identity");
        assert!(settings.application_id.is_empty());
        activate(
            &mut state,
            &mut settings,
            "Show ClipCraft activity on Discord",
        );
        assert!(!settings.enabled);
    }
    #[test]
    fn normal_settings_preview_works_without_id_and_has_no_developer_form() {
        let context = crate::kit::test_context();
        context.enable_accesskit();
        let mut state = State::default();
        let mut settings = PresenceSettings {
            enabled: true,
            ..Default::default()
        };
        // Preview is independent of shared build identity and never starts a worker.
        let CommandOutput::DiscordCard(card) = execute(Command::PreviewDiscordCard {
            settings: settings.clone(),
            context: Box::default(),
        })
        .unwrap() else {
            panic!("wrong command output");
        };
        state.preview = card;
        let mut labels = Vec::new();
        for _ in 0..3 {
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960.0, 1200.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    crate::kit::apply_theme(ctx);
                    egui::CentralPanel::default().show(ctx, |ui| state.show(ui, &mut settings));
                },
            );
            if let Some(update) = output.platform_output.accesskit_update {
                labels.extend(
                    update
                        .nodes
                        .into_iter()
                        .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned)),
                );
            }
        }
        assert!(labels.iter().any(|s| s == "LOCAL CARD PREVIEW"));
        assert!(labels.iter().any(|s| s == "with ClipCraft"));
        assert!(
            !labels
                .iter()
                .any(|s| s == "Discord Application ID" || s == "Apply Application ID")
        );
        assert!(state.worker.is_none());
    }
    #[test]
    fn missing_id_and_disabled_settings_never_start_ipc_workers() {
        let context = crate::kit::test_context();
        let mut state = State::default();
        for enabled in [false, true]
            .into_iter()
            .filter(|enabled| !enabled || shared_application_id().is_none())
        {
            let settings = PresenceSettings {
                enabled,
                ..Default::default()
            };
            state.sync(&settings, PresenceContext::default(), &context);
            assert!(state.worker.is_none());
        }
        assert!(clipcraft_engine::command_ids().contains(&"presence.preview"));
    }
}
