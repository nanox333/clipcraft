//! Browsing the library by favorites, collections, and games.

use clipcraft_engine::{LibraryScanResult, MediaEntry, game_of, games};
use eframe::egui;
use egui::{Color32, RichText, Vec2};

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
}

fn rail_row(
    ui: &mut egui::Ui,
    label: &str,
    count: usize,
    selected: bool,
    icon: Option<kit::Icon>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(0.0), 32.0),
        egui::Sense::click(),
    );
    if selected {
        ui.painter().rect_filled(rect, 8.0, CARD_HOVER);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, CARD);
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
    let count_text = count.to_string();
    let count_width = 8.0 + count_text.len() as f32 * 7.5;
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_owned(),
        egui::FontId::new(13.5, kit::medium()),
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
        egui::FontId::proportional(12.0),
        FAINT,
    );
    response
}

fn section_header(ui: &mut egui::Ui, title: &str) -> egui::Response {
    ui.add_space(12.0);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(0.0), 22.0),
        egui::Sense::click(),
    );
    ui.painter().text(
        egui::pos2(rect.left() + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title.to_uppercase(),
        egui::FontId::new(11.5, kit::semibold()),
        FAINT,
    );
    response
}

/// The list of views down the left of the library.
pub fn rail(
    ui: &mut egui::Ui,
    result: &LibraryScanResult,
    filter: &LibraryFilter,
    edit: &mut Option<CollectionEdit>,
) -> Vec<RailAction> {
    let mut actions = Vec::new();
    let favorites = result.entries.iter().filter(|entry| entry.favorite).count();
    egui::ScrollArea::vertical()
        .id_salt("library_rail")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if rail_row(ui, "All clips", result.entries.len(), *filter == LibraryFilter::All, Some(kit::Icon::Library)).clicked() {
                actions.push(RailAction::Select(LibraryFilter::All));
            }
            if rail_row(ui, "Favorites", favorites, *filter == LibraryFilter::Favorites, Some(kit::Icon::Heart)).clicked() {
                actions.push(RailAction::Select(LibraryFilter::Favorites));
            }

            // Collections, with a "+" to start a new one.
            let header = section_header(ui, "Collections");
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
                let count = result
                    .entries
                    .iter()
                    .filter(|entry| entry.collections.contains(name))
                    .count();
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
            } else if result.collections.is_empty() {
                ui.label(
                    RichText::new("Group clips your way. Press + to make one.")
                        .color(FAINT)
                        .size(12.0),
                );
            }

            // Games.
            let groups = games(&result.entries, &result.directory);
            section_header(ui, "Games");
            if groups.is_empty() {
                ui.label(
                    RichText::new("Games are found from folder and file names.")
                        .color(FAINT)
                        .size(12.0),
                );
            }
            for group in groups {
                let selected = matches!(filter, LibraryFilter::Game(open) if open.to_lowercase() == group.name.to_lowercase());
                if rail_row(ui, &group.name, group.clips, selected, None).clicked() {
                    actions.push(RailAction::Select(LibraryFilter::Game(group.name)));
                }
            }
            ui.add_space(12.0);
        });
    actions
}

fn edit_row(ui: &mut egui::Ui, edit: &mut Option<CollectionEdit>, actions: &mut Vec<RailAction>) {
    let Some(state) = edit.as_mut() else {
        return;
    };
    egui::Frame::new()
        .fill(Color32::from_rgb(13, 13, 15))
        .stroke(egui::Stroke::new(1.0_f32, LIME))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
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
