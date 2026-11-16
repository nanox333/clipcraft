//! A small colour picker: a square for how strong and how light, a strip for which colour, and a
//! field for a typed value. Drawn by hand so it matches the rest of the window.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2, ecolor::Hsva, epaint::Mesh};

/// What happened to the colour this frame.
#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub struct Picked {
    /// The colour moved (dragging, or a valid value typed). Worth drawing; not yet worth saving.
    pub moved: bool,
    /// The person let go, or finished typing: save this one.
    pub settled: bool,
}

/// `#rrggbb`, lower case.
pub fn to_hex(colour: Hsva) -> String {
    let [r, g, b] = colour.to_srgb();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// A colour from `#rrggbb` or `rrggbb`, in any case. Anything else is `None`.
pub fn from_hex(text: &str) -> Option<Hsva> {
    let digits = text.trim().trim_start_matches('#');
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some(Hsva::from_srgb([
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ]))
}

/// The round swatch that opens the picker: every colour at once.
pub fn rainbow_swatch(painter: &egui::Painter, centre: Pos2, radius: f32) {
    const STEPS: usize = 36;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(centre, Color32::WHITE);
    for step in 0..=STEPS {
        let turn = step as f32 / STEPS as f32;
        let angle = turn * std::f32::consts::TAU;
        let colour: Color32 = Hsva::new(turn, 0.9, 1.0, 1.0).into();
        mesh.colored_vertex(centre + Vec2::angled(angle) * radius, colour);
    }
    for step in 0..STEPS {
        mesh.add_triangle(0, step as u32 + 1, step as u32 + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// A picture of the colours of one hue: white to the hue across, to black downwards.
fn square_image(hue: f32) -> egui::ColorImage {
    const W: usize = 64;
    const H: usize = 40;
    let mut pixels = Vec::with_capacity(W * H);
    for row in 0..H {
        for column in 0..W {
            let strength = column as f32 / (W - 1) as f32;
            let value = 1.0 - row as f32 / (H - 1) as f32;
            pixels.push(Color32::from(Hsva::new(hue, strength, value, 1.0)));
        }
    }
    egui::ColorImage::new([W, H], pixels)
}

/// A picture of every hue in a row.
fn strip_image() -> egui::ColorImage {
    const W: usize = 256;
    let pixels = (0..W)
        .map(|column| Color32::from(Hsva::new(column as f32 / (W - 1) as f32, 1.0, 1.0, 1.0)))
        .collect();
    egui::ColorImage::new([W, 1], pixels)
}

/// The picker, filling the width it is given. `hex` is the text of the typed field, kept by the
/// caller between frames.
pub fn show(ui: &mut egui::Ui, colour: &mut Hsva, hex: &mut String) -> Picked {
    let mut picked = Picked::default();
    let width = ui.available_width().max(120.0);
    let context = ui.ctx().clone();
    let linear = egui::TextureOptions::LINEAR;

    // Which colours: white to the colour across, to black downwards, with rounded corners.
    let (area, response) = ui.allocate_exact_size(Vec2::new(width, 132.0), Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::DragValue,
            true,
            "Colour strength and lightness",
        )
    });
    if let Some(pointer) = response.interact_pointer_pos()
        && (response.dragged() || response.clicked())
    {
        colour.s = ((pointer.x - area.left()) / area.width()).clamp(0.0, 1.0);
        colour.v = (1.0 - (pointer.y - area.top()) / area.height()).clamp(0.0, 1.0);
        picked.moved = true;
    }
    if response.drag_stopped() || response.clicked() {
        picked.settled = true;
    }
    let square = context.load_texture("colour-picker-square", square_image(colour.h), linear);
    egui::Image::new((square.id(), area.size()))
        .corner_radius(8.0)
        .paint_at(ui, area);
    ui.painter().rect_stroke(
        area,
        8.0,
        Stroke::new(1.0_f32, Color32::from_white_alpha(18)),
        egui::StrokeKind::Inside,
    );
    let handle = Pos2::new(
        area.left() + colour.s * area.width(),
        area.top() + (1.0 - colour.v) * area.height(),
    );
    ui.painter()
        .circle_filled(handle, 10.0, Color32::from_black_alpha(90));
    ui.painter().circle_filled(handle, 8.5, Color32::WHITE);
    ui.painter()
        .circle_filled(handle, 6.5, Color32::from(Hsva { a: 1.0, ..*colour }));

    ui.add_space(crate::kit::space::MD);

    // Which hue: a pill with every colour and a round thumb.
    let (strip, response) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::click_and_drag());
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, "Colour hue"));
    let travel = (strip.width() - strip.height()).max(1.0);
    if let Some(pointer) = response.interact_pointer_pos()
        && (response.dragged() || response.clicked())
    {
        colour.h = ((pointer.x - strip.left() - strip.height() / 2.0) / travel).clamp(0.0, 1.0);
        picked.moved = true;
    }
    if response.drag_stopped() || response.clicked() {
        picked.settled = true;
    }
    let hues = ui.data_mut(|data| data.get_temp::<egui::TextureHandle>(egui::Id::new("hue-strip")));
    let hues = hues.unwrap_or_else(|| {
        let handle = context.load_texture("colour-picker-hues", strip_image(), linear);
        ui.data_mut(|data| data.insert_temp(egui::Id::new("hue-strip"), handle.clone()));
        handle
    });
    let pill = Rect::from_center_size(strip.center(), Vec2::new(strip.width(), 10.0));
    egui::Image::new((hues.id(), pill.size()))
        .corner_radius(5.0)
        .paint_at(ui, pill);
    let knob = Pos2::new(
        strip.left() + strip.height() / 2.0 + colour.h * travel,
        strip.center().y,
    );
    ui.painter()
        .circle_filled(knob, 10.0, Color32::from_black_alpha(90));
    ui.painter().circle_filled(knob, 8.5, Color32::WHITE);
    ui.painter()
        .circle_filled(knob, 6.5, Color32::from(Hsva::new(colour.h, 1.0, 1.0, 1.0)));

    ui.add_space(crate::kit::space::MD);

    // The colour and its typed value, in a field like the rest of the window's.
    ui.horizontal(|ui| {
        let (swatch, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
        ui.painter().circle_filled(
            swatch.center(),
            14.0,
            Color32::from(Hsva { a: 1.0, ..*colour }),
        );
        ui.painter().circle_stroke(
            swatch.center(),
            14.0,
            Stroke::new(1.0_f32, Color32::from_white_alpha(40)),
        );
        crate::kit::field_frame().show(ui, |ui| {
            ui.set_min_height(crate::kit::control::FIELD_INNER);
            let field = ui.add(
                egui::TextEdit::singleline(hex)
                    .frame(false)
                    .char_limit(7)
                    .hint_text("#rrggbb")
                    .desired_width(ui.available_width().max(60.0)),
            );
            if field.changed()
                && let Some(typed) = from_hex(hex)
            {
                *colour = typed;
                picked.moved = true;
                picked.settled = true;
            } else if !field.has_focus() {
                *hex = to_hex(*colour);
            }
        });
    });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_what_is_not_a_colour() {
        for text in ["#bfff2b", "#000000", "#ffffff", "#9b5de5"] {
            assert_eq!(to_hex(from_hex(text).unwrap()), text);
        }
        assert_eq!(to_hex(from_hex("FF6B6B").unwrap()), "#ff6b6b");
        assert_eq!(to_hex(from_hex("  #ff6b6b ").unwrap()), "#ff6b6b");
        for bad in [
            "", "#", "#fff", "#12345", "#1234567", "#gggggg", "red", "#ff6b6b;",
        ] {
            assert!(from_hex(bad).is_none(), "{bad}");
        }
    }
}
