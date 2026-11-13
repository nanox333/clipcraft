//! The page for one clip: player, trim panel, organizing (favorite, game, collections) and
//! details.

use super::*;

impl ClipCraftApp {
    pub(crate) fn clip_details(&mut self, ui: &mut egui::Ui, entry: &MediaEntry) {
        let mut back = false;
        ui.horizontal(|ui| {
            if kit::labelled_button(ui, Icon::ChevronLeft, "Clips", false, true).clicked() {
                back = true;
            }
            ui.add_space(6.0);
            if kit::labelled_button(ui, Icon::Scissors, "Send to editor", false, !entry.offline)
                .clicked()
            {
                self.send_to_editor(entry.clone());
            }

            ui.add(
                egui::Label::new(
                    RichText::new(&entry.title)
                        .color(TEXT)
                        .size(22.0)
                        .family(kit::semibold()),
                )
                .truncate(),
            );
        });
        if back {
            self.selected = None;
            return;
        }

        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            kit::chip(ui, &entry.extension.to_uppercase());
            if let Some(probe) = entry.probe.as_ref() {
                if let (Some(width), Some(height)) = (probe.width, probe.height) {
                    kit::chip(ui, &format!("{width}×{height}"));
                }
                if let Some(rate) = probe.frame_rate {
                    kit::chip(ui, &format!("{} fps", rate.label()));
                }
                if let Some(codec) = probe.video_codec.as_deref() {
                    kit::chip(ui, codec);
                }
                kit::chip(ui, &format_duration(Some(probe)));
            }
            kit::chip(ui, &format_bytes(entry.size_bytes));
        });
        ui.add_space(6.0);

        let side = 340.0_f32;
        let gap = 22.0_f32;
        let main = (ui.available_width() - side - gap).max(360.0);
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            ui.allocate_ui_with_layout(
                Vec2::new(main, height),
                Layout::top_down(Align::Min),
                |ui| {
                    let mut forget = None;
                    let mut find = None;
                    if let Some(open) = self.playback.as_mut() {
                        open.show(ui);
                        if open.file_missing() {
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                if kit::labelled_button(ui, Icon::Search, "Find file", true, true)
                                    .on_hover_text("Search a folder for this clip")
                                    .clicked()
                                {
                                    find = Some(open.path().to_owned());
                                }
                                if kit::labelled_button(
                                    ui,
                                    Icon::Trash,
                                    "Remove from library",
                                    false,
                                    true,
                                )
                                .on_hover_text("Forget this clip. Nothing on your disk is changed.")
                                .clicked()
                                {
                                    forget = Some(open.path().to_owned());
                                }
                            });
                        }
                    }
                    if let Some(path) = find {
                        let name = entry.title.clone();
                        self.start_find(path, name, entry.size_bytes);
                    }
                    if let Some(path) = forget {
                        self.edit_library(MetaEdit::RemoveFromLibrary { path });
                        // The clip is gone from the list, so go back to it.
                        self.selected = None;
                    }
                },
            );
            ui.allocate_ui_with_layout(
                Vec2::new(side, height),
                Layout::top_down(Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            self.organize_card(ui, entry, side);
                            ui.add_space(12.0);
                            self.details_card(ui, entry, side);
                        });
                },
            );
        });
    }

    /// Favorite, game, and collections for the open clip.
    pub(crate) fn organize_card(&mut self, ui: &mut egui::Ui, entry: &MediaEntry, width: f32) {
        let (directory, collections) = self.library.as_ref().map_or_else(
            || (String::new(), Vec::new()),
            |result| (result.directory.clone(), result.collections.clone()),
        );
        let detected = game_of(entry, &directory);
        // The draft follows the open clip; it is reset when another clip opens.
        if self
            .game_draft
            .as_ref()
            .is_none_or(|(path, _)| *path != entry.path)
        {
            self.game_draft = Some((entry.path.clone(), detected.clone().unwrap_or_default()));
        }
        let mut favorite = None;
        let mut game_change = None;
        let mut collection_change = None;
        egui::Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(egui::CornerRadius::same(14))
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| {
                ui.set_width((width - 42.0).max(0.0));
                ui.label(
                    RichText::new("Organize")
                        .color(TEXT)
                        .size(16.0)
                        .family(kit::semibold()),
                );
                ui.add_space(8.0);
                let label = if entry.favorite {
                    "Favorited"
                } else {
                    "Add to favorites"
                };
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width().max(0.0), 36.0),
                    egui::Sense::click(),
                );
                ui.painter().rect_filled(
                    rect,
                    18.0,
                    if response.hovered() { CARD_HOVER } else { CARD },
                );
                kit::draw_heart(
                    ui.painter(),
                    egui::pos2(rect.left() + 22.0, rect.center().y),
                    if entry.favorite { LIME } else { TEXT },
                    entry.favorite,
                    0.8,
                );
                ui.painter().text(
                    egui::pos2(rect.left() + 42.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::new(14.0, kit::medium()),
                    TEXT,
                );
                if response.clicked() {
                    favorite = Some(!entry.favorite);
                }
                ui.add_space(12.0);
                ui.label(RichText::new("Game").color(MUTED).size(12.5));
                if let Some((_, text)) = self.game_draft.as_mut() {
                    let field = egui::Frame::new()
                        .fill(Color32::from_rgb(13, 13, 15))
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(text)
                                    .frame(false)
                                    .hint_text("Which game is this?")
                                    .char_limit(clipcraft_engine::MAX_NAME_CHARS)
                                    .desired_width(ui.available_width().max(0.0)),
                            )
                        })
                        .inner;
                    kit::name_text_box(&field, "Game", text);
                    if field.lost_focus() {
                        let typed = text.trim().to_owned();
                        if typed != detected.clone().unwrap_or_default() {
                            game_change = Some(typed);
                        }
                    }
                }
                ui.label(
                    RichText::new(if entry.game.is_some() {
                        "Set by you. Clear the box to use the detected game."
                    } else {
                        "Detected from the folder or file name. Type to change it."
                    })
                    .color(FAINT)
                    .size(12.0),
                );
                ui.add_space(12.0);
                ui.label(RichText::new("Collections").color(MUTED).size(12.5));
                ui.add_space(4.0);
                if collections.is_empty() {
                    ui.label(
                        RichText::new("Make a collection with + in the Library sidebar.")
                            .color(FAINT)
                            .size(12.0),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                    for name in &collections {
                        let member = entry.collections.contains(name);
                        if settings::toggle_chip(ui, name, member, true, "Add or remove") {
                            collection_change = Some(name.clone());
                        }
                    }
                });
            });
        if let Some(favorite) = favorite {
            self.edit_library(MetaEdit::SetFavorite {
                path: entry.path.clone(),
                favorite,
            });
        }
        if let Some(typed) = game_change {
            self.edit_library(MetaEdit::SetGame {
                path: entry.path.clone(),
                game: (!typed.is_empty()).then_some(typed),
            });
            // Show what is now in effect, including the guess when the box was cleared.
            self.game_draft = None;
        }
        if let Some(name) = collection_change {
            let add = !entry.collections.contains(&name);
            self.edit_library(if add {
                MetaEdit::AddToCollection {
                    path: entry.path.clone(),
                    name,
                }
            } else {
                MetaEdit::RemoveFromCollection {
                    path: entry.path.clone(),
                    name,
                }
            });
        }
    }

    pub(crate) fn details_card(&mut self, ui: &mut egui::Ui, entry: &MediaEntry, width: f32) {
        egui::Frame::new()
            .fill(PANEL)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(egui::CornerRadius::same(14))
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| {
                ui.set_width((width - 42.0).max(0.0));
                ui.label(
                    RichText::new("Details")
                        .color(TEXT)
                        .size(16.0)
                        .family(kit::semibold()),
                );
                ui.add_space(6.0);
                detail_row(ui, "Source", "Local video");
                detail_row(ui, "Modified", &format_date(entry.modified_unix_seconds));
                if let Some(probe) = entry.probe.as_ref() {
                    detail_row(ui, "Duration", &format_duration(Some(probe)));
                    if let (Some(width), Some(height)) = (probe.width, probe.height) {
                        detail_row(ui, "Resolution", &format!("{width}×{height}"));
                    }
                    if let Some(rate) = probe.frame_rate {
                        detail_row(ui, "Frame rate", &format!("{} fps", rate.label()));
                    }
                    if let Some(codec) = probe.video_codec.as_deref() {
                        detail_row(ui, "Video codec", codec);
                    }
                    detail_row(
                        ui,
                        "Audio",
                        &match probe.audio_tracks {
                            0 => "None".to_owned(),
                            1 => "1 track".to_owned(),
                            count => format!("{count} tracks"),
                        },
                    );
                }
                detail_row(ui, "File size", &format_bytes(entry.size_bytes));

                ui.add_space(4.0);
                let (line, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(line, 0.0, BORDER);
                ui.add_space(6.0);
                ui.label(RichText::new("Location").color(FAINT).size(12.0));
                ui.add(
                    egui::Label::new(RichText::new(&entry.path).color(MUTED).size(12.5))
                        .wrap_mode(egui::TextWrapMode::Wrap),
                );
                ui.add_space(8.0);
                let full = ui.available_width();
                if kit::labelled_button_sized(
                    ui,
                    Icon::Copy,
                    "Copy file",
                    true,
                    !entry.offline,
                    full,
                )
                .on_hover_text("Copy the clip itself, so you can paste it into Discord or a folder")
                .clicked()
                {
                    self.copy_clip_file(ui.ctx(), PathBuf::from(&entry.path));
                }
                if kit::labelled_button_sized(ui, Icon::Copy, "Copy file path", false, true, full)
                    .clicked()
                {
                    self.copy_media_path(ui.ctx(), PathBuf::from(&entry.path));
                }
                if kit::labelled_button_sized(
                    ui,
                    Icon::Folder,
                    open_in_file_manager_label(),
                    false,
                    true,
                    full,
                )
                .clicked()
                {
                    self.reveal_media(PathBuf::from(&entry.path));
                }
                if kit::labelled_button_sized(
                    ui,
                    Icon::Pencil,
                    "Rename clip",
                    false,
                    !entry.offline,
                    full,
                )
                .on_hover_text("F2")
                .clicked()
                {
                    self.ask_to_rename(entry.clone());
                }
                if kit::labelled_button_sized(ui, Icon::Trash, "Delete clip", false, true, full)
                    .clicked()
                {
                    self.ask_to_delete(entry.clone());
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Quick trim and export is available below the player.")
                        .color(MUTED)
                        .size(12.0),
                );
            });
    }
}
