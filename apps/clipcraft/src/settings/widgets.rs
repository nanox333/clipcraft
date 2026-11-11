//! Small shared pieces of the Settings screens: cards, rows, switches, dropdowns.

use super::*;

/// One option of a row of choices. True when a different option is clicked. A disabled option
/// explains itself on hover.
pub(crate) fn choice(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    enabled: bool,
    hint: &str,
) -> bool {
    toggle_chip(ui, label, selected, enabled, hint) && !selected
}

/// Like [`choice`], but true on every click, so a selected chip can be switched off.
pub(crate) fn toggle_chip(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    enabled: bool,
    hint: &str,
) -> bool {
    let font = egui::FontId::new(13.5, kit::medium());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let size = Vec2::new(galley.size().x + 28.0, 34.0);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, selected, label)
    });
    let (fill, stroke, text) = if !enabled {
        (Color32::from_rgb(20, 20, 23), BORDER, FAINT)
    } else if selected {
        (
            Color32::from_rgba_unmultiplied(190, 255, 43, 30),
            LIME,
            LIME,
        )
    } else if response.hovered() {
        (Color32::from_rgb(34, 34, 40), BORDER, TEXT)
    } else {
        (Color32::from_rgb(26, 26, 30), BORDER, TEXT)
    };
    ui.painter().rect_filled(rect, 10.0, fill);
    ui.painter().rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0_f32, stroke),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        egui::pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        text,
    );
    let clicked = response.clone().on_hover_text(hint).clicked();
    clicked && enabled
}

/// The heading over a group of settings.
pub(crate) fn section_heading(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .color(TEXT)
            .size(15.5)
            .family(kit::semibold()),
    );
    ui.add_space(10.0);
}

/// A titled group of setting rows on a rounded surface. Every settings page is built from these,
/// so spacing, edges and headings are the same everywhere.
pub(crate) fn card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    section_heading(ui, title);
    egui::Frame::new()
        .fill(kit::CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::symmetric(20, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
    ui.add_space(28.0);
}

/// A titled group that draws its own surfaces, such as a row of option tiles.
pub(crate) fn card_plain(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    section_heading(ui, title);
    body(ui);
    ui.add_space(28.0);
}

/// Paints an on/off switch into `rect` (about 40 x 22).
pub(crate) fn paint_switch(painter: &egui::Painter, rect: egui::Rect, on: bool) {
    painter.rect_filled(
        rect,
        11.0,
        if on {
            LIME
        } else {
            Color32::from_rgb(62, 62, 69)
        },
    );
    let knob_x = if on {
        rect.right() - 11.0
    } else {
        rect.left() + 11.0
    };
    painter.circle_filled(
        egui::pos2(knob_x, rect.center().y),
        8.0,
        if on { Color32::BLACK } else { Color32::WHITE },
    );
}

/// A search box with a magnifying glass, in the same style as the other text inputs.
pub(crate) fn search_box(ui: &mut egui::Ui, text: &mut String, hint: &str) -> egui::Response {
    let mut response = None;
    egui::Frame::new()
        .fill(Color32::from_rgb(13, 13, 15))
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (icon, _) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
                kit::draw_icon(ui.painter(), icon.center(), kit::Icon::Search, MUTED, 0.75);
                response = Some(
                    ui.add(
                        egui::TextEdit::singleline(text)
                            .hint_text(hint)
                            .frame(false)
                            .char_limit(100)
                            .desired_width(f32::INFINITY),
                    ),
                );
            });
        });
    response.unwrap_or_else(|| ui.allocate_response(Vec2::ZERO, egui::Sense::hover()))
}

/// A row with a switch on the right.
pub(crate) fn toggle_row(ui: &mut egui::Ui, title: &str, help: &str, value: &mut bool, last: bool) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        let text_width = (ui.available_width() - 56.0).max(0.0);
        ui.vertical(|ui| {
            ui.set_width(text_width);
            ui.label(
                RichText::new(title)
                    .color(TEXT)
                    .size(14.5)
                    .family(kit::semibold()),
            );
            ui.label(RichText::new(help).color(MUTED).size(12.5));
        });
        ui.add_space(12.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *value, title)
        });
        paint_switch(ui.painter(), rect, *value);
        if response.on_hover_text(title).clicked() {
            *value = !*value;
        }
    });
    ui.add_space(14.0);
    if !last {
        divider(ui);
    }
}

pub(crate) fn divider(ui: &mut egui::Ui) {
    let (line, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(line, 0.0, BORDER);
}

/// A label on the left and its value on the right, for facts rather than settings.
pub(crate) fn info_row(ui: &mut egui::Ui, label: &str, value: &str, last: bool) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(label)
                .color(TEXT)
                .size(14.5)
                .family(kit::semibold()),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).color(MUTED).size(13.5));
        });
    });
    ui.add_space(14.0);
    if !last {
        divider(ui);
    }
}

/// One option of a radio list: a circle, a title, and a description. True when clicked.
pub(crate) fn radio_row(ui: &mut egui::Ui, title: &str, help: &str, selected: bool) -> bool {
    let width = ui.available_width().max(0.0);
    let text_width = (width - 30.0).max(1.0);
    let title_galley = ui.painter().layout(
        title.to_owned(),
        egui::FontId::new(14.0, kit::medium()),
        TEXT,
        text_width,
    );
    let help_galley = ui.painter().layout(
        help.to_owned(),
        egui::FontId::proportional(12.5),
        MUTED,
        text_width,
    );
    let height = (title_galley.size().y + help_galley.size().y + 18.0).max(46.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, title)
    });
    let center = egui::pos2(rect.left() + 10.0, rect.top() + 14.0);
    ui.painter().circle_stroke(
        center,
        8.5,
        Stroke::new(1.5_f32, if selected { LIME } else { MUTED }),
    );
    if selected {
        ui.painter().circle_filled(center, 4.5, LIME);
    }
    let help_top = rect.top() + 10.0 + title_galley.size().y;
    ui.painter().galley(
        egui::pos2(rect.left() + 30.0, rect.top() + 5.0),
        title_galley,
        TEXT,
    );
    ui.painter()
        .galley(egui::pos2(rect.left() + 30.0, help_top), help_galley, MUTED);
    response.clicked() && !selected
}

/// A title and help line, then the row's controls, with a divider after it unless `last`.
pub(crate) fn setting_row(
    ui: &mut egui::Ui,
    title: &str,
    help: &str,
    last: bool,
    controls: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(14.0);
    ui.label(
        RichText::new(title)
            .color(TEXT)
            .size(14.5)
            .family(kit::semibold()),
    );
    ui.label(RichText::new(help).color(MUTED).size(12.5));
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        controls(ui);
    });
    ui.add_space(14.0);
    if !last {
        let (line, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().rect_filled(line, 0.0, BORDER);
    }
}

/// A dropdown with its label above it. Returns the index chosen this frame.
pub(crate) fn dropdown(
    ui: &mut egui::Ui,
    label: &str,
    current: &str,
    options: &[(String, bool)],
) -> Option<usize> {
    let mut chosen = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(RichText::new(label).color(MUTED).size(12.5));
        let shown: String = if current.chars().count() > 30 {
            current
                .chars()
                .take(29)
                .chain(std::iter::once('…'))
                .collect()
        } else {
            current.to_owned()
        };
        kit::pill_menu(ui, &shown, label, |ui| {
            ui.set_min_width(180.0);
            for (index, (text, enabled)) in options.iter().enumerate() {
                let item = ui.add_enabled(*enabled, egui::Button::new(text.as_str()).frame(false));
                if item.clicked() {
                    chosen = Some(index);
                    ui.close();
                }
            }
        });
    });
    chosen
}
