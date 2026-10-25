//! Fixed-height virtual card rows with stable IDs and logical keyboard traversal.
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
#[derive(Default)]
pub(crate) struct GridState {
    visible: Range<usize>,
    target: Option<(usize, usize)>,
    layout: Option<(usize, u32)>,
    #[cfg(test)]
    pub painted: usize,
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
impl GridState {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        result: &LibraryScanResult,
        view: &ViewCache,
        thumbs: &mut Thumbnails,
    ) -> Option<ClipCardAction> {
        if self
            .target
            .is_some_and(|(index, _)| index >= view.indices.len())
        {
            self.target = None;
        }
        let columns = columns(ui.available_width());
        let width =
            ((ui.available_width() - (columns - 1) as f32 * 16.0) / columns as f32).max(180.0);
        let height = (width * 0.5625).clamp(120.0, 240.0) + 66.0 + 42.0;
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
        self.navigate(ui, result, &view.indices, columns);
        let mut scroll = egui::ScrollArea::vertical().id_salt("clip-grid");
        if let Some((index, _)) = self.target {
            let row = index / columns;
            if !self.visible.contains(&row) {
                scroll = scroll.vertical_scroll_offset(row as f32 * (height + 16.0));
            }
        }
        let mut action = None;
        #[cfg(test)]
        {
            self.painted = 0;
        }
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(16.0, 16.0);
            scroll.show_rows(
                ui,
                height,
                view.indices.len().div_ceil(columns),
                |ui, rows| {
                    self.visible = rows.clone();
                    for row in rows {
                        ui.horizontal_top(|ui| {
                            for column in 0..columns {
                                let index = row * columns + column;
                                let Some(&source) = view.indices.get(index) else {
                                    break;
                                };
                                let Some(entry) = result.entries.get(source) else {
                                    continue;
                                };
                                if let Some(clicked) = clip_card(
                                    ui,
                                    entry,
                                    width,
                                    thumbs,
                                    view.game(source),
                                    &result.collections,
                                ) {
                                    action = Some(clicked);
                                }
                                #[cfg(test)]
                                {
                                    self.painted += 1;
                                }
                                if self.target.is_some_and(|(target, _)| target == index)
                                    && let Some((_, part)) = self.target.take()
                                {
                                    ui.memory_mut(|m| m.request_focus(control_id(entry, part)));
                                }
                            }
                        });
                    }
                },
            );
        });
        action
    }
    fn navigate(
        &mut self,
        ui: &mut egui::Ui,
        result: &LibraryScanResult,
        indices: &[usize],
        columns: usize,
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
        let next = match key {
            Key::Tab => {
                let ordinal = index * 4 + part;
                let next = if shift {
                    ordinal.checked_sub(1)
                } else {
                    (ordinal + 1 < indices.len() * 4).then_some(ordinal + 1)
                };
                next.map(|n| (n / 4, n % 4))
            }
            Key::Home => Some((0, part)),
            Key::End => Some((indices.len().saturating_sub(1), part)),
            Key::ArrowUp => Some((index.saturating_sub(columns), part)),
            Key::ArrowDown => Some(((index + columns).min(indices.len().saturating_sub(1)), part)),
            Key::PageUp => Some((index.saturating_sub(columns * 3), part)),
            Key::PageDown => Some((
                (index + columns * 3).min(indices.len().saturating_sub(1)),
                part,
            )),
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
                        grid.show(ui, &result, &view, &mut thumbs);
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
                        grid.show(ui, &result, &view, thumbs);
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
                        grid.show(ui, &result, &view, &mut thumbs);
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
