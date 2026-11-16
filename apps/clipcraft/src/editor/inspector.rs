//! The editor's side panels for the selected segment: effects (colour looks, fades) and text.

use super::*;
use clipcraft_engine::{TextFont, TextWeight};

impl EditorState {
    /// Transitions and colour effects, applied to the selected segment with one click.
    pub(super) fn effects_tab(&mut self, ui: &mut egui::Ui, height: f32) {
        let selected = self.selected_look();
        let target_text = self.selected_text_layer();
        let has_target = selected.is_some() || target_text.is_some();
        let enabled = !self.busy();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = crate::kit::space::SM;
            if let Some(index) = sub_tabs(
                ui,
                &["Transitions", "Effects"],
                usize::from(self.effect_tab),
            ) {
                self.effect_tab = index == 1;
            }
            // What the choice will apply to, on the same line instead of a paragraph of its own.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (note, lit) = if target_text.is_some() {
                    ("Applies to the text layer", true)
                } else if selected.is_some() {
                    ("Applies to the segment", true)
                } else if self.effect_tab {
                    ("Select a segment first", false)
                } else {
                    ("Select a segment, or pick a preset", false)
                };
                ui.label(
                    RichText::new(note)
                        .size(crate::kit::text_size::CAPTION)
                        .color(if lit { MUTED } else { FAINT }),
                );
            });
        });
        ui.add_space(crate::kit::space::MD);
        // Room is kept for the scrollbar, so the last card of a row is not pushed to the next.
        let usable = (ui.available_width() - 14.0).max(20.0);
        let columns = (((usable + 8.0) / 118.0).floor() as usize).clamp(2, 6);
        let card = Vec2::new(
            ((usable - 8.0 * (columns - 1) as f32) / columns as f32).max(20.0),
            78.0,
        );
        let mut action = None;
        egui::ScrollArea::vertical()
            .id_salt("editor-effects")
            .max_height((height - 130.0).max(30.0))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing =
                    Vec2::new(crate::kit::space::SM, crate::kit::space::SM);
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
        let selected = self.selected_text_layer();
        if let Some((id, layer)) = &selected
            && self.title_draft.0 != *id
        {
            self.title_draft = (*id, layer.text.clone());
        }
        let mut typed: Option<String> = None;
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
            // What the selected text says, on the same line as the button.
            if selected.is_some() {
                ui.add_enabled_ui(enabled, |ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.title_draft.1)
                            .hint_text("Type the text, then press Enter")
                            .char_limit(clipcraft_engine::MAX_TITLE_CHARS)
                            .margin(egui::Margin::symmetric(12, 6))
                            .desired_width((ui.available_width() - 34.0).max(40.0)),
                    );
                    field.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Text layer")
                    });
                    if field.lost_focus() {
                        typed = Some(self.title_draft.1.replace(char::is_control, " "));
                    }
                });
            }
            kit::help(
                ui,
                "Drag the text in the preview to move it anywhere. Effects and Transitions apply to it too.",
            );
        });
        ui.add_space(crate::kit::space::SM);
        let Some((id, layer)) = selected else {
            // Nothing selected: say what text is for, in the room the options would have.
            ui.add_space(crate::kit::space::LG);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(if self.session.texts().is_empty() {
                        "No text yet"
                    } else {
                        "Pick a text layer"
                    })
                    .size(crate::kit::text_size::BODY)
                    .family(kit::semibold())
                    .color(TEXT),
                );
                ui.add_space(crate::kit::space::XS);
                ui.label(
                    RichText::new(if self.session.texts().is_empty() {
                        "Add a title or caption on top of the video."
                    } else {
                        "Click one on the text track above the video."
                    })
                    .size(crate::kit::text_size::CAPTION)
                    .color(MUTED),
                );
            });
            return;
        };
        let mut next = layer.clone();
        if let Some(text) = typed {
            next.text = text;
        }
        let total = self.session.duration();
        egui::ScrollArea::vertical()
            .id_salt("editor-text")
            .auto_shrink([false, false])
            .max_height((height - 100.0).max(100.0))
            .show(ui, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(crate::kit::space::SM, 5.0);
                    let mut panel = TextPanel {
                        layer: &layer,
                        next: &mut next,
                        total,
                        id,
                    };
                    // Compact rows, top to bottom: ready-made styles, the letters, the sliders,
                    // colour and look, then when and where.
                    panel.styles(ui);
                    panel.font_row(ui);
                    panel.sliders_row(ui);
                    panel.colour_and_look(ui);
                    panel.timing_and_position(ui);
                });
            });
        if next != layer {
            self.edit(EditorAction::UpdateText { id, layer: next });
        }
    }
}

/// The options of the selected text layer. Each part writes its changes to `next`; the layer
/// itself (`layer`) is only read.
struct TextPanel<'a> {
    layer: &'a SequenceText,
    next: &'a mut SequenceText,
    total: Tick,
    id: u64,
}

impl TextPanel<'_> {
    /// Ready-made looks, as a row of small tiles.
    fn styles(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for preset in STYLE_PRESETS {
                if style_tile(ui, &preset, preset.matches(self.layer)) {
                    preset.apply(self.next);
                }
            }
        });
    }

    /// Typeface, weight and the three switches for the letters, on one line.
    fn font_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let names: Vec<&str> = TextFont::ALL.iter().map(|font| font.name()).collect();
            let current = TextFont::ALL
                .iter()
                .position(|font| *font == self.layer.font)
                .unwrap_or(0);
            if let Some(index) = sub_tabs(ui, &names, current) {
                self.next.font = TextFont::ALL[index];
            }
            let weights: Vec<&str> = TextWeight::ALL.iter().map(|weight| weight.name()).collect();
            let current = TextWeight::ALL
                .iter()
                .position(|weight| *weight == self.layer.weight)
                .unwrap_or(0);
            if let Some(index) = sub_tabs(ui, &weights, current) {
                self.next.weight = TextWeight::ALL[index];
            }
            if mini_toggle(ui, "I", self.layer.italic, "Italic") {
                self.next.italic = !self.layer.italic;
            }
            if mini_toggle(ui, "U", self.layer.underline, "Underline") {
                self.next.underline = !self.layer.underline;
            }
            if mini_toggle(ui, "AA", self.layer.caps, "All capitals") {
                self.next.caps = !self.layer.caps;
            }
        });
    }

    /// Size, letter spacing and opacity as three small sliders on one line.
    fn sliders_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = crate::kit::space::LG;
            if let Some(value) = mini_slider(
                ui,
                "Size",
                (f32::from(self.layer.size) - 20.0) / 230.0,
                &format!("Letter height: {}", self.layer.size),
            ) {
                self.next.size = (20.0 + value * 230.0).round().clamp(20.0, 250.0) as u16;
            }
            if let Some(value) = mini_slider(
                ui,
                "Spacing",
                (f32::from(self.layer.tracking) + 50.0) / 450.0,
                &format!("Space between the letters: {}", self.layer.tracking),
            ) {
                self.next.tracking = (value * 450.0 - 50.0).round().clamp(-50.0, 400.0) as i16;
            }
            if let Some(value) = mini_slider(
                ui,
                "Opacity",
                (f32::from(self.layer.opacity) - 10.0) / 90.0,
                &format!("How solid the text is: {}%", self.layer.opacity),
            ) {
                self.next.opacity = (10.0 + value * 90.0).round().clamp(10.0, 100.0) as u8;
            }
        });
    }

    /// A few good colours and any other, then what makes the letters stand out.
    fn colour_and_look(&mut self, ui: &mut egui::Ui) {
        let picker_open_id = egui::Id::new(("text-picker-open", self.id));
        let picker_colour_id = egui::Id::new(("text-picker-colour", self.id));
        let picker_hex_id = egui::Id::new(("text-picker-hex", self.id));
        let layer = self.layer;
        let mut picker_open =
            ui.data_mut(|data| data.get_temp::<bool>(picker_open_id).unwrap_or(false));
        let mut picking: Option<egui::ecolor::Hsva> =
            ui.data_mut(|data| data.get_temp(picker_colour_id));
        let in_row = TEXT_COLORS.iter().any(|(_, rgb)| layer.color == *rgb);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for (name, rgb) in TEXT_COLORS {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name)
                });
                ui.painter().circle_filled(
                    rect.center(),
                    8.5,
                    Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
                );
                if layer.color == rgb {
                    ui.painter()
                        .circle_stroke(rect.center(), 10.5, Stroke::new(1.5_f32, TEXT));
                } else if response.hovered() {
                    ui.painter()
                        .circle_stroke(rect.center(), 10.5, Stroke::new(1.5_f32, MUTED));
                }
                if response
                    .on_hover_text(name)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.next.color = rgb;
                    picker_open = false;
                    picking = None;
                }
            }
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Pick any colour")
            });
            crate::colour_picker::rainbow_swatch(ui.painter(), rect.center(), 8.5);
            if picker_open || !in_row || response.hovered() {
                ui.painter().circle_stroke(
                    rect.center(),
                    10.5,
                    Stroke::new(1.5_f32, if picker_open || !in_row { TEXT } else { MUTED }),
                );
            }
            if response
                .on_hover_text("Any colour")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                picker_open = !picker_open;
                if picker_open && picking.is_none() {
                    picking = Some(egui::ecolor::Hsva::from_srgb(layer.color));
                }
            }
            ui.add_space(crate::kit::space::SM);
            if mini_toggle(
                ui,
                "Shadow",
                layer.shadow,
                "A soft shadow under the letters",
            ) {
                self.next.shadow = !layer.shadow;
            }
            if mini_toggle(ui, "Outline", layer.outline, "A thin dark outline") {
                self.next.outline = !layer.outline;
            }
            if mini_toggle(ui, "Box", layer.backdrop, "A dark box behind the text") {
                self.next.backdrop = !layer.backdrop;
            }
        });
        if picker_open {
            let mut hsva = picking.unwrap_or_else(|| egui::ecolor::Hsva::from_srgb(layer.color));
            let mut hex = ui
                .data_mut(|data| data.get_temp::<String>(picker_hex_id))
                .unwrap_or_else(|| crate::colour_picker::to_hex(hsva));
            ui.allocate_ui(
                Vec2::new((ui.available_width() - 14.0).clamp(160.0, 260.0), 0.0),
                |ui| {
                    let picked = crate::colour_picker::show(ui, &mut hsva, &mut hex);
                    // The colour is applied when it is let go, so one drag is one undo step.
                    if picked.settled {
                        self.next.color = hsva.to_srgb();
                    }
                },
            );
            picking = Some(hsva);
            ui.data_mut(|data| data.insert_temp(picker_hex_id, hex));
        }
        ui.data_mut(|data| {
            data.insert_temp(picker_open_id, picker_open);
            match picking {
                Some(hsva) if picker_open => data.insert_temp(picker_colour_id, hsva),
                _ => data.remove::<egui::ecolor::Hsva>(picker_colour_id),
            }
        });
    }

    /// When it appears and for how long, and where on the picture it sits.
    fn timing_and_position(&mut self, ui: &mut egui::Ui) {
        let (layer, total) = (self.layer, self.total);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = crate::kit::space::SM;
            let mut start = layer.start.seconds();
            if ui
                .add(
                    egui::DragValue::new(&mut start)
                        .speed(0.05)
                        .range(0.0..=total.seconds())
                        .fixed_decimals(2)
                        .prefix("Starts  ")
                        .suffix(" s"),
                )
                .changed()
            {
                let moved = Tick::from_units((start * 1000.0).round() as i64, 1000);
                let length = layer.duration();
                self.next.start = moved.min(total - MIN_TEXT).max(Tick::ZERO);
                self.next.end =
                    (self.next.start + length).min(total.max(self.next.start + MIN_TEXT));
            }
            let mut length = layer.duration().seconds();
            if ui
                .add(
                    egui::DragValue::new(&mut length)
                        .speed(0.05)
                        .range(0.04..=total.seconds().max(0.04))
                        .fixed_decimals(2)
                        .prefix("Lasts  ")
                        .suffix(" s"),
                )
                .changed()
            {
                let ticks = Tick::from_units((length * 1000.0).round() as i64, 1000);
                self.next.end = (layer.start + ticks).min(total.max(layer.start + MIN_TEXT));
            }
            ui.add_space(crate::kit::space::SM);
            ui.label(
                RichText::new("Position")
                    .size(crate::kit::text_size::SMALL)
                    .color(MUTED),
            );
            if let Some((x, y)) = position_grid(ui, layer.x, layer.y) {
                (self.next.x, self.next.y) = (x, y);
            }
        });
    }
}

/// A small tile showing a ready-made look; its name is the tooltip. True when clicked.
fn style_tile(ui: &mut egui::Ui, preset: &StylePreset, active: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(56.0, 36.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, preset.name));
    preset.paint(&ui.painter().with_clip_rect(rect), rect);
    let corner = f32::from(crate::kit::radius::MD);
    round_corners(ui.painter(), rect, corner, PANEL);
    ui.painter().rect_stroke(
        rect,
        corner,
        Stroke::new(
            if active { 2.0_f32 } else { 1.0_f32 },
            if active {
                LIME
            } else if response.hovered() {
                MUTED
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    response
        .on_hover_text(preset.name)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// A nine-spot picture of where the text sits (corners, edges, middle). Returns the spot clicked,
/// as the place it stands for.
fn position_grid(ui: &mut egui::Ui, x: u16, y: u16) -> Option<(u16, u16)> {
    const XS: [u16; 3] = [150, 500, 850];
    const YS: [u16; 3] = [120, 500, 880];
    const NAMES: [[&str; 3]; 3] = [
        ["Top left", "Top", "Top right"],
        ["Left", "Middle", "Right"],
        ["Bottom left", "Lower third", "Bottom right"],
    ];
    let (frame, _) = ui.allocate_exact_size(Vec2::new(64.0, 42.0), Sense::hover());
    ui.painter().rect_filled(frame, 6.0, CANVAS);
    ui.painter().rect_stroke(
        frame,
        6.0,
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    let mut chosen = None;
    for (row, name_row) in NAMES.iter().enumerate() {
        for (column, name) in name_row.iter().enumerate() {
            let spot = egui::Rect::from_center_size(
                egui::pos2(
                    frame.left() + 11.0 + column as f32 * 21.0,
                    frame.top() + 9.0 + row as f32 * 12.0,
                ),
                Vec2::new(16.0, 10.0),
            );
            let response = ui.interact(
                spot,
                egui::Id::new(("text-position", row, column)),
                Sense::click(),
            );
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, *name));
            let here = x.abs_diff(XS[column]) < 60 && y.abs_diff(YS[row]) < 60;
            let nearest = XS
                .iter()
                .enumerate()
                .min_by_key(|(_, value)| x.abs_diff(**value))
                .map(|(i, _)| i)
                == Some(column)
                && YS
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, value)| y.abs_diff(**value))
                    .map(|(i, _)| i)
                    == Some(row);
            let lit = here && nearest;
            ui.painter().rect_filled(
                spot.shrink2(Vec2::new(3.0, 2.0)),
                2.0,
                if lit {
                    LIME
                } else if response.hovered() {
                    MUTED
                } else {
                    Color32::from_gray(70)
                },
            );
            if response
                .on_hover_text(*name)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                chosen = Some((XS[column], YS[row]));
            }
        }
    }
    chosen
}

/// A small square or pill switch for the letters and look. Lime when on.
fn mini_toggle(ui: &mut egui::Ui, label: &str, on: bool, tip: &str) -> bool {
    const HEIGHT: f32 = 24.0;
    let font = FontId::new(crate::kit::text_size::CAPTION, kit::medium());
    let width = (ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER)
        .size()
        .x
        + 18.0)
        .max(HEIGHT + 2.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, HEIGHT), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, tip));
    let (fill, stroke, text) = if on {
        (
            Color32::from_rgba_unmultiplied(190, 255, 43, 30),
            LIME,
            LIME,
        )
    } else if response.hovered() {
        (kit::CONTROL_HOVER, BORDER, TEXT)
    } else {
        (kit::CONTROL, BORDER, MUTED)
    };
    ui.painter().rect_filled(rect, HEIGHT / 2.0, fill);
    ui.painter().rect_stroke(
        rect,
        HEIGHT / 2.0,
        Stroke::new(1.0_f32, stroke),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, label, font, text);
    response
        .on_hover_text(tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// A label and a slim slider together; the value is in the tooltip. Returns the new position
/// (`0..=1`) while it is dragged.
fn mini_slider(ui: &mut egui::Ui, label: &str, value: f32, tip: &str) -> Option<f32> {
    let mut changed = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = crate::kit::space::SM;
        ui.label(
            RichText::new(label)
                .size(crate::kit::text_size::SMALL)
                .color(MUTED),
        );
        changed = kit::slim_slider(ui, 96.0, value, true, tip);
    });
    changed
}

/// A look for a text layer in one click: the letters, colour and extras, and where it sits.
struct StylePreset {
    name: &'static str,
    font: TextFont,
    weight: TextWeight,
    italic: bool,
    caps: bool,
    tracking: i16,
    size: u16,
    color: [u8; 3],
    shadow: bool,
    outline: bool,
    backdrop: bool,
    opacity: u8,
    x: u16,
    y: u16,
}

const STYLE_PRESETS: [StylePreset; 6] = [
    StylePreset {
        name: "Title",
        font: TextFont::Sans,
        weight: TextWeight::Bold,
        italic: false,
        caps: false,
        tracking: 0,
        size: 110,
        color: [255, 255, 255],
        shadow: true,
        outline: false,
        backdrop: false,
        opacity: 100,
        x: 500,
        y: 500,
    },
    StylePreset {
        name: "Subtitle",
        font: TextFont::Sans,
        weight: TextWeight::Medium,
        italic: false,
        caps: false,
        tracking: 0,
        size: 55,
        color: [255, 255, 255],
        shadow: false,
        outline: false,
        backdrop: true,
        opacity: 100,
        x: 500,
        y: 880,
    },
    StylePreset {
        name: "Shout",
        font: TextFont::Sans,
        weight: TextWeight::Bold,
        italic: false,
        caps: true,
        tracking: 40,
        size: 130,
        color: [190, 255, 0],
        shadow: false,
        outline: true,
        backdrop: false,
        opacity: 100,
        x: 500,
        y: 500,
    },
    StylePreset {
        name: "Classic",
        font: TextFont::Serif,
        weight: TextWeight::Regular,
        italic: true,
        caps: false,
        tracking: 20,
        size: 95,
        color: [255, 255, 255],
        shadow: true,
        outline: false,
        backdrop: false,
        opacity: 100,
        x: 500,
        y: 500,
    },
    StylePreset {
        name: "Terminal",
        font: TextFont::Mono,
        weight: TextWeight::Bold,
        italic: false,
        caps: true,
        tracking: 80,
        size: 60,
        color: [255, 220, 40],
        shadow: false,
        outline: false,
        backdrop: true,
        opacity: 100,
        x: 150,
        y: 120,
    },
    StylePreset {
        name: "Quiet",
        font: TextFont::Sans,
        weight: TextWeight::Regular,
        italic: false,
        caps: true,
        tracking: 120,
        size: 50,
        color: [255, 255, 255],
        shadow: false,
        outline: false,
        backdrop: false,
        opacity: 80,
        x: 500,
        y: 900,
    },
];

impl StylePreset {
    /// Puts this look on `layer`, keeping what it says and when.
    fn apply(&self, layer: &mut SequenceText) {
        layer.font = self.font;
        layer.weight = self.weight;
        layer.italic = self.italic;
        layer.caps = self.caps;
        layer.underline = false;
        layer.tracking = self.tracking;
        layer.size = self.size;
        layer.color = self.color;
        layer.shadow = self.shadow;
        layer.outline = self.outline;
        layer.backdrop = self.backdrop;
        layer.opacity = self.opacity;
        layer.x = self.x;
        layer.y = self.y;
    }
    /// Whether `layer` has exactly this look.
    fn matches(&self, layer: &SequenceText) -> bool {
        layer.font == self.font
            && layer.weight == self.weight
            && layer.italic == self.italic
            && layer.caps == self.caps
            && layer.tracking == self.tracking
            && layer.size == self.size
            && layer.color == self.color
            && layer.shadow == self.shadow
            && layer.outline == self.outline
            && layer.backdrop == self.backdrop
            && layer.opacity == self.opacity
    }
    /// A small "Aa" in the look, as the picture of the card.
    fn paint(&self, painter: &egui::Painter, rect: egui::Rect) {
        painter.rect_filled(rect, 0, Color32::from_gray(58));
        let centre = rect.center();
        let family = match self.font {
            TextFont::Mono => egui::FontFamily::Monospace,
            _ if self.weight == TextWeight::Bold => kit::semibold(),
            _ => kit::medium(),
        };
        let font = FontId::new(20.0, family);
        let text = if self.caps { "AA" } else { "Aa" };
        let ink = Color32::from_rgba_unmultiplied(
            self.color[0],
            self.color[1],
            self.color[2],
            (f32::from(self.opacity) / 100.0 * 255.0) as u8,
        );
        if self.backdrop {
            painter.rect_filled(
                egui::Rect::from_center_size(centre, Vec2::new(46.0, 30.0)),
                3.0,
                Color32::from_black_alpha(170),
            );
        }
        if self.outline {
            for offset in [(-1.5, 0.0), (1.5, 0.0), (0.0, -1.5), (0.0, 1.5)] {
                painter.text(
                    centre + Vec2::new(offset.0, offset.1),
                    Align2::CENTER_CENTER,
                    text,
                    font.clone(),
                    Color32::BLACK,
                );
            }
        }
        if self.shadow {
            painter.text(
                centre + Vec2::new(1.5, 1.5),
                Align2::CENTER_CENTER,
                text,
                font.clone(),
                Color32::from_black_alpha(180),
            );
        }
        painter.text(centre, Align2::CENTER_CENTER, text, font, ink);
    }
}
