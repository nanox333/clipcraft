//! Locally prepare a report the user can review before choosing to share it.

use clipcraft_engine::{BugReportInput, Command, CommandOutput, execute};
use eframe::egui::{self, RichText};

use crate::kit::{self, MUTED, TEXT};
use crate::{ClipCraftApp, Icon};

const BUG_FORM: &str = "https://github.com/nanox333/clipcraft/issues/new?template=bug.yml";

#[derive(Default)]
pub(crate) struct Draft {
    input: BugReportInput,
    copied: bool,
}

impl ClipCraftApp {
    pub(crate) fn bug_report_card(&mut self, ui: &mut egui::Ui) {
        crate::settings::card(ui, "Need help?", |ui| {
            ui.add_space(12.0);
            ui.label(
                RichText::new("Found a bug? Describe it and review a report before sharing it.")
                    .color(MUTED)
                    .size(13.5),
            );
            ui.add_space(10.0);
            if kit::labelled_button(ui, Icon::Plus, "Report a bug", false, true).clicked() {
                self.bug_report = Some(Draft::default());
            }
            ui.add_space(14.0);
        });
    }

    pub(crate) fn bug_report_dialog(&mut self, context: &egui::Context) {
        let Some(draft) = self.bug_report.as_mut() else {
            return;
        };
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("bug-report-dialog"))
            .frame(egui::Frame::new().fill(kit::PANEL).stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
                .corner_radius(egui::CornerRadius::same(14)).inner_margin(egui::Margin::same(22)))
            .show(context, |ui| {
                ui.set_width(620.0_f32.min((context.content_rect().width() - 80.0).max(300.0)));
                let height = (context.content_rect().height() - 100.0).clamp(400.0, 600.0);
                ui.set_height(height);
                ui.label(RichText::new("Report a bug").color(TEXT).size(19.0).family(kit::semibold()));
                ui.add_space(4.0);
                ui.label(RichText::new("Your draft stays on this device until you close it. Review your text for private information before sharing.").color(MUTED).size(13.0));
                ui.add_space(8.0);
                let (preview, valid) = egui::ScrollArea::vertical().max_height((height - 165.0).max(180.0)).show(ui, |ui| {
                    for (label, text) in [
                        ("What happened?", &mut draft.input.observed),
                        ("What did you expect?", &mut draft.input.expected),
                        ("Steps to reproduce", &mut draft.input.steps),
                    ] {
                        ui.label(RichText::new(label).color(TEXT).size(13.0).family(kit::semibold()));
                        let response = ui.add(egui::TextEdit::multiline(text).desired_rows(2).char_limit(1024).desired_width(f32::INFINITY));
                        if response.changed() { draft.copied = false; }
                        ui.add_space(8.0);
                    }
                    if ui.checkbox(&mut draft.input.include_settings, "Include recording settings (no paths or device names)").changed() {
                        draft.copied = false;
                    }
                    ui.add_space(10.0);
                    ui.label(RichText::new("Report preview").color(TEXT).size(13.0).family(kit::semibold()));
                    let output = execute(Command::PrepareBugReport {
                        input: draft.input.clone(),
                        settings: Box::new(self.recording.clone()),
                    });
                    let (mut preview, valid) = match output {
                        Ok(CommandOutput::BugReport(text)) => (text, true),
                        Err(error) => (error.to_string(), false),
                        Ok(_) => ("Could not prepare the report.".to_owned(), false),
                    };
                    ui.add(egui::TextEdit::multiline(&mut preview).desired_rows(6).desired_width(f32::INFINITY).interactive(false));
                    (preview, valid)
                }).inner;
                ui.add_space(8.0);
                ui.label(RichText::new("Copy the report, then paste it into the GitHub bug form. Opening the form does not send the report.").color(MUTED).size(12.5));
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    if kit::labelled_button(ui, Icon::Copy, if draft.copied { "Copied" } else { "Copy report" }, true, valid).clicked() {
                        context.copy_text(preview.clone());
                        draft.copied = true;
                    }
                    if kit::labelled_button(ui, Icon::Import, "Open GitHub form", false, true).clicked() {
                        context.open_url(egui::OpenUrl::new_tab(BUG_FORM));
                    }
                    if kit::labelled_button(ui, Icon::ChevronLeft, "Close", false, true).clicked() { close = true; }
                });
            });
        if close || modal.should_close() {
            self.bug_report = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_draft_never_copies_or_opens_a_browser_and_escape_closes_it() {
        let context = crate::kit::test_context();
        let mut app = ClipCraftApp {
            bug_report: Some(Draft::default()),
            ..Default::default()
        };
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1440.0, 900.0),
            )),
            events,
            ..Default::default()
        };
        let first = context.run(input(vec![]), |ctx| app.bug_report_dialog(ctx));
        assert!(first.platform_output.commands.is_empty());
        assert!(app.bug_report.is_some());
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let second = context.run(input(vec![escape]), |ctx| app.bug_report_dialog(ctx));
        assert!(second.platform_output.commands.is_empty());
        assert!(app.bug_report.is_none());
    }
}
