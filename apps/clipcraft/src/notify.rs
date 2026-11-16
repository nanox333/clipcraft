//! The one look and the one motion every notification in ClipCraft follows. The written rules are
//! in `docs/notifications.md`; this file is where they live in code, so a notification anywhere in
//! the app (the toast at the bottom of the window, the saved-clip pop-up, a banner inside a page,
//! a line under a form field) is made from the same parts.
//!
//! * **Kind** decides colour, badge and how long it stays: success (lime tick), information
//!   (blue "i"), warning (amber "!"), error (red cross).
//! * **Card** (a floating message) and **banner** (a message that belongs to a page) share the
//!   rounded shape, the hairline in the kind's colour, the round badge, the text size and the
//!   close button; only the card has a shadow and a time-left line.
//! * **Motion** is one ease-out entrance and one ease-in exit, the same length everywhere.

use eframe::egui::{
    self, Align, Color32, Context, CornerRadius, Id, Layout, Margin, Painter, Pos2, Rect, RichText,
    Sense, Stroke, Vec2,
};

use crate::kit::{self, DANGER, INFO, Icon, LIME, MUTED, PANEL, TEXT, WARNING};

/// What a notification says about what happened.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Kind {
    /// Something the person asked for is done.
    Success,
    /// Something they should know, neither good nor bad (progress, a note, a change of mode).
    Info,
    /// It worked, or carries on, but something needs a look.
    Warning,
    /// It did not work.
    Error,
}

impl Kind {
    /// The colour of the badge and the hairline.
    pub(crate) fn accent(self) -> Color32 {
        match self {
            Kind::Success => LIME,
            Kind::Info => INFO,
            Kind::Warning => WARNING,
            Kind::Error => DANGER,
        }
    }

    /// The glyph inside the badge.
    pub(crate) fn icon(self) -> Icon {
        match self {
            Kind::Success => Icon::Check,
            Kind::Info => Icon::Info,
            Kind::Warning => Icon::Alert,
            Kind::Error => Icon::Close,
        }
    }

    /// The word a screen reader says for the badge.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Success => "Success",
            Kind::Info => "Information",
            Kind::Warning => "Warning",
            Kind::Error => "Error",
        }
    }

    /// How long a floating notification stays, in seconds, not counting the time the pointer is
    /// on it. The worse the news, the longer it is given.
    pub(crate) fn stay(self) -> f32 {
        match self {
            Kind::Success | Kind::Info => 8.0,
            Kind::Warning => 16.0,
            Kind::Error => 30.0,
        }
    }

    /// The old two-way choice: an error, or plain information.
    pub(crate) fn from_error(error: bool) -> Self {
        if error { Kind::Error } else { Kind::Info }
    }

    /// The kind of a message that is kept as text and an error flag (the editor's and the trim
    /// page's status line): an error, a finished save, a note that something could not be done
    /// (a warning), or plain information.
    pub(crate) fn from_message(text: &str, error: bool) -> Self {
        if error {
            Kind::Error
        } else if text.starts_with("Saved ") || text.starts_with("Edit saved") {
            Kind::Success
        } else if text.contains("could not") {
            Kind::Warning
        } else {
            Kind::Info
        }
    }

    fn with_alpha(self, alpha: u8) -> Color32 {
        let colour = self.accent();
        Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), alpha)
    }

    /// The thin outline in the kind's colour.
    pub(crate) fn hairline(self) -> Color32 {
        self.with_alpha(80)
    }

    /// The faint fill of a banner.
    pub(crate) fn wash(self) -> Color32 {
        self.with_alpha(22)
    }
}

/// How long a notification takes to come in, and to go.
pub(crate) const ENTER_SECONDS: f32 = 0.28;
pub(crate) const EXIT_SECONDS: f32 = 0.22;
/// How far a floating card rises as it comes in, and sinks as it goes, in points.
pub(crate) const RISE: f32 = 16.0;
/// The round badge's width.
pub(crate) const BADGE: f32 = 28.0;
/// Widest a floating message grows before its text wraps.
pub(crate) const CARD_MAX_WIDTH: f32 = 440.0;

/// Starts fast and settles.
pub(crate) fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// Starts slowly and speeds up.
pub(crate) fn ease_in(t: f32) -> f32 {
    t * t * t
}

/// How present a notification is, from 0 (not there) to 1 (fully there): it comes in over
/// [`ENTER_SECONDS`] after `age` seconds, and leaves over [`EXIT_SECONDS`] once it has been
/// leaving for `leaving` seconds.
pub(crate) fn presence(age: f32, leaving: f32) -> f32 {
    ease_out((age / ENTER_SECONDS).clamp(0.0, 1.0))
        * (1.0 - ease_in((leaving / EXIT_SECONDS).clamp(0.0, 1.0)))
}

/// The quiet outline of a floating card: the kind shows in the badge and the time-left line, not
/// in a coloured border.
pub(crate) const CARD_OUTLINE: Color32 = Color32::from_rgba_premultiplied(30, 30, 30, 30);

/// The frame of a floating message: the panel colour, a quiet outline, round corners and a soft
/// shadow. There is room under the content for the time-left line.
pub(crate) fn card(_kind: Kind) -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, CARD_OUTLINE))
        .corner_radius(CornerRadius::same(kit::radius::LG))
        .inner_margin(Margin {
            left: 14,
            right: 12,
            top: 13,
            bottom: 15,
        })
        .shadow(egui::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(150),
        })
}

/// The ink of a glyph on a badge: nearly black, which reads on all four kind colours.
const BADGE_INK: Color32 = Color32::from_rgb(16, 16, 18);

/// Paints the round badge for `kind` centred on `center`: a solid disc in the kind's colour with
/// its glyph in dark ink, so the kind is known at a glance.
pub(crate) fn paint_badge(painter: &Painter, center: Pos2, kind: Kind) {
    painter.circle_filled(center, BADGE / 2.0, kind.accent());
    kit::draw_icon(painter, center, kind.icon(), BADGE_INK, 0.9);
}

/// The round badge as a widget.
pub(crate) fn badge(ui: &mut egui::Ui, kind: Kind) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(BADGE), Sense::hover());
    paint_badge(ui.painter(), rect.center(), kind);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, kind.name()));
    response
}

/// Paints the cross of a close button inside `rect`.
pub(crate) fn paint_close(painter: &Painter, rect: Rect, hovered: bool) {
    if hovered {
        painter.circle_filled(
            rect.center(),
            rect.width() / 2.0,
            Color32::from_white_alpha(16),
        );
    }
    kit::draw_icon(
        painter,
        rect.center(),
        Icon::Close,
        if hovered { TEXT } else { MUTED },
        1.0,
    );
}

/// The close button every notification has.
pub(crate) fn close_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(BADGE), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Dismiss message")
    });
    paint_close(ui.painter(), rect, response.hovered());
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Dismiss")
}

/// The thin line along the bottom of a floating card that shows how much of its stay is left.
pub(crate) fn paint_time_left(painter: &Painter, card: Rect, fraction: f32, kind: Kind) {
    let inset = f32::from(kit::radius::LG);
    let full = card.width() - 2.0 * inset;
    let top = card.bottom() - 7.0;
    painter.rect_filled(
        Rect::from_min_size(Pos2::new(card.left() + inset, top), Vec2::new(full, 3.0)),
        1.5,
        Color32::from_white_alpha(14),
    );
    let width = (full * fraction.clamp(0.0, 1.0)).max(0.0);
    if width >= 3.0 {
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(card.left() + inset, top), Vec2::new(width, 3.0)),
            1.5,
            kind.with_alpha(220),
        );
    }
}

/// 0 to 1: how far into its entrance a banner with this `id` is. The first frame it is drawn
/// starts the clock; it asks for frames until it is in.
fn appear(context: &Context, id: Id) -> f32 {
    let now = context.input(|input| input.time);
    let start = context.data_mut(|data| *data.get_temp_mut_or_insert_with(id, || now));
    let age = (now - start) as f32;
    if age < ENTER_SECONDS {
        context.request_repaint();
    }
    ease_out((age / ENTER_SECONDS).clamp(0.0, 1.0))
}

/// A message that belongs to a page, in the shape of the floating card without its shadow: a
/// faint wash of the kind's colour, the hairline, the badge, then `contents`. It fades in. `key`
/// names the message, so a new one fades in again and the same one does not.
pub(crate) fn banner_with(
    ui: &mut egui::Ui,
    kind: Kind,
    key: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    let shown = appear(ui.ctx(), Id::new(("notify-banner", kind, key)));
    ui.scope(|ui| {
        ui.set_opacity(shown);
        egui::Frame::new()
            .fill(kind.wash())
            .stroke(Stroke::new(1.0_f32, kind.hairline()))
            .corner_radius(CornerRadius::same(kit::radius::LG))
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    badge(ui, kind);
                    contents(ui);
                });
            });
    });
}

/// A plain message banner, with a close button when `dismissible`. True when it was closed.
pub(crate) fn banner(ui: &mut egui::Ui, kind: Kind, text: &str, dismissible: bool) -> bool {
    banner_actions(ui, kind, text, dismissible, |_| {})
}

/// A message banner with buttons of its own (drawn right to left, before the close button). True
/// when it was closed.
pub(crate) fn banner_actions(
    ui: &mut egui::Ui,
    kind: Kind,
    text: &str,
    dismissible: bool,
    actions: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut closed = false;
    banner_with(ui, kind, text, |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if dismissible {
                closed = close_button(ui).clicked();
            }
            actions(ui);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(text).color(TEXT).size(kit::text_size::BODY))
                        .wrap(),
                );
            });
        });
    });
    closed
}

/// A one-line message under a field or beside a control: the kind's glyph and the words, with no
/// card around them. Errors and warnings are written in the kind's colour; the rest are quiet.
pub(crate) fn inline(ui: &mut egui::Ui, kind: Kind, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = kit::space::SM;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
        kit::draw_icon(ui.painter(), rect.center(), kind.icon(), kind.accent(), 0.9);
        ui.label(
            RichText::new(text)
                .size(kit::text_size::CAPTION)
                .color(match kind {
                    Kind::Error | Kind::Warning => kind.accent(),
                    Kind::Success | Kind::Info => MUTED,
                }),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_its_own_colour_badge_and_a_longer_stay_for_worse_news() {
        let kinds = [Kind::Success, Kind::Info, Kind::Warning, Kind::Error];
        for (index, one) in kinds.iter().enumerate() {
            for other in &kinds[index + 1..] {
                assert_ne!(one.accent(), other.accent(), "{one:?} and {other:?}");
                assert_ne!(one.icon(), other.icon(), "{one:?} and {other:?}");
            }
        }
        assert!(Kind::Info.stay() <= Kind::Warning.stay());
        assert!(Kind::Warning.stay() < Kind::Error.stay());
        assert_eq!(Kind::from_error(true), Kind::Error);
        assert_eq!(Kind::from_error(false), Kind::Info);
    }

    #[test]
    fn a_status_line_is_sorted_into_its_kind() {
        assert_eq!(
            Kind::from_message("Edit saved to C:\\x", false),
            Kind::Success
        );
        assert_eq!(
            Kind::from_message("Saved clip.mp4 · 2 MB", false),
            Kind::Success
        );
        assert_eq!(
            Kind::from_message("Draft restored, but 1 file could not be opened", false),
            Kind::Warning
        );
        assert_eq!(Kind::from_message("Export cancelled", false), Kind::Info);
        assert_eq!(Kind::from_message("Edit saved", true), Kind::Error);
    }

    #[test]
    fn a_notification_comes_in_and_goes_out_smoothly() {
        assert_eq!(presence(0.0, 0.0), 0.0, "starts absent");
        assert!(
            (presence(ENTER_SECONDS, 0.0) - 1.0).abs() < 1e-6,
            "fully in"
        );
        assert!((presence(10.0, 0.0) - 1.0).abs() < 1e-6, "stays in");
        let mut last = 0.0;
        for step in 0..=10 {
            let now = presence(ENTER_SECONDS * step as f32 / 10.0, 0.0);
            assert!(now >= last, "never goes backwards on the way in");
            last = now;
        }
        assert!(
            presence(10.0, EXIT_SECONDS) < 1e-6,
            "gone when the exit is over"
        );
        assert!(presence(10.0, EXIT_SECONDS / 2.0) < 1.0, "part way out");
    }

    #[test]
    fn banners_and_inline_messages_draw_for_every_kind() {
        let context = kit::test_context();
        let mut closed = None;
        let _ = context.run(egui::RawInput::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                for kind in [Kind::Success, Kind::Info, Kind::Warning, Kind::Error] {
                    closed = Some(banner(ui, kind, "Something happened", true));
                    inline(ui, kind, "A short note");
                    let _ = badge(ui, kind);
                }
            });
        });
        assert_eq!(closed, Some(false), "nothing was clicked");
    }
}
