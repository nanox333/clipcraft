//! A clip's card in the library grid, and what its buttons and right-click menu can ask for.

use super::*;

pub(crate) enum ClipCardAction {
    Open(Box<MediaEntry>),
    SendToEditor(Box<MediaEntry>),
    CopyPath(String),
    /// Put the clip's file on the clipboard.
    CopyFile(String),
    Reveal(String),
    SetFavorite {
        path: String,
        favorite: bool,
    },
    SetCollection {
        path: String,
        name: String,
        add: bool,
    },
    /// Forget a clip whose file is gone.
    RemoveFromLibrary(String),
    /// Ask about moving the clip's file to the Recycle Bin.
    Delete(Box<MediaEntry>),
    /// Start dragging the clip out of the window, into a chat.
    DragOut(Box<MediaEntry>),
    /// The clips were put in a new order by dragging: the paths shown, in their new order.
    Reordered(Vec<String>),
    /// The pointer has rested on the clip: get its share copy ready.
    Prepare(Box<MediaEntry>),
    /// Open the rename box for the clip.
    Rename(Box<MediaEntry>),
    /// Look for a clip whose file moved: its path, name and size.
    FindFile(String, String, u64),
}

/// Height of a clip in the list layout, and of its small picture.
pub(crate) const ROW_HEIGHT: f32 = 76.0;
const ROW_PICTURE: f32 = 60.0;

/// One clip: a picture card, or with `row` a line of the list (a small picture, the name and
/// details beside it, the length at the right). Both have the same actions, menu and keys.
pub(crate) fn clip_card(
    ui: &mut egui::Ui,
    entry: &MediaEntry,
    width: f32,
    thumbs: &mut thumbs::Thumbnails,
    game: Option<&str>,
    collections: &[String],
    row: bool,
) -> Option<ClipCardAction> {
    const RADIUS: u8 = crate::kit::radius::LG;
    let thumb_height = if row {
        ROW_PICTURE
    } else {
        (width * 0.5625).clamp(120.0, 240.0)
    };
    let card_height = if row {
        ROW_HEIGHT
    } else {
        thumb_height + CARD_TEXT_HEIGHT
    };
    let (_, card) = ui.allocate_space(Vec2::new(width, card_height));
    let response = ui.interact(
        card,
        library_grid::card_id(entry),
        egui::Sense::click_and_drag(),
    );
    let thumbnail = if row {
        egui::Rect::from_min_size(
            card.min + Vec2::new(8.0, 8.0),
            Vec2::new(ROW_PICTURE * 16.0 / 9.0, ROW_PICTURE),
        )
    } else {
        egui::Rect::from_min_size(card.min, Vec2::new(width, thumb_height))
    };
    // Quick actions: on the picture from its right edge in a card, and in a list at the right
    // of the line. Favorite, folder, editor.
    let button_area = |slot: f32| {
        if row {
            egui::Rect::from_center_size(
                egui::pos2(card.right() - 28.0 - slot * 40.0, card.center().y),
                Vec2::splat(36.0),
            )
        } else {
            egui::Rect::from_center_size(
                egui::pos2(
                    thumbnail.right() - 22.0 - slot * 40.0,
                    thumbnail.top() + 22.0,
                ),
                Vec2::splat(36.0),
            )
        }
    };
    let copy_area = button_area(2.0);
    let reveal_area = button_area(1.0);
    // Added after the card so they sit on top of it and take their own clicks.
    let copy = ui.interact(copy_area, response.id.with("copy"), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Open {}", entry.title),
        )
    });
    copy.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            if entry.offline {
                format!("Find the file for {}", entry.title)
            } else {
                format!("Send {} to editor", entry.title)
            },
        )
    });
    let reveal = ui.interact(
        reveal_area,
        response.id.with("reveal"),
        egui::Sense::click(),
    );
    reveal.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            if entry.offline {
                format!("Remove {} from the library", entry.title)
            } else {
                format!("Show {} in the file manager", entry.title)
            },
        )
    });
    let heart_area = button_area(0.0);
    let heart = ui.interact(heart_area, response.id.with("heart"), egui::Sense::click());
    heart.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            true,
            entry.favorite,
            format!("Favorite {}", entry.title),
        )
    });
    for control in [&response, &copy, &reveal, &heart] {
        if control.clicked() {
            control.request_focus();
        }
    }
    let hovered = response.hovered() || copy.hovered() || reveal.hovered() || heart.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let painter = ui.painter().clone();

    if row && (hovered || response.has_focus()) {
        painter.rect_filled(card, egui::CornerRadius::same(RADIUS), CARD);
    }
    // Picture, then the hover play button over it.
    let visible = ui.is_rect_visible(thumbnail);
    let texture = thumbs.texture(ui.ctx(), &entry.path, visible);
    paint_video_placeholder(ui, thumbnail, entry, texture, RADIUS);
    painter.rect_stroke(
        thumbnail,
        egui::CornerRadius::same(RADIUS),
        Stroke::new(
            if hovered { 2.0_f32 } else { 1.0_f32 },
            if hovered {
                Color32::from_rgba_unmultiplied(190, 255, 43, 190)
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    if hovered && !entry.offline {
        painter.circle_filled(
            thumbnail.center(),
            if row { 16.0 } else { 26.0 },
            Color32::from_black_alpha(165),
        );
        kit::draw_icon(
            &painter,
            thumbnail.center() + Vec2::new(1.0, 0.0),
            Icon::Play,
            LIME,
            if row { 0.7 } else { 1.15 },
        );
    }

    if entry.offline && !row {
        painter.text(
            thumbnail.center() + Vec2::new(0.0, -8.0),
            egui::Align2::CENTER_CENTER,
            "File not found",
            egui::FontId::new(crate::kit::text_size::LEAD, kit::medium()),
            TEXT,
        );
        painter.text(
            thumbnail.center() + Vec2::new(0.0, 14.0),
            egui::Align2::CENTER_CENTER,
            "It was moved, renamed or deleted.",
            egui::FontId::proportional(crate::kit::text_size::CAPTION),
            MUTED,
        );
    }

    // Favorite heart (always shown when set) and the game badge.
    if entry.favorite || hovered {
        painter.circle_filled(heart_area.center(), 16.0, Color32::from_black_alpha(150));
        if entry.favorite {
            kit::draw_heart(&painter, heart_area.center(), LIME, true, 0.85);
        } else {
            kit::draw_heart(
                &painter,
                heart_area.center(),
                if heart.hovered() { LIME } else { TEXT },
                false,
                0.85,
            );
        }
    }
    // The title is the game for a clip ClipCraft named, so a badge saying the same is noise.
    let shown_title = crate::clip_label::title(&entry.title, &entry.extension);
    if let Some(game) = game.filter(|game| *game != shown_title && !row) {
        let galley = painter.layout_no_wrap(
            game.to_owned(),
            egui::FontId::new(crate::kit::text_size::CAPTION, kit::medium()),
            TEXT,
        );
        let badge_width = (galley.size().x + 18.0).min(width - 24.0).max(0.0);
        let badge = egui::Rect::from_min_size(
            egui::pos2(thumbnail.left() + 10.0, thumbnail.bottom() - 32.0),
            Vec2::new(badge_width, 22.0),
        );
        painter.rect_filled(badge, 11.0, Color32::from_black_alpha(170));
        painter.with_clip_rect(badge).galley(
            egui::pos2(badge.left() + 9.0, badge.center().y - galley.size().y / 2.0),
            galley,
            TEXT,
        );
    }

    // Title (one line, ellipsized) and details.
    let text_left = if row {
        thumbnail.right() + 16.0
    } else {
        card.left() + 4.0
    };
    let text_top = if row {
        card.top() + 15.0
    } else {
        thumbnail.bottom() + 10.0
    };
    let mut title = egui::text::LayoutJob::simple_singleline(
        shown_title.clone(),
        egui::FontId::new(crate::kit::text_size::BODY, kit::semibold()),
        TEXT,
    );
    // In a list the buttons take the right of the line.
    title.wrap.max_width = if row {
        (card.right() - 150.0 - text_left).max(60.0)
    } else {
        width - 8.0
    };
    title.wrap.max_rows = 1;
    title.wrap.break_anywhere = true;
    title.wrap.overflow_character = Some('…');
    let title = painter.layout_job(title);
    painter.galley(egui::pos2(text_left, text_top), title, TEXT);
    let mut details = format!(
        "{}  ·  {}",
        crate::clip_label::when(&entry.title, &entry.extension, entry.modified_unix_seconds),
        format_bytes(entry.size_bytes),
    );
    // A list has no badge on the picture, so the game goes in the details.
    if row && let Some(game) = game.filter(|game| *game != shown_title) {
        details = format!("{game}  \u{00b7}  {details}");
    }
    if entry.offline {
        details = format!("File not found  \u{00b7}  {details}");
    }
    let mut details_job = egui::text::LayoutJob::simple_singleline(
        details,
        egui::FontId::proportional(crate::kit::text_size::CAPTION),
        if entry.offline { DANGER } else { MUTED },
    );
    details_job.wrap.max_width = if row {
        (card.right() - 150.0 - text_left).max(60.0)
    } else {
        width - 8.0
    };
    details_job.wrap.max_rows = 1;
    details_job.wrap.break_anywhere = true;
    details_job.wrap.overflow_character = Some('\u{2026}');
    let details = painter.layout_job(details_job);
    painter.galley(
        egui::pos2(
            text_left,
            if row {
                card.top() + 40.0
            } else {
                thumbnail.bottom() + 32.0
            },
        ),
        details,
        MUTED,
    );

    // Round quick actions on the picture, shown while the card is hovered or focused.
    // A clip whose file is gone cannot be shown in the file manager; it can be forgotten.
    let ((first_icon, _), (second_icon, _)) = if entry.offline {
        ((Icon::Search, "Find file"), (Icon::Trash, "Remove"))
    } else {
        (
            (Icon::Scissors, "Send to editor"),
            (Icon::Folder, open_in_file_manager_label()),
        )
    };
    let focused =
        response.has_focus() || copy.has_focus() || reveal.has_focus() || heart.has_focus();
    if hovered || focused {
        for (area, response, icon) in [
            (copy_area, &copy, first_icon),
            (reveal_area, &reveal, second_icon),
        ] {
            let hot = response.hovered();
            painter.circle_filled(
                area.center(),
                16.0,
                if hot {
                    Color32::from_black_alpha(215)
                } else {
                    Color32::from_black_alpha(150)
                },
            );
            let danger = entry.offline && icon == Icon::Trash;
            kit::draw_icon(
                &painter,
                area.center(),
                icon,
                if danger {
                    DANGER
                } else if hot {
                    LIME
                } else {
                    TEXT
                },
                0.72,
            );
        }
        copy.clone().on_hover_text(if entry.offline {
            "Find the file"
        } else {
            "Send to editor"
        });
        reveal.clone().on_hover_text(if entry.offline {
            "Remove from the library"
        } else {
            open_in_file_manager_label()
        });
    }

    let mut menu_action = None;
    response.context_menu(|ui| {
        if !entry.offline && ui.button("Open").clicked() {
            menu_action = Some(ClipCardAction::Open(Box::new(entry.clone())));
            ui.close();
        }
        if !entry.offline && ui.button("Send to editor").clicked() {
            menu_action = Some(ClipCardAction::SendToEditor(Box::new(entry.clone())));
            ui.close();
        }
        if !entry.offline && ui.button(open_in_file_manager_label()).clicked() {
            menu_action = Some(ClipCardAction::Reveal(entry.path.clone()));
            ui.close();
        }
        if !entry.offline && ui.button("Copy file").clicked() {
            menu_action = Some(ClipCardAction::CopyFile(entry.path.clone()));
            ui.close();
        }
        if ui.button("Copy path").clicked() {
            menu_action = Some(ClipCardAction::CopyPath(entry.path.clone()));
            ui.close();
        }
        let label = if entry.favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        };
        if ui.button(label).clicked() {
            menu_action = Some(ClipCardAction::SetFavorite {
                path: entry.path.clone(),
                favorite: !entry.favorite,
            });
            ui.close();
        }
        if entry.offline && ui.button("Find file\u{2026}").clicked() {
            menu_action = Some(ClipCardAction::FindFile(
                entry.path.clone(),
                entry.title.clone(),
                entry.size_bytes,
            ));
            ui.close();
        }
        if entry.offline && ui.button("Remove from library").clicked() {
            menu_action = Some(ClipCardAction::RemoveFromLibrary(entry.path.clone()));
            ui.close();
        }
        if !entry.offline && ui.button("Rename\u{2026}").clicked() {
            menu_action = Some(ClipCardAction::Rename(Box::new(entry.clone())));
            ui.close();
        }
        if !entry.offline {
            ui.separator();
            if ui
                .button(RichText::new("Delete clip…").color(DANGER))
                .clicked()
            {
                menu_action = Some(ClipCardAction::Delete(Box::new(entry.clone())));
                ui.close();
            }
        }
        ui.menu_button("Add to collection", |ui| {
            if collections.is_empty() {
                ui.label(RichText::new("No collections yet").color(MUTED));
            }
            for name in collections {
                let member = entry.collections.contains(name);
                if ui.selectable_label(member, name.as_str()).clicked() {
                    menu_action = Some(ClipCardAction::SetCollection {
                        path: entry.path.clone(),
                        name: name.clone(),
                        add: !member,
                    });
                    ui.close();
                }
            }
        });
    });
    // Keys on a focused card: Enter opens (as a click), F2 renames, F favorites, Delete asks.
    let key = |key| response.has_focus() && ui.input(|input| input.key_pressed(key));
    if menu_action.is_some() {
        menu_action
    } else if key(egui::Key::F2) && !entry.offline {
        Some(ClipCardAction::Rename(Box::new(entry.clone())))
    } else if key(egui::Key::Delete) && !entry.offline {
        Some(ClipCardAction::Delete(Box::new(entry.clone())))
    } else if heart.clicked() || key(egui::Key::F) {
        Some(ClipCardAction::SetFavorite {
            path: entry.path.clone(),
            favorite: !entry.favorite,
        })
    } else if copy.clicked() {
        Some(if entry.offline {
            ClipCardAction::FindFile(entry.path.clone(), entry.title.clone(), entry.size_bytes)
        } else {
            ClipCardAction::SendToEditor(Box::new(entry.clone()))
        })
    } else if reveal.clicked() {
        Some(if entry.offline {
            ClipCardAction::RemoveFromLibrary(entry.path.clone())
        } else {
            ClipCardAction::Reveal(entry.path.clone())
        })
    } else if response.clicked() && !entry.offline {
        Some(ClipCardAction::Open(Box::new(entry.clone())))
    } else if rested_on(ui, &response) && !entry.offline {
        Some(ClipCardAction::Prepare(Box::new(entry.clone())))
    } else {
        None
    }
}

/// True once, when the pointer has rested on a card for a moment: the cue to get its share copy
/// ready while the person decides what to do with it.
fn rested_on(ui: &egui::Ui, response: &egui::Response) -> bool {
    let key = response.id.with("rested");
    let now = ui.input(|input| input.time);
    if !response.hovered() {
        ui.data_mut(|data| data.remove_temp::<(f64, bool)>(key));
        return false;
    }
    let (since, announced) = ui
        .data_mut(|data| data.get_temp::<(f64, bool)>(key))
        .unwrap_or((now, false));
    if announced {
        return false;
    }
    if now - since >= 0.6 {
        ui.data_mut(|data| data.insert_temp(key, (since, true)));
        return true;
    }
    ui.data_mut(|data| data.insert_temp(key, (since, false)));
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(650));
    false
}

pub(crate) fn paint_video_placeholder(
    ui: &egui::Ui,
    rect: egui::Rect,
    entry: &MediaEntry,
    texture: Option<&egui::TextureHandle>,
    radius: u8,
) {
    let palette = [
        (
            Color32::from_rgb(29, 46, 42),
            Color32::from_rgb(108, 166, 103),
        ),
        (
            Color32::from_rgb(47, 38, 34),
            Color32::from_rgb(190, 125, 70),
        ),
        (
            Color32::from_rgb(34, 40, 55),
            Color32::from_rgb(104, 140, 204),
        ),
        (
            Color32::from_rgb(44, 35, 52),
            Color32::from_rgb(170, 115, 196),
        ),
    ];
    let seed = entry.title.bytes().fold(0_u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    let (background, accent) = palette[(seed as usize) % palette.len()];
    let top_corners = egui::CornerRadius::same(radius);
    let painter = ui.painter();
    if let Some(texture) = texture {
        // A real thumbnail, cropped to fill the card without stretching.
        egui::Image::from_texture(egui::load::SizedTexture::from_handle(texture))
            .uv(thumbs::cover_uv(texture.size_vec2(), rect.size()))
            .corner_radius(top_corners)
            .paint_at(ui, rect);
    } else {
        // No picture yet (or none possible): a soft tinted tile with a film-frame mark.
        painter.rect_filled(rect, top_corners, background);
        painter.circle_filled(
            egui::pos2(
                rect.right() - rect.width() * 0.17,
                rect.top() + rect.height() * 0.3,
            ),
            rect.height() * 0.1,
            Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 90),
        );
        let horizon = rect.center().y + rect.height() * 0.18;
        painter.line_segment(
            [
                egui::pos2(rect.left() + 16.0, horizon),
                egui::pos2(rect.right() - 16.0, horizon),
            ],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 30)),
        );
    }
    // A fade at the bottom keeps the duration badge readable on bright pictures.
    kit::bottom_fade(
        painter,
        egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 54.0), rect.max),
        150,
    );

    // Only a format that is not the usual one is worth a badge: MP4 on every card is noise.
    if !entry.extension.eq_ignore_ascii_case("mp4") {
        let badge = egui::Rect::from_min_size(
            rect.left_top() + Vec2::new(10.0, 10.0),
            Vec2::new(44.0, 22.0),
        );
        painter.rect_filled(
            badge,
            egui::CornerRadius::same(crate::kit::radius::SM),
            Color32::from_black_alpha(170),
        );
        painter.text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            entry.extension.to_uppercase(),
            egui::FontId::new(crate::kit::text_size::CAPTION, kit::medium()),
            TEXT,
        );
    }
    let duration = format_duration(entry.probe.as_ref());
    let duration_badge = egui::Rect::from_min_size(
        rect.right_bottom() - Vec2::new(58.0, 32.0),
        Vec2::new(48.0, 22.0),
    );
    painter.rect_filled(
        duration_badge,
        egui::CornerRadius::same(crate::kit::radius::SM),
        Color32::from_black_alpha(200),
    );
    painter.text(
        duration_badge.center(),
        egui::Align2::CENTER_CENTER,
        duration,
        egui::FontId::new(crate::kit::text_size::CAPTION, kit::medium()),
        TEXT,
    );
}
