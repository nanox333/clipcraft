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
/// Quiet buttons and menus: lighter than a card so they stand out on one, with an outline.
pub const CONTROL: Color32 = Color32::from_rgb(46, 46, 53);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(58, 58, 66);
pub const CONTROL_BORDER: Color32 = Color32::from_rgb(64, 64, 73);
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
    Plus,
    Heart,
    Record,
    Stop,
    Scissors,
    Trash,
    Restart,
}

impl Icon {
    /// Every icon, in declaration order (the cache below is indexed by it).
    pub const ALL: [Icon; 26] = [
        Icon::Play,
        Icon::Pause,
        Icon::StepBack,
        Icon::StepForward,
        Icon::SkipBack,
        Icon::SkipForward,
        Icon::Loop,
        Icon::Fullscreen,
        Icon::ExitFullscreen,
        Icon::Speaker,
        Icon::SpeakerOff,
        Icon::Copy,
        Icon::Folder,
        Icon::Search,
        Icon::ChevronDown,
        Icon::ChevronLeft,
        Icon::Library,
        Icon::Gear,
        Icon::Import,
        Icon::Plus,
        Icon::Heart,
        Icon::Record,
        Icon::Stop,
        Icon::Scissors,
        Icon::Trash,
        Icon::Restart,
    ];

    /// The icon's file under `assets/icons`, a plain SVG that any vector editor can open.
    pub(crate) fn source(self) -> &'static str {
        match self {
            Icon::Play => include_str!("../../../assets/icons/play.svg"),
            Icon::Pause => include_str!("../../../assets/icons/pause.svg"),
            Icon::StepBack => include_str!("../../../assets/icons/step-back.svg"),
            Icon::StepForward => include_str!("../../../assets/icons/step-forward.svg"),
            Icon::SkipBack => include_str!("../../../assets/icons/skip-back.svg"),
            Icon::SkipForward => include_str!("../../../assets/icons/skip-forward.svg"),
            Icon::Loop => include_str!("../../../assets/icons/loop.svg"),
            Icon::Fullscreen => include_str!("../../../assets/icons/fullscreen.svg"),
            Icon::ExitFullscreen => include_str!("../../../assets/icons/exit-fullscreen.svg"),
            Icon::Speaker => include_str!("../../../assets/icons/speaker.svg"),
            Icon::SpeakerOff => include_str!("../../../assets/icons/speaker-off.svg"),
            Icon::Copy => include_str!("../../../assets/icons/copy.svg"),
            Icon::Folder => include_str!("../../../assets/icons/folder.svg"),
            Icon::Search => include_str!("../../../assets/icons/search.svg"),
            Icon::ChevronDown => include_str!("../../../assets/icons/chevron-down.svg"),
            Icon::ChevronLeft => include_str!("../../../assets/icons/chevron-left.svg"),
            Icon::Library => include_str!("../../../assets/icons/library.svg"),
            Icon::Gear => include_str!("../../../assets/icons/gear.svg"),
            Icon::Import => include_str!("../../../assets/icons/import.svg"),
            Icon::Plus => include_str!("../../../assets/icons/plus.svg"),
            Icon::Heart => include_str!("../../../assets/icons/heart.svg"),
            Icon::Record => include_str!("../../../assets/icons/record.svg"),
            Icon::Stop => include_str!("../../../assets/icons/stop.svg"),
            Icon::Scissors => include_str!("../../../assets/icons/scissors.svg"),
            Icon::Trash => include_str!("../../../assets/icons/trash.svg"),
            Icon::Restart => include_str!("../../../assets/icons/restart.svg"),
        }
    }
}

/// The icons, read from their SVG files once.
fn icons() -> &'static [Option<crate::svg::Svg>] {
    static ICONS: std::sync::OnceLock<Vec<Option<crate::svg::Svg>>> = std::sync::OnceLock::new();
    ICONS.get_or_init(|| {
        Icon::ALL
            .iter()
            .map(|icon| crate::svg::Svg::parse(icon.source()))
            .collect()
    })
}

/// Draws `icon` centered on `c`, about 18 px across at `scale` 1.0.
pub fn draw_icon(painter: &Painter, c: Pos2, icon: Icon, color: Color32, scale: f32) {
    // The files use a 24-unit square; one unit is 0.85 px at scale 1.0.
    let unit = 0.85 * scale;
    if let Some(svg) = icons().get(icon as usize).and_then(Option::as_ref) {
        svg.paint(painter, c, unit, color, false);
    }
    if matches!(icon, Icon::SkipBack | Icon::SkipForward) {
        painter.text(
            c + Vec2::new(0.0, 0.5 * scale),
            Align2::CENTER_CENTER,
            "10",
            FontId::proportional(8.5 * scale),
            color,
        );
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
        (false, _, true) => (CONTROL_HOVER, TEXT),
        (false, _, false) => (CONTROL, TEXT),
    };
    ui.painter().rect_filled(rect, 17.0, fill);
    if !primary && enabled {
        ui.painter().rect_stroke(
            rect,
            17.0,
            Stroke::new(1.0_f32, CONTROL_BORDER),
            egui::StrokeKind::Inside,
        );
    }
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
        CONTROL_HOVER
    } else {
        CONTROL
    };
    ui.painter().rect_filled(rect, 16.0, fill);
    ui.painter().rect_stroke(
        rect,
        16.0,
        Stroke::new(1.0_f32, CONTROL_BORDER),
        egui::StrokeKind::Inside,
    );
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
    if let Some(svg) = icons().get(Icon::Heart as usize).and_then(Option::as_ref) {
        svg.paint(painter, c, 0.85 * scale, color, filled);
    }
}

/// Names a text box for a screen reader (its hint is not enough) and gives its current text.
pub fn name_text_box(response: &egui::Response, name: &str, text: &str) {
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, name);
        info.current_text_value = Some(text.to_owned());
        info
    });
}

/// The window and taskbar icon: the lime diamond from the top bar, on a transparent square.
pub fn app_icon() -> egui::IconData {
    const SIZE: u32 = 256;
    let centre = (SIZE as f32 - 1.0) / 2.0;
    // The diamond reaches almost to the edge of the square.
    let reach = centre - 6.0;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let distance = (x as f32 - centre).abs() + (y as f32 - centre).abs();
            // One pixel of soft edge, measured across the slanted side.
            let coverage = ((reach - distance) / std::f32::consts::SQRT_2 + 0.5).clamp(0.0, 1.0);
            rgba.extend_from_slice(&[
                LIME.r(),
                LIME.g(),
                LIME.b(),
                (coverage * 255.0).round() as u8,
            ]);
        }
    }
    egui::IconData {
        rgba,
        width: SIZE,
        height: SIZE,
    }
}

#[cfg(test)]
mod icon_tests {
    use super::*;

    #[test]
    fn every_icon_file_reads_and_stays_inside_its_square() {
        for (index, icon) in Icon::ALL.iter().enumerate() {
            assert_eq!(
                *icon as usize, index,
                "{icon:?} is out of order in Icon::ALL"
            );
            assert!(
                crate::svg::Svg::parse(icon.source()).is_some(),
                "{icon:?} has no drawable shapes"
            );
            assert!(icons().get(index).is_some_and(Option::is_some), "{icon:?}");
        }
    }

    #[test]
    fn the_app_icon_is_a_lime_diamond_on_a_transparent_square() {
        let icon = app_icon();
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
        let at = |x: u32, y: u32| {
            let start = ((y * icon.width + x) * 4) as usize;
            icon.rgba[start..start + 4].to_vec()
        };
        // The middle is solid lime; the corners, outside the diamond, are clear.
        assert_eq!(at(128, 128), vec![LIME.r(), LIME.g(), LIME.b(), 255]);
        for corner in [(0, 0), (255, 0), (0, 255), (255, 255)] {
            assert_eq!(at(corner.0, corner.1)[3], 0);
        }
        // The points of the diamond are inside it.
        assert_eq!(at(128, 12)[3], 255);
    }
}
