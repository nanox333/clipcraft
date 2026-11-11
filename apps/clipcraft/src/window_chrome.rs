//! The app's own title bar buttons, edge resizing and side navigation rows (the window has no
//! system frame).

use super::*;

/// A sidebar icon button with an active plate and a lime marker. True when clicked.
/// The three buttons at the right of the title bar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowButton {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// One title-bar button: a quiet square that lights up under the pointer, red for close. True
/// when it is clicked.
pub(crate) fn window_button(ui: &mut egui::Ui, button: WindowButton) -> bool {
    let label = match button {
        WindowButton::Minimize => "Minimize",
        WindowButton::Maximize => "Maximize",
        WindowButton::Restore => "Restore",
        WindowButton::Close => "Close",
    };
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(48.0, ui.available_height().max(40.0)),
        egui::Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let hovered = response.hovered();
    if hovered {
        let fill = if button == WindowButton::Close {
            Color32::from_rgb(214, 48, 56)
        } else {
            Color32::from_white_alpha(18)
        };
        ui.painter().rect_filled(rect, 0.0, fill);
    }
    let ink = if hovered && button == WindowButton::Close {
        Color32::WHITE
    } else if hovered {
        TEXT
    } else {
        MUTED
    };
    let stroke = Stroke::new(1.4_f32, ink);
    let c = rect.center();
    let painter = ui.painter();
    match button {
        WindowButton::Minimize => {
            painter.line_segment([c + Vec2::new(-5.5, 3.0), c + Vec2::new(5.5, 3.0)], stroke);
        }
        WindowButton::Maximize => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, Vec2::splat(11.0)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        WindowButton::Restore => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c + Vec2::new(-1.5, 1.5), Vec2::splat(9.0)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment(
                [c + Vec2::new(-0.5, -5.5), c + Vec2::new(5.5, -5.5)],
                stroke,
            );
            painter.line_segment([c + Vec2::new(5.5, -5.5), c + Vec2::new(5.5, 0.5)], stroke);
        }
        WindowButton::Close => {
            painter.line_segment([c + Vec2::new(-5.0, -5.0), c + Vec2::new(5.0, 5.0)], stroke);
            painter.line_segment([c + Vec2::new(5.0, -5.0), c + Vec2::new(-5.0, 5.0)], stroke);
        }
    }
    response.on_hover_text(label).clicked()
}

/// Without the system frame there is nothing to grab to resize the window, so a thin band along
/// each edge does it: the pointer changes shape there and a press starts a resize.
pub(crate) fn resize_from_the_edges(context: &egui::Context) {
    const GRIP: f32 = 5.0;
    let (window, position, pressed, maximised, fullscreen) = context.input(|input| {
        (
            input.content_rect(),
            input.pointer.hover_pos(),
            input.pointer.primary_pressed(),
            input.viewport().maximized.unwrap_or(false),
            input.viewport().fullscreen.unwrap_or(false),
        )
    });
    if maximised || fullscreen {
        return;
    }
    let Some(position) = position else {
        return;
    };
    let left = position.x - window.left() < GRIP;
    let right = window.right() - position.x < GRIP;
    let top = position.y - window.top() < GRIP;
    let bottom = window.bottom() - position.y < GRIP;
    use egui::{CursorIcon, ResizeDirection};
    let (direction, cursor) = match (left, right, top, bottom) {
        (true, _, true, _) => (ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest),
        (_, true, true, _) => (ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast),
        (true, _, _, true) => (ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest),
        (_, true, _, true) => (ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast),
        (true, ..) => (ResizeDirection::West, CursorIcon::ResizeWest),
        (_, true, ..) => (ResizeDirection::East, CursorIcon::ResizeEast),
        (_, _, true, _) => (ResizeDirection::North, CursorIcon::ResizeNorth),
        (_, _, _, true) => (ResizeDirection::South, CursorIcon::ResizeSouth),
        _ => return,
    };
    context.set_cursor_icon(cursor);
    if pressed {
        context.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
    }
}

pub(crate) fn nav_button(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(68.0, 48.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label)
    });
    let plate = egui::Rect::from_center_size(rect.center(), Vec2::splat(42.0));
    if selected {
        ui.painter().rect_filled(
            plate,
            12.0,
            Color32::from_rgba_unmultiplied(190, 255, 43, 26),
        );
        ui.painter().rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 1.5, rect.center().y),
                Vec2::new(3.0, 22.0),
            ),
            1.5,
            LIME,
        );
    } else if response.hovered() {
        ui.painter()
            .rect_filled(plate, 12.0, Color32::from_white_alpha(12));
    }
    let color = if selected {
        LIME
    } else if response.hovered() {
        TEXT
    } else {
        MUTED
    };
    kit::draw_icon(ui.painter(), rect.center(), icon, color, 1.0);
    response.on_hover_text(label).clicked()
}
