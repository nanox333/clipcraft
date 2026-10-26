//! The row kit behind the Settings pages. A row is one setting: a name, a line of help, and a
//! control on the right (a switch, a slider, a few choices). Rows are described by data, so the
//! same page can be drawn normally or filtered in place by the search box, and every row can tell
//! whether it differs from the default and put it back with one click.

use clipcraft_engine::RecordingSettings;
use eframe::egui::{
    self, Align, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Shape, Stroke, Vec2,
};

use crate::kit::{self, BORDER, CARD, LIME, MUTED, TEXT};

/// What a row says about itself, for display and for search.
#[derive(Clone, Copy)]
pub struct Meta<'a> {
    pub title: &'a str,
    pub help: &'a str,
    /// Other words a person might type to find this row.
    pub keywords: &'a str,
}

impl<'a> Meta<'a> {
    pub const fn new(title: &'a str, help: &'a str, keywords: &'a str) -> Self {
        Self {
            title,
            help,
            keywords,
        }
    }
}

/// A section that has been started but not yet drawn, because no row of it matched yet.
struct Pending {
    title: String,
}

/// The surface behind the rows of the section being drawn.
struct Surface {
    slot: egui::layers::ShapeIdx,
    top: f32,
    left: f32,
    width: f32,
    rows: usize,
}

pub struct Rows {
    words: Vec<String>,
    scratch: RecordingSettings,
    page: &'static str,
    pending: Option<Pending>,
    surface: Option<Surface>,
    /// Rows drawn so far this frame.
    pub shown: usize,
    /// Rows that differ from the default, among those drawn.
    pub changed: usize,
}

const CONTROL_GAP: f32 = 24.0;
const MIN_ROW: f32 = 56.0;

impl Rows {
    /// `query` is what is typed in the search box; empty draws everything.
    pub fn new(query: &str) -> Self {
        Self {
            words: query.split_whitespace().map(str::to_lowercase).collect(),
            scratch: RecordingSettings::default(),
            page: "",
            pending: None,
            surface: None,
            shown: 0,
            changed: 0,
        }
    }

    /// True while the search box has something in it. Pages skip content that is not a row.
    pub fn searching(&self) -> bool {
        !self.words.is_empty()
    }

    /// Names the page the next rows belong to; search results show it.
    pub fn page(&mut self, ui: &mut egui::Ui, name: &'static str) {
        self.close_section(ui);
        self.page = name;
    }

    /// Starts a section. Its heading appears with its first row.
    pub fn section(&mut self, ui: &mut egui::Ui, title: &str) {
        self.close_section(ui);
        self.pending = Some(Pending {
            title: title.to_owned(),
        });
    }

    /// Draws the surface behind the section just finished. Call after the last row of a page.
    pub fn finish(&mut self, ui: &mut egui::Ui) {
        self.close_section(ui);
    }

    fn close_section(&mut self, ui: &mut egui::Ui) {
        self.pending = None;
        if let Some(surface) = self.surface.take() {
            let bottom = ui.cursor().top();
            let rect = Rect::from_min_max(
                Pos2::new(surface.left, surface.top),
                Pos2::new(surface.left + surface.width, bottom),
            );
            ui.painter().set(
                surface.slot,
                Shape::Rect(egui::epaint::RectShape::new(
                    rect,
                    14.0,
                    CARD,
                    Stroke::new(1.0_f32, BORDER),
                    egui::StrokeKind::Inside,
                )),
            );
            ui.add_space(26.0);
        }
    }

    fn matches(&self, meta: &Meta<'_>) -> bool {
        if self.words.is_empty() {
            return true;
        }
        let haystack = format!(
            "{} {} {} {}",
            meta.title, meta.help, meta.keywords, self.page
        )
        .to_lowercase();
        self.words
            .iter()
            .all(|word| haystack.contains(word.as_str()))
    }

    /// Starts a row if it matches the search: draws the section heading and surface if this is
    /// the section's first row. Returns whether the row should be drawn.
    fn begin(&mut self, ui: &mut egui::Ui, meta: &Meta<'_>) -> bool {
        if !self.matches(meta) {
            return false;
        }
        if let Some(pending) = self.pending.take() {
            let heading = if self.searching() && !self.page.is_empty() {
                format!("{}  \u{b7}  {}", self.page, pending.title)
            } else {
                pending.title
            };
            ui.label(
                RichText::new(heading)
                    .color(TEXT)
                    .size(15.5)
                    .family(kit::semibold()),
            );
            ui.add_space(10.0);
            let slot = ui.painter().add(Shape::Noop);
            self.surface = Some(Surface {
                slot,
                top: ui.cursor().top(),
                left: ui.max_rect().left(),
                width: ui.available_width(),
                rows: 0,
            });
        }
        if self.surface.is_none() {
            // A row outside any section gets a surface of its own.
            let slot = ui.painter().add(Shape::Noop);
            self.surface = Some(Surface {
                slot,
                top: ui.cursor().top(),
                left: ui.max_rect().left(),
                width: ui.available_width(),
                rows: 0,
            });
        }
        self.shown += 1;
        true
    }

    /// The generic row: text on the left, `control_width` of control on the right (or under the
    /// text when the page is too narrow). Returns whether Reset was clicked.
    fn row(
        &mut self,
        ui: &mut egui::Ui,
        meta: &Meta<'_>,
        changed: bool,
        control_width: f32,
        control: impl FnOnce(&mut egui::Ui),
    ) -> bool {
        let inset = 20.0;
        let full = (ui.available_width() - inset * 2.0).max(120.0);
        let stacked = control_width > full * 0.55;
        let text_width = if stacked {
            full
        } else {
            (full - control_width - CONTROL_GAP).max(120.0)
        };
        let first = self.surface.as_ref().is_some_and(|s| s.rows == 0);
        if let Some(surface) = self.surface.as_mut() {
            surface.rows += 1;
        }
        let title = ui.painter().layout_no_wrap(
            meta.title.to_owned(),
            FontId::new(14.5, kit::semibold()),
            TEXT,
        );
        let help = ui.painter().layout(
            meta.help.to_owned(),
            FontId::proportional(12.5),
            MUTED,
            text_width,
        );
        let text_height = 14.0
            + title.size().y
            + if meta.help.is_empty() {
                0.0
            } else {
                3.0 + help.size().y
            }
            + 14.0;
        let height = if stacked {
            text_height + 36.0 + 4.0
        } else {
            text_height.max(MIN_ROW)
        };
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        let left = rect.left() + inset;
        if !first {
            ui.painter().hline(
                (rect.left() + inset)..=(rect.right() - inset),
                rect.top(),
                Stroke::new(1.0_f32, BORDER),
            );
        }
        let title_pos = Pos2::new(left, rect.top() + 14.0);
        let title_width = title.size().x;
        ui.painter().galley(title_pos, title, TEXT);
        if !meta.help.is_empty() {
            ui.painter()
                .galley(Pos2::new(left, title_pos.y + 14.5 + 6.0), help, MUTED);
        }
        let mut reset = false;
        if changed {
            let tag = Rect::from_min_size(
                Pos2::new(left + title_width + 10.0, title_pos.y - 1.0),
                Vec2::new(46.0, 20.0),
            );
            let response = ui
                .interact(tag, ui.id().with(("reset", meta.title)), Sense::click())
                .on_hover_text("Put this back to its default");
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("Reset {}", meta.title),
                )
            });
            ui.painter().text(
                tag.left_center(),
                egui::Align2::LEFT_CENTER,
                "\u{21ba} Reset",
                FontId::proportional(12.0),
                if response.hovered() { LIME } else { MUTED },
            );
            reset = response.clicked();
        }
        let control_rect = if stacked {
            Rect::from_min_size(Pos2::new(left, rect.bottom() - 40.0), Vec2::new(full, 34.0))
        } else {
            Rect::from_min_size(
                Pos2::new(rect.right() - inset - control_width, rect.center().y - 17.0),
                Vec2::new(control_width, 34.0),
            )
        };
        let layout = if stacked {
            Layout::left_to_right(Align::Center)
        } else {
            Layout::right_to_left(Align::Center)
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(control_rect).layout(layout));
        child.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        control(&mut child);
        reset
    }

    fn default_of<T: Copy>(&mut self, field: fn(&mut RecordingSettings) -> &mut T) -> T {
        *field(&mut self.scratch)
    }

    /// An on/off row.
    pub fn toggle(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut RecordingSettings,
        meta: Meta<'_>,
        field: fn(&mut RecordingSettings) -> &mut bool,
    ) {
        if !self.begin(ui, &meta) {
            return;
        }
        let default = self.default_of(field);
        let value = field(settings);
        let changed = *value != default;
        self.changed += usize::from(changed);
        let mut flip = false;
        let on = *value;
        let reset = self.row(ui, &meta, changed, 44.0, |ui| {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, meta.title)
            });
            crate::settings::paint_switch(ui.painter(), rect, on);
            flip = response.on_hover_text(meta.title).clicked();
        });
        if flip {
            *value = !*value;
        }
        if reset {
            *value = default;
        }
    }

    /// A slider that snaps to `stops`.
    #[allow(clippy::too_many_arguments)]
    pub fn stepped(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut RecordingSettings,
        meta: Meta<'_>,
        field: fn(&mut RecordingSettings) -> &mut u32,
        stops: &[u32],
        show: fn(u32) -> String,
    ) {
        if !self.begin(ui, &meta) {
            return;
        }
        let Some(last) = stops.len().checked_sub(1) else {
            return;
        };
        let default = self.default_of(field);
        let value = field(settings);
        let changed = *value != default;
        self.changed += usize::from(changed);
        let nearest = stops
            .iter()
            .enumerate()
            .min_by_key(|(_, stop)| stop.abs_diff(*value))
            .map_or(0, |(index, _)| index);
        let divisor = last.max(1) as f32;
        let mut next_value = None;
        let shown = show(*value);
        let reset = self.row(ui, &meta, changed, 330.0, |ui| {
            slider_control(
                ui,
                meta.title,
                nearest as f32 / divisor,
                1.0 / divisor,
                stops.len(),
                &shown,
                |fraction| {
                    let index = ((fraction * divisor).round() as usize).min(last);
                    next_value = Some(stops[index]);
                },
            );
        });
        if let Some(next) = next_value {
            *value = next;
        }
        if reset {
            *value = default;
        }
    }

    /// A slider over a range in whole `step`s.
    #[allow(clippy::too_many_arguments)]
    pub fn range(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut RecordingSettings,
        meta: Meta<'_>,
        field: fn(&mut RecordingSettings) -> &mut u32,
        range: std::ops::RangeInclusive<u32>,
        step: u32,
        show: fn(u32) -> String,
    ) {
        if !self.begin(ui, &meta) {
            return;
        }
        let default = self.default_of(field);
        let value = field(settings);
        let changed = *value != default;
        self.changed += usize::from(changed);
        let (min, max) = (*range.start(), *range.end());
        let span = max.saturating_sub(min).max(1) as f32;
        let step = step.max(1);
        let fraction = (*value).clamp(min, max).saturating_sub(min) as f32 / span;
        let mut next_value = None;
        let shown = show(*value);
        let reset = self.row(ui, &meta, changed, 330.0, |ui| {
            slider_control(
                ui,
                meta.title,
                fraction,
                step as f32 / span,
                0,
                &shown,
                |next| {
                    let steps = (next * span / step as f32).round() as u32;
                    next_value = Some((min + steps * step).clamp(min, max));
                },
            );
        });
        if let Some(next) = next_value {
            *value = next;
        }
        if reset {
            *value = default;
        }
    }

    /// A row of choices, one of which is selected. `options` are (value, label, enabled, hint).
    pub fn choose<T: Copy + PartialEq>(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut RecordingSettings,
        meta: Meta<'_>,
        field: fn(&mut RecordingSettings) -> &mut T,
        options: &[(T, &str, bool, &str)],
    ) {
        if !self.begin(ui, &meta) {
            return;
        }
        let default = self.default_of(field);
        let value = field(settings);
        let changed = *value != default;
        self.changed += usize::from(changed);
        let current = *value;
        let width = options
            .iter()
            .map(|(_, label, _, _)| label.chars().count() as f32 * 7.6 + 28.0 + 8.0)
            .sum::<f32>();
        let mut picked = None;
        let reset = self.row(ui, &meta, changed, width, |ui| {
            let stacked = ui.layout().main_dir() == egui::Direction::LeftToRight;
            let mut draw = |ui: &mut egui::Ui, option: &(T, &str, bool, &str)| {
                if crate::settings::choice(ui, option.1, current == option.0, option.2, option.3) {
                    picked = Some(option.0);
                }
            };
            if stacked {
                for option in options {
                    draw(ui, option);
                }
            } else {
                for option in options.iter().rev() {
                    draw(ui, option);
                }
            }
        });
        if let Some(next) = picked {
            *value = next;
        }
        if reset {
            *value = default;
        }
    }

    /// A row with a control of your own, such as a dropdown or a button. `changed` says whether
    /// it differs from the default, and `reset` puts it back.
    pub fn custom(
        &mut self,
        ui: &mut egui::Ui,
        meta: Meta<'_>,
        changed: bool,
        control_width: f32,
        control: impl FnOnce(&mut egui::Ui),
    ) -> bool {
        if !self.begin(ui, &meta) {
            return false;
        }
        self.changed += usize::from(changed);
        self.row(ui, &meta, changed, control_width, control)
    }

    /// Draws something that is not a row, such as a note or a device list, unless searching.
    pub fn extra(&mut self, ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
        if !self.searching() {
            body(ui);
        }
    }
}

/// A slider with tick marks and its value beside it. `on_change` gets the new 0 to 1 position.
fn slider_control(
    ui: &mut egui::Ui,
    name: &str,
    fraction: f32,
    key_step: f32,
    ticks: usize,
    shown: &str,
    on_change: impl FnOnce(f32),
) {
    let stacked = ui.layout().main_dir() == egui::Direction::LeftToRight;
    let value_width = 92.0;
    let value = |ui: &mut egui::Ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(value_width, 24.0), Sense::hover());
        let (align, x) = if stacked {
            (egui::Align2::LEFT_CENTER, rect.left())
        } else {
            (egui::Align2::RIGHT_CENTER, rect.right())
        };
        ui.painter().text(
            Pos2::new(x, rect.center().y),
            align,
            shown,
            FontId::new(14.0, kit::semibold()),
            LIME,
        );
    };
    if !stacked {
        value(ui);
    }
    let track_width =
        (ui.available_width() - if stacked { value_width + 8.0 } else { 8.0 }).clamp(80.0, 230.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(track_width, 22.0), Sense::hover());
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    if let Some(next) =
        kit::slim_slider_step(&mut inner, track_width, fraction, true, name, key_step)
    {
        on_change(next);
    }
    if ticks > 1 {
        let track_left = rect.left() + 5.0;
        let track_span = (rect.width() - 10.0).max(0.0);
        let divisor = (ticks - 1) as f32;
        for index in 0..ticks {
            let x = track_left + track_span * (index as f32 / divisor);
            ui.painter().line_segment(
                [
                    Pos2::new(x, rect.center().y + 7.0),
                    Pos2::new(x, rect.center().y + 10.0),
                ],
                Stroke::new(1.0_f32, Color32::from_rgb(70, 70, 78)),
            );
        }
    }
    if stacked {
        value(ui);
    }
}
