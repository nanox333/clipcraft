//! The library's grid of clips: virtual rows (only what is on screen is built), date headings when
//! the clips are in date order, keyboard traversal, and picking a clip up to put it in a new place.
//!
//! Cards glide to their places (an eased move, so a re-sort, a resize or a drag all animate). Drag a
//! card inside the window and the others make room; let go and the new order is kept. Drag it past
//! the window's edge, or hold Alt while dragging, and it becomes a file being dragged out to another
//! program (Discord, a folder).
use crate::clip_label::{Clock, DateGroup};
use crate::{ClipCardAction, MediaEntry, clip_card, library_view::ViewCache, thumbs::Thumbnails};
use clipcraft_engine::LibraryScanResult;
use eframe::egui::{self, Key, Modifiers};
use std::ops::Range;

pub(crate) fn card_id(entry: &MediaEntry) -> egui::Id {
    egui::Id::new(("clip-card", &entry.path))
}
fn control_id(entry: &MediaEntry, part: usize) -> egui::Id {
    let id = card_id(entry);
    match part {
        1 => id.with("copy"),
        2 => id.with("reveal"),
        3 => id.with("heart"),
        _ => id,
    }
}

/// How the clips are laid out: picture cards in one of three sizes, or a list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GridShape {
    /// One clip to a line, with a small picture.
    pub list: bool,
    /// The size of a card: 0 small, 1 medium, 2 large. Others are taken as medium.
    pub size: u8,
}

impl Default for GridShape {
    fn default() -> Self {
        Self {
            list: false,
            size: 1,
        }
    }
}

/// Height of a date heading, and of the space under it.
const HEADER_HEIGHT: f32 = 40.0;
/// How long a card takes to glide to its new place.
const GLIDE_SECONDS: f32 = 0.18;
/// How far the pointer has to move before a press becomes a pick-up.
const PICK_UP_DISTANCE: f32 = 6.0;
/// How far it has to move before a press on a card or the empty page becomes a drawn box.
const BOX_DISTANCE: f32 = 6.0;

/// A box being drawn over the clips to choose them, like on a desktop.
struct Marquee {
    /// Where the press was, from the top left of the content (so scrolling keeps it in place).
    start: egui::Vec2,
    /// What was chosen before, kept when the box adds to it.
    base: std::collections::BTreeSet<String>,
    /// Ctrl, ⌘ or Shift was held: the box adds to what was chosen.
    additive: bool,
    /// The pointer has moved far enough for it to be a box and not a click.
    active: bool,
    /// The press was on a clip (a click there chooses that clip).
    on_card: bool,
}

#[derive(Default)]
pub(crate) struct GridState {
    visible: Range<usize>,
    target: Option<(usize, usize)>,
    layout: Option<(usize, u32)>,
    /// The card being carried.
    carrying: Option<Carry>,
    /// A press that has already been used (a card was dropped, or handed to the system) is
    /// ignored until the button is let go, so it cannot pick the card up a second time.
    spent: bool,
    /// The scroll position last frame, and a nudge to apply (dragging near an edge scrolls).
    offset: f32,
    nudge: f32,
    /// The box being drawn while choosing clips.
    marquee: Option<Marquee>,
    #[cfg(test)]
    pub painted: usize,
}

/// A card that has been picked up.
struct Carry {
    /// Its place in the library's current order.
    from: usize,
    /// Where the pointer holds it, from its top-left corner.
    grab: egui::Vec2,
    /// The place it would take if let go now.
    slot: usize,
}

enum RowKind {
    Header(DateGroup, usize),
    Cards(Range<usize>),
}

struct RowItem {
    top: f32,
    height: f32,
    kind: RowKind,
}

/// Where everything sits, top to bottom.
struct Layout {
    rows: Vec<RowItem>,
    /// For each place in the order: its row and its column.
    cells: Vec<(usize, usize)>,
    total: f32,
}

fn columns(width: f32) -> usize {
    match width {
        x if x < 560.0 => 1,
        x if x < 850.0 => 2,
        x if x < 1140.0 => 3,
        x if x < 1430.0 => 4,
        x if x < 1720.0 => 5,
        _ => 6,
    }
}

/// Lays the clips out in `columns`, with a heading before each group when `groups` is given (one
/// group per place, in order).
fn build_layout(
    count: usize,
    columns: usize,
    card_height: f32,
    gap: f32,
    groups: Option<&[DateGroup]>,
) -> Layout {
    let mut rows: Vec<RowItem> = Vec::new();
    let mut cells = Vec::with_capacity(count);
    let mut top = 0.0_f32;
    let mut current: Option<DateGroup> = None;
    for place in 0..count {
        if let Some(groups) = groups {
            let group = groups[place].clone();
            if current.as_ref() != Some(&group) {
                // A heading goes below whatever came before it.
                if let Some(last) = rows.last() {
                    top = last.top + last.height + gap;
                }
                let members = groups[place..]
                    .iter()
                    .take_while(|next| **next == group)
                    .count();
                rows.push(RowItem {
                    top,
                    height: HEADER_HEIGHT,
                    kind: RowKind::Header(group.clone(), members),
                });
                top += HEADER_HEIGHT + gap * 0.5;
                current = Some(group);
                // The next card starts a row of its own.
                cells.push((rows.len(), 0));
                rows.push(RowItem {
                    top,
                    height: card_height,
                    kind: RowKind::Cards(place..place + 1),
                });
                continue;
            }
        }
        let continues = match rows.last() {
            Some(RowItem {
                kind: RowKind::Cards(range),
                ..
            }) => range.len() < columns,
            _ => false,
        };
        if continues {
            let index = rows.len() - 1;
            if let RowKind::Cards(range) = &mut rows[index].kind {
                cells.push((index, range.len()));
                range.end += 1;
            }
        } else {
            // A new row: below the one before it.
            if let Some(last) = rows.last() {
                top = last.top + last.height + gap;
            }
            cells.push((rows.len(), 0));
            rows.push(RowItem {
                top,
                height: card_height,
                kind: RowKind::Cards(place..place + 1),
            });
        }
        // A heading's first row already sits at `top`.
        if let Some(last) = rows.last() {
            top = last.top;
        }
    }
    let total = rows.last().map_or(0.0, |last| last.top + last.height);
    Layout { rows, cells, total }
}

impl GridState {
    /// Draws the grid. `by_date` puts a heading before each day's clips (only meaningful when the
    /// clips are in date order). The answer is what the person asked for this frame, if anything.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        result: &LibraryScanResult,
        view: &ViewCache,
        thumbs: &mut Thumbnails,
        by_date: bool,
        shape: GridShape,
        selecting: Option<&std::collections::BTreeSet<String>>,
    ) -> Option<ClipCardAction> {
        let count = view.indices.len();
        if self.target.is_some_and(|(index, _)| index >= count) {
            self.target = None;
        }
        if self
            .carrying
            .as_ref()
            .is_some_and(|carry| carry.from >= count || carry.slot >= count)
        {
            self.carrying = None;
        }
        let available = ui.available_width();
        let gap = if shape.list {
            crate::kit::space::SM
        } else {
            crate::kit::space::LG
        };
        // Columns: one in a list. Otherwise as many as suit the width, one more or fewer for
        // the small and large sizes, and never so many that a card gets narrower than 180.
        let columns = if shape.list {
            1
        } else {
            let base = columns(available);
            let wanted = match shape.size {
                0 => base + 1,
                2 => base.saturating_sub(1).max(1),
                _ => base,
            };
            (1..=wanted)
                .rev()
                .find(|&count| (available - (count - 1) as f32 * gap) / count as f32 >= 180.0)
                .unwrap_or(1)
        };
        let width = if shape.list {
            available.max(300.0)
        } else {
            ((available - (columns - 1) as f32 * gap) / columns as f32).max(180.0)
        };
        let height = if shape.list {
            crate::clip_card::ROW_HEIGHT
        } else {
            (width * 0.5625).clamp(120.0, 240.0) + crate::CARD_TEXT_HEIGHT
        };
        // Headings are put aside while a card is carried: the clips flow as one set.
        let grouping = by_date && self.carrying.is_none();
        let clock = Clock::system();
        let groups: Option<Vec<DateGroup>> = grouping.then(|| {
            view.indices
                .iter()
                .map(|&source| {
                    crate::clip_label::group_at(
                        result
                            .entries
                            .get(source)
                            .and_then(|entry| entry.modified_unix_seconds)
                            .and_then(|seconds| i64::try_from(seconds).ok()),
                        &clock,
                    )
                })
                .collect()
        });
        // The clips as they are in the library's order, and as they are shown (a carried card has
        // moved to the place it would take).
        let natural = build_layout(count, columns, height, gap, groups.as_deref());
        let order: Vec<usize> = match self.carrying.as_ref() {
            Some(carry) => {
                let mut order: Vec<usize> = (0..count).filter(|&p| p != carry.from).collect();
                order.insert(carry.slot.min(order.len()), carry.from);
                order
            }
            None => (0..count).collect(),
        };
        let shown = if self.carrying.is_some() {
            build_layout(count, columns, height, gap, None)
        } else {
            build_layout(count, columns, height, gap, groups.as_deref())
        };
        if self.layout != Some((columns, width.to_bits())) {
            if let Some(focus) = ui.memory(|m| m.focused()) {
                self.target = view
                    .indices
                    .iter()
                    .enumerate()
                    .find_map(|(index, &source)| {
                        let entry = result.entries.get(source)?;
                        (0..4)
                            .find(|&part| control_id(entry, part) == focus)
                            .map(|part| (index, part))
                    })
                    .or(self.target);
            }
            self.layout = Some((columns, width.to_bits()));
            self.visible = 0..0;
        }
        if self.carrying.is_none() {
            // A card has four places to Tab to, and one while choosing several clips.
            let parts = if selecting.is_some() { 1 } else { 4 };
            self.navigate(ui, result, &view.indices, &natural, parts);
        }
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("clip-grid")
            .auto_shrink([false, false]);
        if let Some((index, _)) = self.target
            && let Some(&(row, _)) = natural.cells.get(index)
            && !self.visible.contains(&row)
            && let Some(item) = natural.rows.get(row)
        {
            scroll = scroll.vertical_scroll_offset(item.top);
        } else if self.nudge != 0.0 {
            scroll = scroll.vertical_scroll_offset((self.offset + self.nudge).max(0.0));
        }
        self.nudge = 0.0;
        let mut action = None;
        #[cfg(test)]
        {
            self.painted = 0;
        }
        let context = ui.ctx().clone();
        let screen = context.content_rect();
        let pointer = context.input(|input| input.pointer.latest_pos());
        let pressed_at = context.input(|input| input.pointer.press_origin());
        let down = context.input(|input| input.pointer.any_down());
        let alt = context.input(|input| input.modifiers.alt);
        let carried_before = self.carrying.as_ref().map(|carry| carry.from);
        let mut picked: Option<(usize, egui::Vec2)> = None;
        let mut origin = egui::Pos2::ZERO;
        let output = scroll.show_viewport(ui, |ui, viewport| {
            ui.spacing_mut().item_spacing = egui::Vec2::new(gap, gap);
            origin = ui.cursor().min;
            ui.set_min_size(egui::vec2(ui.available_width(), shown.total.max(1.0)));
            // The rows on screen, with a little margin above and below.
            let first = shown
                .rows
                .partition_point(|row| row.top + row.height < viewport.min.y - 40.0);
            let last = shown
                .rows
                .partition_point(|row| row.top <= viewport.max.y + 40.0);
            self.visible = first..last;
            for row_index in first..last {
                let Some(row) = shown.rows.get(row_index) else {
                    continue;
                };
                match &row.kind {
                    RowKind::Header(group, members) => {
                        paint_header(
                            ui,
                            origin,
                            row,
                            group,
                            *members,
                            width + (columns - 1) as f32 * (width + gap),
                        );
                    }
                    RowKind::Cards(places) => {
                        for place in places.clone() {
                            let flat = order[place];
                            let Some(&source) = view.indices.get(flat) else {
                                continue;
                            };
                            let Some(entry) = result.entries.get(source) else {
                                continue;
                            };
                            let column = shown.cells[place].1;
                            // Eased move to its place, measured from the top of the grid so that
                            // scrolling does not make cards drift.
                            let id = card_id(entry);
                            let x = context.animate_value_with_time(
                                id.with("glide-x"),
                                column as f32 * (width + gap),
                                GLIDE_SECONDS,
                            );
                            let y = context.animate_value_with_time(
                                id.with("glide-y"),
                                row.top,
                                GLIDE_SECONDS,
                            );
                            let at = origin + egui::vec2(x, y);
                            let rect = egui::Rect::from_min_size(at, egui::vec2(width, height));
                            let carried = carried_before == Some(flat);
                            if carried {
                                // The place it will take, marked; the card itself is at the
                                // pointer. An unseen widget would end the drag, so one is kept.
                                paint_slot(ui, rect);
                                if let Some(pointer) = pointer {
                                    ui.interact(
                                        egui::Rect::from_center_size(
                                            pointer,
                                            egui::Vec2::splat(4.0),
                                        ),
                                        id,
                                        egui::Sense::click_and_drag(),
                                    );
                                }
                            } else {
                                let clicked = ui
                                    .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                                        clip_card(
                                            ui,
                                            entry,
                                            width,
                                            thumbs,
                                            view.game(source),
                                            &result.collections,
                                            crate::clip_card::CardMode {
                                                row: shape.list,
                                                select: selecting
                                                    .map(|chosen| chosen.contains(&entry.path)),
                                            },
                                        )
                                    })
                                    .inner;
                                if let Some(clicked) = clicked {
                                    action = Some(clicked);
                                }
                                #[cfg(test)]
                                {
                                    self.painted += 1;
                                }
                                // A press that has moved far enough picks the card up.
                                if self.carrying.is_none()
                                    && selecting.is_none()
                                    && !self.spent
                                    && picked.is_none()
                                    && context.dragged_id() == Some(id)
                                    && let (Some(pointer), Some(pressed)) = (pointer, pressed_at)
                                    && (pointer - pressed).length() > PICK_UP_DISTANCE
                                {
                                    picked = Some((flat, pressed - rect.min));
                                }
                                if self.target.is_some_and(|(target, _)| target == flat)
                                    && let Some((_, part)) = self.target.take()
                                {
                                    ui.memory_mut(|m| m.request_focus(control_id(entry, part)));
                                }
                            }
                        }
                    }
                }
            }
        });
        self.offset = output.state.offset.y;
        if !down {
            self.spent = false;
        }
        // While choosing clips, a box can be drawn over them (and over empty space).
        if let Some(chosen) = selecting {
            let paths: Vec<&str> = view
                .indices
                .iter()
                .filter_map(|&source| result.entries.get(source))
                .map(|entry| entry.path.as_str())
                .collect();
            if let Some(paths) = self.drive_box(
                &context,
                origin,
                output.inner_rect,
                &shown,
                egui::vec2(width, height),
                gap,
                &paths,
                chosen,
            ) {
                action = Some(ClipCardAction::SetSelection(paths));
            }
        } else {
            self.marquee = None;
        }
        // Picking up: holding Alt, or an offline clip, never reorders. Alt hands the file to the
        // system at once.
        if let Some((flat, grab)) = picked {
            let entry = view
                .indices
                .get(flat)
                .and_then(|&source| result.entries.get(source));
            if alt && let Some(entry) = entry {
                self.spent = true;
                if !entry.offline {
                    action = Some(ClipCardAction::DragOut(Box::new(entry.clone())));
                }
            } else if entry.is_some_and(|entry| !entry.offline) {
                self.carrying = Some(Carry {
                    from: flat,
                    grab,
                    slot: flat,
                });
            }
        }
        if let Some(carry) = self.carrying.as_mut() {
            context.set_cursor_icon(egui::CursorIcon::Grabbing);
            context.request_repaint();
            let entry = view
                .indices
                .get(carry.from)
                .and_then(|&source| result.entries.get(source));
            let outside = pointer.is_none_or(|pointer| !screen.shrink(2.0).contains(pointer));
            if !down {
                // Let go: the new order, if the card moved.
                let moved = carry.slot != carry.from;
                let paths: Vec<String> = order
                    .iter()
                    .filter_map(|&flat| view.indices.get(flat))
                    .filter_map(|&source| result.entries.get(source))
                    .map(|entry| entry.path.clone())
                    .collect();
                self.carrying = None;
                self.spent = true;
                if moved {
                    action = Some(ClipCardAction::Reordered(paths));
                }
            } else if outside {
                // Past the window's edge: it becomes a file dragged out to another program.
                let entry = entry.cloned();
                self.carrying = None;
                self.spent = true;
                if let Some(entry) = entry {
                    action = Some(ClipCardAction::DragOut(Box::new(entry)));
                }
            } else if let (Some(pointer), Some(entry)) = (pointer, entry) {
                // Which place is nearest the pointer, from the grid without the heading rows.
                let flat = build_layout(count, columns, height, gap, None);
                let relative = pointer - origin;
                let row = flat
                    .rows
                    .iter()
                    .position(|row| relative.y < row.top + row.height + gap / 2.0)
                    .unwrap_or(flat.rows.len().saturating_sub(1));
                if let Some(RowItem {
                    kind: RowKind::Cards(places),
                    ..
                }) = flat.rows.get(row)
                {
                    let column =
                        ((relative.x / (width + gap)).floor().max(0.0) as usize).min(columns - 1);
                    carry.slot = (places.start + column).min(places.end.saturating_sub(1));
                }
                // The card in hand, over everything, a little larger and lifted.
                if shape.list {
                    // A line is carried as its small picture.
                    paint_carried(
                        &context,
                        entry,
                        thumbs,
                        pointer - carry.grab,
                        crate::clip_card::ROW_HEIGHT * 0.8 * 16.0 / 9.0,
                        crate::clip_card::ROW_HEIGHT * 0.8 + crate::CARD_TEXT_HEIGHT,
                    );
                } else {
                    paint_carried(&context, entry, thumbs, pointer - carry.grab, width, height);
                }
                // Near the top or bottom edge the list scrolls.
                let viewport = output.inner_rect;
                let edge = 56.0;
                if pointer.y < viewport.top() + edge {
                    self.nudge = -((viewport.top() + edge - pointer.y) / edge * 18.0);
                } else if pointer.y > viewport.bottom() - edge {
                    self.nudge = (pointer.y - (viewport.bottom() - edge)) / edge * 18.0;
                }
            }
        }
        action
    }

    /// Draws and follows the box while choosing clips. Returns the whole new set of chosen clips
    /// whenever the box (or a click on empty space) changes it.
    #[allow(clippy::too_many_arguments)]
    fn drive_box(
        &mut self,
        context: &egui::Context,
        origin: egui::Pos2,
        area: egui::Rect,
        layout: &Layout,
        card: egui::Vec2,
        gap: f32,
        paths: &[&str],
        chosen: &std::collections::BTreeSet<String>,
    ) -> Option<Vec<String>> {
        let (pressed, down, pointer, modifiers) = context.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_down(),
                input.pointer.latest_pos(),
                input.modifiers,
            )
        });
        let rect_of = |flat: usize| {
            layout.cells.get(flat).and_then(|&(row, column)| {
                let top = layout.rows.get(row)?.top;
                Some(egui::Rect::from_min_size(
                    origin + egui::vec2(column as f32 * (card.x + gap), top),
                    card,
                ))
            })
        };
        if pressed
            && !egui::Popup::is_any_open(context)
            && let Some(point) = pointer
            && area.contains(point)
            // Not on the scrollbar at the right edge.
            && point.x < area.right() - 14.0
        {
            self.marquee = Some(Marquee {
                start: point - origin,
                base: chosen.clone(),
                additive: modifiers.ctrl || modifiers.command || modifiers.shift,
                active: false,
                on_card: (0..paths.len())
                    .any(|flat| rect_of(flat).is_some_and(|r| r.contains(point))),
            });
        }
        let marquee = self.marquee.as_mut()?;
        if !down {
            let done = self.marquee.take()?;
            // A plain click on empty space chooses nothing.
            return (!done.active && !done.on_card && !done.additive).then(Vec::new);
        }
        let point = pointer?;
        if !marquee.active && (point - origin - marquee.start).length() > BOX_DISTANCE {
            marquee.active = true;
        }
        if !marquee.active {
            return None;
        }
        let band = egui::Rect::from_two_pos(origin + marquee.start, point);
        let painter = context
            .layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("clip-box"),
            ))
            .with_clip_rect(area);
        painter.rect_filled(
            band,
            3.0,
            egui::Color32::from_rgba_unmultiplied(190, 255, 43, 34),
        );
        painter.rect_stroke(
            band,
            3.0,
            egui::Stroke::new(1.5_f32, crate::kit::LIME),
            egui::StrokeKind::Inside,
        );
        // Near the top or bottom edge the page scrolls, so the box can reach further.
        let edge = 56.0;
        if point.y < area.top() + edge {
            self.nudge = -((area.top() + edge - point.y) / edge * 18.0);
        } else if point.y > area.bottom() - edge {
            self.nudge = (point.y - (area.bottom() - edge)) / edge * 18.0;
        }
        context.request_repaint();
        let mut now = if marquee.additive {
            marquee.base.clone()
        } else {
            std::collections::BTreeSet::new()
        };
        for (flat, path) in paths.iter().enumerate() {
            if rect_of(flat).is_some_and(|rect| rect.intersects(band)) {
                now.insert((*path).to_owned());
            }
        }
        Some(now.into_iter().collect())
    }

    fn navigate(
        &mut self,
        ui: &mut egui::Ui,
        result: &LibraryScanResult,
        indices: &[usize],
        layout: &Layout,
        parts: usize,
    ) {
        if egui::Popup::is_any_open(ui.ctx()) {
            return;
        }
        let key = ui.input(|i| {
            [
                Key::Tab,
                Key::ArrowUp,
                Key::ArrowDown,
                Key::Home,
                Key::End,
                Key::PageUp,
                Key::PageDown,
            ]
            .into_iter()
            .find(|k| i.key_pressed(*k))
        });
        let Some(key) = key else {
            return;
        };
        let Some(focus) = ui.memory(|m| m.focused()) else {
            return;
        };
        let current = indices.iter().enumerate().find_map(|(index, &source)| {
            let entry = result.entries.get(source)?;
            (0..4)
                .find(|&part| control_id(entry, part) == focus)
                .map(|part| (index, part))
        });
        let Some((index, part)) = current else {
            return;
        };
        let shift = ui.input(|i| i.modifiers.shift);
        if ui.input(|i| i.modifiers.ctrl || i.modifiers.alt || i.modifiers.command) {
            return;
        }
        let last = indices.len().saturating_sub(1);
        // The card in the same column, `rows` card rows up (negative) or down.
        let vertical = |rows: isize| -> usize {
            let Some(&(row, column)) = layout.cells.get(index) else {
                return index;
            };
            let mut target_row = row;
            let mut remaining = rows.abs();
            while remaining > 0 {
                let mut next = target_row as isize + rows.signum();
                while next >= 0
                    && layout
                        .rows
                        .get(next as usize)
                        .is_some_and(|item| matches!(item.kind, RowKind::Header(..)))
                {
                    next += rows.signum();
                }
                if next < 0 || next as usize >= layout.rows.len() {
                    break;
                }
                target_row = next as usize;
                remaining -= 1;
            }
            match layout.rows.get(target_row).map(|item| &item.kind) {
                Some(RowKind::Cards(places)) => (places.start + column)
                    .min(places.end.saturating_sub(1))
                    .min(last),
                _ => index,
            }
        };
        let next = match key {
            Key::Tab => {
                let ordinal = index * parts + part;
                let next = if shift {
                    ordinal.checked_sub(1)
                } else {
                    (ordinal + 1 < indices.len() * parts).then_some(ordinal + 1)
                };
                next.map(|n| (n / parts, n % parts))
            }
            Key::Home => Some((0, part)),
            Key::End => Some((last, part)),
            Key::ArrowUp => Some((vertical(-1), part)),
            Key::ArrowDown => Some((vertical(1), part)),
            Key::PageUp => Some((vertical(-3), part)),
            Key::PageDown => Some((vertical(3), part)),
            _ => None,
        };
        if let Some(target) = next {
            ui.input_mut(|i| {
                i.consume_key(
                    if shift {
                        Modifiers::SHIFT
                    } else {
                        Modifiers::NONE
                    },
                    key,
                )
            });
            ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
            self.target = Some(target);
        }
    }
}

/// A date heading with how many clips are under it.
fn paint_header(
    ui: &egui::Ui,
    origin: egui::Pos2,
    row: &RowItem,
    group: &DateGroup,
    members: usize,
    width: f32,
) {
    let painter = ui.painter();
    let left = origin.x + 2.0;
    let middle = origin.y + row.top + row.height - 14.0;
    let galley = painter.layout_no_wrap(
        group.label(),
        egui::FontId::new(crate::kit::text_size::LEAD, crate::kit::semibold()),
        crate::kit::TEXT,
    );
    painter.galley(
        egui::pos2(left, middle - galley.size().y / 2.0),
        galley.clone(),
        crate::kit::TEXT,
    );
    painter.text(
        egui::pos2(left + galley.size().x + 10.0, middle),
        egui::Align2::LEFT_CENTER,
        members.to_string(),
        egui::FontId::proportional(crate::kit::text_size::SMALL),
        crate::kit::FAINT,
    );
    painter.line_segment(
        [
            egui::pos2(left + galley.size().x + 40.0, middle),
            egui::pos2(origin.x + width, middle),
        ],
        egui::Stroke::new(1.0_f32, crate::kit::BORDER),
    );
}

/// The place a carried card would take: a faint plate with a dashed outline.
fn paint_slot(ui: &egui::Ui, rect: egui::Rect) {
    let painter = ui.painter();
    painter.rect_filled(
        rect.shrink(1.0),
        crate::kit::radius::LG as f32,
        egui::Color32::from_rgba_unmultiplied(190, 255, 43, 12),
    );
    let stroke = egui::Stroke::new(
        1.5_f32,
        egui::Color32::from_rgba_unmultiplied(190, 255, 43, 120),
    );
    let inner = rect.shrink(2.0);
    let corners = [
        inner.left_top(),
        inner.right_top(),
        inner.right_bottom(),
        inner.left_bottom(),
        inner.left_top(),
    ];
    for pair in corners.windows(2) {
        painter.extend(egui::Shape::dashed_line(
            &[pair[0], pair[1]],
            stroke,
            8.0,
            6.0,
        ));
    }
}

/// The card in hand: its picture, a little larger, with a shadow, drawn above everything.
fn paint_carried(
    context: &egui::Context,
    entry: &MediaEntry,
    thumbs: &mut Thumbnails,
    top_left: egui::Pos2,
    width: f32,
    height: f32,
) {
    let picture = egui::vec2(width, height - crate::CARD_TEXT_HEIGHT);
    let rect = egui::Rect::from_min_size(top_left, picture).expand(6.0);
    let texture = thumbs.texture(context, &entry.path, true);
    egui::Area::new(egui::Id::new("carried-clip"))
        .order(egui::Order::Tooltip)
        .fixed_pos(rect.min)
        .interactable(false)
        .show(context, |ui| {
            ui.allocate_exact_size(rect.size(), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(
                rect.translate(egui::vec2(0.0, 10.0)).expand(2.0),
                crate::kit::radius::LG as f32,
                egui::Color32::from_black_alpha(110),
            );
            crate::clip_card::paint_video_placeholder(
                ui,
                rect,
                entry,
                texture,
                crate::kit::radius::LG,
            );
            painter.rect_stroke(
                rect,
                crate::kit::radius::LG as f32,
                egui::Stroke::new(
                    2.0_f32,
                    egui::Color32::from_rgba_unmultiplied(190, 255, 43, 220),
                ),
                egui::StrokeKind::Inside,
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_rows_bound_widgets_and_keyboard_can_reach_last_and_first_cards() {
        let context = crate::kit::test_context();
        let result = LibraryScanResult {
            directory: "/clips".into(),
            entries: (0..10_000)
                .map(|i| crate::tests::media_entry(&format!("Clip {i:05}"), None))
                .collect(),
            persisted: false,
            collections: Vec::new(),
        };
        let mut view = ViewCache::default();
        view.prepare(&result, &crate::LibraryFilter::All, "", false);
        let mut grid = GridState::default();
        let mut thumbs = Thumbnails::default();
        for (key, expected) in [
            (None, None),
            (Some(Key::End), Some(9999)),
            (Some(Key::Home), Some(0)),
        ] {
            if key.is_some() {
                context.memory_mut(|m| m.request_focus(card_id(&result.entries[0])));
            }
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 700.0),
                    )),
                    events: key
                        .into_iter()
                        .map(|key| egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: Modifiers::NONE,
                        })
                        .collect(),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        grid.show(
                            ui,
                            &result,
                            &view,
                            &mut thumbs,
                            false,
                            GridShape::default(),
                            None,
                        );
                    });
                },
            );
            assert!(grid.painted < 40, "{} cards built", grid.painted);
            if let Some(expected) = expected {
                assert_eq!(
                    context.memory(|m| m.focused()),
                    Some(card_id(&result.entries[expected]))
                );
            }
        }
    }
}

#[cfg(test)]
mod shape_tests {
    use super::*;

    /// Draws `grid` once at `size`, with the keys pressed, and returns how many clips were built.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        context: &egui::Context,
        grid: &mut GridState,
        result: &LibraryScanResult,
        view: &ViewCache,
        thumbs: &mut Thumbnails,
        size: egui::Vec2,
        by_date: bool,
        shape: GridShape,
        keys: Vec<Key>,
    ) -> usize {
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: keys
                    .into_iter()
                    .map(|key| egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::NONE,
                    })
                    .collect(),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    grid.show(ui, result, view, thumbs, by_date, shape, None);
                });
            },
        );
        grid.painted
    }

    #[test]
    fn a_list_and_every_card_size_stay_virtual_in_any_window_and_keep_the_keyboard() {
        let context = crate::kit::test_context();
        let result = LibraryScanResult {
            directory: "/clips".into(),
            entries: (0..10_000)
                .map(|i| crate::tests::media_entry(&format!("Clip {i:05}"), None))
                .collect(),
            persisted: false,
            collections: Vec::new(),
        };
        let mut view = ViewCache::default();
        view.prepare(&result, &crate::LibraryFilter::All, "", false);
        let mut thumbs = Thumbnails::default();
        let shapes = [
            GridShape {
                list: false,
                size: 0,
            },
            GridShape {
                list: false,
                size: 1,
            },
            GridShape {
                list: false,
                size: 2,
            },
            // A size from a damaged file is drawn as the medium one.
            GridShape {
                list: false,
                size: 250,
            },
            GridShape {
                list: true,
                size: 1,
            },
        ];
        for shape in shapes {
            for by_date in [false, true] {
                let mut grid = GridState::default();
                for size in [
                    egui::vec2(240.0, 400.0),
                    egui::vec2(700.0, 500.0),
                    egui::vec2(1200.0, 700.0),
                    egui::vec2(3000.0, 1500.0),
                ] {
                    let built = draw(
                        &context,
                        &mut grid,
                        &result,
                        &view,
                        &mut thumbs,
                        size,
                        by_date,
                        shape,
                        vec![],
                    );
                    assert!(
                        built > 0 && built < 120,
                        "{shape:?} {by_date} {size:?}: {built} built"
                    );
                }
                // From the first clip the End key reaches the last, and Home goes back.
                for (key, expected) in [(Key::End, 9999), (Key::Home, 0)] {
                    context.memory_mut(|m| m.request_focus(card_id(&result.entries[0])));
                    draw(
                        &context,
                        &mut grid,
                        &result,
                        &view,
                        &mut thumbs,
                        egui::vec2(1200.0, 700.0),
                        by_date,
                        shape,
                        vec![key],
                    );
                    assert_eq!(
                        context.memory(|m| m.focused()),
                        Some(card_id(&result.entries[expected])),
                        "{shape:?} {by_date} {key:?}"
                    );
                }
            }
        }
        // More fit in a row at the small size and fewer at the large one; a list is one column.
        let mut built = std::collections::BTreeMap::new();
        for (name, shape) in [
            (
                "small",
                GridShape {
                    list: false,
                    size: 0,
                },
            ),
            (
                "large",
                GridShape {
                    list: false,
                    size: 2,
                },
            ),
        ] {
            let mut grid = GridState::default();
            built.insert(
                name,
                draw(
                    &context,
                    &mut grid,
                    &result,
                    &view,
                    &mut thumbs,
                    egui::vec2(1600.0, 900.0),
                    false,
                    shape,
                    vec![],
                ),
            );
        }
        assert!(built["small"] > built["large"], "{built:?}");
    }
}

#[cfg(test)]
mod box_tests {
    use super::*;

    fn clips(count: usize) -> (LibraryScanResult, ViewCache) {
        let result = LibraryScanResult {
            directory: "/clips".into(),
            entries: (0..count)
                .map(|i| crate::tests::media_entry(&format!("Clip {i:03}"), None))
                .collect(),
            persisted: false,
            collections: Vec::new(),
        };
        let mut view = ViewCache::default();
        view.prepare(&result, &crate::LibraryFilter::All, "", false);
        (result, view)
    }

    /// One frame with these pointer events; the action the grid answered with.
    fn frame(
        context: &egui::Context,
        grid: &mut GridState,
        result: &LibraryScanResult,
        view: &ViewCache,
        chosen: &std::collections::BTreeSet<String>,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> Option<ClipCardAction> {
        let mut answer = None;
        let mut thumbs = Thumbnails::default();
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 700.0),
                )),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    answer = grid.show(
                        ui,
                        result,
                        view,
                        &mut thumbs,
                        false,
                        GridShape::default(),
                        Some(chosen),
                    );
                });
            },
        );
        answer
    }

    fn press(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn drag(
        context: &egui::Context,
        grid: &mut GridState,
        result: &LibraryScanResult,
        view: &ViewCache,
        chosen: &std::collections::BTreeSet<String>,
        from: egui::Pos2,
        to: egui::Pos2,
        modifiers: egui::Modifiers,
    ) -> Option<Vec<String>> {
        let none = egui::Modifiers::NONE;
        let mut set = None;
        let steps = [
            vec![egui::Event::PointerMoved(from)],
            vec![press(from, true, modifiers)],
            vec![egui::Event::PointerMoved(from.lerp(to, 0.5))],
            vec![egui::Event::PointerMoved(to)],
        ];
        for events in steps {
            if let Some(ClipCardAction::SetSelection(paths)) =
                frame(context, grid, result, view, chosen, events, modifiers)
            {
                set = Some(paths);
            }
        }
        // Let go.
        if let Some(ClipCardAction::SetSelection(paths)) = frame(
            context,
            grid,
            result,
            view,
            chosen,
            vec![press(to, false, modifiers)],
            none,
        ) {
            set = Some(paths);
        }
        set
    }

    #[test]
    fn a_drawn_box_chooses_what_it_touches_ctrl_adds_and_empty_space_clears() {
        let context = crate::kit::test_context();
        let (result, view) = clips(40);
        let mut grid = GridState::default();
        let none = egui::Modifiers::NONE;
        let empty = std::collections::BTreeSet::new();
        // Start in the page margin (not on a card) and drag down and right over the first cards.
        let from = egui::pos2(14.0, 14.0);
        let to = egui::pos2(700.0, 300.0);
        let chosen = drag(&context, &mut grid, &result, &view, &empty, from, to, none)
            .expect("the box chose something");
        assert!(chosen.len() >= 3, "{chosen:?}");
        assert!(chosen.contains(&result.entries[0].path));
        // The box covers the top two rows of cards: chosen clips come from the first clips
        // only, from more than one row, and not every card of a row is necessarily touched.
        let index_of = |path: &String| result.entries.iter().position(|e| &e.path == path).unwrap();
        let indices: Vec<usize> = chosen.iter().map(index_of).collect();
        assert!(indices.iter().all(|&index| index < 12), "{indices:?}");
        assert!(
            indices.iter().any(|&index| index >= 3),
            "more than one row: {indices:?}"
        );
        // A smaller box chooses fewer.
        let small = drag(
            &context,
            &mut grid,
            &result,
            &view,
            &empty,
            from,
            egui::pos2(200.0, 150.0),
            none,
        )
        .expect("a small box");
        assert!(
            !small.is_empty() && small.len() < chosen.len(),
            "{small:?} vs {chosen:?}"
        );
        // With Ctrl the box adds to what was already chosen.
        let already: std::collections::BTreeSet<String> =
            [result.entries[30].path.clone()].into_iter().collect();
        let added = drag(
            &context,
            &mut grid,
            &result,
            &view,
            &already,
            from,
            egui::pos2(200.0, 150.0),
            egui::Modifiers::CTRL,
        )
        .expect("an adding box");
        assert!(added.contains(&result.entries[30].path), "{added:?}");
        assert!(added.len() > 1);
        // Without Ctrl it replaces what was chosen.
        let replaced = drag(
            &context,
            &mut grid,
            &result,
            &view,
            &already,
            from,
            egui::pos2(200.0, 150.0),
            none,
        )
        .expect("a replacing box");
        assert!(!replaced.contains(&result.entries[30].path), "{replaced:?}");
        // A plain click on empty space chooses nothing at all (two clips leave room to click).
        let (few, few_view) = clips(2);
        let blank = egui::pos2(800.0, 600.0);
        let cleared = drag(
            &context, &mut grid, &few, &few_view, &already, blank, blank, none,
        );
        assert_eq!(cleared, Some(Vec::new()));
        // A click on a clip is that clip's own click and clears nothing.
        let on_card = drag(
            &context, &mut grid, &few, &few_view, &already, from, from, none,
        );
        assert_eq!(on_card, None);
        // And Ctrl on empty space keeps what was chosen.
        let kept = drag(
            &context,
            &mut grid,
            &few,
            &few_view,
            &already,
            blank,
            blank,
            egui::Modifiers::CTRL,
        );
        assert_eq!(kept, None);
        // And not while the page is not choosing: the box is not drawn and nothing is returned.
        let mut thumbs = Thumbnails::default();
        for events in [
            vec![egui::Event::PointerMoved(from)],
            vec![press(from, true, none)],
            vec![egui::Event::PointerMoved(to)],
            vec![press(to, false, none)],
        ] {
            let mut answer = None;
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        answer = grid.show(
                            ui,
                            &result,
                            &view,
                            &mut thumbs,
                            false,
                            GridShape::default(),
                            None,
                        );
                    });
                },
            );
            assert!(!matches!(answer, Some(ClipCardAction::SetSelection(_))));
        }
    }
}

#[cfg(test)]
mod traversal_tests {
    use super::*;
    #[test]
    fn tab_and_shift_tab_cross_a_virtual_row_without_double_advancing_egui_focus() {
        let context = crate::kit::test_context();
        let result = LibraryScanResult {
            directory: "/clips".into(),
            entries: (0..100)
                .map(|i| crate::tests::media_entry(&format!("Clip {i:03}"), None))
                .collect(),
            persisted: false,
            collections: vec![],
        };
        let mut view = ViewCache::default();
        view.prepare(&result, &crate::LibraryFilter::All, "", false);
        let mut grid = GridState::default();
        let mut thumbs = Thumbnails::default();
        let draw = |grid: &mut GridState, thumbs: &mut Thumbnails, events: Vec<egui::Event>| {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        grid.show(
                            ui,
                            &result,
                            &view,
                            thumbs,
                            false,
                            GridShape::default(),
                            None,
                        );
                    });
                },
            );
        };
        draw(&mut grid, &mut thumbs, vec![]);
        // Pointer activation establishes focus even when an offline card cannot open.
        draw(
            &mut grid,
            &mut thumbs,
            vec![
                egui::Event::PointerMoved(egui::pos2(80.0, 80.0)),
                egui::Event::PointerButton {
                    pos: egui::pos2(80.0, 80.0),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: egui::pos2(80.0, 80.0),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        assert_eq!(
            context.memory(|m| m.focused()),
            Some(card_id(&result.entries[0]))
        );
        let last = grid.visible.end * columns(1184.0) - 1;
        context.memory_mut(|m| m.request_focus(control_id(&result.entries[last], 3)));
        for (modifiers, expected, part) in
            [(Modifiers::NONE, last + 1, 0), (Modifiers::SHIFT, last, 3)]
        {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 700.0),
                    )),
                    modifiers,
                    events: vec![egui::Event::Key {
                        key: Key::Tab,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        grid.show(
                            ui,
                            &result,
                            &view,
                            &mut thumbs,
                            false,
                            GridShape::default(),
                            None,
                        );
                    });
                },
            );
            assert_eq!(
                context.memory(|m| m.focused()),
                Some(control_id(&result.entries[expected], part))
            );
        }
    }
}

#[cfg(test)]
mod layout_and_drag_tests {
    use super::*;
    use crate::clip_label::DateGroup;

    fn result(count: usize) -> LibraryScanResult {
        LibraryScanResult {
            directory: "/clips".into(),
            entries: (0..count)
                .map(|i| crate::tests::media_entry(&format!("Clip {i:02}"), Some(1_000 + i as u64)))
                .collect(),
            persisted: false,
            collections: Vec::new(),
        }
    }

    #[test]
    fn headings_start_their_own_rows_and_columns_restart_after_them() {
        let groups = [
            DateGroup::Today,
            DateGroup::Today,
            DateGroup::Today,
            DateGroup::Today,
            DateGroup::Today,
            DateGroup::Yesterday,
            DateGroup::Yesterday,
        ];
        let layout = build_layout(7, 3, 100.0, 16.0, Some(&groups));
        let kinds: Vec<String> = layout
            .rows
            .iter()
            .map(|row| match &row.kind {
                RowKind::Header(group, members) => format!("{}:{members}", group.label()),
                RowKind::Cards(places) => format!("{}..{}", places.start, places.end),
            })
            .collect();
        assert_eq!(kinds, ["Today:5", "0..3", "3..5", "Yesterday:2", "5..7"]);
        // Every card knows its row and column; nothing overlaps.
        assert_eq!(layout.cells[0], (1, 0));
        assert_eq!(layout.cells[4], (2, 1));
        assert_eq!(layout.cells[5], (4, 0));
        assert!(
            layout
                .rows
                .windows(2)
                .all(|pair| pair[0].top + pair[0].height <= pair[1].top)
        );
        assert_eq!(build_layout(0, 3, 100.0, 16.0, None).total, 0.0);
        // Without headings the rows are plain and even.
        let plain = build_layout(7, 3, 100.0, 16.0, None);
        assert_eq!(plain.rows.len(), 3);
        assert_eq!(plain.rows[1].top, 116.0);
    }

    struct Frames {
        context: egui::Context,
        grid: GridState,
        thumbs: Thumbnails,
    }

    impl Frames {
        fn new() -> Self {
            Self {
                context: crate::kit::test_context(),
                grid: GridState::default(),
                thumbs: Thumbnails::default(),
            }
        }

        fn frame(
            &mut self,
            result: &LibraryScanResult,
            view: &ViewCache,
            events: Vec<egui::Event>,
            by_date: bool,
        ) -> Option<ClipCardAction> {
            let mut action = None;
            let _ = self.context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        action = self.grid.show(
                            ui,
                            result,
                            view,
                            &mut self.thumbs,
                            by_date,
                            GridShape::default(),
                            None,
                        );
                    });
                },
            );
            action
        }
    }

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    /// Presses on card 0, moves to `to` over a few frames, and lets go (or not).
    fn carry(
        frames: &mut Frames,
        result: &LibraryScanResult,
        view: &ViewCache,
        to: egui::Pos2,
        release: bool,
    ) -> Vec<ClipCardAction> {
        let start = egui::pos2(100.0, 100.0);
        let mut actions = Vec::new();
        let mut push = |action: Option<ClipCardAction>| actions.extend(action);
        push(frames.frame(result, view, vec![], false));
        push(frames.frame(
            result,
            view,
            vec![egui::Event::PointerMoved(start), button(start, true)],
            false,
        ));
        for step in 1..=8 {
            let at = start + (to - start) * (step as f32 / 8.0);
            push(frames.frame(result, view, vec![egui::Event::PointerMoved(at)], false));
        }
        if release {
            push(frames.frame(result, view, vec![button(to, false)], false));
        }
        actions
    }

    #[test]
    fn dragging_a_card_to_another_place_gives_the_new_order_when_let_go() {
        let result = result(6);
        let mut view = ViewCache::default();
        view.prepare(&result, &crate::LibraryFilter::All, "", false);
        let before: Vec<String> = view
            .indices
            .iter()
            .map(|&i| result.entries[i].path.clone())
            .collect();
        let mut frames = Frames::new();
        let actions = carry(&mut frames, &result, &view, egui::pos2(700.0, 100.0), true);
        let order = actions.into_iter().find_map(|action| match action {
            ClipCardAction::Reordered(paths) => Some(paths),
            _ => None,
        });
        let order = order.expect("the new order was given");
        assert_eq!(order.len(), 6);
        // The first card moved to the third place; the rest closed up behind it.
        assert_eq!(order[2], before[0]);
        assert_eq!(order[0], before[1]);
        assert_eq!(order[1], before[2]);
        assert_eq!(order[3], before[3]);
        assert!(frames.grid.carrying.is_none());
    }

    #[test]
    fn letting_go_where_it_started_changes_nothing() {
        let result = result(6);
        let mut view = ViewCache::default();
        view.prepare(&result, &crate::LibraryFilter::All, "", false);
        let mut frames = Frames::new();
        let actions = carry(&mut frames, &result, &view, egui::pos2(110.0, 112.0), true);
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, ClipCardAction::Reordered(_))),
            "a card that did not move is not a new order"
        );
    }

    #[test]
    fn taking_a_card_past_the_edge_of_the_window_hands_it_to_the_system_instead() {
        let result = result(6);
        let mut view = ViewCache::default();
        view.prepare(&result, &crate::LibraryFilter::All, "", false);
        let mut frames = Frames::new();
        let actions = carry(
            &mut frames,
            &result,
            &view,
            egui::pos2(1300.0, 100.0),
            false,
        );
        let dragged = actions.iter().find_map(|action| match action {
            ClipCardAction::DragOut(entry) => Some(entry.path.clone()),
            _ => None,
        });
        assert_eq!(dragged, Some(result.entries[view.indices[0]].path.clone()));
        assert!(frames.grid.carrying.is_none(), "the reorder ended");
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, ClipCardAction::Reordered(_)))
        );
    }

    #[test]
    fn a_card_handed_to_the_system_is_not_picked_up_again_by_the_same_press() {
        let result = result(6);
        let mut view = ViewCache::default();
        view.prepare(&result, &crate::LibraryFilter::All, "", false);
        let mut frames = Frames::new();
        let mut actions = carry(
            &mut frames,
            &result,
            &view,
            egui::pos2(1300.0, 100.0),
            false,
        );
        // The button is still down and the pointer still outside for many more frames.
        for _ in 0..12 {
            actions.extend(frames.frame(
                &result,
                &view,
                vec![egui::Event::PointerMoved(egui::pos2(1310.0, 105.0))],
                false,
            ));
        }
        let drags = actions
            .iter()
            .filter(|action| matches!(action, ClipCardAction::DragOut(_)))
            .count();
        assert_eq!(drags, 1, "one press, one drag out");
        // Letting go and pressing again can pick a card up once more.
        let _ = frames.frame(
            &result,
            &view,
            vec![button(egui::pos2(1310.0, 105.0), false)],
            false,
        );
        assert!(!frames.grid.spent);
    }

    #[test]
    fn arrow_keys_cross_a_date_heading_in_the_same_column() {
        // Four clips today (one row of four) and four yesterday.
        let now = clipcraft_engine::unix_now();
        let mut result = result(8);
        for (index, entry) in result.entries.iter_mut().enumerate() {
            entry.modified_unix_seconds = Some(if index < 4 { now } else { now - 86_400 });
        }
        let mut view = ViewCache::default();
        view.prepare(
            &result,
            &crate::LibraryFilter::All,
            "",
            crate::library_view::Sort::Newest,
        );
        let mut frames = Frames::new();
        let _ = frames.frame(&result, &view, vec![], true);
        let first_today = view.indices[1];
        frames
            .context
            .memory_mut(|m| m.request_focus(card_id(&result.entries[first_today])));
        let _ = frames.frame(
            &result,
            &view,
            vec![egui::Event::Key {
                key: Key::ArrowDown,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            true,
        );
        let _ = frames.frame(&result, &view, vec![], true);
        // The second card of the day below.
        assert_eq!(
            frames.context.memory(|m| m.focused()),
            Some(card_id(&result.entries[view.indices[5]]))
        );
    }
}
