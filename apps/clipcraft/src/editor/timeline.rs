//! The editor's toolbar, timeline (segments, playhead, text bars, zoom) and range controls.

use super::*;

impl EditorState {
    /// Only the selected layer needs geometry here; the worker composites all text pixels.
    pub(super) fn paint_text_handles(&mut self, ui: &mut egui::Ui, area: egui::Rect) {
        let text = self
            .session
            .texts()
            .iter()
            .find(|text| {
                let (start, end) = self.live_range(text.id, &text.layer);
                self.selected_text == Some(text.id)
                    && self.playhead >= start
                    && self.playhead < end
                    && !text.layer.text.trim().is_empty()
            })
            .cloned();
        let Some(text) = text else {
            self.text_geometry = None;
            return;
        };
        if self.busy() {
            return;
        }
        let id = text.id;
        let mut layer = text.layer;
        if let Some((dragged, x, y)) = self.title_drag
            && dragged == id
        {
            (layer.x, layer.y) = (x, y);
        }
        let (raster_width, raster_height) = self.export_canvas().unwrap_or((1280, 720));
        let aspect = raster_width as f32 / raster_height as f32;
        let (width, height) = if area.width() / area.height().max(1.0) > aspect {
            (area.height() * aspect, area.height())
        } else {
            (area.width(), area.width() / aspect)
        };
        let canvas = egui::Rect::from_center_size(area.center(), Vec2::new(width, height));
        let raster_canvas = [raster_width, raster_height];
        let scale = Vec2::new(width / raster_width as f32, height / raster_height as f32);
        // Position and timing never change the glyph bounds. Keep geometry only, no UI texture.
        let mut key = layer.clone();
        key.start = Tick::ZERO;
        key.end = Tick(1);
        key.x = 0;
        key.y = 0;
        key.fade_in = Fade::None;
        key.fade_out = Fade::None;
        if self
            .text_geometry
            .as_ref()
            .is_none_or(|entry| entry.key != key || entry.canvas != raster_canvas)
        {
            let raster =
                match clipcraft_engine::TextRaster::render(&layer, raster_width, raster_height) {
                    Ok(Some(raster)) => raster,
                    Ok(None) => {
                        self.text_geometry = None;
                        return;
                    }
                    Err(error) => {
                        self.text_geometry = None;
                        self.message = Some((error.to_string(), true));
                        return;
                    }
                };
            self.text_geometry = Some(PreviewTextGeometry {
                key,
                canvas: raster_canvas,
                size: raster.size(),
            });
        }
        let Some(entry) = &self.text_geometry else {
            return;
        };
        let position = clipcraft_engine::text_position(entry.size, raster_canvas, layer.x, layer.y);
        let at = canvas.min + Vec2::new(position[0] as f32 * scale.x, position[1] as f32 * scale.y);
        let size = Vec2::new(
            entry.size[0] as f32 * scale.x,
            entry.size[1] as f32 * scale.y,
        );
        let rect = egui::Rect::from_min_size(at, size).expand(6.0);
        let response = ui
            .interact(rect, egui::Id::new("editor-title-drag"), Sense::drag())
            .on_hover_cursor(egui::CursorIcon::Grab)
            .on_hover_text("Drag to move the text");
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Move text"));
        if response.hovered() || response.dragged() {
            ui.painter().with_clip_rect(canvas).rect_stroke(
                rect,
                4,
                Stroke::new(1.0_f32, LIME),
                egui::StrokeKind::Inside,
            );
        }
        if response.dragged() {
            let delta = response.drag_delta();
            let x = (f32::from(layer.x) + delta.x / width.max(1.0) * 1000.0).clamp(0.0, 1000.0);
            let y = (f32::from(layer.y) + delta.y / height.max(1.0) * 1000.0).clamp(0.0, 1000.0);
            self.title_drag = Some((id, x.round() as u16, y.round() as u16));
        }
        if response.drag_stopped()
            && let Some((_, x, y)) = self.title_drag.take()
        {
            let mut moved = layer.clone();
            (moved.x, moved.y) = (x, y);
            self.edit(EditorAction::UpdateText { id, layer: moved });
        }
    }
    /// A text layer's range, including a move or trim still being dragged on the timeline.
    pub(super) fn live_range(&self, id: u64, layer: &SequenceText) -> (Tick, Tick) {
        match self.text_drag {
            Some((dragged, start, end)) if dragged == id => (start, end),
            _ => (layer.start, layer.end),
        }
    }
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            let enabled = !self.busy();
            if tool(
                ui,
                kit::Icon::Scissors,
                "Split (S)",
                false,
                enabled && self.can_split(),
            )
            .on_hover_text("Cut the selected segment or text layer at the playhead")
            .clicked()
            {
                self.split();
            }
            if let Some(id) = self.selected_text {
                if tool(ui, kit::Icon::Copy, "Duplicate", false, enabled)
                    .on_hover_text("Copy the selected text layer (Ctrl/\u{2318} D)")
                    .clicked()
                {
                    self.duplicate_text(id);
                }
                if tool(ui, kit::Icon::Trash, "Delete", false, enabled)
                    .on_hover_text("Remove the selected text (Del)")
                    .clicked()
                {
                    self.edit(EditorAction::DeleteText { id });
                }
            } else if let Some(id) = self.selected {
                if tool(ui, kit::Icon::Copy, "Duplicate", false, enabled)
                    .on_hover_text("Copy the selected segment (Ctrl/\u{2318} D)")
                    .clicked()
                {
                    self.edit(EditorAction::Duplicate { id });
                }
                if tool(ui, kit::Icon::Trash, "Delete", false, enabled)
                    .on_hover_text("Remove the selected segment (Del)")
                    .clicked()
                {
                    self.edit(EditorAction::Delete { id });
                }
                if let Some(index) = self
                    .session
                    .clips()
                    .iter()
                    .position(|c| c.id == id)
                    .filter(|_| self.session.clips().len() > 1)
                {
                    separator(ui);
                    if tool(
                        ui,
                        kit::Icon::StepBack,
                        "Move left",
                        false,
                        enabled && index > 0,
                    )
                    .clicked()
                    {
                        self.edit(EditorAction::Move {
                            id,
                            index: index.saturating_sub(1),
                        });
                    }
                    if tool(
                        ui,
                        kit::Icon::StepForward,
                        "Move right",
                        false,
                        enabled && index + 1 < self.session.clips().len(),
                    )
                    .clicked()
                    {
                        self.edit(EditorAction::Move {
                            id,
                            index: index + 1,
                        });
                    }
                }
            }
            // Zoom and reset sit at the right edge.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if tool(
                    ui,
                    kit::Icon::Restart,
                    "Reset edit",
                    false,
                    enabled && !self.session.clips().is_empty(),
                )
                .on_hover_text("Clear the timeline")
                .clicked()
                {
                    self.edit(EditorAction::Reset);
                }
                if button(ui, "Fit", self.fit, true)
                    .on_hover_text("Show the whole edit")
                    .clicked()
                {
                    self.fit = true;
                }
                if button(ui, "+", false, true)
                    .on_hover_text("Zoom in")
                    .clicked()
                {
                    self.fit = false;
                    self.zoom = (self.zoom * 1.5).min(160.0);
                }
                if button(ui, "\u{2212}", false, true)
                    .on_hover_text("Zoom out")
                    .clicked()
                {
                    self.fit = false;
                    self.zoom = (self.zoom / 1.5).max(4.0);
                }
            });
        });
        ui.add_space(8.0);
    }
    pub(super) fn timeline(
        &mut self,
        ui: &mut egui::Ui,
        _thumbs: &mut crate::thumbs::Thumbnails,
        lift: f32,
    ) {
        let lift = lift.max(0.0);
        let card_lift = lift * 0.75;
        let total = self.session.duration();
        let width = if self.fit {
            ui.available_width().max(1.0)
        } else {
            ui.available_width()
                .max(total.seconds() as f32 * self.zoom)
                .max(1.0)
        };
        let mut action = None;
        let mut dropped = None;
        let mut seek = None;
        let mut selection = None;
        let mut text_select: Option<Option<u64>> = None;
        let mut text_action = None;
        let mut drag_now = self.text_drag;
        let mut drag_done = false;
        egui::ScrollArea::horizontal()
            .id_salt("edit-timeline-scroll")
            .show(ui, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(width, 142.0 + TEXT_LANE + lift),
                    Sense::click_and_drag(),
                );
                let painter = ui.painter();
                painter.rect_filled(rect, 10, CANVAS);
                painter.line_segment(
                    [
                        egui::pos2(rect.left(), rect.top() + 20.0),
                        egui::pos2(rect.right(), rect.top() + 20.0),
                    ],
                    Stroke::new(1.0_f32, BORDER),
                );
                // Short ticks between the labelled ones.
                for i in 0..=50 {
                    if i % 5 == 0 {
                        continue;
                    }
                    let x = rect.left() + width * i as f32 / 50.0;
                    painter.line_segment(
                        [
                            egui::pos2(x, rect.top() + 14.0),
                            egui::pos2(x, rect.top() + 20.0),
                        ],
                        Stroke::new(1.0_f32, kit::FAINT),
                    );
                }
                let seconds = total.seconds().max(1.0);
                for i in 0..=10 {
                    let x = rect.left() + width * i as f32 / 10.0;
                    painter.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        Stroke::new(0.5_f32, BORDER),
                    );
                    painter.text(
                        egui::pos2(x + 5.0, rect.top() + 10.0),
                        Align2::LEFT_CENTER,
                        stamp(Tick::from_units(
                            (seconds * 1000.0 * i as f64 / 10.0).round() as i64,
                            1000,
                        )),
                        FontId::monospace(10.0),
                        MUTED,
                    );
                }
                let mut offset = Tick::ZERO;
                let mut last_thin_pixel = f32::NEG_INFINITY;
                for (index, clip) in self.session.clips().iter().enumerate() {
                    let x = rect.left() + width * (offset.0 as f64 / total.0.max(1) as f64) as f32;
                    let end = offset + clip.duration();
                    let right = rect.left() + width * (end.0 as f64 / total.0.max(1) as f64) as f32;
                    // Dense fit views aggregate subpixel cuts. Zoom reveals individual cuts;
                    // hit testing the ruler still resolves the exact timeline range.
                    if right - x < 1.0 {
                        if x.floor() != last_thin_pixel {
                            let small = egui::Rect::from_min_size(
                                egui::pos2(x, rect.top() + 28.0 + TEXT_LANE),
                                Vec2::new(1.0, 72.0 + card_lift),
                            );
                            if ui.is_rect_visible(small) {
                                painter.rect_filled(small, 0, CARD_HOVER);
                            }
                            last_thin_pixel = x.floor();
                        }
                        offset = end;
                        continue;
                    }
                    let card = egui::Rect::from_min_max(
                        egui::pos2(x, rect.top() + 28.0 + TEXT_LANE),
                        egui::pos2(
                            right.max(x + 2.0),
                            rect.top() + 100.0 + TEXT_LANE + card_lift,
                        ),
                    )
                    .shrink(0.5);
                    if !ui.is_rect_visible(card) {
                        offset = end;
                        continue;
                    }
                    let r = ui.interact(
                        card,
                        ui.id().with(("edit-segment", clip.id)),
                        Sense::click_and_drag(),
                    );
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            format!("Select segment {}: {}", index + 1, clip.name),
                        )
                    });
                    painter.rect_filled(card, 6, CARD_HOVER);
                    let sample = timeline_window(clip.start, clip.end, card, ui.clip_rect());
                    if let Some((start, end, frames, buckets)) = sample {
                        self.timeline_media.request(
                            ui.ctx(),
                            &clip.input,
                            start,
                            end,
                            frames,
                            buckets,
                        );
                        if let Some(data) =
                            self.timeline_media
                                .get(&clip.input, start, end, frames, buckets)
                        {
                            let cell = (end.0 - start.0) as f64 / f64::from(frames);
                            for frame in &data.frames {
                                let left = source_x(
                                    card,
                                    clip.start,
                                    clip.end,
                                    frame.position.0 as f64 - cell / 2.0,
                                );
                                let right = source_x(
                                    card,
                                    clip.start,
                                    clip.end,
                                    frame.position.0 as f64 + cell / 2.0,
                                );
                                let tile = egui::Rect::from_min_max(
                                    egui::pos2(left, card.top()),
                                    egui::pos2(right, card.bottom()),
                                );
                                if tile.is_positive() {
                                    painter
                                        .with_clip_rect(card.intersect(ui.clip_rect()))
                                        .image(
                                            frame.texture.id(),
                                            tile,
                                            cover_uv(tile.size(), frame.texture.aspect_ratio()),
                                            Color32::WHITE,
                                        );
                                }
                            }
                            painter.rect_filled(
                                card.intersect(ui.clip_rect()),
                                0,
                                Color32::from_black_alpha(55),
                            );
                        } else {
                            let label = if self
                                .timeline_media
                                .error(&clip.input, start, end, frames, buckets)
                                .is_some()
                            {
                                "Timeline unavailable"
                            } else {
                                "Loading timeline…"
                            };
                            painter.with_clip_rect(card).text(
                                card.center(),
                                Align2::CENTER_CENTER,
                                label,
                                FontId::new(10.0, kit::medium()),
                                FAINT,
                            );
                        }
                        if let Some(error) =
                            self.timeline_media
                                .error(&clip.input, start, end, frames, buckets)
                        {
                            r.clone().on_hover_text(error);
                        }
                    }
                    let clipped = painter.with_clip_rect(card);
                    clipped.rect_filled(
                        egui::Rect::from_min_size(card.left_top(), Vec2::new(card.width(), 24.0)),
                        0,
                        Color32::from_black_alpha(145),
                    );
                    clipped.text(
                        card.left_top() + Vec2::new(6.0, 6.0),
                        Align2::LEFT_TOP,
                        format!("{} · {}", index + 1, clip.name),
                        FontId::new(11.5, kit::medium()),
                        TEXT,
                    );
                    clipped.text(
                        card.left_bottom() + Vec2::new(6.0, -5.0),
                        Align2::LEFT_BOTTOM,
                        stamp(clip.duration()),
                        FontId::monospace(10.0),
                        Color32::WHITE,
                    );
                    for (fade, left) in [(clip.look.fade_in, true), (clip.look.fade_out, false)] {
                        if fade == Fade::None || card.width() < 30.0 {
                            continue;
                        }
                        let shade = if fade == Fade::White {
                            Color32::from_white_alpha(150)
                        } else {
                            Color32::from_black_alpha(170)
                        };
                        let (edge, inner) = if left {
                            (card.left(), card.left() + 18.0)
                        } else {
                            (card.right(), card.right() - 18.0)
                        };
                        painter.add(egui::Shape::convex_polygon(
                            vec![
                                egui::pos2(edge, card.top()),
                                egui::pos2(edge, card.bottom()),
                                egui::pos2(inner, card.bottom()),
                            ],
                            shade,
                            Stroke::NONE,
                        ));
                    }
                    if self.selected == Some(clip.id) {
                        painter.rect_stroke(
                            card,
                            6,
                            Stroke::new(2.0_f32, LIME),
                            egui::StrokeKind::Inside,
                        );
                        // Grips at both ends show which segment the controls below belong to.
                        for edge in [card.left() + 5.0, card.right() - 5.0] {
                            painter.rect_filled(
                                egui::Rect::from_center_size(
                                    egui::pos2(edge, card.center().y),
                                    Vec2::new(3.0, 22.0),
                                ),
                                2,
                                LIME,
                            );
                        }
                    }
                    let audio = egui::Rect::from_min_max(
                        egui::pos2(x + 1.0, rect.top() + 108.0 + TEXT_LANE + card_lift),
                        egui::pos2(
                            right.max(x + 2.0) - 1.0,
                            rect.top() + 135.0 + TEXT_LANE + lift,
                        ),
                    );
                    painter.rect_filled(
                        audio,
                        4,
                        if clip.volume_percent == 0 {
                            Color32::from_gray(36)
                        } else {
                            Color32::from_rgb(42, 50, 30)
                        },
                    );
                    let mut audio_label = if clip.volume_percent == 0 {
                        "Muted"
                    } else {
                        "Audio"
                    };
                    let wave_painter = painter.with_clip_rect(audio.intersect(ui.clip_rect()));
                    wave_painter.line_segment(
                        [audio.left_center(), audio.right_center()],
                        Stroke::new(0.5_f32, FAINT),
                    );
                    if let Some((start, end, frames, buckets)) = sample
                        && let Some(data) =
                            self.timeline_media
                                .get(&clip.input, start, end, frames, buckets)
                    {
                        if !data.has_audio {
                            audio_label = "No audio";
                        }
                        let count = data.waveform.len().max(1) as i64;
                        for (i, bucket) in data.waveform.iter().enumerate() {
                            let at = Tick(
                                start.0
                                    + ((i as i128 * 2 + 1) * i128::from(end.0 - start.0)
                                        / (i128::from(count) * 2))
                                        as i64,
                            );
                            let x = source_x(card, clip.start, clip.end, at.0 as f64);
                            let gain = clip.look.audio_gain_at(
                                at - clip.start,
                                clip.duration(),
                                clip.volume_percent,
                            );
                            let height = (audio.height() / 2.0 - 3.0).max(0.0);
                            let low = f32::from(bucket.min) / 32767.0 * gain * height;
                            let high = f32::from(bucket.max) / 32767.0 * gain * height;
                            wave_painter.line_segment(
                                [
                                    egui::pos2(x, audio.center().y - high),
                                    egui::pos2(x, audio.center().y - low),
                                ],
                                Stroke::new(
                                    1.0_f32,
                                    if clip.volume_percent == 0 {
                                        FAINT
                                    } else {
                                        Color32::from_rgb(156, 185, 112)
                                    },
                                ),
                            );
                        }
                    }
                    let label = egui::Rect::from_min_size(
                        audio.left_top(),
                        Vec2::new(audio.width().min(96.0), audio.height()),
                    );
                    wave_painter.rect_filled(
                        label,
                        4,
                        if clip.volume_percent == 0 {
                            Color32::from_gray(28)
                        } else {
                            Color32::from_rgb(30, 35, 23)
                        },
                    );
                    wave_painter.text(
                        audio.left_center() + Vec2::new(5.0, 0.0),
                        Align2::LEFT_CENTER,
                        format!("{audio_label} · {}%", clip.volume_percent),
                        FontId::new(10.0, kit::medium()),
                        MUTED,
                    );
                    if r.clicked() || r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        selection = Some(clip.id);
                        text_select = Some(None);
                        seek = Some(offset);
                    }
                    if r.drag_started() {
                        r.dnd_set_drag_payload(clip.id);
                    }
                    if let Some(id) = r.dnd_release_payload::<u64>() {
                        action = Some(EditorAction::Move { id: *id, index });
                    }
                    if let Some(entry) = r.dnd_release_payload::<MediaEntry>() {
                        dropped = Some((*entry).clone());
                    }
                    offset = end;
                }
                if (response.clicked() || response.dragged())
                    && let Some(point) = response.interact_pointer_pos()
                {
                    let frame = ((point.x - rect.left()) / width * (RATE.frame_at(total) as f32))
                        .round()
                        .max(0.0) as i64;
                    seek = Some(RATE.tick_of(frame));
                    text_select = Some(None);
                    selection = self
                        .session
                        .at(RATE.tick_of(frame))
                        .map(|(clip, _)| clip.id);
                }
                if let Some(entry) = response.dnd_release_payload::<MediaEntry>() {
                    dropped = Some((*entry).clone());
                }
                // The text track sits above the video.
                let lane = egui::Rect::from_min_size(
                    egui::pos2(rect.left(), rect.top() + 24.0),
                    Vec2::new(width, TEXT_LANE - 6.0),
                );
                painter.rect_filled(lane, 4, Color32::from_white_alpha(6));
                if self.session.texts().is_empty() {
                    painter.text(
                        lane.left_center() + Vec2::new(8.0, 0.0),
                        Align2::LEFT_CENTER,
                        "Text track",
                        FontId::new(11.0, kit::medium()),
                        FAINT,
                    );
                }
                let per_second = if total.0 > 0 {
                    width / total.seconds() as f32
                } else {
                    0.0
                };
                for text in self.session.texts() {
                    if per_second <= 0.0 {
                        break;
                    }
                    let (start, end) = match drag_now {
                        Some((id, start, end)) if id == text.id => (start, end),
                        _ => (text.layer.start, text.layer.end),
                    };
                    let block = egui::Rect::from_min_max(
                        egui::pos2(
                            rect.left() + start.seconds() as f32 * per_second,
                            lane.top() + 2.0,
                        ),
                        egui::pos2(
                            (rect.left() + end.seconds() as f32 * per_second)
                                .max(rect.left() + start.seconds() as f32 * per_second + 6.0),
                            lane.bottom() - 2.0,
                        ),
                    );
                    if !ui.is_rect_visible(block) {
                        continue;
                    }
                    let selected = self.selected_text == Some(text.id);
                    painter.rect_filled(
                        block,
                        5,
                        if selected {
                            Color32::from_rgb(86, 74, 150)
                        } else {
                            Color32::from_rgb(62, 54, 110)
                        },
                    );
                    painter.with_clip_rect(block).text(
                        block.left_center() + Vec2::new(8.0, 0.0),
                        Align2::LEFT_CENTER,
                        &text.layer.text,
                        FontId::new(11.0, kit::medium()),
                        TEXT,
                    );
                    for (fade, left) in [(text.layer.fade_in, true), (text.layer.fade_out, false)] {
                        if fade != Fade::None && block.width() > 24.0 {
                            let (edge, inner) = if left {
                                (block.left(), block.left() + 12.0)
                            } else {
                                (block.right(), block.right() - 12.0)
                            };
                            painter.add(egui::Shape::convex_polygon(
                                vec![
                                    egui::pos2(edge, block.top()),
                                    egui::pos2(edge, block.bottom()),
                                    egui::pos2(inner, block.bottom()),
                                ],
                                Color32::from_black_alpha(150),
                                Stroke::NONE,
                            ));
                        }
                    }
                    if selected {
                        painter.rect_stroke(
                            block,
                            5,
                            Stroke::new(2.0_f32, LIME),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let busy = self.busy();
                    let grip = 7.0_f32.min(block.width() / 3.0);
                    let body = ui.interact(
                        block.shrink2(Vec2::new(grip, 0.0)),
                        ui.id().with(("edit-text", text.id)),
                        Sense::click_and_drag(),
                    );
                    body.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            !busy,
                            format!("Text layer: {}", text.layer.text),
                        )
                    });
                    let left_edge = ui.interact(
                        egui::Rect::from_min_size(block.min, Vec2::new(grip, block.height())),
                        ui.id().with(("edit-text-left", text.id)),
                        Sense::click_and_drag(),
                    );
                    let right_edge = ui.interact(
                        egui::Rect::from_min_max(
                            egui::pos2(block.right() - grip, block.top()),
                            block.max,
                        ),
                        ui.id().with(("edit-text-right", text.id)),
                        Sense::click_and_drag(),
                    );
                    let left_edge = left_edge.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                    let right_edge = right_edge.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                    let body = body.on_hover_cursor(egui::CursorIcon::Grab);
                    let shift = |response: &egui::Response| {
                        Tick::from_units(
                            (response.drag_delta().x / per_second * 1000.0).round() as i64,
                            1000,
                        )
                    };
                    let mut now = (start, end);
                    let mut touched = false;
                    if !busy && body.dragged() {
                        let dt = shift(&body);
                        let length = end - start;
                        let moved =
                            (start + dt).clamp(Tick::ZERO, (total - length).max(Tick::ZERO));
                        now = (moved, moved + length);
                        touched = true;
                    } else if !busy && left_edge.dragged() {
                        let moved = (start + shift(&left_edge)).clamp(Tick::ZERO, end - MIN_TEXT);
                        now = (moved, end);
                        touched = true;
                    } else if !busy && right_edge.dragged() {
                        let moved = (end + shift(&right_edge)).clamp(start + MIN_TEXT, total);
                        now = (start, moved);
                        touched = true;
                    }
                    if touched {
                        drag_now = Some((text.id, now.0, now.1));
                    }
                    if body.clicked()
                        || body.drag_started()
                        || left_edge.drag_started()
                        || right_edge.drag_started()
                    {
                        text_select = Some(Some(text.id));
                        if self.playhead < start || self.playhead >= end {
                            seek = Some(start);
                        }
                    }
                    if body.drag_stopped() || left_edge.drag_stopped() || right_edge.drag_stopped()
                    {
                        drag_done = true;
                        if let Some((id, a, b)) = drag_now
                            && id == text.id
                        {
                            let mut layer = text.layer.clone();
                            (layer.start, layer.end) = (a, b);
                            text_action = Some(EditorAction::UpdateText { id, layer });
                        }
                    }
                }
                let x = rect.left()
                    + width
                        * (self.playhead.0 as f64 / total.0.max(1) as f64).clamp(0.0, 1.0) as f32;
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    Stroke::new(2.0_f32, LIME),
                );
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(x - 6.0, rect.top()),
                        egui::pos2(x + 6.0, rect.top()),
                        egui::pos2(x, rect.top() + 10.0),
                    ],
                    LIME,
                    Stroke::NONE,
                ));
                if self.session.clips().is_empty() {
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "Drag clips here, or double-click one in the library to add it",
                        FontId::new(14.0, kit::medium()),
                        MUTED,
                    );
                }
            });
        self.text_drag = if drag_done { None } else { drag_now };
        if let Some(selected) = text_select {
            self.selected_text = selected;
            if selected.is_some() {
                self.selected = None;
            }
        }
        if let Some(action) = text_action {
            self.edit(action);
        }
        if let Some(action) = action {
            self.edit(action);
        }
        if let Some(entry) = dropped {
            self.append(&entry);
        }
        if let Some(id) = selection {
            self.selected = Some(id);
            self.selected_text = None;
        }
        if let Some(time) = seek {
            self.playing = false;
            self.seek(time);
        }
    }
    pub(super) fn range_controls(&mut self, ui: &mut egui::Ui) {
        let Some(clip) = self
            .session
            .clips()
            .iter()
            .find(|c| Some(c.id) == self.selected)
            .cloned()
        else {
            return;
        };
        ui.add_space(6.0);
        let label = if self.segment_settings {
            "Hide segment settings"
        } else {
            "Segment settings: trim and volume"
        };
        if ui
            .add(
                egui::Label::new(RichText::new(label).size(12.5).color(MUTED))
                    .sense(Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            self.segment_settings = !self.segment_settings;
        }
        if !self.segment_settings {
            return;
        }
        ui.add_space(6.0);
        ui.add_enabled_ui(!self.busy(), |ui| {
            egui::Frame::new()
                .fill(CANVAS)
                .corner_radius(10)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                        let mut start = clip.start.seconds();
                        let mut end = clip.end.seconds();
                        let mut volume = clip.volume_percent;
                        ui.label(RichText::new("Source in / out").size(13.0).color(MUTED));
                        let changed = ui
                            .add(
                                egui::DragValue::new(&mut start)
                                    .speed(1.0 / 30.0)
                                    .range(0.0..=clip.source_duration.seconds())
                                    .fixed_decimals(3)
                                    .suffix(" s"),
                            )
                            .on_hover_text("Selected segment source start")
                            .changed()
                            | ui.add(
                                egui::DragValue::new(&mut end)
                                    .speed(1.0 / 30.0)
                                    .range(0.0..=clip.source_duration.seconds())
                                    .fixed_decimals(3)
                                    .suffix(" s"),
                            )
                            .on_hover_text("Selected segment source end")
                            .changed();
                        if changed {
                            self.edit(EditorAction::Trim {
                                id: clip.id,
                                start: Tick::from_units((start * 1000.0).round() as i64, 1000),
                                end: Tick::from_units((end * 1000.0).round() as i64, 1000),
                            });
                        }
                        separator(ui);
                        ui.label(RichText::new("Volume").size(13.0).color(MUTED));
                        if ui
                            .add(egui::DragValue::new(&mut volume).range(0..=100).suffix("%"))
                            .on_hover_text("Selected segment audio volume")
                            .changed()
                        {
                            self.edit(EditorAction::Volume {
                                id: clip.id,
                                percent: volume,
                            });
                        }
                        if button(
                            ui,
                            if clip.volume_percent == 0 {
                                "Unmute"
                            } else {
                                "Mute"
                            },
                            false,
                            true,
                        )
                        .clicked()
                        {
                            self.edit(EditorAction::Volume {
                                id: clip.id,
                                percent: if clip.volume_percent == 0 { 100 } else { 0 },
                            });
                        }
                    });
                });
        });
    }
}
