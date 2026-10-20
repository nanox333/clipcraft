//! Shared look and feel: the palette, the egui theme, painted icons, and small reusable widgets.
//!
//! Icons are painted from strokes and polygons rather than text glyphs, because egui's bundled
//! fonts miss many symbols and drawn icons stay crisp and consistently weighted at any size.

use eframe::egui;
use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Response, Sense,
    Stroke, StrokeKind, TextStyle, Vec2,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const CANVAS: Color32 = Color32::from_rgb(17, 17, 19);
pub const SIDEBAR: Color32 = Color32::from_rgb(22, 22, 25);
pub const PANEL: Color32 = Color32::from_rgb(27, 27, 31);
pub const CARD: Color32 = Color32::from_rgb(32, 32, 37);
pub const CARD_HOVER: Color32 = Color32::from_rgb(41, 41, 47);
pub const BORDER: Color32 = Color32::from_rgb(44, 44, 50);
pub const TEXT: Color32 = Color32::from_rgb(240, 240, 244);
pub const MUTED: Color32 = Color32::from_rgb(150, 150, 161);
pub const FAINT: Color32 = Color32::from_rgb(92, 92, 102);
pub const LIME: Color32 = Color32::from_rgb(190, 255, 43);
pub const DANGER: Color32 = Color32::from_rgb(255, 112, 112);

// Inter (SIL OFL 1.1), unmodified; see ATTRIBUTION.md and assets/fonts/OFL-Inter.md.
const INTER_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");

static FONTS_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Installs Inter as the UI face, with Medium and SemiBold as named families (egui has no
/// synthetic bold, and `RichText::strong` only changes color). egui's own fonts stay behind Inter
/// as fallbacks, so symbols and emoji Inter lacks still draw. Call once at startup, before the
/// first frame; in tests, run one empty frame afterwards so the fonts take effect.
pub fn install_fonts(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("inter", INTER_REGULAR),
        ("inter-medium", INTER_MEDIUM),
        ("inter-semibold", INTER_SEMIBOLD),
    ] {
        fonts.font_data.insert(
            name.to_owned(),
            Arc::new(egui::FontData::from_static(bytes)),
        );
    }
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (family, face) in [
        (FontFamily::Proportional, "inter"),
        (FontFamily::Name("medium".into()), "inter-medium"),
        (FontFamily::Name("semibold".into()), "inter-semibold"),
    ] {
        let mut chain = vec![face.to_owned()];
        chain.extend(fallbacks.iter().cloned());
        fonts.families.insert(family, chain);
    }
    context.set_fonts(fonts);
    FONTS_INSTALLED.store(true, Ordering::SeqCst);
}

/// The semibold face for headings, or the regular face if fonts were not installed.
pub fn semibold() -> FontFamily {
    if FONTS_INSTALLED.load(Ordering::SeqCst) {
        FontFamily::Name("semibold".into())
    } else {
        FontFamily::Proportional
    }
}

/// The medium face for buttons and labels, or the regular face if fonts were not installed.
pub fn medium() -> FontFamily {
    if FONTS_INSTALLED.load(Ordering::SeqCst) {
        FontFamily::Name("medium".into())
    } else {
        FontFamily::Proportional
    }
}

/// Applies the ClipCraft theme: dark surfaces, rounded widgets, a lime accent, readable type.
pub fn apply_theme(context: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = CANVAS;
    visuals.window_fill = PANEL;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.popup_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    visuals.faint_bg_color = CARD;
    visuals.extreme_bg_color = Color32::from_rgb(12, 12, 14);
    visuals.selection.bg_fill = Color32::from_rgb(72, 102, 23);
    visuals.selection.stroke = Stroke::new(1.0_f32, LIME);
    visuals.hyperlink_color = LIME;
    visuals.widgets.noninteractive.fg_stroke.color = TEXT;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);

    let radius = CornerRadius::same(9);
    for (state, fill) in [
        (&mut visuals.widgets.inactive, CARD),
        (&mut visuals.widgets.hovered, CARD_HOVER),
        (&mut visuals.widgets.active, Color32::from_rgb(58, 74, 30)),
        (&mut visuals.widgets.open, CARD_HOVER),
    ] {
        state.corner_radius = radius;
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.0_f32, TEXT);
        state.expansion = 0.0;
    }
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(70, 70, 78));
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, LIME);
    context.set_visuals(visuals);

    context.all_styles_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.5, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(24.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.5, FontFamily::Monospace),
            ),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.scroll = egui::style::ScrollStyle {
            bar_width: 7.0,
            floating_width: 5.0,
            ..egui::style::ScrollStyle::floating()
        };
    });
}

/// Every painted icon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Play,
    Pause,
    StepBack,
    StepForward,
    SkipBack,
    SkipForward,
    Loop,
    Fullscreen,
    ExitFullscreen,
    Speaker,
    SpeakerOff,
    Copy,
    Folder,
    Search,
    ChevronDown,
    ChevronLeft,
    Library,
    Gear,
    Import,
    Heart,
    Record,
    Stop,
    Scissors,
    Trash,
}

/// Draws `icon` centered on `c`, about 18 px across at `scale` 1.0.
pub fn draw_icon(painter: &Painter, c: Pos2, icon: Icon, color: Color32, scale: f32) {
    let stroke = Stroke::new(1.8_f32 * scale.max(0.6), color);
    let at = |x: f32, y: f32| c + Vec2::new(x, y) * scale;
    let fill = |points: Vec<Pos2>| {
        painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
    };
    let line = |points: &[Pos2]| {
        painter.add(egui::Shape::line(points.to_vec(), stroke));
    };
    let bar = |x: f32, y: f32, w: f32, h: f32, r: f32| {
        painter.rect_filled(
            Rect::from_center_size(at(x, y), Vec2::new(w, h) * scale),
            r,
            color,
        );
    };
    let outline = |from: Pos2, to: Pos2, r: f32| {
        painter.rect_stroke(
            Rect::from_min_max(from, to),
            r * scale,
            stroke,
            StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Play => fill(vec![at(-4.0, -7.0), at(7.0, 0.0), at(-4.0, 7.0)]),
        Icon::Pause => {
            bar(-4.0, 0.0, 3.6, 13.0, 1.2);
            bar(4.0, 0.0, 3.6, 13.0, 1.2);
        }
        Icon::StepForward => {
            fill(vec![at(-6.0, -6.0), at(3.0, 0.0), at(-6.0, 6.0)]);
            bar(6.0, 0.0, 2.6, 12.0, 0.8);
        }
        Icon::StepBack => {
            fill(vec![at(6.0, -6.0), at(-3.0, 0.0), at(6.0, 6.0)]);
            bar(-6.0, 0.0, 2.6, 12.0, 0.8);
        }
        Icon::SkipBack | Icon::SkipForward => {
            // An open circle with an arrowhead where it starts, and "10" inside.
            let direction: f32 = if icon == Icon::SkipForward { 1.0 } else { -1.0 };
            let radius = 9.0_f32 * scale;
            let start = -std::f32::consts::FRAC_PI_2 + direction * 0.45;
            let points: Vec<Pos2> = (0..=22)
                .map(|step| {
                    let angle = start - direction * (step as f32) * 0.2;
                    c + Vec2::new(angle.cos(), angle.sin()) * radius
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
            let tip = c + Vec2::new(start.cos(), start.sin()) * radius;
            let tangent = Vec2::new(-start.sin(), start.cos()) * direction;
            let normal = Vec2::new(tangent.y, -tangent.x);
            fill(vec![
                tip + tangent * 4.5 * scale,
                tip - tangent * 1.5 * scale + normal * 3.5 * scale,
                tip - tangent * 1.5 * scale - normal * 3.5 * scale,
            ]);
            painter.text(
                c,
                Align2::CENTER_CENTER,
                "10",
                FontId::proportional(8.5 * scale),
                color,
            );
        }
        Icon::Loop => {
            line(&[at(-9.0, 2.0), at(-9.0, -3.0), at(6.0, -3.0)]);
            fill(vec![at(5.0, -7.0), at(10.0, -3.0), at(5.0, 1.0)]);
            line(&[at(9.0, -2.0), at(9.0, 3.0), at(-6.0, 3.0)]);
            fill(vec![at(-5.0, -1.0), at(-10.0, 3.0), at(-5.0, 7.0)]);
        }
        Icon::Fullscreen | Icon::ExitFullscreen => {
            let (distance, arm, direction) = if icon == Icon::Fullscreen {
                (8.0, 5.0, -1.0)
            } else {
                (3.5, 5.0, 1.0)
            };
            for (sx, sy) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let corner = at(sx * distance, sy * distance);
                line(&[
                    corner + Vec2::new(direction * sx * arm, 0.0) * scale,
                    corner,
                    corner + Vec2::new(0.0, direction * sy * arm) * scale,
                ]);
            }
        }
        Icon::Speaker | Icon::SpeakerOff => {
            painter.rect_filled(
                Rect::from_center_size(at(-6.0, 0.0), Vec2::new(4.0, 6.0) * scale),
                0.0,
                color,
            );
            fill(vec![
                at(-4.0, -3.0),
                at(1.0, -7.5),
                at(1.0, 7.5),
                at(-4.0, 3.0),
            ]);
            if icon == Icon::SpeakerOff {
                line(&[at(5.0, -4.5), at(10.5, 4.5)]);
                line(&[at(10.5, -4.5), at(5.0, 4.5)]);
            } else {
                for radius in [5.0_f32, 8.5] {
                    let points: Vec<Pos2> = (-4..=4)
                        .map(|step| {
                            let angle = step as f32 * 0.19;
                            at(2.0 + radius * angle.cos() * 0.9, radius * angle.sin())
                        })
                        .collect();
                    line(&points);
                }
            }
        }
        Icon::Copy => {
            line(&[
                at(-3.0, -5.0),
                at(-3.0, -8.0),
                at(8.0, -8.0),
                at(8.0, 3.0),
                at(5.0, 3.0),
            ]);
            outline(at(-8.0, -4.0), at(4.0, 8.0), 2.0);
        }
        Icon::Folder => {
            outline(at(-9.0, -5.0), at(9.0, 7.0), 2.5);
            line(&[
                at(-9.0, -5.0),
                at(-9.0, -7.5),
                at(-3.0, -7.5),
                at(-1.0, -5.0),
            ]);
        }
        Icon::Search => {
            painter.circle_stroke(at(-2.0, -2.0), 5.5 * scale, stroke);
            line(&[at(2.0, 2.0), at(8.0, 8.0)]);
        }
        Icon::ChevronDown => line(&[at(-5.0, -2.0), at(0.0, 3.0), at(5.0, -2.0)]),
        Icon::ChevronLeft => line(&[at(2.0, -6.0), at(-3.0, 0.0), at(2.0, 6.0)]),
        Icon::Library => {
            for (x, y) in [(-8.0, -8.0), (1.5, -8.0), (-8.0, 1.5), (1.5, 1.5)] {
                painter.rect_filled(
                    Rect::from_min_size(at(x, y), Vec2::splat(6.5) * scale),
                    2.0 * scale,
                    color,
                );
            }
        }
        Icon::Gear => {
            for tooth in 0..8 {
                let angle = tooth as f32 * std::f32::consts::FRAC_PI_4;
                let direction = Vec2::new(angle.cos(), angle.sin());
                painter.line_segment(
                    [c + direction * 6.5 * scale, c + direction * 9.5 * scale],
                    Stroke::new(3.2_f32 * scale, color),
                );
            }
            painter.circle_stroke(c, 5.6 * scale, Stroke::new(2.4_f32 * scale, color));
        }
        Icon::Import => {
            line(&[at(-8.0, 2.0), at(-8.0, 7.0), at(8.0, 7.0), at(8.0, 2.0)]);
            line(&[at(0.0, -8.0), at(0.0, 3.0)]);
            line(&[at(-4.0, -1.0), at(0.0, 3.5), at(4.0, -1.0)]);
        }
        Icon::Heart => draw_heart(painter, c, color, false, scale),
        Icon::Record => {
            painter.circle_filled(c, 6.5 * scale, color);
        }
        Icon::Trash => {
            // A bin: lid, handle, and a body with two slats.
            line(&[at(-8.0, -5.0), at(8.0, -5.0)]);
            line(&[at(-3.0, -5.0), at(-3.0, -8.0), at(3.0, -8.0), at(3.0, -5.0)]);
            line(&[at(-6.0, -5.0), at(-5.0, 8.0), at(5.0, 8.0), at(6.0, -5.0)]);
            line(&[at(-2.0, -1.0), at(-2.0, 5.0)]);
            line(&[at(2.0, -1.0), at(2.0, 5.0)]);
        }
        Icon::Scissors => {
            // Two blades crossing, and two rings for the fingers.
            line(&[at(-6.0, 6.0), at(7.0, -8.0)]);
            line(&[at(6.0, 6.0), at(-7.0, -8.0)]);
            painter.circle_stroke(at(-6.0, 6.5), 2.6 * scale, stroke);
            painter.circle_stroke(at(6.0, 6.5), 2.6 * scale, stroke);
        }
        Icon::Stop => {
            painter.rect_filled(
                Rect::from_center_size(c, Vec2::splat(11.0 * scale)),
                2.0,
                color,
            );
        }
    }
}

fn icon_color(enabled: bool, hovered: bool, active: bool) -> Color32 {
    if !enabled {
        FAINT
    } else if hovered || active {
        LIME
    } else {
        TEXT
    }
}

/// A square icon button with a rounded hover plate.
pub fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    size: f32,
    active: bool,
    enabled: bool,
    tooltip: &str,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tooltip));
    if response.hovered() && enabled {
        ui.painter()
            .rect_filled(rect, 8.0, Color32::from_white_alpha(14));
    }
    draw_icon(
        ui.painter(),
        rect.center(),
        icon,
        icon_color(enabled, response.hovered(), active),
        size / 32.0,
    );
    response.on_hover_text(tooltip)
}

/// A button with an icon and a label, either filled in lime (`primary`) or quiet.
pub fn labelled_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    primary: bool,
    enabled: bool,
) -> Response {
    labelled_button_sized(ui, icon, label, primary, enabled, 0.0)
}

/// Like [`labelled_button`], at least `min_width` wide (pass the available width for a full-width
/// button).
pub fn labelled_button_sized(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    primary: bool,
    enabled: bool,
    min_width: f32,
) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::new(14.0, medium()),
        Color32::PLACEHOLDER,
    );
    let size = Vec2::new((galley.size().x + 52.0).max(min_width), 36.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let hovered = response.hovered() && enabled;
    let (fill, content) = match (primary, enabled, hovered) {
        (_, false, _) => (CARD, FAINT),
        (true, _, true) => (Color32::from_rgb(210, 255, 90), Color32::BLACK),
        (true, _, false) => (LIME, Color32::BLACK),
        (false, _, true) => (CARD_HOVER, TEXT),
        (false, _, false) => (CARD, TEXT),
    };
    ui.painter().rect_filled(rect, 17.0, fill);
    draw_icon(
        ui.painter(),
        Pos2::new(rect.left() + 20.0, rect.center().y),
        icon,
        content,
        0.75,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::new(14.0, medium()),
        content,
    );
    response
}

/// A small rounded label such as `MP4` or `1920×1080`.
pub fn chip(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(9, 4))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(MUTED).size(12.5));
        });
}

/// A slim horizontal slider from 0 to 1, filled in lime. Returns the new value while dragged or
/// clicked.
pub fn slim_slider(
    ui: &mut egui::Ui,
    width: f32,
    value: f32,
    enabled: bool,
    tooltip: &str,
) -> Option<f32> {
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width.max(0.0), 22.0), sense);
    response.widget_info(|| egui::WidgetInfo::slider(enabled, f64::from(value * 100.0), tooltip));
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width() - 10.0, 4.0));
    let value = value.clamp(0.0, 1.0);
    let painter = ui.painter();
    painter.rect_filled(track, 2.0, Color32::from_rgb(62, 62, 69));
    painter.rect_filled(
        Rect::from_min_max(
            track.min,
            Pos2::new(track.left() + track.width() * value, track.bottom()),
        ),
        2.0,
        if enabled { LIME } else { FAINT },
    );
    let active = enabled && (response.hovered() || response.dragged());
    painter.circle_filled(
        Pos2::new(track.left() + track.width() * value, rect.center().y),
        if active { 6.5 } else { 5.0 },
        if enabled { Color32::WHITE } else { FAINT },
    );
    let changed = (response.dragged() || response.clicked())
        .then(|| response.interact_pointer_pos())
        .flatten()
        .map(|pointer| ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0));
    response.on_hover_text(tooltip);
    changed.filter(|_| enabled)
}

/// A vertical dark gradient, transparent at the top and `alpha` opaque at the bottom, for text and
/// controls laid over a picture.
pub fn bottom_fade(painter: &Painter, rect: Rect, alpha: u8) {
    let clear = Color32::from_black_alpha(0);
    let solid = Color32::from_black_alpha(alpha);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), clear);
    mesh.colored_vertex(rect.right_top(), clear);
    mesh.colored_vertex(rect.right_bottom(), solid);
    mesh.colored_vertex(rect.left_bottom(), solid);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// A rounded dropdown: `label` and a chevron that open `content` as a menu below it. Items inside
/// close the menu with `ui.close()`.
pub fn pill_menu(
    ui: &mut egui::Ui,
    label: &str,
    tooltip: &str,
    content: impl FnOnce(&mut egui::Ui),
) -> Response {
    let font = FontId::new(13.5, medium());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let size = Vec2::new(galley.size().x + 44.0, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, label));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let fill = if open || response.hovered() {
        CARD_HOVER
    } else {
        CARD
    };
    ui.painter().rect_filled(rect, 16.0, fill);
    ui.painter().text(
        Pos2::new(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        TEXT,
    );
    draw_icon(
        ui.painter(),
        Pos2::new(rect.right() - 16.0, rect.center().y),
        Icon::ChevronDown,
        MUTED,
        0.6,
    );
    egui::Popup::menu(&response).show(content);
    response
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A key-style label such as `F8`, painted at an exact size so it never stretches in a row.
pub fn keycap(ui: &mut egui::Ui, text: &str) {
    let font = FontId::new(12.0, medium());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(galley.size().x + 16.0, 22.0), Sense::hover());
    ui.painter().rect(
        rect,
        6.0,
        Color32::from_rgb(40, 40, 46),
        Stroke::new(1.0_f32, Color32::from_rgb(58, 58, 66)),
        StrokeKind::Inside,
    );
    ui.painter()
        .text(rect.center(), Align2::CENTER_CENTER, text, font, TEXT);
}

/// A rounded status label with a colored dot, painted at an exact size.
pub fn status_pill(ui: &mut egui::Ui, dot: Color32, text: &str) {
    let font = FontId::proportional(13.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(galley.size().x + 38.0, 28.0), Sense::hover());
    ui.painter().rect_filled(rect, 14.0, CARD);
    ui.painter()
        .circle_filled(Pos2::new(rect.left() + 15.0, rect.center().y), 4.0, dot);
    ui.painter().text(
        Pos2::new(rect.left() + 26.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font,
        MUTED,
    );
}

/// A context with the app fonts installed and active, for tests that draw text.
#[cfg(test)]
pub fn test_context() -> egui::Context {
    let context = egui::Context::default();
    install_fonts(&context);
    // Fonts take effect at the start of the next frame.
    let _ = context.run(egui::RawInput::default(), |_| {});
    context
}

/// A heart about 18 px across at `scale` 1.0, outlined or filled.
pub fn draw_heart(painter: &Painter, c: Pos2, color: Color32, filled: bool, scale: f32) {
    if filled {
        // Two round lobes and a point: simple shapes that need no concave fill.
        painter.circle_filled(c + Vec2::new(-4.0, -2.5) * scale, 4.7 * scale, color);
        painter.circle_filled(c + Vec2::new(4.0, -2.5) * scale, 4.7 * scale, color);
        painter.add(egui::Shape::convex_polygon(
            vec![
                c + Vec2::new(-8.4, -0.8) * scale,
                c + Vec2::new(8.4, -0.8) * scale,
                c + Vec2::new(0.0, 8.6) * scale,
            ],
            color,
            Stroke::NONE,
        ));
        return;
    }
    let points: Vec<Pos2> = (0..=40)
        .map(|step| {
            let t = step as f32 / 40.0 * std::f32::consts::TAU;
            let x = 16.0 * t.sin().powi(3);
            let y =
                13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            c + Vec2::new(x, -y) * (0.5 * scale) + Vec2::new(0.0, 0.5 * scale)
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        Stroke::new(1.7_f32 * scale.max(0.6), color),
    ));
}

/// Names a text box for a screen reader (its hint is not enough) and gives its current text.
pub fn name_text_box(response: &egui::Response, name: &str, text: &str) {
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, name);
        info.current_text_value = Some(text.to_owned());
        info
    });
}
