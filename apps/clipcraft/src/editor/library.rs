//! The editor's Library tab: the clips to build the edit from, as a grid of cards or a list.

use super::*;

/// How the clips are laid out.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum LibraryLayout {
    /// Picture cards, several to a row.
    #[default]
    Grid,
    /// One clip to a row, with its details beside a small picture.
    List,
}

/// Card sizes for the grid, as the least width of a card.
const SCALES: [(&str, f32); 3] = [("S", 108.0), ("M", 148.0), ("L", 210.0)];
/// Height of one row of the list.
const ROW: f32 = 58.0;

impl EditorState {
    /// The search, the layout choice and the clips themselves.
    pub(super) fn library_list(
        &mut self,
        ui: &mut egui::Ui,
        thumbs: &mut crate::thumbs::Thumbnails,
        height: f32,
        media: &Arc<Vec<MediaEntry>>,
        add: &mut Option<MediaEntry>,
        details: &mut Option<MediaEntry>,
    ) {
        let busy = self.busy();
        // Search on the left, how to show the clips on the right.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = crate::kit::space::SM;
            let controls = if self.library_layout == LibraryLayout::Grid {
                // Size choice and the two layouts.
                (SCALES.len() as f32) * 34.0 + 12.0 + 72.0
            } else {
                72.0
            };
            ui.allocate_ui_with_layout(
                Vec2::new(
                    (ui.available_width() - controls - 14.0).max(80.0),
                    crate::kit::control::STANDARD,
                ),
                Layout::top_down(Align::Min),
                |ui| {
                    crate::settings::search_box(ui, &mut self.search, "Search clips");
                },
            );
            if self.library_layout == LibraryLayout::Grid {
                let names: Vec<&str> = SCALES.iter().map(|(name, _)| *name).collect();
                if let Some(index) = sub_tabs(ui, &names, usize::from(self.card_scale)) {
                    self.card_scale = index as u8;
                }
            }
            if let Some(list) = kit::view_switch(ui, self.library_layout == LibraryLayout::List) {
                self.library_layout = if list {
                    LibraryLayout::List
                } else {
                    LibraryLayout::Grid
                };
            }
        });
        ui.add_space(crate::kit::space::MD);
        let filtered = self.browser_search.prepare(media, &self.search);
        let max_height = (height - 174.0).max(30.0);
        match self.library_layout {
            LibraryLayout::Grid => {
                let least = SCALES[usize::from(self.card_scale).min(SCALES.len() - 1)].1;
                // Room for the scrollbar, so the last card of a row is not pushed to the next.
                let usable = (ui.available_width() - 14.0).max(20.0);
                let mut columns = (((usable + 8.0) / least).floor() as usize).clamp(1, 8);
                let width_for = |columns: usize| {
                    ((usable - 8.0 * (columns - 1) as f32) / columns as f32).max(20.0)
                };
                // A card never grows far past its size: another column is added instead.
                while columns < 8 && width_for(columns) > least * 1.35 {
                    columns += 1;
                }
                let card_width = width_for(columns);
                // Thumbnails are 16:9, so wider cards get taller ones.
                let thumb_height = card_width * 9.0 / 16.0;
                let card_height = thumb_height + 36.0;
                egui::ScrollArea::vertical()
                    .id_salt("editor-browser")
                    .max_height(max_height)
                    .show_rows(
                        ui,
                        card_height + 6.0,
                        filtered.len().div_ceil(columns),
                        |ui, rows| {
                            for row in rows {
                                ui.horizontal_top(|ui| {
                                    for entry in filtered
                                        .iter()
                                        .skip(row * columns)
                                        .take(columns)
                                        .filter_map(|&i| media.get(i))
                                    {
                                        grid_card(
                                            ui,
                                            thumbs,
                                            entry,
                                            Vec2::new(card_width, card_height),
                                            thumb_height,
                                            busy,
                                            add,
                                            details,
                                        );
                                    }
                                });
                            }
                        },
                    );
            }
            LibraryLayout::List => {
                egui::ScrollArea::vertical()
                    .id_salt("editor-browser-list")
                    .max_height(max_height)
                    .show_rows(ui, ROW + 4.0, filtered.len(), |ui, rows| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for entry in rows
                            .into_iter()
                            .filter_map(|row| filtered.get(row).and_then(|&i| media.get(i)))
                        {
                            list_row(ui, thumbs, entry, busy, add, details);
                        }
                    });
            }
        }
        if self.importing.is_some() {
            ui.spinner();
            ui.label(RichText::new("Importing clips…").small().color(MUTED));
        } else if filtered.is_empty() {
            ui.add_space(crate::kit::space::LG);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("No clips here yet")
                        .size(crate::kit::text_size::BODY)
                        .family(kit::semibold())
                        .color(kit::TEXT),
                );
                ui.add_space(crate::kit::space::XS);
                ui.label(
                    RichText::new("Import clips, or send them from Clips.")
                        .size(crate::kit::text_size::CAPTION)
                        .color(MUTED),
                );
            });
        }
    }
}

/// Whether a clip can go on the timeline: it is on disk and its length is known.
fn ready(entry: &MediaEntry) -> bool {
    !entry.offline && entry.probe.as_ref().and_then(|p| p.duration).is_some()
}

/// What a click, double click, Enter, drag or right-click on a clip does. One response owns all of
/// them; nested drag or picture responses would compete for the pointer.
fn item_interaction(ui: &mut egui::Ui, rect: egui::Rect, entry: &MediaEntry) -> egui::Response {
    let response = ui.interact(
        rect,
        egui::Id::new(("editor-source", &entry.path)),
        Sense::click_and_drag(),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &entry.title));
    response
}

/// The follow-through once the clip is drawn: focus, adding, dragging out and the menu.
fn finish_item(
    ui: &mut egui::Ui,
    response: egui::Response,
    entry: &MediaEntry,
    busy: bool,
    add: &mut Option<MediaEntry>,
    details: &mut Option<MediaEntry>,
) {
    let can_add = ready(entry);
    if response.clicked() {
        response.request_focus();
    }
    if can_add
        && (response.double_clicked()
            || response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
    {
        *add = Some(entry.clone());
    }
    if response.drag_started() && can_add {
        response.dnd_set_drag_payload(entry.clone());
    }
    response.on_hover_text(&entry.title).context_menu(|ui| {
        if ui.button("Details").clicked() {
            *details = Some(entry.clone());
            ui.close();
        }
        if ui
            .add_enabled(can_add && !busy, egui::Button::new("Add to timeline"))
            .clicked()
        {
            *add = Some(entry.clone());
            ui.close();
        }
    });
}

/// A picture card: the clip's picture with its length, then its name and when it was made.
#[allow(clippy::too_many_arguments)]
fn grid_card(
    ui: &mut egui::Ui,
    thumbs: &mut crate::thumbs::Thumbnails,
    entry: &MediaEntry,
    size: Vec2,
    thumb_height: f32,
    busy: bool,
    add: &mut Option<MediaEntry>,
    details: &mut Option<MediaEntry>,
) {
    ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
        let (card, _) = ui.allocate_exact_size(size, Sense::hover());
        let response = item_interaction(ui, card, entry);
        let rect = egui::Rect::from_min_size(card.min, Vec2::new(size.x, thumb_height));
        paint_thumbnail(ui, thumbs, entry, rect, &response);
        let duration = crate::format_duration(entry.probe.as_ref());
        let badge = egui::Rect::from_min_max(
            rect.right_bottom() - Vec2::new(44.0, 23.0),
            rect.right_bottom() - Vec2::splat(4.0),
        );
        ui.painter()
            .rect_filled(badge, 4, Color32::from_black_alpha(200));
        ui.painter().text(
            badge.center(),
            Align2::CENTER_CENTER,
            duration,
            FontId::new(crate::kit::text_size::CAPTION, kit::medium()),
            TEXT,
        );
        let below = egui::Rect::from_min_max(rect.left_bottom(), card.right_bottom());
        let clip = ui.painter().with_clip_rect(below);
        let mut title_job = egui::text::LayoutJob::simple_singleline(
            crate::clip_label::title(&entry.title, &entry.extension),
            FontId::new(crate::kit::text_size::CAPTION, kit::medium()),
            TEXT,
        );
        title_job.wrap.max_width = size.x;
        title_job.wrap.max_rows = 1;
        title_job.wrap.break_anywhere = true;
        title_job.wrap.overflow_character = Some('\u{2026}');
        let title = ui.painter().layout_job(title_job);
        clip.galley(rect.left_bottom() + Vec2::new(0.0, 5.0), title, TEXT);
        let mut meta_job = egui::text::LayoutJob::simple_singleline(
            format!(
                "{} \u{00b7} {}",
                crate::clip_label::when(
                    &entry.title,
                    &entry.extension,
                    entry.modified_unix_seconds
                ),
                crate::format_bytes(entry.size_bytes)
            ),
            FontId::new(crate::kit::text_size::CAPTION, kit::medium()),
            MUTED,
        );
        meta_job.wrap.max_width = size.x;
        meta_job.wrap.max_rows = 1;
        meta_job.wrap.break_anywhere = true;
        meta_job.wrap.overflow_character = Some('\u{2026}');
        let meta = ui.painter().layout_job(meta_job);
        clip.galley(rect.left_bottom() + Vec2::new(0.0, 22.0), meta, MUTED);
        finish_item(ui, response, entry, busy, add, details);
    });
}

/// The clip's picture in `rect`, with a ring while it is hovered or focused.
fn paint_thumbnail(
    ui: &mut egui::Ui,
    thumbs: &mut crate::thumbs::Thumbnails,
    entry: &MediaEntry,
    rect: egui::Rect,
    response: &egui::Response,
) {
    ui.painter()
        .rect_filled(rect, f32::from(crate::kit::radius::MD), CARD);
    if let Some(texture) = thumbs.texture(ui.ctx(), &entry.path, ui.is_rect_visible(rect)) {
        egui::Image::from_texture(egui::load::SizedTexture::from_handle(texture))
            .corner_radius(crate::kit::radius::MD)
            .paint_at(ui, rect);
    }
    if response.hovered() || response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            8,
            Stroke::new(1.0_f32, if response.has_focus() { LIME } else { MUTED }),
            egui::StrokeKind::Inside,
        );
    }
}

/// A row: a small picture, the name and what is known about the clip, and its length. A round
/// "+" appears on the row under the pointer to put the clip on the timeline.
fn list_row(
    ui: &mut egui::Ui,
    thumbs: &mut crate::thumbs::Thumbnails,
    entry: &MediaEntry,
    busy: bool,
    add: &mut Option<MediaEntry>,
    details: &mut Option<MediaEntry>,
) {
    let (row, _) = ui.allocate_exact_size(
        Vec2::new((ui.available_width() - 14.0).max(40.0), ROW),
        Sense::hover(),
    );
    let response = item_interaction(ui, row, entry);
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(row, f32::from(crate::kit::radius::MD), CARD);
    }
    let thumb = egui::Rect::from_min_size(
        row.min + Vec2::new(4.0, 4.0),
        Vec2::new((ROW - 8.0) * 16.0 / 9.0, ROW - 8.0),
    );
    paint_thumbnail(ui, thumbs, entry, thumb, &response);
    let text_left = thumb.right() + 12.0;
    let can_add = ready(entry);
    // Room on the right for the length and the add button.
    let right_room = 100.0;
    let text_width = (row.right() - text_left - right_room).max(20.0);
    let mut title_job = egui::text::LayoutJob::simple_singleline(
        crate::clip_label::title(&entry.title, &entry.extension),
        FontId::new(crate::kit::text_size::SMALL, kit::semibold()),
        TEXT,
    );
    title_job.wrap.max_width = text_width;
    title_job.wrap.max_rows = 1;
    title_job.wrap.break_anywhere = true;
    title_job.wrap.overflow_character = Some('\u{2026}');
    let title = ui.painter().layout_job(title_job);
    ui.painter()
        .galley(egui::pos2(text_left, row.top() + 11.0), title, TEXT);
    let detail = {
        let mut parts = vec![crate::clip_label::when(
            &entry.title,
            &entry.extension,
            entry.modified_unix_seconds,
        )];
        if let Some((width, height)) = entry
            .probe
            .as_ref()
            .and_then(|probe| probe.width.zip(probe.height))
        {
            parts.push(format!("{width}\u{00d7}{height}"));
        }
        parts.push(crate::format_bytes(entry.size_bytes));
        parts.join(" \u{00b7} ")
    };
    let mut detail_job = egui::text::LayoutJob::simple_singleline(
        detail,
        FontId::new(crate::kit::text_size::CAPTION, kit::medium()),
        MUTED,
    );
    detail_job.wrap.max_width = text_width;
    detail_job.wrap.max_rows = 1;
    detail_job.wrap.break_anywhere = true;
    detail_job.wrap.overflow_character = Some('\u{2026}');
    let detail = ui.painter().layout_job(detail_job);
    ui.painter()
        .galley(egui::pos2(text_left, row.top() + 32.0), detail, MUTED);
    // The length, then the add button (only on the row under the pointer).
    ui.painter().text(
        egui::pos2(row.right() - 52.0, row.center().y),
        Align2::RIGHT_CENTER,
        crate::format_duration(entry.probe.as_ref()),
        FontId::monospace(12.0),
        TEXT,
    );
    if can_add && !busy && (response.hovered() || response.has_focus()) {
        let button = egui::Rect::from_center_size(
            egui::pos2(row.right() - 28.0, row.center().y),
            Vec2::splat(28.0),
        );
        let plus = ui.interact(
            button,
            egui::Id::new(("editor-source-add", &entry.path)),
            Sense::click(),
        );
        ui.painter().circle_filled(
            button.center(),
            14.0,
            if plus.hovered() {
                LIME
            } else {
                kit::CONTROL_HOVER
            },
        );
        let colour = if plus.hovered() { Color32::BLACK } else { TEXT };
        let stroke = Stroke::new(1.8_f32, colour);
        ui.painter().line_segment(
            [
                button.center() + Vec2::new(-5.0, 0.0),
                button.center() + Vec2::new(5.0, 0.0),
            ],
            stroke,
        );
        ui.painter().line_segment(
            [
                button.center() + Vec2::new(0.0, -5.0),
                button.center() + Vec2::new(0.0, 5.0),
            ],
            stroke,
        );
        if plus
            .on_hover_text("Add to the timeline")
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            *add = Some(entry.clone());
        }
    }
    finish_item(ui, response, entry, busy, add, details);
}
