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
            ui.add_space(crate::kit::space::SM);
            if kit::labelled_button(ui, Icon::Scissors, "Send to editor", false, !entry.offline)
                .clicked()
            {
                self.send_to_editor(entry.clone());
            }

            ui.add(
                egui::Label::new(
                    RichText::new(crate::clip_label::title(&entry.title, &entry.extension))
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
            ui.spacing_mut().item_spacing.y = crate::kit::space::SM;
            kit::chip(
                ui,
                &crate::clip_label::when(
                    &entry.title,
                    &entry.extension,
                    entry.modified_unix_seconds,
                ),
            );
            if !entry.extension.eq_ignore_ascii_case("mp4") {
                kit::chip(ui, &entry.extension.to_uppercase());
            }
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
        ui.add_space(crate::kit::space::SM);

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
                            ui.add_space(crate::kit::space::MD);
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
                            self.actions_card(ui, entry, side);
                            ui.add_space(crate::kit::space::MD);
                            self.organize_card(ui, entry, side);
                            ui.add_space(crate::kit::space::MD);
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
        kit::card_frame()
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| {
                ui.set_width((width - 42.0).max(0.0));
                ui.label(
                    RichText::new("Organize")
                        .color(TEXT)
                        .size(crate::kit::text_size::LEAD)
                        .family(kit::semibold()),
                );
                ui.add_space(crate::kit::space::SM);
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
                    egui::FontId::new(crate::kit::text_size::BODY, kit::medium()),
                    TEXT,
                );
                if response.clicked() {
                    favorite = Some(!entry.favorite);
                }
                ui.add_space(crate::kit::space::MD);
                ui.label(
                    RichText::new("Game")
                        .color(MUTED)
                        .size(crate::kit::text_size::CAPTION),
                );
                if let Some((_, text)) = self.game_draft.as_mut() {
                    let field = kit::field_frame()
                        .show(ui, |ui| {
                            ui.set_min_height(crate::kit::control::FIELD_INNER);
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
                    .size(crate::kit::text_size::CAPTION),
                );
                ui.add_space(crate::kit::space::MD);
                ui.label(
                    RichText::new("Collections")
                        .color(MUTED)
                        .size(crate::kit::text_size::CAPTION),
                );
                ui.add_space(crate::kit::space::XS);
                if collections.is_empty() {
                    ui.label(
                        RichText::new("Make a collection with + in the Library sidebar.")
                            .color(FAINT)
                            .size(crate::kit::text_size::CAPTION),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing =
                        Vec2::new(crate::kit::space::SM, crate::kit::space::SM);
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

    /// What a person does with a finished clip, first in the panel: share it, copy it, find it,
    /// rename it or delete it.
    pub(crate) fn actions_card(&mut self, ui: &mut egui::Ui, entry: &MediaEntry, width: f32) {
        kit::card_frame()
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| {
                ui.set_width((width - 42.0).max(0.0));
                let full = ui.available_width();
                let state = self.share_state(entry);
                let megabytes =
                    u16::try_from(self.recording.share_megabytes.clamp(1, 500)).unwrap_or(10);
                match share::share_chip(ui, &state, megabytes, !entry.offline) {
                    share::ShareChip::Drag => self.drag_clip_out(ui.ctx(), entry),
                    share::ShareChip::Press => self.prepare_share(entry, true),
                    share::ShareChip::Hover => self.prepare_share(entry, false),
                    share::ShareChip::Nothing => {}
                }
                // One main action (the lime one above); the rest are quieter, and the less common
                // ones share rows two to a line instead of a tall stack of full-width buttons.
                if kit::labelled_button_sized(
                    ui,
                    Icon::Copy,
                    "Copy file",
                    "",
                    false,
                    !entry.offline,
                    full,
                )
                .on_hover_text("Copy the clip itself, so you can paste it into Discord or a folder")
                .clicked()
                {
                    self.copy_clip_file(ui.ctx(), PathBuf::from(&entry.path));
                }
                let gap = crate::kit::space::SM;
                let half = ((full - gap) / 2.0).max(40.0);
                ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
                ui.horizontal(|ui| {
                    if kit::labelled_button_sized(
                        ui,
                        Icon::Folder,
                        "Open folder",
                        "",
                        false,
                        true,
                        half,
                    )
                    .on_hover_text(open_in_file_manager_label())
                    .clicked()
                    {
                        self.reveal_media(PathBuf::from(&entry.path));
                    }
                    if kit::labelled_button_sized(
                        ui,
                        Icon::Copy,
                        "Copy path",
                        "",
                        false,
                        true,
                        half,
                    )
                    .on_hover_text("Copy the file's location as text")
                    .clicked()
                    {
                        self.copy_media_path(ui.ctx(), PathBuf::from(&entry.path));
                    }
                });
                ui.horizontal(|ui| {
                    if kit::labelled_button_sized(
                        ui,
                        Icon::Pencil,
                        "Rename",
                        "F2",
                        false,
                        !entry.offline,
                        half,
                    )
                    .clicked()
                    {
                        self.ask_to_rename(entry.clone());
                    }
                    if kit::labelled_button_sized(ui, Icon::Trash, "Delete", "", false, true, half)
                        .clicked()
                    {
                        self.ask_to_delete(entry.clone());
                    }
                });
                ui.add_space(crate::kit::space::XS);
                ui.label(
                    RichText::new("Quick trim and export is available below the player.")
                        .color(MUTED)
                        .size(crate::kit::text_size::CAPTION),
                );
            });
    }

    pub(crate) fn details_card(&mut self, ui: &mut egui::Ui, entry: &MediaEntry, width: f32) {
        kit::card_frame()
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| {
                ui.set_width((width - 42.0).max(0.0));
                ui.label(
                    RichText::new("Details")
                        .color(TEXT)
                        .size(crate::kit::text_size::LEAD)
                        .family(kit::semibold()),
                );
                ui.add_space(crate::kit::space::SM);
                detail_row(
                    ui,
                    "Recorded",
                    &crate::clip_label::when(
                        &entry.title,
                        &entry.extension,
                        entry.modified_unix_seconds,
                    ),
                );
                if let Some(probe) = entry.probe.as_ref() {
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
                detail_row(ui, "File", &entry.title);

                ui.add_space(crate::kit::space::XS);
                let (line, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(line, 0.0, BORDER);
                ui.add_space(crate::kit::space::SM);
                ui.label(
                    RichText::new("Location")
                        .color(FAINT)
                        .size(crate::kit::text_size::CAPTION),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(&entry.path)
                            .color(MUTED)
                            .size(crate::kit::text_size::CAPTION),
                    )
                    .wrap_mode(egui::TextWrapMode::Wrap),
                );
                ui.add_space(crate::kit::space::SM);
            });
    }
}
