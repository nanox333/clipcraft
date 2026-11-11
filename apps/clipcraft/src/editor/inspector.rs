//! The editor's side panels for the selected segment: effects (colour looks, fades) and text.

use super::*;

impl EditorState {
    /// Transitions and colour effects, applied to the selected segment with one click.
    pub(super) fn effects_tab(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (name, effects) in [("Transitions", false), ("Effects", true)] {
                if tab_pill(ui, name, self.effect_tab == effects).clicked() {
                    self.effect_tab = effects;
                }
            }
        });
        ui.add_space(10.0);
        let selected = self.selected_look();
        let target_text = self.selected_text_layer();
        let has_target = selected.is_some() || target_text.is_some();
        let enabled = !self.busy();
        if target_text.is_some() {
            ui.label(
                RichText::new("Applies to the selected text layer.")
                    .size(12.0)
                    .color(FAINT),
            );
            ui.add_space(6.0);
        } else if selected.is_none() && !self.effect_tab {
            ui.label(
                RichText::new("Pick a segment on the timeline, or use a preset for every segment.")
                    .size(12.0)
                    .color(FAINT),
            );
            ui.add_space(6.0);
        } else if !has_target {
            ui.label(
                RichText::new("Pick a segment or text layer on the timeline first.")
                    .size(12.0)
                    .color(FAINT),
            );
            ui.add_space(6.0);
        }
        let columns = (((ui.available_width() + 8.0) / 118.0).floor() as usize).clamp(2, 6);
        let card = Vec2::new(
            ((ui.available_width() - 8.0 * (columns - 1) as f32) / columns as f32).max(20.0),
            78.0,
        );
        let mut action = None;
        egui::ScrollArea::vertical()
            .id_salt("editor-effects")
            .max_height((height - 130.0).max(30.0))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                ui.horizontal_wrapped(|ui| {
                    if self.effect_tab {
                        for effect in Effect::ALL {
                            let active = match (&target_text, &selected) {
                                (Some((_, t)), _) => t.effect == effect,
                                (None, Some((_, l))) => l.effect == effect,
                                _ => false,
                            };
                            let response = preset_card(
                                ui,
                                card,
                                effect.name(),
                                active,
                                enabled && has_target,
                                |painter, rect| {
                                    let bars = [
                                        [255, 40, 20],
                                        [20, 200, 20],
                                        [250, 240, 0],
                                        [30, 40, 250],
                                        [250, 40, 250],
                                        [0, 240, 250],
                                    ];
                                    let width = rect.width() / bars.len() as f32;
                                    for (i, bar) in bars.iter().enumerate() {
                                        let mut pixel = [bar[0], bar[1], bar[2], 255];
                                        effect.apply_rgba(&mut pixel);
                                        painter.rect_filled(
                                            egui::Rect::from_min_size(
                                                rect.min + Vec2::new(width * i as f32, 0.0),
                                                Vec2::new(width + 0.5, rect.height()),
                                            ),
                                            0,
                                            Color32::from_rgb(pixel[0], pixel[1], pixel[2]),
                                        );
                                    }
                                },
                            );
                            if response.clicked() {
                                let value = if active { Effect::None } else { effect };
                                if let Some((id, layer)) = &target_text {
                                    let mut layer = layer.clone();
                                    layer.effect = value;
                                    action = Some(EditorAction::UpdateText { id: *id, layer });
                                } else if let Some((id, look)) = &selected {
                                    let mut look = look.clone();
                                    look.effect = value;
                                    action = Some(EditorAction::Look { id: *id, look });
                                }
                            }
                        }
                    } else {
                        for item in TRANSITIONS.iter().filter(|item| {
                            target_text.is_none()
                                || matches!(
                                    item.kind,
                                    TransitionKind::In(Fade::Black)
                                        | TransitionKind::Out(Fade::Black)
                                )
                        }) {
                            let active = match (&target_text, &selected, item.kind) {
                                (Some((_, t)), _, TransitionKind::In(_)) => t.fade_in != Fade::None,
                                (Some((_, t)), _, TransitionKind::Out(_)) => {
                                    t.fade_out != Fade::None
                                }
                                (None, Some((_, l)), TransitionKind::In(f)) => l.fade_in == f,
                                (None, Some((_, l)), TransitionKind::Out(f)) => l.fade_out == f,
                                _ => false,
                            };
                            let local = !matches!(item.kind, TransitionKind::All(_));
                            let response = preset_card(
                                ui,
                                card,
                                item.name,
                                active,
                                enabled && (has_target || !local),
                                |painter, rect| {
                                    painter.rect_filled(rect, 0, Color32::from_gray(72));
                                    let (left, right, color) = match item.kind {
                                        TransitionKind::In(f) => (true, false, f),
                                        TransitionKind::Out(f) => (false, true, f),
                                        TransitionKind::All(TransitionPreset::DipWhite) => {
                                            (true, true, Fade::White)
                                        }
                                        TransitionKind::All(TransitionPreset::None) => {
                                            (false, false, Fade::None)
                                        }
                                        TransitionKind::All(_) => (true, true, Fade::Black),
                                    };
                                    let shade = if color == Fade::White {
                                        Color32::WHITE
                                    } else {
                                        Color32::BLACK
                                    };
                                    for (on, from_left) in [(left, true), (right, false)] {
                                        if !on {
                                            continue;
                                        }
                                        let w = rect.width() * 0.45;
                                        let (edge, inner) = if from_left {
                                            (rect.left(), rect.left() + w)
                                        } else {
                                            (rect.right(), rect.right() - w)
                                        };
                                        painter.add(egui::Shape::convex_polygon(
                                            vec![
                                                egui::pos2(edge, rect.top()),
                                                egui::pos2(edge, rect.bottom()),
                                                egui::pos2(inner, rect.bottom()),
                                            ],
                                            shade,
                                            Stroke::NONE,
                                        ));
                                    }
                                },
                            );
                            if response.clicked() {
                                match (item.kind, &selected) {
                                    (TransitionKind::All(preset), _) => {
                                        action = Some(EditorAction::Preset { preset });
                                    }
                                    (TransitionKind::In(_) | TransitionKind::Out(_), _)
                                        if target_text.is_some() =>
                                    {
                                        if let Some((id, layer)) = &target_text {
                                            let mut layer = layer.clone();
                                            let fade =
                                                if active { Fade::None } else { Fade::Black };
                                            if matches!(item.kind, TransitionKind::In(_)) {
                                                layer.fade_in = fade;
                                            } else {
                                                layer.fade_out = fade;
                                            }
                                            action =
                                                Some(EditorAction::UpdateText { id: *id, layer });
                                        }
                                    }
                                    (TransitionKind::In(f), Some((id, look))) => {
                                        let mut look = look.clone();
                                        look.fade_in = if active { Fade::None } else { f };
                                        action = Some(EditorAction::Look { id: *id, look });
                                    }
                                    (TransitionKind::Out(f), Some((id, look))) => {
                                        let mut look = look.clone();
                                        look.fade_out = if active { Fade::None } else { f };
                                        action = Some(EditorAction::Look { id: *id, look });
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                });
            });
        if let Some(action) = action {
            self.edit(action);
        }
    }
    /// Text layers: add one at the playhead, then style it. Each lives on its own track above the
    /// video, can be moved and trimmed on the timeline and takes effects and fades like a layer.
    pub(super) fn text_tab(&mut self, ui: &mut egui::Ui, height: f32) {
        let enabled = !self.busy();
        ui.horizontal(|ui| {
            if tool(
                ui,
                kit::Icon::Plus,
                "Add text",
                true,
                enabled && self.has_clips(),
            )
            .on_hover_text("Put a text layer on the text track at the playhead")
            .clicked()
            {
                let total = self.session.duration();
                let length = Tick::from_units(3, 1).min(total);
                let start = self.playhead.min(total - length).max(Tick::ZERO);
                self.edit(EditorAction::AddText {
                    start,
                    duration: length,
                    text: "Your title".into(),
                });
                if let Some(layer) = self.session.texts().last() {
                    self.selected_text = Some(layer.id);
                    self.selected = None;
                }
            }
        });
        ui.add_space(10.0);
        let Some((id, layer)) = self.selected_text_layer() else {
            ui.label(
                RichText::new(
                    "Add text, or pick a text layer on the track above the video. Effects and Transitions apply to it too.",
                )
                .size(12.0)
                .color(FAINT),
            );
            return;
        };
        if self.title_draft.0 != id {
            self.title_draft = (id, layer.text.clone());
        }
        let mut next = layer.clone();
        let total = self.session.duration();
        ui.add_enabled_ui(enabled, |ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.title_draft.1)
                    .hint_text("Type the text, then press Enter")
                    .char_limit(clipcraft_engine::MAX_TITLE_CHARS)
                    .margin(egui::Margin::symmetric(10, 8))
                    .desired_width(ui.available_width()),
            );
            field.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Text layer")
            });
            if field.lost_focus() {
                next.text = self.title_draft.1.replace(char::is_control, " ");
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new("Starts").size(13.0).color(MUTED));
                let mut start = layer.start.seconds();
                if ui
                    .add(
                        egui::DragValue::new(&mut start)
                            .speed(0.05)
                            .range(0.0..=total.seconds())
                            .fixed_decimals(2)
                            .suffix(" s"),
                    )
                    .changed()
                {
                    let moved = Tick::from_units((start * 1000.0).round() as i64, 1000);
                    let length = layer.duration();
                    next.start = moved.min(total - MIN_TEXT).max(Tick::ZERO);
                    next.end = (next.start + length).min(total.max(next.start + MIN_TEXT));
                }
                ui.label(RichText::new("Lasts").size(13.0).color(MUTED));
                let mut length = layer.duration().seconds();
                if ui
                    .add(
                        egui::DragValue::new(&mut length)
                            .speed(0.05)
                            .range(0.04..=total.seconds().max(0.04))
                            .fixed_decimals(2)
                            .suffix(" s"),
                    )
                    .changed()
                {
                    let ticks = Tick::from_units((length * 1000.0).round() as i64, 1000);
                    next.end = (layer.start + ticks).min(total.max(layer.start + MIN_TEXT));
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new("Size").size(13.0).color(MUTED));
                let mut size = u32::from(layer.size);
                if ui
                    .add(
                        egui::Slider::new(&mut size, 20..=250)
                            .show_value(false)
                            .trailing_fill(true),
                    )
                    .on_hover_text("Letter height")
                    .changed()
                {
                    next.size = size.clamp(20, 250) as u16;
                }
                separator(ui);
                ui.label(RichText::new("Colour").size(13.0).color(MUTED));
                for (name, rgb) in TEXT_COLORS {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name)
                    });
                    ui.painter().circle_filled(
                        rect.center(),
                        9.0,
                        Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
                    );
                    if layer.color == rgb {
                        ui.painter()
                            .circle_stroke(rect.center(), 11.0, Stroke::new(2.0_f32, LIME));
                    }
                    if response
                        .on_hover_text(name)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        next.color = rgb;
                    }
                }
            });
            ui.add_space(10.0);
            let columns = (((ui.available_width() + 8.0) / 118.0).floor() as usize).clamp(2, 6);
            let card = Vec2::new(
                ((ui.available_width() - 8.0 * (columns - 1) as f32) / columns as f32).max(20.0),
                78.0,
            );
            egui::ScrollArea::vertical()
                .id_salt("editor-text")
                .max_height((height - 215.0).max(100.0))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                    ui.horizontal_wrapped(|ui| {
                        for (name, x, y) in [
                            ("Title", 500_u16, 500_u16),
                            ("Lower third", 500, 880),
                            ("Top banner", 500, 120),
                            ("Corner tag", 150, 900),
                        ] {
                            let active = layer.x.abs_diff(x) < 25 && layer.y.abs_diff(y) < 25;
                            let response =
                                preset_card(ui, card, name, active, true, |painter, rect| {
                                    painter.rect_filled(rect, 0, Color32::from_gray(72));
                                    painter.text(
                                        egui::pos2(
                                            rect.left() + rect.width() * f32::from(x) / 1000.0,
                                            rect.top() + rect.height() * f32::from(y) / 1000.0,
                                        ),
                                        Align2::CENTER_CENTER,
                                        "Aa",
                                        FontId::new(15.0, kit::semibold()),
                                        Color32::WHITE,
                                    );
                                });
                            if response.clicked() {
                                (next.x, next.y) = (x, y);
                            }
                        }
                    });
                });
            ui.label(
                RichText::new("Drag the text in the preview to move it anywhere.")
                    .size(12.0)
                    .color(FAINT),
            );
        });
        if next != layer {
            self.edit(EditorAction::UpdateText { id, layer: next });
        }
    }
}
