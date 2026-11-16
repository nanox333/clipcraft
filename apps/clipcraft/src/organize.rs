//! Browsing the library by favorites, collections, and games.

use clipcraft_engine::LibraryScanResult;
#[cfg(test)]
use clipcraft_engine::{MediaEntry, game_of};
use eframe::egui;
use egui::{RichText, Vec2};

use crate::kit::{self, CARD, CARD_HOVER, DANGER, FAINT, LIME, MUTED, TEXT};

/// Which clips the library shows.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum LibraryFilter {
    /// Every clip.
    #[default]
    All,
    /// Clips marked as favorites.
    Favorites,
    /// Clips in one collection.
    Collection(String),
    /// Clips from one game.
    Game(String),
}

impl LibraryFilter {
    /// True when `entry` belongs in this view.
    #[cfg(test)]
    pub fn matches(&self, entry: &MediaEntry, directory: &str) -> bool {
        match self {
            Self::All => true,
            Self::Favorites => entry.favorite,
            Self::Collection(name) => entry.collections.contains(name),
            Self::Game(name) => game_of(entry, directory)
                .is_some_and(|game| game.to_lowercase() == name.to_lowercase()),
        }
    }

    /// The heading shown above the clips.
    pub fn title(&self) -> String {
        match self {
            Self::All => "Clips".to_owned(),
            Self::Favorites => "Favorites".to_owned(),
            Self::Collection(name) | Self::Game(name) => name.clone(),
        }
    }

    /// What to tell the person when the view is empty.
    pub fn empty_message(&self) -> (&'static str, &'static str) {
        match self {
            Self::All => (
                "No clips here yet",
                "Videos in your clips folder show up here.",
            ),
            Self::Favorites => (
                "No favorites yet",
                "Tap the heart on a clip to keep it here.",
            ),
            Self::Collection(_) => (
                "This collection is empty",
                "Right-click a clip, or open it, to add it to a collection.",
            ),
            Self::Game(_) => (
                "No clips from this game",
                "Clips you assign to it appear here.",
            ),
        }
    }
}

/// A collection being created or renamed in the rail.
#[derive(Clone, Debug, Default)]
pub struct CollectionEdit {
    /// The collection being renamed; `None` while creating a new one.
    pub original: Option<String>,
    /// The name typed so far.
    pub text: String,
    /// Whether the text box has been given focus yet.
    pub focused: bool,
}

/// What the person did in the rail this frame.
pub enum RailAction {
    /// Show a different set of clips.
    Select(LibraryFilter),
    /// Start naming a new collection.
    BeginCreate,
    /// Start renaming a collection.
    BeginRename(String),
    /// Delete a collection (its clips stay).
    Delete(String),
    /// Save the name being typed.
    Commit,
    /// Throw the name being typed away.
    Cancel,
    /// Change how a game looks in the rail; `None` goes back to the automatic look.
    SetBadge {
        game: String,
        badge: Option<clipcraft_engine::GameBadge>,
    },
}

/// The colours offered for a game, before the picker for any other.
const SWATCHES: [&str; 11] = [
    "#bfff2b", "#ff6b6b", "#ff9f43", "#ffd93d", "#6bcb77", "#00f5d4", "#00bbf9", "#4d96ff",
    "#9b5de5", "#f15bb5", "#e8e8ee",
];

fn parse_hex(text: &str) -> Option<egui::Color32> {
    let digits = text.strip_prefix('#')?;
    (digits.len() == 6).then_some(())?;
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some(egui::Color32::from_rgb(
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ))
}

/// A game's colour and mark in the rail: what the person chose, else the automatic colour and the
/// first letter.
fn badge_look(
    name: &str,
    badges: &std::collections::BTreeMap<String, clipcraft_engine::GameBadge>,
) -> (egui::Color32, String, bool) {
    let chosen = badges.get(&name.trim().to_lowercase());
    let colour = chosen
        .and_then(|badge| parse_hex(&badge.color))
        .unwrap_or_else(|| name_colour(name));
    let mark = chosen
        .map(|badge| badge.mark.clone())
        .filter(|mark| !mark.is_empty())
        .unwrap_or_else(|| {
            name.chars()
                .find(|c| c.is_alphanumeric())
                .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().to_string())
        });
    (colour, mark, chosen.is_some())
}

/// How a game's badge is drawn: a soft square in its colour with the mark in the colour itself.
fn paint_badge(painter: &egui::Painter, square: egui::Rect, colour: egui::Color32, mark: &str) {
    painter.rect_filled(
        square,
        if square.width() > 24.0 { 8.0 } else { 6.0 },
        egui::Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), 46),
    );
    painter.text(
        square.center(),
        egui::Align2::CENTER_CENTER,
        mark,
        egui::FontId::new(
            (square.width() * 0.48).max(crate::kit::text_size::CAPTION),
            kit::semibold(),
        ),
        colour,
    );
}

/// Marks offered besides the first letter.
const MARKS: [&str; 10] = [
    "\u{2605}", "\u{2665}", "\u{2694}", "\u{26a1}", "\u{25cf}", "\u{25b2}", "\u{25c6}", "\u{2660}",
    "\u{2663}", "\u{2620}",
];
/// Size of one tile of the mark row.
const MARK_TILE: f32 = 30.0;

/// One mark as a small badge in the game's colour; ringed when it is the one in use.
fn mark_tile(
    ui: &mut egui::Ui,
    mark: &str,
    selected: bool,
    colour: egui::Color32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(MARK_TILE), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Mark {mark}"))
    });
    paint_badge(ui.painter(), rect, colour, mark);
    if selected || response.hovered() {
        ui.painter().rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.5_f32, if selected { TEXT } else { MUTED }),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The menu on a game: how its badge looks now, a palette ending in a picker for any colour, and
/// the mark.
fn badge_menu(
    ui: &mut egui::Ui,
    game: &str,
    badges: &std::collections::BTreeMap<String, clipcraft_engine::GameBadge>,
    actions: &mut Vec<RailAction>,
) {
    use crate::colour_picker;
    let key = game.trim().to_lowercase();
    let current = badges.get(&key).cloned().unwrap_or_default();
    let (shown, shown_mark, _) = badge_look(game, badges);
    // What the badge shows without a choice, as the hint of the field.
    let auto_mark = badge_look(game, &std::collections::BTreeMap::new()).1;
    ui.set_min_width(220.0);
    ui.set_max_width(220.0);

    let picker_open_id = egui::Id::new(("game-picker-open", &key));
    let picker_colour_id = egui::Id::new(("game-picker-colour", &key));
    let picker_hex_id = egui::Id::new(("game-picker-hex", &key));
    let mark_id = egui::Id::new(("game-mark", &key));
    let mut picker_open =
        ui.data_mut(|data| data.get_temp::<bool>(picker_open_id).unwrap_or(false));
    let mut dragged: Option<egui::ecolor::Hsva> =
        ui.data_mut(|data| data.get_temp(picker_colour_id));

    // How it looks right now, following the picker while it is being dragged.
    let preview = dragged.map_or(shown, |hsva| egui::Color32::from(hsva));
    ui.horizontal(|ui| {
        let (square, _) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::hover());
        paint_badge(ui.painter(), square, preview, &shown_mark);
        ui.add(
            egui::Label::new(
                RichText::new(game)
                    .color(TEXT)
                    .size(crate::kit::text_size::BODY)
                    .family(kit::semibold()),
            )
            .truncate(),
        );
    });
    ui.add_space(crate::kit::space::MD);

    ui.label(
        RichText::new("Colour")
            .color(MUTED)
            .size(crate::kit::text_size::CAPTION),
    );
    ui.add_space(crate::kit::space::XS);
    let is_swatch = SWATCHES.iter().any(|hex| *hex == current.color);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(10.0);
        for hex in SWATCHES {
            let Some(colour) = parse_hex(hex) else {
                continue;
            };
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Colour {hex}"))
            });
            ui.painter().circle_filled(rect.center(), 10.5, colour);
            let chosen = current.color == hex;
            if chosen || response.hovered() {
                ui.painter().circle_stroke(
                    rect.center(),
                    12.5,
                    egui::Stroke::new(1.5_f32, if chosen { TEXT } else { MUTED }),
                );
            }
            if response.clicked() {
                picker_open = false;
                dragged = None;
                ui.data_mut(|data| data.remove::<egui::ecolor::Hsva>(picker_colour_id));
                actions.push(RailAction::SetBadge {
                    game: game.to_owned(),
                    badge: Some(clipcraft_engine::GameBadge {
                        color: (*hex).to_owned(),
                        mark: current.mark.clone(),
                    }),
                });
            }
        }
        // Any other colour: opens the picker.
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Pick any colour")
        });
        colour_picker::rainbow_swatch(ui.painter(), rect.center(), 10.5);
        let custom = !current.color.is_empty() && !is_swatch;
        if picker_open || custom || response.hovered() {
            ui.painter().circle_stroke(
                rect.center(),
                12.5,
                egui::Stroke::new(1.5_f32, if picker_open || custom { TEXT } else { MUTED }),
            );
        }
        if response.on_hover_text("Any colour").clicked() {
            picker_open = !picker_open;
            if picker_open && dragged.is_none() {
                let start = parse_hex(&current.color).unwrap_or(shown);
                dragged = Some(egui::ecolor::Hsva::from(start));
            }
        }
    });

    if picker_open {
        ui.add_space(crate::kit::space::MD);
        let mut hsva = dragged.unwrap_or_else(|| egui::ecolor::Hsva::from(shown));
        let mut hex = ui
            .data_mut(|data| data.get_temp::<String>(picker_hex_id))
            .unwrap_or_else(|| colour_picker::to_hex(hsva));
        let picked = colour_picker::show(ui, &mut hsva, &mut hex);
        dragged = Some(hsva);
        ui.data_mut(|data| data.insert_temp(picker_hex_id, hex));
        if picked.settled {
            actions.push(RailAction::SetBadge {
                game: game.to_owned(),
                badge: Some(clipcraft_engine::GameBadge {
                    color: colour_picker::to_hex(hsva),
                    mark: current.mark.clone(),
                }),
            });
        }
    }
    ui.data_mut(|data| {
        data.insert_temp(picker_open_id, picker_open);
        match dragged {
            Some(hsva) if picker_open => data.insert_temp(picker_colour_id, hsva),
            _ => data.remove::<egui::ecolor::Hsva>(picker_colour_id),
        }
    });

    ui.add_space(crate::kit::space::LG);
    ui.label(
        RichText::new("Mark")
            .color(MUTED)
            .size(crate::kit::text_size::CAPTION),
    );
    ui.add_space(crate::kit::space::XS);
    let mut text: String = ui
        .data_mut(|data| data.get_temp::<String>(mark_id))
        .unwrap_or_else(|| current.mark.clone());
    let picked_mark = current.mark.clone();
    let is_quick = MARKS.contains(&picked_mark.as_str());
    let custom = !picked_mark.is_empty() && !is_quick;
    let mut chosen: Option<String> = None;
    // The marks as a row of small badges in the game's colour, like the colours above them.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(8.0);
        if mark_tile(ui, &auto_mark, picked_mark.is_empty(), preview)
            .on_hover_text("Automatic: the first letter")
            .clicked()
        {
            chosen = Some(String::new());
        }
        for mark in MARKS {
            if mark_tile(ui, mark, picked_mark == mark, preview).clicked() {
                chosen = Some((*mark).to_owned());
            }
        }
        // Anything else: a slot to type a letter, number or emoji into.
        let (slot, _) = ui.allocate_exact_size(Vec2::splat(MARK_TILE), egui::Sense::hover());
        let focused = ui.memory(|memory| memory.has_focus(mark_id.with("field")));
        ui.painter().rect_stroke(
            slot,
            8.0,
            egui::Stroke::new(1.0_f32, if focused || custom { TEXT } else { kit::BORDER }),
            egui::StrokeKind::Inside,
        );
        let field = ui
            .put(
                slot.shrink(2.0),
                egui::TextEdit::singleline(&mut text)
                    .id(mark_id.with("field"))
                    .frame(false)
                    .char_limit(2)
                    .hint_text("Aa")
                    .horizontal_align(egui::Align::Center)
                    .vertical_align(egui::Align::Center),
            )
            .on_hover_text("Type a letter, number or emoji");
        if field.changed() {
            chosen = Some(text.clone());
        }
    });
    if let Some(mark) = chosen {
        actions.push(RailAction::SetBadge {
            game: game.to_owned(),
            badge: Some(clipcraft_engine::GameBadge {
                color: current.color.clone(),
                mark: mark.clone(),
            }),
        });
        // A picked tile clears what was typed; typing keeps its own text.
        if MARKS.contains(&mark.as_str()) || mark.is_empty() {
            text.clear();
        }
    }
    ui.data_mut(|data| data.insert_temp(mark_id, text));

    ui.add_space(crate::kit::space::LG);
    let customised = badges.contains_key(&key);
    if ui
        .add_enabled(
            customised,
            egui::Button::new(RichText::new("Back to automatic").color(MUTED)).frame(false),
        )
        .clicked()
    {
        ui.data_mut(|data| {
            data.remove::<String>(mark_id);
            data.remove::<String>(picker_hex_id);
            data.remove::<egui::ecolor::Hsva>(picker_colour_id);
            data.remove::<bool>(picker_open_id);
        });
        actions.push(RailAction::SetBadge {
            game: game.to_owned(),
            badge: None,
        });
        ui.close();
    }
}

/// A colour for a name, the same every time: a fixed saturation and lightness that read on the dark
/// window, with the hue taken from the letters.
fn name_colour(name: &str) -> egui::Color32 {
    let hash = name
        .to_lowercase()
        .bytes()
        .fold(2_166_136_261_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        });
    let hue = (hash % 360) as f32 / 60.0;
    let (saturation, lightness) = (0.55_f32, 0.62_f32);
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let x = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = lightness - chroma / 2.0;
    let channel = |value: f32| ((value + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    egui::Color32::from_rgb(channel(r), channel(g), channel(b))
}

fn rail_row(
    ui: &mut egui::Ui,
    label: &str,
    count: usize,
    selected: bool,
    icon: Option<kit::Icon>,
) -> egui::Response {
    rail_row_with_badge(ui, label, count, selected, icon, None)
}

/// A row of the rail; `badge` is a name to show as a coloured initial (a game).
fn rail_row_with_badge(
    ui: &mut egui::Ui,
    label: &str,
    count: usize,
    selected: bool,
    icon: Option<kit::Icon>,
    badge: Option<(egui::Color32, String, bool)>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(0.0), 32.0),
        egui::Sense::click(),
    );
    if selected {
        ui.painter()
            .rect_filled(rect, f32::from(crate::kit::radius::MD), CARD_HOVER);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, f32::from(crate::kit::radius::MD), CARD);
    }
    let mut left = rect.left() + 10.0;
    if let Some(icon) = icon {
        kit::draw_icon(
            ui.painter(),
            egui::pos2(left + 8.0, rect.center().y),
            icon,
            if selected { LIME } else { MUTED },
            0.7,
        );
        left += 24.0;
    }
    if let Some((colour, mark, _)) = badge {
        let square = egui::Rect::from_center_size(
            egui::pos2(left + 9.0, rect.center().y),
            Vec2::splat(20.0),
        );
        paint_badge(ui.painter(), square, colour, &mark);
        left += 28.0;
    }
    let count_text = count.to_string();
    let count_width = 8.0 + count_text.len() as f32 * 7.5;
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_owned(),
        egui::FontId::new(crate::kit::text_size::SMALL, kit::medium()),
        if selected { TEXT } else { MUTED },
    );
    job.wrap.max_width = (rect.right() - left - count_width - 12.0).max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(left, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT,
    );
    ui.painter().text(
        egui::pos2(rect.right() - 10.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        count_text,
        egui::FontId::proportional(crate::kit::text_size::CAPTION),
        FAINT,
    );
    response
}

fn section_header(ui: &mut egui::Ui, title: &str, help: Option<&str>) -> egui::Response {
    ui.add_space(crate::kit::space::MD);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(0.0), 22.0),
        egui::Sense::click(),
    );
    let galley = ui.painter().layout_no_wrap(
        title.to_uppercase(),
        egui::FontId::new(crate::kit::text_size::CAPTION, kit::semibold()),
        FAINT,
    );
    let width = galley.size().x;
    ui.painter().galley(
        egui::pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        FAINT,
    );
    if let Some(help) = help {
        // The "?" explains the section, so no sentence has to sit under it.
        kit::help_at(
            ui,
            egui::pos2(rect.left() + 10.0 + width + 14.0, rect.center().y),
            response.id.with("help"),
            help,
        );
    }
    response
}

/// The list of views down the left of the library.
pub fn rail(
    ui: &mut egui::Ui,
    result: &LibraryScanResult,
    view: &crate::library_view::ViewCache,
    filter: &LibraryFilter,
    edit: &mut Option<CollectionEdit>,
    badges: &std::collections::BTreeMap<String, clipcraft_engine::GameBadge>,
) -> Vec<RailAction> {
    let mut actions = Vec::new();
    let favorites = view.favorites;
    egui::ScrollArea::vertical()
        .id_salt("library_rail")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = crate::kit::space::XS;
            if rail_row(ui, "All clips", result.entries.len(), *filter == LibraryFilter::All, Some(kit::Icon::Library)).clicked() {
                actions.push(RailAction::Select(LibraryFilter::All));
            }
            if rail_row(ui, "Favorites", favorites, *filter == LibraryFilter::Favorites, Some(kit::Icon::Heart)).clicked() {
                actions.push(RailAction::Select(LibraryFilter::Favorites));
            }

            // Collections, with a "+" to start a new one.
            let header = section_header(
                ui,
                "Collections",
                None,
            );
            let plus = egui::Rect::from_center_size(
                egui::pos2(header.rect.right() - 16.0, header.rect.center().y),
                Vec2::splat(22.0),
            );
            let plus_response = ui.interact(plus, header.id.with("add"), egui::Sense::click());
            let color = if plus_response.hovered() { LIME } else { MUTED };
            let stroke = egui::Stroke::new(1.6_f32, color);
            ui.painter().line_segment([plus.center() + Vec2::new(-5.0, 0.0), plus.center() + Vec2::new(5.0, 0.0)], stroke);
            ui.painter().line_segment([plus.center() + Vec2::new(0.0, -5.0), plus.center() + Vec2::new(0.0, 5.0)], stroke);
            if plus_response.on_hover_text("New collection").clicked() {
                actions.push(RailAction::BeginCreate);
            }
            for name in &result.collections {
                let renaming = edit
                    .as_ref()
                    .is_some_and(|edit| edit.original.as_deref() == Some(name.as_str()));
                if renaming {
                    edit_row(ui, edit, &mut actions);
                    continue;
                }
                let count = view.collections.get(name).copied().unwrap_or(0);
                let selected = matches!(filter, LibraryFilter::Collection(open) if open == name);
                let response = rail_row(ui, name, count, selected, None);
                if response.clicked() {
                    actions.push(RailAction::Select(LibraryFilter::Collection(name.clone())));
                }
                response.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        actions.push(RailAction::BeginRename(name.clone()));
                        ui.close();
                    }
                    if ui.button(RichText::new("Delete collection").color(DANGER)).clicked() {
                        actions.push(RailAction::Delete(name.clone()));
                        ui.close();
                    }
                });
            }
            if edit.as_ref().is_some_and(|edit| edit.original.is_none()) {
                edit_row(ui, edit, &mut actions);
            }

            // Games.
            let groups = &view.games;
            section_header(
                ui,
                "Games",
                None,
            );
            for group in groups {
                let selected = matches!(filter, LibraryFilter::Game(open) if open.to_lowercase() == group.name.to_lowercase());
                let look = badge_look(&group.name, badges);
                let row = rail_row_with_badge(
                    ui,
                    &group.name,
                    group.clips,
                    selected,
                    None,
                    Some(look),
                );
                if row.clicked() {
                    actions.push(RailAction::Select(LibraryFilter::Game(group.name.clone())));
                }
                row.on_hover_text("Right-click to change its colour")
                    .context_menu(|ui| badge_menu(ui, &group.name, badges, &mut actions));
            }
            ui.add_space(crate::kit::space::MD);
        });
    actions
}

fn edit_row(ui: &mut egui::Ui, edit: &mut Option<CollectionEdit>, actions: &mut Vec<RailAction>) {
    let Some(state) = edit.as_mut() else {
        return;
    };
    kit::field_frame()
        .stroke(egui::Stroke::new(1.0_f32, LIME))
        .show(ui, |ui| {
            ui.set_min_height(crate::kit::control::FIELD_INNER);
            let field = ui.add(
                egui::TextEdit::singleline(&mut state.text)
                    .frame(false)
                    .hint_text("Collection name")
                    .char_limit(clipcraft_engine::MAX_NAME_CHARS)
                    .desired_width((ui.available_width() - 4.0).max(0.0)),
            );
            crate::kit::name_text_box(&field, "Collection name", &state.text);
            if !state.focused {
                field.request_focus();
                state.focused = true;
            }
            if field.lost_focus() {
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    actions.push(RailAction::Commit);
                } else {
                    actions.push(RailAction::Cancel);
                }
            }
        });
}

#[cfg(test)]
mod avatar_tests {
    use super::name_colour;

    #[test]
    fn a_games_colour_is_stable_readable_and_differs_between_games() {
        let a = name_colour("Counter-Strike 2");
        assert_eq!(a, name_colour("counter-strike 2"), "case does not matter");
        assert_ne!(a, name_colour("Valorant"));
        for name in ["", "x", "Counter-Strike 2", "Valorant", "\u{1f3ae}"] {
            let colour = name_colour(name);
            let brightest = colour.r().max(colour.g()).max(colour.b());
            assert!(brightest > 120, "{name:?} is too dark to read: {colour:?}");
        }
    }
}
