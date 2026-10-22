//! Compact media browser, sequence preview and one linked video/audio timeline.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use clipcraft_engine::{
    Command, CommandOutput, EditorAction, EditorSession, MediaEntry, SequenceRequest,
    SequenceSegment, TrimmedClip, execute,
};
use clipcraft_time::{FrameRate, Tick};
use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, RichText, Sense, Stroke, Vec2};

use crate::kit::{self, BORDER, CANVAS, CARD, CARD_HOVER, DANGER, FAINT, LIME, MUTED, PANEL, TEXT};

const RATE: FrameRate = FrameRate { num: 30, den: 1 };

struct ExportJob {
    receiver: mpsc::Receiver<Result<TrimmedClip, String>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
}
impl Drop for ExportJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub struct EditorState {
    pub open: bool,
    media: Arc<Vec<MediaEntry>>,
    details: Option<MediaEntry>,
    importing: Option<mpsc::Receiver<Result<Imported, String>>>,
    imported_thumbs: Vec<(String, PathBuf)>,
    session: EditorSession,
    selected: Option<u64>,
    playhead: Tick,
    playing: bool,
    preview: Option<crate::playback::Playback>,
    preview_id: Option<u64>,
    seek_pending: bool,
    clock: Option<(Instant, Tick)>,
    search: String,
    title: String,
    zoom: f32,
    fit: bool,
    message: Option<(String, bool)>,
    job: Option<ExportJob>,
    saved: Option<PathBuf>,
}
impl Default for EditorState {
    fn default() -> Self {
        Self {
            open: false,
            media: Arc::default(),
            details: None,
            importing: None,
            imported_thumbs: Vec::new(),
            session: EditorSession::default(),
            selected: None,
            playhead: Tick::ZERO,
            playing: false,
            preview: None,
            preview_id: None,
            seek_pending: true,
            clock: None,
            search: String::new(),
            title: "Untitled edit".into(),
            zoom: 32.0,
            fit: true,
            message: None,
            job: None,
            saved: None,
        }
    }
}
struct Imported {
    entries: Vec<MediaEntry>,
    thumbnails: Vec<(String, PathBuf)>,
}

impl EditorState {
    pub fn receive(&mut self, entry: MediaEntry) {
        if let Some(old) = Arc::make_mut(&mut self.media)
            .iter_mut()
            .find(|old| old.path == entry.path)
        {
            *old = entry;
        } else if self.media.len() < clipcraft_engine::MAX_LIBRARY_RESULTS {
            Arc::make_mut(&mut self.media).push(entry);
        } else {
            self.message = Some((
                "The editor media library is full. Import a smaller selection.".into(),
                true,
            ));
        }
    }
    fn import(&mut self, paths: Option<Vec<PathBuf>>, context: &egui::Context) {
        if self.importing.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        let wake = context.clone();
        let spawned = std::thread::Builder::new()
            .name("editor-import".into())
            .spawn(move || {
                let result = (|| {
                    let paths = paths.unwrap_or_else(|| {
                        rfd::FileDialog::new()
                            .set_title("Import clips to editor")
                            .add_filter("Videos", &["mp4", "mov", "mkv"])
                            .pick_files()
                            .unwrap_or_default()
                    });
                    let CommandOutput::EditorMedia(entries) =
                        execute(Command::ImportEditorMedia { paths }).map_err(|e| e.to_string())?
                    else {
                        return Err("Import returned an unexpected result.".to_owned());
                    };
                    let mut thumbnails = Vec::new();
                    if let Some(cache_dir) = clipcraft_engine::default_thumbnail_dir() {
                        for entry in &entries {
                            if let Ok(CommandOutput::Thumbnail(file)) =
                                execute(Command::EnsureThumbnail {
                                    path: entry.path.clone().into(),
                                    cache_dir: cache_dir.clone(),
                                })
                            {
                                thumbnails.push((entry.path.clone(), PathBuf::from(file.file)));
                            }
                        }
                    }
                    Ok(Imported {
                        entries,
                        thumbnails,
                    })
                })();
                let _ = sender.send(result);
                wake.request_repaint();
            });
        if spawned.is_err() {
            self.message = Some(("Import could not start. Try again.".into(), true));
            return;
        }
        self.importing = Some(receiver);
    }
    pub fn close(&mut self) {
        self.open = false;
        self.playing = false;
        self.preview = None;
        self.preview_id = None;
        self.clock = None;
    }
    pub fn take_saved(&mut self) -> Option<PathBuf> {
        self.saved.take()
    }
    pub fn has_clips(&self) -> bool {
        !self.session.clips().is_empty()
    }
    fn busy(&self) -> bool {
        self.job.is_some()
    }
    fn edit(&mut self, action: EditorAction) {
        if self.busy() {
            return;
        }
        let split = matches!(&action, EditorAction::Split { .. });
        match execute(Command::EditSequence {
            session: Box::new(self.session.clone()),
            action,
        }) {
            Ok(CommandOutput::EditorSession(state)) => {
                self.session = *state;
                if split {
                    self.selected = self.session.at(self.playhead).map(|(clip, _)| clip.id);
                }
                if self
                    .selected
                    .is_none_or(|id| !self.session.clips().iter().any(|c| c.id == id))
                {
                    self.selected = self.session.clips().first().map(|c| c.id);
                }
                self.playhead = self.playhead.clamp(Tick::ZERO, self.session.duration());
                self.playing = false;
                self.seek_pending = true;
                self.clock = None;
                self.message = None;
            }
            Ok(_) => self.message = Some(("The edit returned an unexpected result.".into(), true)),
            Err(error) => self.message = Some((error.to_string(), true)),
        }
    }
    pub fn append(&mut self, entry: &MediaEntry) {
        let Some(duration) = entry.probe.as_ref().and_then(|p| p.duration) else {
            self.message = Some((
                "Wait for clip details to load before adding this video.".into(),
                true,
            ));
            return;
        };
        let offset = self.session.duration();
        let count = self.session.clips().len();
        self.edit(EditorAction::Append {
            input: PathBuf::from(&entry.path),
            name: entry.title.clone(),
            duration,
        });
        if self.session.clips().len() > count {
            self.selected = self.session.clips().last().map(|c| c.id);
            self.playhead = offset;
        }
    }
    fn seek(&mut self, time: Tick) {
        self.playhead = time.clamp(Tick::ZERO, self.session.duration());
        self.seek_pending = true;
        self.clock = None;
    }
    fn toggle(&mut self) {
        if !self.has_clips() {
            return;
        }
        if self.playhead >= self.session.duration() {
            self.seek(Tick::ZERO);
        }
        self.playing = !self.playing;
        self.clock = None;
    }
    fn poll_preview(&mut self, context: &egui::Context) {
        if !self.open {
            return;
        }
        if self.playing
            && let Some((at, start)) = self.clock
        {
            let nanos = at.elapsed().as_nanos().min(i64::MAX as u128) as i64;
            let time = start + Tick::from_units(nanos, 1_000_000_000);
            if let Some((clip, source)) = self.session.at(self.playhead) {
                let offset = self.session.offset_of(clip.id).unwrap_or(Tick::ZERO);
                self.playhead = self
                    .preview
                    .as_ref()
                    .and_then(|p| p.editor_audio_clock())
                    .filter(|audio| *audio >= source)
                    .map(|audio| offset + (audio - clip.start))
                    .unwrap_or(time)
                    .min(self.session.duration());
            }
            if self.playhead >= self.session.duration() {
                self.playing = false;
                self.clock = None;
            }
        }
        let Some((clip, source_time)) = self
            .session
            .at(self.playhead)
            .map(|(clip, time)| (clip.clone(), time))
        else {
            self.preview = None;
            self.preview_id = None;
            return;
        };
        if self.preview_id != Some(clip.id) {
            self.preview = Some(crate::playback::Playback::start(
                &clip.input.to_string_lossy(),
                context,
                crate::playback::Prefs {
                    volume: f32::from(clip.volume_percent) / 100.0,
                    muted: clip.volume_percent == 0,
                    looping: false,
                },
            ));
            self.preview_id = Some(clip.id);
            self.seek_pending = true;
            self.clock = None;
        }
        if let Some(preview) = self.preview.as_mut() {
            preview.poll(context);
            if let Some(error) = preview.editor_error() {
                self.message = Some((error.to_owned(), true));
                self.playing = false;
                self.clock = None;
            }
            let ready = preview.editor_ready();
            let seek = self.seek_pending.then_some(source_time);
            preview.editor_transport(seek, self.playing && ready, clip.volume_percent);
            if ready {
                self.seek_pending = false;
                if self.playing && self.clock.is_none() {
                    self.clock = Some((Instant::now(), self.playhead));
                }
            }
        }
        if self.playing {
            context.request_repaint_after(Duration::from_millis(16));
        }
    }
    pub fn poll(&mut self, context: &egui::Context) {
        if self.open {
            let paths: Vec<_> = context.input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .filter_map(|f| f.path.clone())
                    .collect()
            });
            if !paths.is_empty() {
                self.import(Some(paths), context);
            } else if self.importing.is_none()
                && self.media.iter().any(|e| e.probe.is_none() && !e.offline)
            {
                self.import(
                    Some(
                        self.media
                            .iter()
                            .filter(|e| e.probe.is_none() && !e.offline)
                            .map(|e| PathBuf::from(&e.path))
                            .collect(),
                    ),
                    context,
                );
            }
        }
        let imported = match self.importing.as_ref().map(|receiver| receiver.try_recv()) {
            Some(Ok(result)) => Some(result),
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                Some(Err("Import stopped unexpectedly. Try again.".into()))
            }
            _ => None,
        };
        if let Some(result) = imported {
            self.importing = None;
            match result {
                Ok(imported) => {
                    for entry in imported.entries {
                        self.receive(entry);
                    }
                    self.imported_thumbs.extend(imported.thumbnails);
                }
                Err(error) => {
                    self.message = Some((error, true));
                    for entry in Arc::make_mut(&mut self.media)
                        .iter_mut()
                        .filter(|e| e.probe.is_none())
                    {
                        entry.offline = true;
                    }
                }
            }
        }
        self.poll_preview(context);
        let result = match self.job.as_ref().map(|j| j.receiver.try_recv()) {
            Some(Ok(result)) => Some(result),
            Some(Err(mpsc::TryRecvError::Disconnected)) => Some(Err(
                "Export stopped unexpectedly. The sources are unchanged.".into(),
            )),
            _ => None,
        };
        if let Some(result) = result {
            self.job = None;
            match result {
                Ok(report) => {
                    self.saved = Some(report.path.into());
                    self.message = Some((
                        format!(
                            "Edit saved · {}",
                            clipcraft_engine::format_bytes(report.bytes)
                        ),
                        false,
                    ));
                }
                Err(error) => self.message = Some((error, true)),
            }
        }
    }
    fn export(&mut self, context: &egui::Context) {
        if self.busy() {
            return;
        }
        let Some(first) = self.session.clips().first() else {
            return;
        };
        let name: String = self
            .title
            .trim()
            .chars()
            .map(|c| {
                if c.is_control() || "<>:\"/\\|?*".contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let base = if name.is_empty() || name == "Untitled edit" {
            first.input.clone()
        } else {
            first.input.with_file_name(format!("{name}.mp4"))
        };
        let Some(output) = clipcraft_engine::edited_path(&base) else {
            self.message = Some((
                "No unused output name is available beside the first clip.".into(),
                true,
            ));
            return;
        };
        let request = SequenceRequest {
            output,
            segments: self
                .session
                .clips()
                .iter()
                .map(|c| SequenceSegment {
                    input: c.input.clone(),
                    start: c.start,
                    end: c.end,
                    volume_percent: c.volume_percent,
                })
                .collect(),
        };
        self.playing = false;
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(1);
        let (job_progress, job_cancel) = (progress.clone(), cancel.clone());
        let wake = context.clone();
        let spawned = std::thread::Builder::new()
            .name("sequence-export".into())
            .spawn(move || {
                let mut report = |fraction: f32| {
                    job_progress.store(fraction.to_bits(), Ordering::Relaxed);
                    wake.request_repaint();
                };
                let result = clipcraft_engine::export_sequence(&request, &mut report, &job_cancel)
                    .map_err(|e| e.to_string());
                let _ = sender.send(result);
                wake.request_repaint();
            });
        if spawned.is_err() {
            self.message = Some(("Export could not start. Try again.".into(), true));
            return;
        }
        self.message = None;
        self.job = Some(ExportJob {
            receiver,
            cancel,
            progress,
        });
    }
    fn keys(&mut self, ui: &egui::Ui) {
        let typing = ui
            .ctx()
            .memory(|m| m.focused())
            .is_some_and(|id| egui::TextEdit::load_state(ui.ctx(), id).is_some());
        if typing || egui::Popup::is_any_open(ui.ctx()) || self.busy() {
            return;
        }
        let pressed = |modifiers, key| ui.input_mut(|i| i.consume_key(modifiers, key));
        let none = egui::Modifiers::NONE;
        let command = egui::Modifiers::COMMAND;
        if pressed(none, egui::Key::Space) {
            self.toggle();
        }
        if pressed(none, egui::Key::S) {
            self.edit(EditorAction::Split { at: self.playhead });
        }
        if pressed(
            egui::Modifiers {
                shift: true,
                ..command
            },
            egui::Key::Z,
        ) || pressed(command, egui::Key::Y)
        {
            self.edit(EditorAction::Redo);
        } else if pressed(command, egui::Key::Z) {
            self.edit(EditorAction::Undo);
        }
        if let Some(id) = self.selected {
            if pressed(none, egui::Key::Delete) {
                self.edit(EditorAction::Delete { id });
            }
            if pressed(command, egui::Key::D) {
                self.edit(EditorAction::Duplicate { id });
            }
        }
        if pressed(none, egui::Key::ArrowLeft) {
            self.playing = false;
            self.seek(self.playhead - RATE.frame_duration());
        }
        if pressed(none, egui::Key::ArrowRight) {
            self.playing = false;
            self.seek(self.playhead + RATE.frame_duration());
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, thumbs: &mut crate::thumbs::Thumbnails) {
        for (path, file) in std::mem::take(&mut self.imported_thumbs) {
            thumbs.set_file(path, file);
        }
        self.keys(ui);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Editor").size(26.0).family(kit::semibold()));
            ui.label(
                RichText::new("Arrange clips. Keep the best moments.")
                    .size(13.0)
                    .color(MUTED),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(
                    ui,
                    "Export video",
                    true,
                    !self.session.clips().is_empty() && !self.busy(),
                )
                .on_hover_text(
                    "H.264 MP4, up to 720p/30 fps, stereo AAC. Saved beside the first source.",
                )
                .clicked()
                {
                    self.export(ui.ctx());
                }
                if button(ui, "Redo", false, self.session.can_redo() && !self.busy()).clicked() {
                    self.edit(EditorAction::Redo);
                }
                if button(ui, "Undo", false, self.session.can_undo() && !self.busy()).clicked() {
                    self.edit(EditorAction::Undo);
                }
            });
        });
        ui.add_space(10.0);
        let status_height = if self.job.is_some() {
            40.0
        } else if self.message.is_some() {
            26.0
        } else {
            0.0
        };
        let top = (ui.available_height() - 306.0 - status_height).max(120.0);
        let width = ui.available_width().max(1.0);
        let browser = if width >= 700.0 {
            260.0
        } else {
            (width * 0.32).max(80.0)
        };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.allocate_ui_with_layout(
                Vec2::new(browser, top),
                Layout::top_down(Align::Min),
                |ui| {
                    self.browser(ui, thumbs, top);
                },
            );
            let preview_width = (width - browser - 12.0).max(1.0);
            ui.allocate_ui_with_layout(
                Vec2::new(preview_width, top),
                Layout::top_down(Align::Min),
                |ui| {
                    egui::Frame::new()
                        .fill(PANEL)
                        .corner_radius(12)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_width((preview_width - 24.0).max(1.0));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.title)
                                    .char_limit(100)
                                    .font(FontId::new(18.0, kit::semibold()))
                                    .horizontal_align(Align::Center)
                                    .frame(false)
                                    .desired_width(ui.available_width()),
                            )
                            .widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::TextEdit,
                                    true,
                                    "Edit title",
                                )
                            });
                            let size = Vec2::new(ui.available_width(), (top - 106.0).max(40.0));
                            if let Some(preview) = self.preview.as_mut() {
                                preview.editor_picture(ui, size);
                            } else {
                                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                                ui.painter().rect_filled(rect, 12, Color32::BLACK);
                                ui.painter().text(
                                    rect.center(),
                                    Align2::CENTER_CENTER,
                                    "Add clips to begin",
                                    FontId::new(16.0, kit::medium()),
                                    MUTED,
                                );
                            }
                            ui.horizontal(|ui| {
                                if button(
                                    ui,
                                    if self.playing {
                                        "Pause edit"
                                    } else {
                                        "Play edit"
                                    },
                                    true,
                                    !self.session.clips().is_empty(),
                                )
                                .clicked()
                                {
                                    self.toggle();
                                }
                                if button(ui, "Start", false, !self.session.clips().is_empty())
                                    .clicked()
                                {
                                    self.seek(Tick::ZERO);
                                }
                                ui.label(
                                    RichText::new(format!(
                                        "{} / {}",
                                        stamp(self.playhead),
                                        stamp(self.session.duration())
                                    ))
                                    .color(MUTED)
                                    .monospace(),
                                );
                            });
                            if let Some(note) =
                                self.preview.as_ref().and_then(|p| p.editor_audio_note())
                            {
                                ui.label(RichText::new(note).small().color(MUTED));
                            }
                        });
                },
            );
        });
        ui.add_space(12.0);
        egui::Frame::new()
            .fill(PANEL)
            .corner_radius(12)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width((width - 24.0).max(1.0));
                self.toolbar(ui);
                self.timeline(ui, thumbs);
                self.range_controls(ui);
            });
        if let Some(job) = &self.job {
            ui.horizontal(|ui| {
                ui.add(
                    egui::ProgressBar::new(f32::from_bits(job.progress.load(Ordering::Relaxed)))
                        .desired_width(240.0)
                        .show_percentage(),
                );
                if button(ui, "Cancel export", false, true).clicked() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            });
        }
        if let Some((text, error)) = &self.message {
            ui.label(RichText::new(text).color(if *error { DANGER } else { LIME }));
        }
        ui.label(RichText::new("Edits stay in this session. Export saves an MP4 beside the first source; originals stay unchanged.").small().color(FAINT));
    }
    fn browser(&mut self, ui: &mut egui::Ui, thumbs: &mut crate::thumbs::Thumbnails, height: f32) {
        let media = Arc::clone(&self.media);
        let mut add = None;
        let mut details = None;
        egui::Frame::new()
            .fill(PANEL)
            .corner_radius(12)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_min_height((height - 24.0).max(0.0));
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Library").size(14.0).family(kit::semibold()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if button(ui, "Import clips", false, self.importing.is_none()).clicked() {
                            self.import(None, ui.ctx());
                        }
                    });
                });
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search clips")
                        .margin(egui::Margin::symmetric(10, 8))
                        .char_limit(100)
                        .desired_width(ui.available_width()),
                );
                ui.add_space(6.0);
                let query = self.search.to_lowercase();
                let filtered: Vec<_> = media
                    .iter()
                    .filter(|e| e.title.to_lowercase().contains(&query))
                    .collect();
                let card_width = ((ui.available_width() - 8.0) / 2.0).max(20.0);
                egui::ScrollArea::vertical()
                    .id_salt("editor-browser")
                    .max_height((height - 174.0).max(30.0))
                    .show_rows(ui, 98.0, filtered.len().div_ceil(2), |ui, rows| {
                        for row in rows {
                            ui.horizontal_top(|ui| {
                                for entry in filtered.iter().skip(row * 2).take(2) {
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(card_width, 92.0),
                                        Layout::top_down(Align::Min),
                                        |ui| {
                                            let ready = !entry.offline
                                                && entry
                                                    .probe
                                                    .as_ref()
                                                    .and_then(|p| p.duration)
                                                    .is_some();
                                            // One response owns clicking, dragging and the menu;
                                            // nested drag/thumbnail responses compete for pointer input.
                                            let (card, _) = ui.allocate_exact_size(
                                                Vec2::new(card_width, 92.0),
                                                Sense::hover(),
                                            );
                                            let response = ui.interact(
                                                card,
                                                egui::Id::new(("editor-source", &entry.path)),
                                                Sense::click_and_drag(),
                                            );
                                            response.widget_info(|| {
                                                egui::WidgetInfo::labeled(
                                                    egui::WidgetType::Button,
                                                    true,
                                                    &entry.title,
                                                )
                                            });
                                            let rect = egui::Rect::from_min_size(
                                                card.min,
                                                Vec2::new(card_width, 68.0),
                                            );
                                            ui.painter().rect_filled(rect, 8, CARD);
                                            if let Some(texture) = thumbs.texture(
                                                ui.ctx(),
                                                &entry.path,
                                                ui.is_rect_visible(rect),
                                            ) {
                                                egui::Image::from_texture(
                                                    egui::load::SizedTexture::from_handle(texture),
                                                )
                                                .corner_radius(8)
                                                .paint_at(ui, rect);
                                            }
                                            if response.hovered() || response.has_focus() {
                                                ui.painter().rect_stroke(
                                                    rect,
                                                    8,
                                                    Stroke::new(
                                                        1.0_f32,
                                                        if response.has_focus() {
                                                            LIME
                                                        } else {
                                                            MUTED
                                                        },
                                                    ),
                                                    egui::StrokeKind::Inside,
                                                );
                                            }
                                            let duration =
                                                crate::format_duration(entry.probe.as_ref());
                                            let badge = egui::Rect::from_min_max(
                                                rect.right_bottom() - Vec2::new(44.0, 23.0),
                                                rect.right_bottom() - Vec2::splat(4.0),
                                            );
                                            ui.painter().rect_filled(
                                                badge,
                                                4,
                                                Color32::from_black_alpha(200),
                                            );
                                            ui.painter().text(
                                                badge.center(),
                                                Align2::CENTER_CENTER,
                                                duration,
                                                FontId::new(11.0, kit::medium()),
                                                TEXT,
                                            );
                                            let mut title_job =
                                                egui::text::LayoutJob::simple_singleline(
                                                    entry.title.clone(),
                                                    FontId::new(11.5, kit::medium()),
                                                    TEXT,
                                                );
                                            title_job.wrap.max_width = card_width;
                                            title_job.wrap.max_rows = 1;
                                            title_job.wrap.break_anywhere = true;
                                            let title = ui.painter().layout_job(title_job);
                                            ui.painter()
                                                .with_clip_rect(egui::Rect::from_min_max(
                                                    rect.left_bottom(),
                                                    card.right_bottom(),
                                                ))
                                                .galley(
                                                    rect.left_bottom() + Vec2::new(0.0, 5.0),
                                                    title,
                                                    TEXT,
                                                );
                                            if response.clicked() {
                                                response.request_focus();
                                            }
                                            if ready
                                                && (response.double_clicked()
                                                    || response.has_focus()
                                                        && ui.input(|i| {
                                                            i.key_pressed(egui::Key::Enter)
                                                        }))
                                            {
                                                add = Some((*entry).clone());
                                            }
                                            if response.drag_started() && ready {
                                                response.dnd_set_drag_payload((*entry).clone());
                                            }
                                            response.on_hover_text(&entry.title).context_menu(
                                                |ui| {
                                                    if ui.button("Details").clicked() {
                                                        details = Some((*entry).clone());
                                                        ui.close();
                                                    }
                                                    if ui
                                                        .add_enabled(
                                                            ready && !self.busy(),
                                                            egui::Button::new("Add to timeline"),
                                                        )
                                                        .clicked()
                                                    {
                                                        add = Some((*entry).clone());
                                                        ui.close();
                                                    }
                                                },
                                            );
                                        },
                                    );
                                }
                            });
                        }
                    });
                if self.importing.is_some() {
                    ui.spinner();
                    ui.label(RichText::new("Importing clips…").small().color(MUTED));
                } else if filtered.is_empty() {
                    ui.label(
                        RichText::new("Import clips here, or send them from Clips.")
                            .small()
                            .color(MUTED),
                    );
                }
                ui.label(
                    RichText::new(
                        "Right-click for details or add to timeline.\nDouble-click or drag to add.",
                    )
                    .size(11.5)
                    .color(MUTED),
                );
            });
        if let Some(entry) = add {
            self.append(&entry);
        }
        if let Some(entry) = details {
            self.details = Some(entry);
        }
        if let Some(entry) = self.details.clone() {
            let mut open = true;
            egui::Window::new("Clip details")
                .open(&mut open)
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.label(RichText::new(&entry.title).family(kit::semibold()));
                    ui.label(clipcraft_engine::format_bytes(entry.size_bytes));
                    ui.label(crate::format_duration(entry.probe.as_ref()));
                    if let Some(probe) = &entry.probe
                        && let (Some(width), Some(height)) = (probe.width, probe.height)
                    {
                        ui.label(format!("{width} × {height}"));
                    }
                    ui.label(&entry.path);
                });
            if !open {
                self.details = None;
            }
        }
    }
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let enabled = !self.busy();
            if button(
                ui,
                "Split (S)",
                false,
                enabled && self.playhead > Tick::ZERO && self.playhead < self.session.duration(),
            )
            .clicked()
            {
                self.edit(EditorAction::Split { at: self.playhead });
            }
            if let Some(id) = self.selected {
                if button(ui, "Duplicate", false, enabled).clicked() {
                    self.edit(EditorAction::Duplicate { id });
                }
                if button(ui, "Delete", false, enabled).clicked() {
                    self.edit(EditorAction::Delete { id });
                }
                if let Some(index) = self.session.clips().iter().position(|c| c.id == id) {
                    if button(ui, "Move left", false, enabled && index > 0).clicked() {
                        self.edit(EditorAction::Move {
                            id,
                            index: index.saturating_sub(1),
                        });
                    }
                    if button(
                        ui,
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
            if button(ui, "Fit", self.fit, true).clicked() {
                self.fit = true;
            }
            if button(ui, "−", false, true)
                .on_hover_text("Zoom out")
                .clicked()
            {
                self.fit = false;
                self.zoom = (self.zoom / 1.5).max(4.0);
            }
            if button(ui, "+", false, true)
                .on_hover_text("Zoom in")
                .clicked()
            {
                self.fit = false;
                self.zoom = (self.zoom * 1.5).min(160.0);
            }
            ui.label(
                RichText::new(format!("{} segments", self.session.clips().len()))
                    .small()
                    .color(MUTED),
            );
            if button(
                ui,
                "Reset edit",
                false,
                enabled && !self.session.clips().is_empty(),
            )
            .clicked()
            {
                self.edit(EditorAction::Reset);
            }
        });
    }
    fn timeline(&mut self, ui: &mut egui::Ui, thumbs: &mut crate::thumbs::Thumbnails) {
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
        egui::ScrollArea::horizontal()
            .id_salt("edit-timeline-scroll")
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(width, 142.0), Sense::click_and_drag());
                let painter = ui.painter();
                painter.rect_filled(rect, 8, CANVAS);
                let seconds = total.seconds().max(1.0);
                for i in 0..=10 {
                    let x = rect.left() + width * i as f32 / 10.0;
                    painter.line_segment(
                        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                        Stroke::new(0.5_f32, BORDER),
                    );
                    painter.text(
                        egui::pos2(x + 4.0, rect.top() + 9.0),
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
                                egui::pos2(x, rect.top() + 28.0),
                                Vec2::new(1.0, 72.0),
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
                        egui::pos2(x, rect.top() + 28.0),
                        egui::pos2(right.max(x + 2.0), rect.top() + 100.0),
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
                    if let Some(texture) = thumbs.texture(
                        ui.ctx(),
                        &clip.input.to_string_lossy(),
                        ui.is_rect_visible(card),
                    ) {
                        // Repeat the cached source thumbnail at its natural aspect ratio.
                        // These are source references, not decoded frame-by-frame filmstrips.
                        let tile_width = (card.height() * texture.aspect_ratio()).max(1.0);
                        let visible = card.intersect(ui.clip_rect());
                        let first = ((visible.left() - card.left()) / tile_width)
                            .floor()
                            .max(0.0);
                        let count = (visible.width() / tile_width).ceil() as usize + 1;
                        for i in 0..count.min(128) {
                            let left = card.left() + (first + i as f32) * tile_width;
                            let tile = egui::Rect::from_min_max(
                                egui::pos2(left, card.top()),
                                egui::pos2((left + tile_width).min(card.right()), card.bottom()),
                            );
                            if tile.is_positive() {
                                painter.with_clip_rect(card).image(
                                    texture.id(),
                                    tile,
                                    egui::Rect::from_min_max(
                                        egui::Pos2::ZERO,
                                        egui::pos2(tile.width() / tile_width, 1.0),
                                    ),
                                    Color32::WHITE,
                                );
                            }
                        }
                        painter.rect_filled(card, 6, Color32::from_black_alpha(70));
                    }
                    let clipped = painter.with_clip_rect(card);
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
                    if self.selected == Some(clip.id) {
                        painter.rect_stroke(
                            card,
                            6,
                            Stroke::new(2.0_f32, LIME),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let audio = egui::Rect::from_min_max(
                        egui::pos2(x + 1.0, rect.top() + 108.0),
                        egui::pos2(right.max(x + 2.0) - 1.0, rect.top() + 135.0),
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
                    painter.with_clip_rect(audio).text(
                        audio.left_center() + Vec2::new(5.0, 0.0),
                        Align2::LEFT_CENTER,
                        format!("Audio · {}%", clip.volume_percent),
                        FontId::new(11.0, kit::medium()),
                        MUTED,
                    );
                    if r.clicked() || r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        selection = Some(clip.id);
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
                    selection = self
                        .session
                        .at(RATE.tick_of(frame))
                        .map(|(clip, _)| clip.id);
                }
                if let Some(entry) = response.dnd_release_payload::<MediaEntry>() {
                    dropped = Some((*entry).clone());
                }
                let x = rect.left()
                    + width
                        * (self.playhead.0 as f64 / total.0.max(1) as f64).clamp(0.0, 1.0) as f32;
                painter.line_segment(
                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                    Stroke::new(2.0_f32, LIME),
                );
                painter.circle_filled(egui::pos2(x, rect.top() + 3.0), 4.0, LIME);
                if self.session.clips().is_empty() {
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "Drag clips here, or right-click a library clip to add it",
                        FontId::new(14.0, kit::medium()),
                        MUTED,
                    );
                }
            });
        if let Some(action) = action {
            self.edit(action);
        }
        if let Some(entry) = dropped {
            self.append(&entry);
        }
        if let Some(id) = selection {
            self.selected = Some(id);
        }
        if let Some(time) = seek {
            self.playing = false;
            self.seek(time);
        }
    }
    fn range_controls(&mut self, ui: &mut egui::Ui) {
        let Some(clip) = self
            .session
            .clips()
            .iter()
            .find(|c| Some(c.id) == self.selected)
            .cloned()
        else {
            return;
        };
        ui.add_enabled_ui(!self.busy(), |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut start = clip.start.seconds();
                let mut end = clip.end.seconds();
                let mut volume = clip.volume_percent;
                ui.label(RichText::new("Source in / out").small().color(MUTED));
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
                ui.label(RichText::new("Volume").small().color(MUTED));
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
                ui.label(
                    RichText::new("First audio track · linked to video")
                        .small()
                        .color(FAINT),
                );
            });
        });
    }
}

/// Standard egui buttons keep keyboard activation and the app's focus/hover states.
fn button(ui: &mut egui::Ui, label: &str, primary: bool, enabled: bool) -> egui::Response {
    let text = RichText::new(label).family(kit::medium()).size(12.5);
    let mut button = egui::Button::new(if primary && enabled {
        text.color(CANVAS)
    } else {
        text
    })
    .corner_radius(16)
    .min_size(Vec2::new(30.0, 32.0));
    if primary && enabled {
        button = button.fill(LIME);
    }
    ui.add_enabled(enabled, button)
}
fn stamp(time: Tick) -> String {
    let ms = time.to_units_floor(1000).max(0);
    format!("{}:{:02}.{:01}", ms / 60_000, ms / 1000 % 60, ms / 100 % 10)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> EditorState {
        let mut state = EditorState {
            open: true,
            ..Default::default()
        };
        state.edit(EditorAction::Append {
            input: "generated.mp4".into(),
            name: "Generated clip".into(),
            duration: Tick::from_units(4, 1),
        });
        state
    }
    fn frame(
        state: &mut EditorState,
        context: &egui::Context,
        events: Vec<egui::Event>,
        size: Vec2,
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ctx| {
                crate::kit::apply_theme(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    state.show(ui, &mut crate::thumbs::Thumbnails::default())
                });
            },
        )
    }
    fn key(
        state: &mut EditorState,
        context: &egui::Context,
        key: egui::Key,
        modifiers: egui::Modifiers,
    ) {
        frame(
            state,
            context,
            vec![
                egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                },
                egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers,
                },
            ],
            Vec2::new(1440.0, 900.0),
        );
    }
    fn media() -> MediaEntry {
        MediaEntry {
            path: "generated.mp4".into(),
            title: "Generated clip".into(),
            size_bytes: 123,
            modified_unix_seconds: None,
            extension: "mp4".into(),
            offline: false,
            favorite: false,
            tags: Vec::new(),
            game: None,
            recorded_game: None,
            collections: Vec::new(),
            probe: Some(clipcraft_engine::MediaProbe {
                container: "mp4".into(),
                duration: Some(Tick::from_units(4, 1)),
                width: Some(160),
                height: Some(90),
                frame_rate: Some(RATE),
                video_codec: Some("avc1".into()),
                audio_tracks: 0,
            }),
        }
    }
    #[test]
    fn sending_from_clips_opens_a_separate_editor_library_without_adding_to_timeline() {
        let mut app = crate::ClipCraftApp::default();
        app.send_to_editor(media());
        assert_eq!(app.page, crate::Page::Editor);
        assert!(app.editor.open);
        assert!(app.selected.is_none());
        assert_eq!(app.editor.media.len(), 1);
        assert!(app.editor.session.clips().is_empty());
        app.editor.receive(media());
        assert_eq!(app.editor.media.len(), 1);
        app.editor.append(&media());
        assert_eq!(app.editor.session.clips().len(), 1);
        app.editor.close();
        assert_eq!(app.editor.media.len(), 1);
        assert!(app.editor.has_clips());
    }
    #[test]
    fn editor_thumbnails_offer_details_and_add_to_timeline_without_per_clip_add_buttons() {
        let context = crate::kit::test_context();
        context.enable_accesskit();
        let mut state = EditorState {
            open: true,
            ..Default::default()
        };
        for i in 0..3 {
            let mut entry = media();
            entry.path = format!("generated{i}.mp4");
            state.receive(entry);
        }
        for _ in 0..3 {
            frame(&mut state, &context, vec![], Vec2::new(1440.0, 900.0));
        }
        let source = context
            .read_response(egui::Id::new(("editor-source", "generated2.mp4")))
            .unwrap();
        let point = source.rect.center();
        frame(
            &mut state,
            &context,
            vec![egui::Event::PointerMoved(point)],
            Vec2::new(1440.0, 900.0),
        );
        frame(
            &mut state,
            &context,
            vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            Vec2::new(1440.0, 900.0),
        );
        let mut labels = Vec::new();
        for i in 0..4 {
            let events = if i == 0 {
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            } else {
                vec![]
            };
            if let Some(update) = frame(&mut state, &context, events, Vec2::new(1440.0, 900.0))
                .platform_output
                .accesskit_update
            {
                labels.extend(
                    update
                        .nodes
                        .iter()
                        .filter_map(|(_, node)| node.label().map(str::to_owned)),
                );
            }
        }
        assert!(
            labels.iter().any(|s| s == "Details"),
            "context menu did not open: {labels:?}"
        );
        assert!(labels.iter().any(|s| s == "Add to timeline"));
        assert!(!labels.iter().any(|s| s == "Add"));
    }
    #[test]
    fn library_double_click_and_enter_append_without_a_separate_add_button() {
        let context = crate::kit::test_context();
        let size = Vec2::new(1440.0, 900.0);
        let mut state = EditorState {
            open: true,
            ..Default::default()
        };
        state.receive(media());
        for _ in 0..3 {
            frame(&mut state, &context, vec![], size);
        }
        let id = egui::Id::new(("editor-source", "generated.mp4"));
        let point = context.read_response(id).unwrap().rect.center();
        frame(
            &mut state,
            &context,
            vec![egui::Event::PointerMoved(point)],
            size,
        );
        for _ in 0..2 {
            for pressed in [true, false] {
                frame(
                    &mut state,
                    &context,
                    vec![egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    size,
                );
            }
        }
        assert_eq!(state.session.clips().len(), 1);
        assert_eq!(context.memory(|m| m.focused()), Some(id));
        key(
            &mut state,
            &context,
            egui::Key::Enter,
            egui::Modifiers::NONE,
        );
        assert_eq!(state.session.clips().len(), 2);
    }
    #[test]
    fn keyboard_split_delete_undo_redo_and_session_resume_preserve_cuts() {
        let mut state = state();
        let context = crate::kit::test_context();
        frame(&mut state, &context, vec![], Vec2::new(1440.0, 900.0));
        state.seek(Tick::from_units(2, 1));
        key(&mut state, &context, egui::Key::S, egui::Modifiers::NONE);
        assert_eq!(state.session.clips().len(), 2);
        state.selected = Some(state.session.clips()[1].id);
        key(
            &mut state,
            &context,
            egui::Key::Delete,
            egui::Modifiers::NONE,
        );
        assert_eq!(state.session.clips().len(), 1);
        let command = egui::Modifiers {
            command: true,
            ctrl: true,
            ..Default::default()
        };
        key(&mut state, &context, egui::Key::Z, command);
        assert_eq!(state.session.clips().len(), 2);
        key(&mut state, &context, egui::Key::Y, command);
        assert_eq!(state.session.clips().len(), 1);
        state.close();
        assert!(!state.open);
        assert!(state.has_clips());
        state.open = true;
        assert_eq!(state.session.duration(), Tick::from_units(2, 1));
    }
    #[test]
    fn timeline_layout_extremes_and_named_controls_are_safe() {
        for size in [
            Vec2::new(1.0, 1.0),
            Vec2::new(320.0, 500.0),
            Vec2::new(900.0, 650.0),
            Vec2::new(1440.0, 900.0),
            Vec2::new(6000.0, 4000.0),
        ] {
            let context = crate::kit::test_context();
            let mut state = state();
            state.edit(EditorAction::Split {
                at: Tick::from_units(1, 30),
            });
            for _ in 0..3 {
                frame(&mut state, &context, vec![], size);
            }
        }
        let context = crate::kit::test_context();
        context.enable_accesskit();
        let mut state = state();
        let mut labels = Vec::new();
        for _ in 0..3 {
            if let Some(update) = frame(&mut state, &context, vec![], Vec2::new(1440.0, 900.0))
                .platform_output
                .accesskit_update
            {
                labels.extend(
                    update
                        .nodes
                        .iter()
                        .filter_map(|(_, node)| node.label().map(str::to_owned)),
                );
            }
        }
        for label in [
            "Export video",
            "Import clips",
            "Undo",
            "Play edit",
            "Duplicate",
            "Delete",
            "Move left",
            "Mute",
            "Reset edit",
        ] {
            assert!(
                labels.iter().any(|name| name == label),
                "missing accessible control: {label}"
            );
        }
    }
    #[test]
    fn tab_and_enter_can_duplicate_a_segment() {
        let context = crate::kit::test_context();
        context.enable_accesskit();
        let mut state = state();
        let mut names = std::collections::HashMap::new();
        let mut reached = false;
        for i in 0..100 {
            let key = if reached {
                egui::Key::Enter
            } else {
                egui::Key::Tab
            };
            let events = if i < 3 {
                vec![]
            } else {
                vec![
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            let output = frame(&mut state, &context, events, Vec2::new(1440.0, 900.0));
            if reached {
                assert_eq!(state.session.clips().len(), 2);
                // A focused button must not disable editor shortcuts after activation.
                state.seek(Tick::from_units(2, 1));
                self::key(&mut state, &context, egui::Key::S, egui::Modifiers::NONE);
                assert_eq!(state.session.clips().len(), 3);
                return;
            }
            if let Some(update) = output.platform_output.accesskit_update {
                for (id, node) in update.nodes {
                    names.insert(id, node.label().unwrap_or_default().to_owned());
                }
                reached = names
                    .get(&update.focus)
                    .is_some_and(|name| name == "Duplicate");
            }
        }
        panic!("Duplicate was not reachable with Tab");
    }
}
