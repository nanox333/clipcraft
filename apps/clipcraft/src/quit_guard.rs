//! Asking before ClipCraft quits while the replay holds footage.
//!
//! Quitting throws the replay away, so closing the window (or choosing Quit in the tray) by
//! accident while clipping would lose the last moments. A close that only minimizes to the tray
//! loses nothing and is never questioned.

use eframe::egui::{self, RichText};

use crate::kit::{self, TEXT};
use crate::{ClipCraftApp, Icon};

/// Less than this much footage is not worth a question.
const WORTH_KEEPING_SECONDS: f32 = 1.0;

/// Whether closing now should ask first.
pub(crate) fn needs_question(
    asking_allowed: bool,
    recording: bool,
    buffered_seconds: f32,
    only_minimizing: bool,
) -> bool {
    asking_allowed && recording && !only_minimizing && buffered_seconds >= WORTH_KEEPING_SECONDS
}

/// The state of the question.
#[derive(Default)]
pub(crate) struct QuitGuard {
    /// The question is on screen.
    pub(crate) asking: bool,
    /// The person said to quit, so the next close goes through.
    confirmed: bool,
}

impl ClipCraftApp {
    /// Turns a close that would lose footage into the question. Runs before the close is acted on.
    pub(crate) fn guard_quit(&mut self, context: &egui::Context) {
        if self.quit_guard.confirmed || !context.input(|input| input.viewport().close_requested()) {
            return;
        }
        let only_minimizing =
            self.tray.is_some() && self.recording.close_to_tray && !crate::tray::Tray::quitting();
        let buffered = self
            .capture
            .record_status()
            .map_or(0.0, |status| status.buffered_seconds);
        if needs_question(
            self.recording.confirm_quit_with_footage,
            self.capture.is_active(),
            buffered,
            only_minimizing,
        ) {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.quit_guard.asking = true;
        }
    }

    /// The question, while there is one. Escape keeps ClipCraft open.
    pub(crate) fn quit_dialog(&mut self, context: &egui::Context) {
        if !self.quit_guard.asking {
            return;
        }
        let buffered = self
            .capture
            .record_status()
            .map_or(0.0, |status| status.buffered_seconds);
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("quit-dialog"))
            .frame(
                egui::Frame::new()
                    .fill(kit::PANEL)
                    .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
                    .corner_radius(egui::CornerRadius::same(crate::kit::radius::LG))
                    .inner_margin(egui::Margin::same(24)),
            )
            .show(context, |ui| {
                ui.set_width(400.0);
                ui.label(
                    RichText::new("Quit and lose the replay?")
                        .color(TEXT)
                        .size(crate::kit::text_size::LEAD)
                        .family(kit::semibold()),
                );
                ui.add_space(crate::kit::space::SM);
                ui.label(
                    RichText::new(format!(
                        "ClipCraft is clipping and holds about {} seconds of footage. \
                         Quitting throws it away. Save it first if you want it, or keep \
                         ClipCraft open.",
                        buffered.round() as u32
                    ))
                    .color(TEXT)
                    .size(crate::kit::text_size::BODY),
                );
                ui.add_space(crate::kit::space::XL);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = crate::kit::space::MD;
                    // Enter does not confirm: keeping the footage is the default.
                    if crate::clip_delete::danger_button(ui, "Quit anyway").clicked() {
                        decision = Some(true);
                    }
                    if kit::labelled_button(ui, Icon::Restart, "Keep clipping", true, true)
                        .clicked()
                    {
                        decision = Some(false);
                    }
                });
            });
        if modal.should_close() {
            decision = decision.or(Some(false));
        }
        if decision.is_some() {
            context.request_repaint();
        }
        match decision {
            Some(true) => {
                self.quit_guard.asking = false;
                self.quit_guard.confirmed = true;
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(false) => {
                self.quit_guard.asking = false;
                crate::tray::Tray::cancel_quit();
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_real_quit_with_footage_while_clipping_asks() {
        assert!(needs_question(true, true, 12.0, false));
        // Off, not clipping, an empty replay or a minimize to the tray never ask.
        assert!(!needs_question(false, true, 12.0, false));
        assert!(!needs_question(true, false, 12.0, false));
        assert!(!needs_question(true, true, 0.2, false));
        assert!(!needs_question(true, true, 12.0, true));
    }
}
