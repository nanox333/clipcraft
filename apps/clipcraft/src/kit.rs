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

/// The spacing scale: every gap, margin and padding in the app is one of these, so the same kind of
/// gap looks the same on every screen. Multiples of four.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 20.0;
    pub const XXL: f32 = 24.0;
    pub const HUGE: f32 = 32.0;
}

/// Corner radii: small for chips and inputs, medium for buttons and rows, large for cards, panels
/// and windows. Fully round shapes (pills) use half their height instead.
pub mod radius {
    pub const SM: u8 = 6;
    pub const MD: u8 = 10;
    pub const LG: u8 = 14;
    /// Buttons, chips, fields and menus: fully round at the standard control height.
    pub const PILL: u8 = 17;
}

/// Type sizes: captions and hints, secondary text, body text, lead text and titles.
pub mod text_size {
    pub const CAPTION: f32 = 12.0;
    pub const SMALL: f32 = 13.0;
    pub const BODY: f32 = 14.0;
    pub const LEAD: f32 = 16.0;
    pub const TITLE: f32 = 20.0;
}

/// Heights of controls, so buttons, inputs and rows line up: compact and standard.
pub mod control {
    pub const COMPACT: f32 = 28.0;
    pub const STANDARD: f32 = 34.0;
    /// The height inside a [`super::field_frame`] (its vertical margins take the rest).
    pub const FIELD_INNER: f32 = STANDARD - 8.0;
}

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
    visuals.window_corner_radius = CornerRadius::same(radius::LG);
    visuals.menu_corner_radius = CornerRadius::same(radius::MD);
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

    let widget_radius = CornerRadius::same(radius::PILL);
    for (state, fill) in [
        (&mut visuals.widgets.inactive, CARD),
        (&mut visuals.widgets.hovered, CARD_HOVER),
        (&mut visuals.widgets.active, Color32::from_rgb(58, 74, 30)),
        (&mut visuals.widgets.open, CARD_HOVER),
    ] {
        state.corner_radius = widget_radius;
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
                FontId::new(text_size::CAPTION, FontFamily::Proportional),
            ),
            (
                TextStyle::Body,
                FontId::new(text_size::BODY, FontFamily::Proportional),
            ),
            (
                TextStyle::Button,
                FontId::new(text_size::BODY, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(24.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(text_size::SMALL, FontFamily::Monospace),
            ),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(space::SM, space::SM);
        style.spacing.button_padding = Vec2::new(space::MD, space::SM);
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
    Pencil,
}

impl Icon {
    /// Every icon, in declaration order (the cache below is indexed by it).
    pub const ALL: [Icon; 27] = [
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
        Icon::Pencil,
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
            Icon::Pencil => include_str!("../../../assets/icons/pencil.svg"),
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
            FontId::proportional(text_size::CAPTION * scale),
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
            .rect_filled(rect, f32::from(radius::MD), Color32::from_white_alpha(14));
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
    labelled_button_hint(ui, icon, label, "", primary, enabled)
}

/// Like [`labelled_button`], with a shortcut hint such as `F8` set into the right of the button.
pub fn labelled_button_hint(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    hint: &str,
    primary: bool,
    enabled: bool,
) -> Response {
    labelled_button_sized(ui, icon, label, hint, primary, enabled, 0.0)
}

/// Like [`labelled_button_hint`], at least `min_width` wide (pass the available width for a
/// full-width button).
pub fn labelled_button_sized(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    hint: &str,
    primary: bool,
    enabled: bool,
    min_width: f32,
) -> Response {
    labelled_button_sensing(
        ui,
        icon,
        label,
        hint,
        primary,
        enabled,
        min_width,
        Sense::click(),
    )
}

/// Like [`labelled_button_sized`], reacting to `sense` (for example a click or a drag) while
/// enabled.
#[allow(clippy::too_many_arguments)]
pub fn labelled_button_sensing(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    hint: &str,
    primary: bool,
    enabled: bool,
    min_width: f32,
    enabled_sense: Sense,
) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::new(text_size::BODY, medium()),
        Color32::PLACEHOLDER,
    );
    let hint_galley = (!hint.is_empty()).then(|| {
        ui.painter().layout_no_wrap(
            hint.to_owned(),
            FontId::new(text_size::CAPTION, medium()),
            Color32::PLACEHOLDER,
        )
    });
    let hint_width = hint_galley
        .as_ref()
        .map_or(0.0, |hint| hint.size().x + space::MD + space::SM);
    let size = Vec2::new(
        (galley.size().x + 52.0 + hint_width).max(min_width),
        control::STANDARD,
    );
    let sense = if enabled {
        enabled_sense
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
    ui.painter()
        .rect_filled(rect, control::STANDARD / 2.0, fill);
    if !primary && enabled {
        ui.painter().rect_stroke(
            rect,
            control::STANDARD / 2.0,
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
        FontId::new(text_size::BODY, medium()),
        content,
    );
    if let Some(hint) = hint_galley {
        // The shortcut sits in a small dark plate inside the button, at its right end.
        let width = hint.size().x + space::MD;
        let plate = Rect::from_center_size(
            Pos2::new(
                rect.right() - space::SM - 2.0 - width / 2.0,
                rect.center().y,
            ),
            Vec2::new(width, 22.0),
        );
        // On the lime button the plate is darker and the key light, so the shortcut is readable.
        let plate_fill = if enabled && primary {
            Color32::from_black_alpha(165)
        } else if enabled {
            Color32::from_black_alpha(70)
        } else {
            Color32::from_black_alpha(30)
        };
        ui.painter()
            .rect_filled(plate, f32::from(radius::SM), plate_fill);
        ui.painter().galley(
            Pos2::new(
                plate.center().x - hint.size().x / 2.0,
                plate.center().y - hint.size().y / 2.0,
            ),
            hint,
            if enabled && primary {
                Color32::from_rgb(235, 255, 190)
            } else if enabled {
                MUTED
            } else {
                FAINT
            },
        );
    }
    response
}

/// A small "?" that explains something when the pointer rests on it (or it has the keyboard focus),
/// so a sentence of help does not have to take up room on the page.
pub fn help_at(ui: &mut egui::Ui, center: Pos2, id: egui::Id, text: &str) {
    let rect = Rect::from_center_size(center, Vec2::splat(20.0));
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Help"));
    let hot = response.hovered() || response.has_focus();
    let ink = if hot { LIME } else { FAINT };
    ui.painter()
        .circle_stroke(center, 8.0, Stroke::new(1.0_f32, ink));
    ui.painter().text(
        center,
        Align2::CENTER_CENTER,
        "?",
        FontId::new(10.5, semibold()),
        if hot { LIME } else { MUTED },
    );
    response
        .on_hover_cursor(egui::CursorIcon::Help)
        .on_hover_ui(|ui| {
            ui.set_max_width(270.0);
            ui.label(egui::RichText::new(text).color(TEXT).size(text_size::SMALL));
        });
}

/// A "?" in the flow of a row, after what it explains.
pub fn help(ui: &mut egui::Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
    help_at(ui, rect.center(), ui.id().with(("help", text)), text);
}

/// The heading over a group of settings, with a short lime tick ahead of it.
pub fn section_heading(ui: &mut egui::Ui, title: &str) {
    ui.horizontal(|ui| {
        // A short lime tick ahead of the heading marks where a group starts.
        let (tick, _) = ui.allocate_exact_size(Vec2::new(3.0, 16.0), egui::Sense::hover());
        ui.painter().rect_filled(tick, 1.5, LIME);
        ui.add_space(space::XS);
        ui.label(
            egui::RichText::new(title)
                .color(TEXT)
                .size(text_size::LEAD)
                .family(semibold()),
        );
    });
    ui.add_space(space::MD);
}

/// `section_heading` with a "?" after the title.
pub fn section_heading_with_help(ui: &mut egui::Ui, title: &str, help_text: &str) {
    ui.horizontal(|ui| {
        let (tick, _) = ui.allocate_exact_size(Vec2::new(3.0, 16.0), egui::Sense::hover());
        ui.painter().rect_filled(tick, 1.5, LIME);
        ui.add_space(space::XS);
        ui.label(
            egui::RichText::new(title)
                .color(TEXT)
                .size(text_size::LEAD)
                .family(semibold()),
        );
        help(ui, help_text);
    });
    ui.add_space(space::MD);
}

/// A raised surface for a group of content: the card colour, a hairline border and a soft shadow so
/// it lifts off the page. Set the inner margin at the call site.
pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(radius::LG))
        .shadow(egui::Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(90),
        })
}

/// The frame of a search box or a folder field: a dark, round-ended plate as tall as a standard
/// button, so a row of fields and buttons lines up. Put a `ui.set_min_height(control::FIELD_INNER)`
/// call first thing inside it.
pub fn field_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgb(13, 13, 15))
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same((control::STANDARD / 2.0) as u8))
        .inner_margin(egui::Margin::symmetric(space::MD as i8, space::XS as i8))
}

/// A small rounded label such as `MP4` or `1920×1080`.
pub fn chip(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(CornerRadius::same(radius::MD))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .color(MUTED)
                    .size(text_size::CAPTION),
            );
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
    slim_slider_step(ui, width, value, enabled, tooltip, 0.01)
}

/// [`slim_slider`] that the arrow keys move by `step` (of the 0 to 1 range) while it has the
/// focus, and Home and End move to the ends.
pub fn slim_slider_step(
    ui: &mut egui::Ui,
    width: f32,
    value: f32,
    enabled: bool,
    tooltip: &str,
    step: f32,
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
    let mut changed = (response.dragged() || response.clicked())
        .then(|| response.interact_pointer_pos())
        .flatten()
        .map(|pointer| ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0));
    if enabled && response.has_focus() {
        let key = ui.input(|input| {
            if input.key_pressed(egui::Key::ArrowRight) || input.key_pressed(egui::Key::ArrowUp) {
                Some(value + step)
            } else if input.key_pressed(egui::Key::ArrowLeft)
                || input.key_pressed(egui::Key::ArrowDown)
            {
                Some(value - step)
            } else if input.key_pressed(egui::Key::Home) {
                Some(0.0)
            } else if input.key_pressed(egui::Key::End) {
                Some(1.0)
            } else {
                None
            }
        });
        if let Some(next) = key {
            changed = Some(next.clamp(0.0, 1.0));
        }
    }
    if enabled && response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(1.0),
            4.0,
            Stroke::new(1.5_f32, LIME),
            egui::StrokeKind::Outside,
        );
    }
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
    let font = FontId::new(text_size::SMALL, medium());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let size = Vec2::new(galley.size().x + 44.0, control::STANDARD);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, label));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let fill = if open || response.hovered() {
        CONTROL_HOVER
    } else {
        CONTROL
    };
    ui.painter()
        .rect_filled(rect, control::STANDARD / 2.0, fill);
    ui.painter().rect_stroke(
        rect,
        control::STANDARD / 2.0,
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
    let font = FontId::new(text_size::CAPTION, medium());
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
    let font = FontId::proportional(text_size::SMALL);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(galley.size().x + 38.0, control::COMPACT),
        Sense::hover(),
    );
    ui.painter().rect_filled(rect, control::COMPACT / 2.0, CARD);
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

/// Shared CUT logo, rasterized once for the native window.
pub fn app_icon() -> egui::IconData {
    let image = clipcraft_engine::brand::window_icon();
    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

/// The label of an export size choice. `original` names "no size target" ("Original quality" in
/// the trim panel, "Standard quality" in the editor). Shared so the two screens cannot drift.
/// The sizes line up with what Discord accepts: 10 MB for everyone, 50 MB and 500 MB with Nitro.
pub fn export_size_label(
    preset: Option<clipcraft_engine::ExportPreset>,
    original: &'static str,
) -> &'static str {
    use clipcraft_engine::ExportPreset;
    match preset {
        None => original,
        Some(ExportPreset::Small) => "Small file \u{b7} ~10 MB (Discord)",
        Some(ExportPreset::Medium) => "Medium file \u{b7} ~25 MB",
        Some(ExportPreset::Large) => "Large file \u{b7} ~50 MB (Discord Nitro Basic)",
        Some(ExportPreset::Largest) => "Largest file \u{b7} ~500 MB (Discord Nitro)",
        Some(ExportPreset::Custom { .. }) => "Custom size",
    }
}

/// The export size choices in menu order. A custom size keeps its current megabytes when one is
/// already chosen, and starts at 10 MB otherwise.
pub fn export_size_choices(
    current: Option<clipcraft_engine::ExportPreset>,
    original: &'static str,
) -> [(Option<clipcraft_engine::ExportPreset>, &'static str); 6] {
    use clipcraft_engine::ExportPreset;
    let custom = match current {
        Some(value @ ExportPreset::Custom { .. }) => value,
        _ => ExportPreset::Custom { megabytes: 10 },
    };
    [
        None,
        Some(ExportPreset::Small),
        Some(ExportPreset::Medium),
        Some(ExportPreset::Large),
        Some(ExportPreset::Largest),
        Some(custom),
    ]
    .map(|value| (value, export_size_label(value, original)))
}

/// The icon for the native window. 128 pixels, not the full 256: X11 carries a window icon in a
/// single request that a server with no big-request support caps at 256 KB, and a 256-pixel icon
/// (262 KB) is refused, leaving the taskbar with no icon at all. 128 is plenty for a taskbar or
/// dock; the .exe resource and the desktop entry carry the 256 version.
pub fn window_icon_data() -> egui::IconData {
    match clipcraft_engine::brand::icon(128, clipcraft_engine::brand::LIME) {
        Ok(image) => egui::IconData {
            width: image.width(),
            height: image.height(),
            rgba: image.into_raw(),
        },
        Err(_) => app_icon(),
    }
}

/// Cached GPU texture; no rasterization or image allocation during subsequent frames.
pub fn brand_mark(ui: &egui::Ui, rect: Rect) {
    let id = egui::Id::new("clipcraft-cut-brand-texture");
    let cached = ui
        .ctx()
        .data_mut(|data| data.get_temp::<egui::TextureHandle>(id));
    let texture = cached.unwrap_or_else(|| {
        let icon = app_icon();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [icon.width as usize, icon.height as usize],
            &icon.rgba,
        );
        let texture = ui
            .ctx()
            .load_texture("ClipCraft CUT", image, egui::TextureOptions::LINEAR);
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    ui.painter().image(
        texture.id(),
        rect,
        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

#[cfg(test)]
mod icon_tests {
    use super::*;

    #[test]
    fn the_window_icon_fits_in_one_x11_request() {
        let icon = window_icon_data();
        // The pixels travel as 32-bit values after a two-value header.
        assert!((icon.rgba.len() + 8) < 262_140, "{} bytes", icon.rgba.len());
        assert!(
            icon.rgba.chunks_exact(4).any(|pixel| pixel[3] > 200),
            "the mark is visible"
        );
    }

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
    fn window_icon_uses_the_shared_cut_artwork() {
        let icon = app_icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba, clipcraft_engine::brand::window_icon().into_raw());
    }

    #[test]
    fn brand_texture_loads_without_nested_context_locks_and_is_reused() {
        let ctx = egui::Context::default();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                    brand_mark(ui, rect);
                    let texture = ctx.data_mut(|data| {
                        data.get_temp::<egui::TextureHandle>(egui::Id::new(
                            "clipcraft-cut-brand-texture",
                        ))
                    });
                    ids.push(texture.map(|handle| handle.id()));
                });
            });
        }
        assert!(ids[0].is_some());
        assert_eq!(ids[0], ids[1]);
    }
}

/// A small segmented switch for the groups inside a tab (Transitions, Effects): a quiet capsule
/// with the chosen part raised, so it reads as a sub-choice next to the lime tabs above it.
/// Returns the part that was clicked.
pub fn sub_tabs(ui: &mut egui::Ui, items: &[&str], active: usize) -> Option<usize> {
    const HEIGHT: f32 = 26.0;
    const PAD: f32 = 12.0;
    let font = FontId::new(text_size::CAPTION, medium());
    let widths: Vec<f32> = items
        .iter()
        .map(|label| {
            ui.painter()
                .layout_no_wrap((*label).to_owned(), font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
                + PAD * 2.0
        })
        .collect();
    let total = widths.iter().sum::<f32>() + 4.0;
    let (frame, _) = ui.allocate_exact_size(Vec2::new(total, HEIGHT), Sense::hover());
    ui.painter().rect_filled(frame, HEIGHT / 2.0, CANVAS);
    ui.painter().rect_stroke(
        frame,
        HEIGHT / 2.0,
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    let mut clicked = None;
    let mut left = frame.left() + 2.0;
    for (index, (label, width)) in items.iter().zip(&widths).enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(left, frame.top() + 2.0),
            Vec2::new(*width, HEIGHT - 4.0),
        );
        left += width;
        let response = ui.interact(rect, egui::Id::new(("sub-tab", *label)), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, *label));
        if index == active {
            ui.painter()
                .rect_filled(rect, (HEIGHT - 4.0) / 2.0, CARD_HOVER);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, (HEIGHT - 4.0) / 2.0, Color32::from_white_alpha(10));
        }
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            *label,
            font.clone(),
            if index == active { TEXT } else { MUTED },
        );
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            clicked = Some(index);
        }
    }
    clicked
}

/// Two icons in a capsule: a grid and a list. Returns `Some(true)` when the list was clicked and
/// `Some(false)` when the grid was.
pub fn view_switch(ui: &mut egui::Ui, list: bool) -> Option<bool> {
    const HEIGHT: f32 = 26.0;
    const PART: f32 = 34.0;
    let (frame, _) = ui.allocate_exact_size(Vec2::new(PART * 2.0 + 4.0, HEIGHT), Sense::hover());
    ui.painter().rect_filled(frame, HEIGHT / 2.0, CANVAS);
    ui.painter().rect_stroke(
        frame,
        HEIGHT / 2.0,
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    let mut clicked = None;
    for (index, name) in ["Show as a grid", "Show as a list"].into_iter().enumerate() {
        let is_list = index == 1;
        let rect = Rect::from_min_size(
            frame.min + Vec2::new(2.0 + PART * index as f32, 2.0),
            Vec2::new(PART, HEIGHT - 4.0),
        );
        let response = ui.interact(rect, egui::Id::new(("view-switch", name)), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
        let on = list == is_list;
        if on {
            ui.painter()
                .rect_filled(rect, (HEIGHT - 4.0) / 2.0, CARD_HOVER);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, (HEIGHT - 4.0) / 2.0, Color32::from_white_alpha(10));
        }
        let colour = if on { TEXT } else { MUTED };
        let centre = rect.center();
        if is_list {
            for dy in [-4.5, 0.0, 4.5] {
                ui.painter().rect_filled(
                    Rect::from_center_size(centre + Vec2::new(0.0, dy), Vec2::new(11.0, 2.0)),
                    1.0,
                    colour,
                );
            }
        } else {
            for (dx, dy) in [(-3.5, -3.5), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.5)] {
                ui.painter().rect_filled(
                    Rect::from_center_size(centre + Vec2::new(dx, dy), Vec2::splat(5.0)),
                    1.0,
                    colour,
                );
            }
        }
        if response
            .on_hover_text(name)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            clicked = Some(is_list);
        }
    }
    clicked
}
