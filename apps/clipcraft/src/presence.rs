//! Discord UI and lifecycle. The engine owns every field allowed out through IPC.
use clipcraft_engine::presence::{PresenceContext, PresenceSettings, valid_application_id};
use clipcraft_engine::{Command, CommandOutput, execute};
use clipcraft_platform::presence::{PresenceStatus, PresenceWorker};
use eframe::egui::{self, RichText};

#[derive(Default)]
pub(crate) struct State {
    worker: Option<PresenceWorker>,
    id_draft: Option<String>,
    failed: bool,
    pub(crate) status: PresenceStatus,
}
impl State {
    pub(crate) fn sync(
        &mut self,
        settings: &PresenceSettings,
        context: PresenceContext,
        ui: &egui::Context,
    ) {
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
                let text = if !valid_application_id(&settings.application_id) {
                    "Add a Discord Application ID to connect."
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
                egui::CentralPanel::default().show(ctx, |ui| state.show(ui, settings));
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
        activate(
            &mut state,
            &mut settings,
            "Show ClipCraft activity on Discord",
        );
        assert!(!settings.enabled);
    }
    #[test]
    fn missing_id_and_disabled_settings_never_start_ipc_workers() {
        let context = crate::kit::test_context();
        let mut state = State::default();
        for enabled in [false, true] {
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
