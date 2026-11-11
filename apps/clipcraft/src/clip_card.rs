//! A clip's card in the library grid, and what its buttons and right-click menu can ask for.

use super::*;

pub(crate) enum ClipCardAction {
    Open(Box<MediaEntry>),
    SendToEditor(Box<MediaEntry>),
    CopyPath(String),
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
    /// Open the rename box for the clip.
    Rename(Box<MediaEntry>),
    /// Look for a clip whose file moved: its path, name and size.
    FindFile(String, String, u64),
}

pub(crate) fn clip_card(
    ui: &mut egui::Ui,
    entry: &MediaEntry,
    width: f32,
    thumbs: &mut thumbs::Thumbnails,
    game: Option<&str>,
    collections: &[String],
) -> Option<ClipCardAction> {
    const RADIUS: u8 = 12;
    const BODY_HEIGHT: f32 = 66.0;
    const FOOTER_HEIGHT: f32 = 42.0;
    let thumb_height = (width * 0.5625).clamp(120.0, 240.0);
    let (_, card) = ui.allocate_space(Vec2::new(width, thumb_height + BODY_HEIGHT + FOOTER_HEIGHT));
    let response = ui.interact(card, library_grid::card_id(entry), egui::Sense::click());
    let thumbnail = egui::Rect::from_min_size(card.min, Vec2::new(width, thumb_height));
    let footer = egui::Rect::from_min_max(
        egui::pos2(card.left(), card.bottom() - FOOTER_HEIGHT),
        card.max,
    );
    let middle = footer.center().x;
    let copy_area = egui::Rect::from_min_max(footer.min, egui::pos2(middle, footer.bottom()));
    let reveal_area = egui::Rect::from_min_max(egui::pos2(middle, footer.top()), footer.max);
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
    let heart_area = egui::Rect::from_center_size(
        egui::pos2(thumbnail.right() - 24.0, thumbnail.top() + 24.0),
        Vec2::splat(36.0),
    );
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
    painter.rect(
        card,
        egui::CornerRadius::same(RADIUS),
        if hovered { CARD_HOVER } else { CARD },
        Stroke::new(
            if hovered { 1.5_f32 } else { 1.0_f32 },
            if hovered {
                Color32::from_rgba_unmultiplied(190, 255, 43, 150)
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );

    // Picture, then the hover play button over it.
    let visible = ui.is_rect_visible(thumbnail);
    let texture = thumbs.texture(ui.ctx(), &entry.path, visible);
    paint_video_placeholder(ui, thumbnail.shrink(1.0), entry, texture, RADIUS - 1);
    if hovered && !entry.offline {
        painter.circle_filled(thumbnail.center(), 26.0, Color32::from_black_alpha(165));
        kit::draw_icon(
            &painter,
            thumbnail.center() + Vec2::new(1.0, 0.0),
            Icon::Play,
            LIME,
            1.15,
        );
    }

    if entry.offline {
        painter.text(
            thumbnail.center() + Vec2::new(0.0, -8.0),
            egui::Align2::CENTER_CENTER,
            "File not found",
            egui::FontId::new(15.0, kit::medium()),
            TEXT,
        );
        painter.text(
            thumbnail.center() + Vec2::new(0.0, 14.0),
            egui::Align2::CENTER_CENTER,
            "It was moved, renamed or deleted.",
            egui::FontId::proportional(12.0),
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
    if let Some(game) = game {
        let galley = painter.layout_no_wrap(
            game.to_owned(),
            egui::FontId::new(12.0, kit::medium()),
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
    let text_left = card.left() + 14.0;
    let mut title = egui::text::LayoutJob::simple_singleline(
        entry.title.clone(),
        egui::FontId::new(15.0, kit::medium()),
        TEXT,
    );
    title.wrap.max_width = width - 28.0;
    title.wrap.max_rows = 1;
    title.wrap.break_anywhere = true;
    title.wrap.overflow_character = Some('…');
    let title = painter.layout_job(title);
    painter.galley(
        egui::pos2(text_left, thumbnail.bottom() + 12.0),
        title,
        TEXT,
    );
    let mut details = format!(
        "{}  ·  {}",
        format_bytes(entry.size_bytes),
        format_date(entry.modified_unix_seconds)
    );
    if entry.offline {
        details = format!("File not found  \u{00b7}  {details}");
    }
    painter.text(
        egui::pos2(text_left, thumbnail.bottom() + 38.0),
        egui::Align2::LEFT_TOP,
        details,
        egui::FontId::proportional(12.5),
        if entry.offline { DANGER } else { MUTED },
    );

    // Footer actions.
    painter.line_segment(
        [footer.left_top(), footer.right_top()],
        Stroke::new(1.0_f32, BORDER),
    );
    let bottom_left = egui::CornerRadius {
        nw: 0,
        ne: 0,
        sw: RADIUS,
        se: 0,
    };
    let bottom_right = egui::CornerRadius {
        nw: 0,
        ne: 0,
        sw: 0,
        se: RADIUS,
    };
    // A clip whose file is gone cannot be shown in the file manager; it can be forgotten.
    let ((first_icon, first_label), (second_icon, second_label)) = if entry.offline {
        ((Icon::Search, "Find file"), (Icon::Trash, "Remove"))
    } else {
        (
            (Icon::Scissors, "Send to editor"),
            (Icon::Folder, open_in_file_manager_label()),
        )
    };
    for (area, rounding, response, icon, label) in [
        (copy_area, bottom_left, &copy, first_icon, first_label),
        (
            reveal_area,
            bottom_right,
            &reveal,
            second_icon,
            second_label,
        ),
    ] {
        let hot = response.hovered();
        if hot {
            painter.rect_filled(
                area.shrink2(Vec2::new(1.0, 1.0)),
                rounding,
                Color32::from_white_alpha(14),
            );
        }
        let danger = entry.offline && icon == Icon::Trash;
        let color = if danger {
            DANGER
        } else if hot {
            LIME
        } else {
            MUTED
        };
        kit::draw_icon(
            &painter,
            egui::pos2(area.left() + 24.0, area.center().y),
            icon,
            color,
            0.72,
        );
        painter.text(
            egui::pos2(area.left() + 42.0, area.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            if danger {
                DANGER
            } else if hot {
                TEXT
            } else {
                MUTED
            },
        );
    }
    painter.line_segment(
        [
            egui::pos2(middle, footer.top() + 10.0),
            egui::pos2(middle, footer.bottom() - 10.0),
        ],
        Stroke::new(1.0_f32, BORDER),
    );

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
    } else {
        None
    }
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
    let top_corners = egui::CornerRadius {
        nw: radius,
        ne: radius,
        sw: 0,
        se: 0,
    };
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

    let badge = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(10.0, 10.0),
        Vec2::new(44.0, 22.0),
    );
    painter.rect_filled(
        badge,
        egui::CornerRadius::same(6),
        Color32::from_black_alpha(170),
    );
    painter.text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        entry.extension.to_uppercase(),
        egui::FontId::new(10.5, kit::medium()),
        TEXT,
    );
    let duration = format_duration(entry.probe.as_ref());
    let duration_badge = egui::Rect::from_min_size(
        rect.right_bottom() - Vec2::new(58.0, 32.0),
        Vec2::new(48.0, 22.0),
    );
    painter.rect_filled(
        duration_badge,
        egui::CornerRadius::same(6),
        Color32::from_black_alpha(200),
    );
    painter.text(
        duration_badge.center(),
        egui::Align2::CENTER_CENTER,
        duration,
        egui::FontId::new(12.0, kit::medium()),
        TEXT,
    );
}
