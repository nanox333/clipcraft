//! When ClipCraft keeps failing to start, get the person in and let them choose.
//!
//! The engine counts starts that never got going (`clipcraft_engine::safe_start`). After three,
//! this start skips the saved settings and asks: keep the default settings (the old file is kept
//! beside them), or go back to the saved ones. Clips, the library and drafts are never touched.

use std::path::PathBuf;
use std::time::Instant;

use clipcraft_engine::safe_start::{self, HEALTHY_AFTER};
use eframe::egui::{self, RichText};

use crate::kit::{self, MUTED, TEXT};
use crate::{ClipCraftApp, Icon};

/// What launch learned from the start counter.
pub(crate) struct SafeStart {
    path: Option<PathBuf>,
    begun: Instant,
    marked: bool,
    /// The question is on screen.
    pub(crate) asking: bool,
}

impl SafeStart {
    /// Counts this start and finds out whether the ones before it kept failing.
    pub(crate) fn begin() -> Self {
        let path = safe_start::default_safe_start_path();
        let before = path.as_deref().map_or(0, safe_start::begin);
        Self {
            path,
            begun: Instant::now(),
            marked: false,
            asking: safe_start::needs_safe_start(before),
        }
    }

    /// A normal exit: this start was good.
    pub(crate) fn finish(&self) {
        if let Some(path) = &self.path {
            safe_start::mark_healthy(path);
        }
    }

    /// Half a minute of running is a good start, even if the window is never closed normally.
    fn tick(&mut self) {
        if !self.marked && !self.asking && self.begun.elapsed() >= HEALTHY_AFTER {
            self.marked = true;
            self.finish();
        }
    }
}

impl ClipCraftApp {
    pub(crate) fn tick_safe_start(&mut self) {
        if let Some(safe_start) = self.safe_start.as_mut() {
            safe_start.tick();
        }
    }

    /// The question, while there is one.
    pub(crate) fn safe_start_dialog(&mut self, context: &egui::Context) {
        if !self.safe_start.as_ref().is_some_and(|start| start.asking) {
            return;
        }
        let mut defaults = false;
        let mut saved = false;
        egui::Modal::new(egui::Id::new("safe-start-dialog"))
            .frame(
                egui::Frame::new()
                    .fill(kit::PANEL)
                    .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
                    .corner_radius(egui::CornerRadius::same(crate::kit::radius::LG))
                    .inner_margin(egui::Margin::same(24)),
            )
            .show(context, |ui| {
                ui.set_width(440.0);
                ui.label(
                    RichText::new("ClipCraft did not start properly the last few times")
                        .color(TEXT)
                        .size(crate::kit::text_size::LEAD)
                        .family(kit::semibold()),
                );
                ui.add_space(crate::kit::space::SM);
                ui.label(
                    RichText::new(
                        "It is open now with its default settings, so you can get in. Your clips \
                         and your library are untouched.",
                    )
                    .color(TEXT)
                    .size(crate::kit::text_size::BODY),
                );
                ui.add_space(crate::kit::space::MD);
                ui.label(
                    RichText::new(
                        "If you keep the defaults, a copy of your saved settings is kept next to \
                         them. If you think the settings were not the problem, go back to them.",
                    )
                    .color(MUTED)
                    .size(crate::kit::text_size::SMALL),
                );
                ui.add_space(crate::kit::space::XL);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = crate::kit::space::MD;
                    if kit::labelled_button(ui, Icon::Check, "Keep the defaults", true, true)
                        .clicked()
                    {
                        defaults = true;
                    }
                    if kit::labelled_button(ui, Icon::Restart, "Use my settings", false, true)
                        .clicked()
                    {
                        saved = true;
                    }
                });
            });
        if defaults || saved {
            context.request_repaint();
        }
        if defaults {
            let safe_to_save = self.keep_default_settings();
            self.answer_safe_start(safe_to_save);
        } else if saved {
            self.load_saved_settings();
            self.answer_safe_start(true);
        }
    }

    /// The person chose. Until now the saved file was not written and the first-run setup was held
    /// back; both now go ahead, from whichever settings were chosen.
    pub(crate) fn answer_safe_start(&mut self, safe_to_save: bool) {
        if let Some(start) = self.safe_start.as_mut() {
            start.asking = false;
        }
        self.settings_on_hold = !safe_to_save;
        if safe_to_save {
            self.reconcile_clips_folder();
        }
        self.start_first_run_setup();
    }

    /// Sets the saved settings aside so the defaults can be used. False when that failed: the
    /// saved file is then left alone and not written to this time.
    pub(crate) fn keep_default_settings(&mut self) -> bool {
        let Some(path) = self.settings_path.clone() else {
            return true;
        };
        match safe_start::reset_settings(&path) {
            Ok(Some(_)) => {
                self.capture.notice = crate::capture::notice(
                    "Started with the default settings. Your old settings are kept in a copy next to them.",
                    false,
                    None,
                );
                true
            }
            Ok(None) => true,
            Err(_) => {
                self.capture.notice = crate::capture::notice(
                    "Could not set the old settings aside, so they were left as they are. Changes to settings will not be saved until the next start.",
                    true,
                    None,
                );
                false
            }
        }
    }
}
