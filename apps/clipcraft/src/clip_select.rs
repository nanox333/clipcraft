//! Choosing several clips on the Clips page, and what can be done with them together: favorite
//! them, add them to a collection, or move them to the Recycle Bin.
//!
//! Deleting is the careful one. It asks first (how many, how big, which), leaves favorites out
//! unless they are included on purpose, waits ten seconds with an **Undo** before touching any
//! file (closing ClipCraft in that time keeps everything), then moves the files to the Recycle
//! Bin one at a time and says which ones, if any, could not be moved. Nothing is deleted for good.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clipcraft_engine::{EngineError, MediaEntry, MetaEdit, clean_name, format_bytes};
use eframe::egui::{self, RichText};

use crate::capture::notice_with_undo;
use crate::kit::{self, DANGER, MUTED, TEXT};
use crate::{ClipCraftApp, Icon};

/// Most clips that can be moved to the Recycle Bin in one go.
pub(crate) const MAX_TRASH_AT_ONCE: usize = 500;
/// How long deleted clips wait, untouched and out of sight, for an Undo.
pub(crate) const UNDO_SECONDS: f32 = 10.0;

/// The clips chosen so far, and the group action in progress.
#[derive(Default)]
pub(crate) struct Selection {
    /// The page is for choosing clips.
    pub active: bool,
    /// The paths of the chosen clips.
    pub chosen: BTreeSet<String>,
    /// The clip clicked last, where a Shift-click starts from.
    anchor: Option<String>,
    /// The name being typed for a new collection.
    new_collection: String,
    /// The "are you sure" box is open.
    confirm: bool,
    /// Favorites are included in the delete (they are left out unless this is on).
    include_favorites: bool,
    /// Clips waiting out their ten seconds.
    pending: Option<PendingTrash>,
    /// Clips being moved to the Recycle Bin.
    trashing: Option<BulkTrashing>,
}

impl Selection {
    /// The clips that are out of sight while they wait to be moved.
    pub fn hidden(&self) -> BTreeSet<String> {
        self.pending
            .as_ref()
            .map(|pending| pending.paths.iter().cloned().collect())
            .unwrap_or_default()
    }
}

struct PendingTrash {
    paths: Vec<String>,
    since: Instant,
}

enum TrashEvent {
    One(String, Result<(), String>),
    Finished,
}

struct BulkTrashing {
    total: usize,
    moved: Vec<String>,
    failed: Vec<(String, String)>,
    receiver: mpsc::Receiver<TrashEvent>,
}

/// What a delete would do, for the box that asks.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TrashSummary {
    /// The files that would be moved.
    pub paths: Vec<String>,
    /// The first few names, for the box.
    pub names: Vec<String>,
    /// How many more there are than `names`.
    pub more: usize,
    pub bytes: u64,
    /// Favorites among the chosen clips whose files exist.
    pub favorites: usize,
    /// Chosen clips whose file is already gone; there is nothing to move.
    pub missing: usize,
}

/// What deleting `entries` would move. Clips whose file is missing are left out, and so are
/// favorites unless `include_favorites`.
pub(crate) fn summarize(entries: &[&MediaEntry], include_favorites: bool) -> TrashSummary {
    let missing = entries.iter().filter(|entry| entry.offline).count();
    let present: Vec<&&MediaEntry> = entries.iter().filter(|entry| !entry.offline).collect();
    let favorites = present.iter().filter(|entry| entry.favorite).count();
    let moved: Vec<&&&MediaEntry> = present
        .iter()
        .filter(|entry| include_favorites || !entry.favorite)
        .collect();
    TrashSummary {
        paths: moved.iter().map(|entry| entry.path.clone()).collect(),
        names: moved
            .iter()
            .take(5)
            .map(|entry| crate::clip_label::title(&entry.title, &entry.extension))
            .collect(),
        more: moved.len().saturating_sub(5),
        bytes: moved.iter().map(|entry| entry.size_bytes).sum(),
        favorites,
        missing,
    }
}

/// The paths from `anchor` to `target` in the order shown, both included. Without a known anchor
/// it is just the target.
pub(crate) fn range_between(order: &[String], anchor: &str, target: &str) -> Vec<String> {
    let (Some(from), Some(to)) = (
        order.iter().position(|path| path == anchor),
        order.iter().position(|path| path == target),
    ) else {
        return vec![target.to_owned()];
    };
    let (low, high) = (from.min(to), from.max(to));
    order[low..=high].to_vec()
}

/// What a click on Favorite should do: favorite them all, unless they all already are.
pub(crate) fn favorite_all(entries: &[&MediaEntry]) -> bool {
    entries.iter().any(|entry| !entry.favorite)
}

/// The edits that make every one of `entries` a favorite (or not).
pub(crate) fn favorite_edits(entries: &[&MediaEntry], favorite: bool) -> Vec<MetaEdit> {
    entries
        .iter()
        .filter(|entry| entry.favorite != favorite)
        .map(|entry| MetaEdit::SetFavorite {
            path: entry.path.clone(),
            favorite,
        })
        .collect()
}

/// How many of `entries` are in a collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Membership {
    None,
    Some,
    All,
}

pub(crate) fn membership(entries: &[&MediaEntry], collection: &str) -> Membership {
    let inside = entries
        .iter()
        .filter(|entry| entry.collections.iter().any(|name| name == collection))
        .count();
    match inside {
        0 => Membership::None,
        n if n == entries.len() => Membership::All,
        _ => Membership::Some,
    }
}

/// The edits that put every one of `entries` in a collection, or take them all out. Clips that
/// are already as asked are left alone.
pub(crate) fn collection_edits(entries: &[&MediaEntry], name: &str, add: bool) -> Vec<MetaEdit> {
    entries
        .iter()
        .filter(|entry| entry.collections.iter().any(|have| have == name) != add)
        .map(|entry| {
            if add {
                MetaEdit::AddToCollection {
                    path: entry.path.clone(),
                    name: name.to_owned(),
                }
            } else {
                MetaEdit::RemoveFromCollection {
                    path: entry.path.clone(),
                    name: name.to_owned(),
                }
            }
        })
        .collect()
}

impl ClipCraftApp {
    /// The chosen clips that are still in the library, in library order.
    fn chosen_entries(&self) -> Vec<&MediaEntry> {
        self.library.as_ref().map_or_else(Vec::new, |result| {
            result
                .entries
                .iter()
                .filter(|entry| self.selection.chosen.contains(&entry.path))
                .collect()
        })
    }

    /// The paths of the clips on screen, in the order shown.
    fn shown_paths(&self) -> Vec<String> {
        let Some(result) = self.library.as_ref() else {
            return Vec::new();
        };
        self.library_view
            .indices
            .iter()
            .filter_map(|&index| result.entries.get(index))
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// Starts or ends choosing clips.
    pub(crate) fn set_choosing(&mut self, active: bool) {
        self.selection.active = active;
        if !active {
            self.selection.chosen.clear();
            self.selection.anchor = None;
            self.selection.confirm = false;
            self.selection.include_favorites = false;
            self.selection.new_collection.clear();
        }
    }

    /// A clip was clicked while choosing. With `range` (Shift) everything from the last clip
    /// clicked to this one is added.
    pub(crate) fn toggle_selected(&mut self, path: String, range: bool) {
        if let Some(anchor) = self.selection.anchor.clone().filter(|_| range) {
            let shown = self.shown_paths();
            self.selection
                .chosen
                .extend(range_between(&shown, &anchor, &path));
        } else if !self.selection.chosen.remove(&path) {
            self.selection.chosen.insert(path.clone());
        }
        self.selection.anchor = Some(path);
    }

    /// Chooses every clip on screen.
    pub(crate) fn select_all_shown(&mut self) {
        let shown = self.shown_paths();
        self.selection.chosen.extend(shown);
    }

    /// Forgets chosen clips that are no longer in the library (deleted, moved, renamed).
    pub(crate) fn prune_selection(&mut self) {
        let Some(result) = self.library.as_ref() else {
            return;
        };
        if self.selection.chosen.is_empty() {
            return;
        }
        let present: BTreeSet<&str> = result.entries.iter().map(|e| e.path.as_str()).collect();
        self.selection
            .chosen
            .retain(|path| present.contains(path.as_str()));
        if self
            .selection
            .anchor
            .as_ref()
            .is_some_and(|anchor| !present.contains(anchor.as_str()))
        {
            self.selection.anchor = None;
        }
    }

    /// Escape ends choosing, and Ctrl or ⌘ A chooses everything on screen, unless the person is
    /// typing or a box or menu is open.
    pub(crate) fn selection_keys(&mut self, ui: &egui::Ui) {
        if !self.selection.active
            || self.selection.confirm
            || ui.ctx().wants_keyboard_input()
            || egui::Popup::is_any_open(ui.ctx())
        {
            return;
        }
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.set_choosing(false);
        } else if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
            self.select_all_shown();
        }
    }

    /// The bar under the search row while choosing: how many, and what to do with them.
    pub(crate) fn selection_bar(&mut self, ui: &mut egui::Ui) {
        if !self.selection.active {
            return;
        }
        let busy = self.selection.pending.is_some() || self.selection.trashing.is_some();
        let chosen = self.chosen_entries();
        let count = chosen.len();
        let shown = self.shown_paths();
        let hidden_by_view = chosen
            .iter()
            .filter(|entry| !shown.contains(&entry.path))
            .count();
        // With nothing chosen the button reads as it will when something is: Favorite.
        let make_favorite = chosen.is_empty() || favorite_all(&chosen);
        let collections = self
            .library
            .as_ref()
            .map(|result| result.collections.clone())
            .unwrap_or_default();
        let enabled = count > 0 && !busy;

        // What the buttons will do is worked out first, so the clips can be borrowed once.
        let mut favorite_click = false;
        let mut trash_click = false;
        let mut all_click = false;
        let mut collection_edit: Option<(String, bool)> = None;
        let mut new_collection: Option<String> = None;
        let memberships: Vec<(String, Membership)> = collections
            .iter()
            .map(|name| (name.clone(), membership(&chosen, name)))
            .collect();
        let progress = self
            .selection
            .trashing
            .as_ref()
            .map(|t| (t.moved.len() + t.failed.len(), t.total));
        let draft = &mut self.selection.new_collection;
        egui::Frame::new()
            .fill(kit::CARD)
            .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
            .corner_radius(egui::CornerRadius::same(crate::kit::radius::MD))
            .inner_margin(egui::Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = crate::kit::space::SM;
                    let status = if let Some((done, total)) = progress {
                        format!("Moving {done} of {total} to the Recycle Bin\u{2026}")
                    } else if self.selection.pending.is_some() {
                        "Waiting to move to the Recycle Bin\u{2026}".to_owned()
                    } else if count == 0 {
                        "Choose clips".to_owned()
                    } else if hidden_by_view > 0 {
                        format!("{count} selected \u{00b7} {hidden_by_view} not shown")
                    } else {
                        format!("{count} selected")
                    };
                    // The count sits on the same line as the buttons, centred on them.
                    let text = RichText::new(status)
                        .size(crate::kit::text_size::BODY)
                        .family(kit::semibold())
                        .color(TEXT);
                    let width = ui
                        .painter()
                        .layout_no_wrap(
                            text.text().to_owned(),
                            egui::FontId::new(crate::kit::text_size::BODY, kit::semibold()),
                            TEXT,
                        )
                        .size()
                        .x
                        + 4.0;
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, crate::kit::control::STANDARD),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(text);
                        },
                    );
                    ui.add_space(crate::kit::space::SM);
                    favorite_click = kit::labelled_button(
                        ui,
                        Icon::Heart,
                        if make_favorite {
                            "Favorite"
                        } else {
                            "Unfavorite"
                        },
                        false,
                        enabled,
                    )
                    .on_hover_text(if make_favorite {
                        "Make every chosen clip a favorite"
                    } else {
                        "Take the favorite mark off every chosen clip"
                    })
                    .clicked();
                    let collect =
                        kit::labelled_button(ui, Icon::Library, "Collection", false, enabled)
                            .on_hover_text("Add the chosen clips to a collection");
                    if enabled {
                        egui::Popup::menu(&collect)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                                ui.set_min_width(240.0);
                                if memberships.is_empty() {
                                    ui.label(
                                        RichText::new("No collections yet")
                                            .size(crate::kit::text_size::SMALL)
                                            .color(MUTED),
                                    );
                                }
                                for (name, state) in &memberships {
                                    if collection_row(ui, name, *state).clicked() {
                                        // All of them in: take them out. Otherwise put them in.
                                        collection_edit =
                                            Some((name.clone(), *state != Membership::All));
                                    }
                                }
                                ui.separator();
                                let field = ui.add(
                                    egui::TextEdit::singleline(draft)
                                        .hint_text("New collection")
                                        .char_limit(clipcraft_engine::MAX_NAME_CHARS)
                                        .desired_width(f32::INFINITY),
                                );
                                if field.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                                    && let Some(name) = clean_name(draft)
                                {
                                    new_collection = Some(name);
                                }
                            });
                    }
                    trash_click = kit::labelled_button(ui, Icon::Trash, "Delete", false, enabled)
                        .on_hover_text("Move the chosen clips to the Recycle Bin")
                        .clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        all_click =
                            kit::labelled_button(ui, Icon::Library, "Select all", false, !busy)
                                .on_hover_text("Choose every clip shown (Ctrl A)")
                                .clicked();
                    });
                });
            });

        if all_click {
            self.select_all_shown();
        }
        // What each button asked for is worked out while the chosen clips are borrowed, then done.
        let chosen = self.chosen_entries();
        let favorite_edits = favorite_click.then(|| favorite_edits(&chosen, make_favorite));
        let collection_edits_now =
            collection_edit.map(|(name, add)| collection_edits(&chosen, &name, add));
        let new_collection_edits = new_collection.map(|name| {
            let mut edits = Vec::new();
            if !collections.iter().any(|have| have == &name) {
                edits.push(MetaEdit::CreateCollection { name: name.clone() });
            }
            edits.extend(collection_edits(&chosen, &name, true));
            edits
        });
        let trash_summary = trash_click.then(|| summarize(&chosen, true));
        if let Some(edits) = favorite_edits {
            let changed = edits.len();
            self.edit_library_batch(edits);
            self.capture.notice = crate::capture::notice(
                format!(
                    "{changed} clip{} {}",
                    if changed == 1 { "" } else { "s" },
                    if make_favorite {
                        "marked as favorite"
                    } else {
                        "no longer favorites"
                    }
                ),
                false,
                None,
            );
        }
        if let Some(edits) = collection_edits_now {
            self.edit_library_batch(edits);
        }
        if let Some(edits) = new_collection_edits {
            self.edit_library_batch(edits);
            self.selection.new_collection.clear();
        }
        if let Some(summary) = trash_summary {
            self.selection.include_favorites = false;
            if summary.paths.is_empty() {
                self.capture.notice = crate::capture::notice(
                    "Those clips' files are already missing, so there is nothing to delete. \
                     Use Remove from library on a clip instead.",
                    true,
                    None,
                );
            } else if summary.paths.len() > MAX_TRASH_AT_ONCE {
                self.capture.notice = crate::capture::notice(
                    format!(
                        "Choose at most {MAX_TRASH_AT_ONCE} clips to delete at once. \
                         {} are chosen.",
                        summary.paths.len()
                    ),
                    true,
                    None,
                );
            } else {
                self.selection.confirm = true;
            }
        }
    }

    /// The "are you sure" box for deleting the chosen clips. Enter does not confirm.
    pub(crate) fn bulk_trash_dialog(&mut self, context: &egui::Context) {
        if !self.selection.confirm {
            return;
        }
        let entries = self.chosen_entries();
        let include = self.selection.include_favorites;
        let summary = summarize(&entries, include);
        let everything = summarize(&entries, true);
        let mut decision = None;
        let mut include_next = include;
        let modal = egui::Modal::new(egui::Id::new("delete-clips-dialog"))
            .frame(
                egui::Frame::new()
                    .fill(kit::PANEL)
                    .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
                    .corner_radius(egui::CornerRadius::same(crate::kit::radius::LG))
                    .inner_margin(egui::Margin::same(24)),
            )
            .show(context, |ui| {
                ui.set_width(400.0);
                let count = summary.paths.len();
                ui.label(
                    RichText::new(format!(
                        "Delete {count} clip{}?",
                        if count == 1 { "" } else { "s" }
                    ))
                    .color(TEXT)
                    .size(crate::kit::text_size::LEAD)
                    .family(kit::semibold()),
                );
                ui.add_space(crate::kit::space::SM);
                for name in &summary.names {
                    ui.label(
                        RichText::new(name)
                            .color(TEXT)
                            .size(crate::kit::text_size::SMALL),
                    );
                }
                if summary.more > 0 {
                    ui.label(
                        RichText::new(format!("and {} more", summary.more))
                            .color(MUTED)
                            .size(crate::kit::text_size::SMALL),
                    );
                }
                ui.add_space(crate::kit::space::SM);
                ui.label(
                    RichText::new(format!("{} in total", format_bytes(summary.bytes)))
                        .color(MUTED)
                        .size(crate::kit::text_size::CAPTION),
                );
                if everything.favorites > 0 {
                    ui.add_space(crate::kit::space::MD);
                    let line = if include {
                        format!(
                            "{} of these {} favorites.",
                            everything.favorites,
                            if everything.favorites == 1 {
                                "is a favorite"
                            } else {
                                "are"
                            }
                        )
                    } else {
                        format!(
                            "{} favorite{} left out, to keep {} safe.",
                            everything.favorites,
                            if everything.favorites == 1 {
                                " is"
                            } else {
                                "s are"
                            },
                            if everything.favorites == 1 {
                                "it"
                            } else {
                                "them"
                            },
                        )
                    };
                    ui.label(
                        RichText::new(line)
                            .color(if include { DANGER } else { MUTED })
                            .size(crate::kit::text_size::SMALL),
                    );
                    let mut checked = include;
                    if ui
                        .checkbox(
                            &mut checked,
                            format!(
                                "Include the favorite{} too",
                                if everything.favorites == 1 { "" } else { "s" }
                            ),
                        )
                        .changed()
                    {
                        include_next = checked;
                    }
                }
                if everything.missing > 0 {
                    ui.add_space(crate::kit::space::SM);
                    ui.label(
                        RichText::new(format!(
                            "{} clip{} whose file is already missing {} left out.",
                            everything.missing,
                            if everything.missing == 1 { "" } else { "s" },
                            if everything.missing == 1 { "is" } else { "are" },
                        ))
                        .color(MUTED)
                        .size(crate::kit::text_size::SMALL),
                    );
                }
                ui.add_space(crate::kit::space::MD);
                ui.label(
                    RichText::new(format!(
                        "They go to the Recycle Bin, after {} seconds in which you can undo. \
                         You can also put them back from the Recycle Bin later.",
                        UNDO_SECONDS as u32
                    ))
                    .color(MUTED)
                    .size(crate::kit::text_size::SMALL),
                );
                ui.add_space(crate::kit::space::XL);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = crate::kit::space::MD;
                    // Enter does not confirm: the default for a destructive step is to keep.
                    if count > 0
                        && crate::clip_delete::danger_button(
                            ui,
                            &format!(
                                "Move {count} clip{} to the Recycle Bin",
                                if count == 1 { "" } else { "s" }
                            ),
                        )
                        .clicked()
                    {
                        decision = Some(true);
                    }
                    if kit::labelled_button(ui, Icon::Restart, "Keep them", false, true).clicked() {
                        decision = Some(false);
                    }
                });
            });
        self.selection.include_favorites = include_next;
        if modal.should_close() {
            decision = decision.or(Some(false));
        }
        match decision {
            Some(true) => {
                self.selection.confirm = false;
                // They leave the page now and wait: nothing is moved until the time is up.
                self.selection.pending = Some(PendingTrash {
                    paths: summary.paths.clone(),
                    since: Instant::now(),
                });
                self.library_view.invalidate();
                let count = summary.paths.len();
                self.capture.notice = notice_with_undo(format!(
                    "{count} clip{} will move to the Recycle Bin in {} seconds",
                    if count == 1 { "" } else { "s" },
                    UNDO_SECONDS as u32
                ));
            }
            Some(false) => self.selection.confirm = false,
            None => {}
        }
    }

    /// Brings back the clips that were waiting to be deleted.
    pub(crate) fn undo_pending_trash(&mut self) {
        if let Some(pending) = self.selection.pending.take() {
            let count = pending.paths.len();
            self.library_view.invalidate();
            self.capture.notice = crate::capture::notice(
                format!(
                    "Kept {count} clip{}. Nothing was deleted.",
                    if count == 1 { "" } else { "s" }
                ),
                false,
                None,
            );
        }
    }

    /// Moves the waiting clips once their time is up, and reports the outcome of a move.
    pub(crate) fn poll_bulk_trash(&mut self, context: &egui::Context) {
        if let Some(pending) = self.selection.pending.as_ref() {
            if pending.since.elapsed().as_secs_f32() >= UNDO_SECONDS {
                if let Some(pending) = self.selection.pending.take() {
                    self.start_bulk_trash(pending.paths);
                }
            } else {
                context.request_repaint_after(Duration::from_millis(250));
            }
        }
        let Some(trashing) = self.selection.trashing.as_mut() else {
            return;
        };
        let mut finished = false;
        loop {
            match trashing.receiver.try_recv() {
                Ok(TrashEvent::One(path, Ok(()))) => trashing.moved.push(path),
                Ok(TrashEvent::One(path, Err(error))) => trashing.failed.push((path, error)),
                Ok(TrashEvent::Finished) | Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if !finished {
            context.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let Some(done) = self.selection.trashing.take() else {
            return;
        };
        let edits: Vec<MetaEdit> = done
            .moved
            .iter()
            .map(|path| MetaEdit::RemoveFromLibrary { path: path.clone() })
            .collect();
        self.edit_library_batch(edits);
        self.selection
            .chosen
            .retain(|path| !done.moved.contains(path));
        self.scan_library();
        let moved = done.moved.len();
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        self.capture.notice = if done.failed.is_empty() {
            crate::capture::notice(
                format!("Moved {moved} clip{} to the Recycle Bin", plural(moved)),
                false,
                None,
            )
        } else {
            let first = done
                .failed
                .first()
                .map(|(path, error)| format!("{}: {error}", file_name(path)))
                .unwrap_or_default();
            crate::capture::notice(
                format!(
                    "Moved {moved} of {} clips. {} could not be moved ({first}).",
                    done.total,
                    done.failed.len()
                ),
                true,
                None,
            )
        };
        if done.failed.is_empty() {
            self.set_choosing(false);
        }
    }

    /// Moves the clips to the Recycle Bin on a thread, one at a time.
    fn start_bulk_trash(&mut self, paths: Vec<String>) {
        if paths.is_empty() || self.selection.trashing.is_some() {
            return;
        }
        // A clip that is open, or playing, is let go of first.
        if self
            .selected
            .as_ref()
            .is_some_and(|open| paths.contains(&open.path))
        {
            self.selected = None;
        }
        self.playback = None;
        let (sender, receiver) = mpsc::channel();
        let total = paths.len();
        std::thread::spawn(move || {
            for path in paths {
                let result = crate::clip_delete::move_clip_to_trash(PathBuf::from(&path))
                    .map_err(|error: EngineError| error.to_string());
                if sender.send(TrashEvent::One(path, result)).is_err() {
                    return;
                }
            }
            let _ = sender.send(TrashEvent::Finished);
        });
        self.selection.trashing = Some(BulkTrashing {
            total,
            moved: Vec::new(),
            failed: Vec::new(),
            receiver,
        });
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(
        || path.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// A collection in the menu: a check when all the chosen clips are in it, a dash when some are.
fn collection_row(ui: &mut egui::Ui, name: &str, state: Membership) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(200.0), 30.0),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            true,
            state == Membership::All,
            name,
        )
    });
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, egui::Color32::from_white_alpha(12));
    }
    let box_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 16.0, rect.center().y),
        egui::Vec2::splat(16.0),
    );
    ui.painter().rect_stroke(
        box_rect,
        4.0,
        egui::Stroke::new(
            1.3_f32,
            if state == Membership::None {
                MUTED
            } else {
                kit::LIME
            },
        ),
        egui::StrokeKind::Inside,
    );
    match state {
        Membership::All => {
            ui.painter().rect_filled(box_rect, 4.0, kit::LIME);
            kit::draw_icon(
                ui.painter(),
                box_rect.center(),
                Icon::Check,
                egui::Color32::BLACK,
                0.55,
            );
        }
        Membership::Some => {
            ui.painter().line_segment(
                [
                    box_rect.center() - egui::vec2(4.0, 0.0),
                    box_rect.center() + egui::vec2(4.0, 0.0),
                ],
                egui::Stroke::new(2.0_f32, kit::LIME),
            );
        }
        Membership::None => {}
    }
    ui.painter().text(
        egui::pos2(rect.left() + 34.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::new(crate::kit::text_size::SMALL, kit::medium()),
        TEXT,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(name: &str) -> MediaEntry {
        crate::tests::media_entry(name, None)
    }

    fn refs(entries: &[MediaEntry]) -> Vec<&MediaEntry> {
        entries.iter().collect()
    }

    #[test]
    fn a_shift_click_chooses_everything_between_in_either_direction() {
        let order: Vec<String> = (0..6).map(|n| format!("/c/{n}.mp4")).collect();
        assert_eq!(
            range_between(&order, "/c/1.mp4", "/c/3.mp4"),
            ["/c/1.mp4", "/c/2.mp4", "/c/3.mp4"]
        );
        assert_eq!(
            range_between(&order, "/c/4.mp4", "/c/2.mp4"),
            ["/c/2.mp4", "/c/3.mp4", "/c/4.mp4"]
        );
        assert_eq!(range_between(&order, "/c/2.mp4", "/c/2.mp4"), ["/c/2.mp4"]);
        // An anchor that is no longer shown, or a target that is not: just the target.
        assert_eq!(range_between(&order, "/gone.mp4", "/c/2.mp4"), ["/c/2.mp4"]);
        assert_eq!(
            range_between(&order, "/c/1.mp4", "/gone.mp4"),
            ["/gone.mp4"]
        );
        assert_eq!(range_between(&[], "/a", "/b"), ["/b"]);
    }

    #[test]
    fn favorite_goes_on_for_all_unless_all_already_are_and_only_changes_what_needs_it() {
        let mut clips = vec![clip("a"), clip("b"), clip("c")];
        clips[1].favorite = true;
        assert!(favorite_all(&refs(&clips)));
        let edits = favorite_edits(&refs(&clips), true);
        assert_eq!(
            edits.len(),
            2,
            "the one that is already a favorite is left alone"
        );
        for clip in &mut clips {
            clip.favorite = true;
        }
        assert!(!favorite_all(&refs(&clips)));
        assert_eq!(favorite_edits(&refs(&clips), false).len(), 3);
        assert!(favorite_edits(&refs(&clips), true).is_empty());
        assert!(!favorite_all(&[]), "nothing chosen is not a favorite click");
    }

    #[test]
    fn collections_show_all_some_or_none_and_a_click_completes_or_empties_them() {
        let mut clips = vec![clip("a"), clip("b"), clip("c")];
        assert_eq!(membership(&refs(&clips), "Best"), Membership::None);
        clips[0].collections = vec!["Best".into()];
        assert_eq!(membership(&refs(&clips), "Best"), Membership::Some);
        // Some in: adding puts only the other two in.
        assert_eq!(collection_edits(&refs(&clips), "Best", true).len(), 2);
        for clip in &mut clips {
            clip.collections = vec!["Best".into(), "Other".into()];
        }
        assert_eq!(membership(&refs(&clips), "Best"), Membership::All);
        assert!(collection_edits(&refs(&clips), "Best", true).is_empty());
        let edits = collection_edits(&refs(&clips), "Best", false);
        assert_eq!(edits.len(), 3);
        assert!(edits.iter().all(|edit| matches!(
            edit,
            MetaEdit::RemoveFromCollection { name, .. } if name == "Best"
        )));
    }

    #[test]
    fn a_delete_leaves_out_favorites_and_missing_files_and_says_so() {
        let mut clips: Vec<MediaEntry> = (0..8).map(|n| clip(&format!("Clip {n}"))).collect();
        clips[0].favorite = true;
        clips[1].favorite = true;
        clips[2].offline = true;
        for (index, clip) in clips.iter_mut().enumerate() {
            clip.size_bytes = 1_000 * (index as u64 + 1);
        }
        let all = refs(&clips);
        let careful = summarize(&all, false);
        assert_eq!(
            careful.paths.len(),
            5,
            "8 chosen, minus 2 favorites and 1 missing"
        );
        assert_eq!((careful.favorites, careful.missing), (2, 1));
        assert!(careful.paths.iter().all(|path| {
            !path.contains("Clip 0") && !path.contains("Clip 1") && !path.contains("Clip 2")
        }));
        assert_eq!(careful.names.len(), 5);
        assert_eq!(careful.more, 0);
        // The size counts what would be moved, not what is left out.
        assert_eq!(careful.bytes, 4_000 + 5_000 + 6_000 + 7_000 + 8_000);
        let with = summarize(&all, true);
        assert_eq!(with.paths.len(), 7, "favorites come along only when asked");
        assert_eq!(with.more, 2);
        assert_eq!(with.names.len(), 5, "the box lists at most five names");
        // Only missing files chosen: nothing to move.
        let gone = [&clips[2]];
        assert!(summarize(&gone, true).paths.is_empty());
        assert!(summarize(&[], true).paths.is_empty());
    }

    #[test]
    fn the_storage_limit_is_told_to_spare_favorites_collections_and_clips_in_the_editor() {
        let mut app = ClipCraftApp::default();
        let mut plain = clip("Plain");
        let mut favorite = clip("Favorite");
        favorite.favorite = true;
        let mut collected = clip("Collected");
        collected.collections = vec!["Best plays".into()];
        let in_editor = clip("In editor");
        plain.size_bytes = 10;
        app.library = Some(clipcraft_engine::LibraryScanResult {
            directory: "/clips".into(),
            entries: vec![
                plain.clone(),
                favorite.clone(),
                collected.clone(),
                in_editor.clone(),
            ],
            persisted: false,
            collections: vec!["Best plays".into()],
        });
        app.editor.receive(in_editor.clone());
        let protected = app.protected_clip_paths();
        let has = |entry: &MediaEntry| protected.contains(&PathBuf::from(&entry.path));
        assert!(has(&favorite) && has(&collected) && has(&in_editor));
        assert!(!has(&plain), "an ordinary clip stays eligible");
    }
}
