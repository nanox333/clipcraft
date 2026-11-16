//! What the Clips page shows before there is a clip: three steps to the first one, with the real
//! shortcut, instead of an empty box (or, as it used to be for a brand-new folder, an error).

use eframe::egui::{self, Color32, RichText, Vec2};

use crate::kit::{self, MUTED, TEXT};
use crate::{ClipCraftApp, Icon};

/// What the person chose on the page.
pub(crate) enum HeroAction {
    StartClipping,
    ChooseFolder,
}

/// A key as a small cap, for the shortcut in step three.
fn keycap(ui: &mut egui::Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::FontId::new(crate::kit::text_size::LEAD, kit::semibold()),
        Color32::PLACEHOLDER,
    );
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2((galley.size().x + 24.0).max(40.0), 36.0),
        egui::Sense::hover(),
    );
    let body = egui::Rect::from_min_max(rect.min, rect.max - egui::vec2(0.0, 3.0));
    ui.painter()
        .rect_filled(rect, 9.0, Color32::from_rgb(20, 20, 23));
    ui.painter()
        .rect_filled(body, 8.0, Color32::from_rgb(58, 58, 66));
    ui.painter().rect_stroke(
        body,
        8.0,
        egui::Stroke::new(1.0_f32, Color32::from_rgb(82, 82, 92)),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(body.center() - galley.size() / 2.0, galley, TEXT);
}

fn seconds_words(seconds: u32) -> String {
    match seconds {
        s if s < 60 => format!("{s} seconds"),
        s if s % 60 == 0 => format!("{} minute{}", s / 60, if s == 60 { "" } else { "s" }),
        s => format!("{}:{:02}", s / 60, s % 60),
    }
}

impl ClipCraftApp {
    /// The first-clip guide. `folder` is where clips will be saved.
    pub(crate) fn empty_hero(&mut self, ui: &mut egui::Ui, folder: &str) -> Option<HeroAction> {
        let mut action = None;
        let shortcut = self
            .recording
            .hotkeys
            .bindings(clipcraft_engine::HotkeyAction::SaveClip)
            .next()
            .unwrap_or("the save shortcut")
            .replace('+', " + ");
        let seconds = seconds_words(self.recording.replay_seconds);
        let clipping = self.capture.is_active();
        ui.add_space((ui.available_height() * 0.10).clamp(16.0, 90.0));
        ui.vertical_centered(|ui| {
            let (circle, _) = ui.allocate_exact_size(Vec2::splat(84.0), egui::Sense::hover());
            // A soft lime glow behind the icon, in the look of the setup's welcome page.
            for (radius, alpha) in [(54.0_f32, 10_u8), (46.0, 16), (40.0, 26)] {
                ui.painter().circle_filled(
                    circle.center(),
                    radius,
                    Color32::from_rgba_unmultiplied(190, 255, 43, alpha),
                );
            }
            ui.painter()
                .circle_filled(circle.center(), 34.0, Color32::from_rgb(28, 32, 20));
            kit::draw_icon(ui.painter(), circle.center(), Icon::Record, kit::LIME, 1.6);
            ui.add_space(crate::kit::space::MD);
            ui.label(
                RichText::new("Your first clip is one key away")
                    .color(TEXT)
                    .size(30.0)
                    .family(kit::semibold()),
            );
            ui.add_space(crate::kit::space::XS);
            ui.label(
                RichText::new(format!(
                    "ClipCraft keeps the last {seconds} of your game in memory. Press your shortcut and that moment becomes a clip, saved only on this computer."
                ))
                .color(MUTED)
                .size(crate::kit::text_size::BODY),
            );
        });
        ui.add_space(crate::kit::space::XXL);
        let room = ui.available_width().min(900.0);
        let gap = 0.0;
        let card = ((room - gap * 2.0) / 3.0).max(180.0);
        ui.vertical_centered(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(card * 3.0 + gap * 2.0, 190.0),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    step_card(
                        ui,
                        card,
                        1,
                        "Start clipping",
                        if clipping {
                            "Clipping is on. ClipCraft is remembering your game."
                        } else {
                            "Press Start clipping at the top. ClipCraft waits for your game."
                        },
                        |ui| {
                            if clipping {
                                kit::chip(ui, "On");
                            } else if kit::labelled_button(
                                ui,
                                Icon::Record,
                                "Start clipping",
                                true,
                                true,
                            )
                            .clicked()
                            {
                                action = Some(HeroAction::StartClipping);
                            }
                        },
                    );
                    step_card(
                        ui,
                        card,
                        2,
                        "Play your game",
                        "Nothing is saved while you play. Only the last moments are kept, and old ones fall away.",
                        |_| {},
                    );
                    step_card(
                        ui,
                        card,
                        3,
                        "Press your shortcut",
                        "That moment is saved here as a clip, ready to watch, trim and share.",
                        |ui| {
                            for (index, part) in shortcut.split(" + ").enumerate() {
                                if index > 0 {
                                    ui.label(RichText::new("+").color(MUTED));
                                }
                                keycap(ui, part);
                            }
                        },
                    );
                },
            );
        });
        ui.add_space(crate::kit::space::XL);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(format!("Clips are saved to {folder}"))
                    .color(kit::FAINT)
                    .size(crate::kit::text_size::SMALL),
            );
            ui.add_space(crate::kit::space::XS);
            if kit::labelled_button(ui, Icon::Folder, "Choose another folder", false, true)
                .clicked()
            {
                action = Some(HeroAction::ChooseFolder);
            }
        });
        action
    }
}

/// One numbered step: no box, only a number, a heading and two lines, with a hairline before it
/// (except the first) so the three read as a row.
fn step_card(
    ui: &mut egui::Ui,
    width: f32,
    number: u32,
    title: &str,
    body: &str,
    extra: impl FnOnce(&mut egui::Ui),
) {
    let inner = (width - 40.0).max(0.0);
    ui.allocate_ui_with_layout(
        Vec2::new(width, 170.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            if number > 1 {
                let line = egui::Rect::from_min_size(
                    ui.max_rect().left_top() + Vec2::new(0.0, 6.0),
                    Vec2::new(1.0, 150.0),
                );
                ui.painter().rect_filled(line, 0.0, kit::BORDER);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.vertical(|ui| {
                    ui.set_width(inner);
                    let (badge, _) =
                        ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::hover());
                    ui.painter().circle_stroke(
                        badge.center(),
                        13.0,
                        egui::Stroke::new(
                            1.0_f32,
                            Color32::from_rgba_unmultiplied(190, 255, 43, 110),
                        ),
                    );
                    ui.painter().text(
                        badge.center(),
                        egui::Align2::CENTER_CENTER,
                        number.to_string(),
                        egui::FontId::new(crate::kit::text_size::SMALL, kit::semibold()),
                        kit::LIME,
                    );
                    ui.add_space(crate::kit::space::SM);
                    ui.label(
                        RichText::new(title)
                            .color(TEXT)
                            .size(crate::kit::text_size::LEAD)
                            .family(kit::semibold()),
                    );
                    ui.add_space(crate::kit::space::XS);
                    ui.add(
                        egui::Label::new(
                            RichText::new(body)
                                .color(MUTED)
                                .size(crate::kit::text_size::SMALL),
                        )
                        .wrap(),
                    );
                    ui.add_space(crate::kit::space::SM);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        extra(ui);
                    });
                });
            });
        },
    );
}
