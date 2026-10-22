//! Compact media browser, sequence preview and one linked video/audio timeline.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use clipcraft_engine::{
    Command, CommandOutput, EditorAction, EditorSession, Effect, Fade, MediaEntry, SegmentLook,
    SequenceRequest, SequenceSegment, SequenceText, TransitionPreset, TrimmedClip, execute,
};
use clipcraft_time::{FrameRate, Tick};
use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, RichText, Sense, Stroke, Vec2};

use crate::kit::{self, BORDER, CANVAS, CARD, CARD_HOVER, DANGER, FAINT, LIME, MUTED, PANEL, TEXT};

/// The least height the Library and preview row can be dragged down to.
const MIN_TOP: f32 = 210.0;
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

/// What the left panel shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LibraryTab {
    Library,
    Effects,
    Text,
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
    /// Width the person dragged the Library to; `None` lets it follow the 16:9 preview.
    selected_text: Option<u64>,
    title_draft: (u64, String),
    title_drag: Option<(u64, u16, u16)>,
    text_drag: Option<(u64, Tick, Tick)>,
    tab: LibraryTab,
    effect_tab: bool,
    library_width: Option<f32>,
    /// Height the person dragged the preview row to; `None` gives it all the room there is.
    top_height: Option<f32>,
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
            selected_text: None,
            title_draft: (0, String::new()),
            title_drag: None,
            text_drag: None,
            tab: LibraryTab::Library,
            effect_tab: false,
            library_width: None,
            top_height: None,
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
                    .selected_text
                    .is_some_and(|id| !self.session.texts().iter().any(|t| t.id == id))
                {
                    self.selected_text = None;
                }
                if self.selected_text.is_none()
                    && self
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
            self.selected_text = None;
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
            if preview.editor_effect(clip.look.effect) {
                self.seek_pending = true;
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
            texts: self
                .session
                .texts()
                .iter()
                .map(|t| t.layer.clone())
                .collect(),
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
                    look: c.look.clone(),
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
            self.split();
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
        if let Some(id) = self.selected_text {
            if pressed(none, egui::Key::Delete) {
                self.edit(EditorAction::DeleteText { id });
            }
            if pressed(command, egui::Key::D) {
                self.duplicate_text(id);
            }
        } else if let Some(id) = self.selected {
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
        ui.horizontal(|ui| {
            ui.set_min_height(44.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if tool(
                    ui,
                    kit::Icon::Folder,
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
        let full_top = (ui.available_height() - 454.0 - status_height).max(120.0);
        let top = self
            .top_height
            .filter(|height| height.is_finite())
            .map_or(full_top, |height| {
                height.clamp(MIN_TOP.min(full_top), full_top)
            });
        // Room given up by the preview row goes to the timeline.
        let lift = full_top - top;
        let width = ui.available_width().max(1.0);
        // The preview is 16:9, so it is only as wide as its picture needs to be tall; the Library
        // gets everything else.
        let picture_height = (top - 90.0).max(40.0);
        let browser = if width >= 700.0 {
            let preview = (picture_height * 16.0 / 9.0 + 24.0).clamp(360.0, width - 12.0 - 260.0);
            let auto = (width - 12.0 - preview).max(260.0);
            self.library_width
                .filter(|chosen| chosen.is_finite())
                .map_or(auto, |chosen| {
                    chosen.clamp(200.0, (width - 12.0 - 300.0).max(200.0))
                })
        } else {
            (width * 0.32).max(80.0)
        };
        let row_left = ui.cursor().left();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let library = ui.allocate_ui_with_layout(
                Vec2::new(browser, top),
                Layout::top_down(Align::Min),
                |ui| {
                    // Library and preview are always exactly the same height.
                    self.browser(ui, thumbs, top);
                },
            );
            // Drag the gap between the Library and the preview to change their widths.
            if width >= 700.0 {
                let gap = egui::Rect::from_min_max(
                    egui::pos2(library.response.rect.right(), library.response.rect.top()),
                    egui::pos2(
                        library.response.rect.right() + 12.0,
                        library.response.rect.top() + top,
                    ),
                );
                let handle = ui
                    .interact(
                        gap,
                        egui::Id::new("editor-split-library"),
                        Sense::click_and_drag(),
                    )
                    .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                    .on_hover_text("Drag to resize the Library. Double-click to reset.");
                handle.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Resize library")
                });
                splitter_grip(ui, gap, true, handle.hovered() || handle.dragged());
                if handle.double_clicked() {
                    self.library_width = None;
                } else if handle.dragged()
                    && let Some(point) = handle.interact_pointer_pos()
                {
                    self.library_width = Some(point.x - row_left - 6.0);
                }
            }
            let preview_width = (width - browser - 12.0).max(1.0);
            ui.allocate_ui_with_layout(
                Vec2::new(preview_width, top),
                Layout::top_down(Align::Min),
                |ui| {
                    egui::Frame::new()
                        .fill(PANEL)
                        .corner_radius(12)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_width((preview_width - 24.0).max(1.0));
                            ui.set_min_height((top - 16.0).max(0.0));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.title)
                                    .char_limit(100)
                                    .font(FontId::new(13.0, kit::semibold()))
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
                            let size = Vec2::new(ui.available_width(), (top - 90.0).max(40.0));
                            let area = egui::Rect::from_min_size(ui.cursor().min, size);
                            if let Some(preview) = self.preview.as_mut() {
                                preview.editor_picture(ui, size);
                                self.paint_look(ui, area);
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
                            ui.add_space(6.0);
                            // Transport centred under the picture, the time at the right edge.
                            let (row, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 28.0),
                                Sense::hover(),
                            );
                            let any = !self.session.clips().is_empty();
                            let playing = self.playing;
                            ui.painter().circle_filled(row.center(), 15.0, kit::CONTROL);
                            ui.scope_builder(
                                egui::UiBuilder::new().max_rect(egui::Rect::from_center_size(
                                    row.center(),
                                    Vec2::splat(28.0),
                                )),
                                |ui| {
                                    if kit::icon_button(
                                        ui,
                                        if playing {
                                            kit::Icon::Pause
                                        } else {
                                            kit::Icon::Play
                                        },
                                        28.0,
                                        true,
                                        any,
                                        if playing { "Pause edit" } else { "Play edit" },
                                    )
                                    .clicked()
                                    {
                                        self.toggle();
                                    }
                                },
                            );
                            ui.painter().text(
                                egui::pos2(row.right(), row.center().y),
                                Align2::RIGHT_CENTER,
                                format!(
                                    "{} / {}",
                                    stamp(self.playhead),
                                    stamp(self.session.duration())
                                ),
                                FontId::monospace(12.0),
                                MUTED,
                            );
                        });
                },
            );
        });
        // Drag the gap above the timeline to trade preview height for timeline height.
        let (gap, handle) = ui.allocate_exact_size(Vec2::new(width, 12.0), Sense::click_and_drag());
        let handle = handle
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text("Drag to resize the preview and timeline. Double-click to reset.");
        handle.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Resize timeline")
        });
        splitter_grip(ui, gap, false, handle.hovered() || handle.dragged());
        if handle.double_clicked() {
            self.top_height = None;
        } else if handle.dragged() {
            self.top_height =
                Some((top + handle.drag_delta().y).clamp(MIN_TOP.min(full_top), full_top));
        }
        egui::Frame::new()
            .fill(PANEL)
            .corner_radius(12)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width((width - 24.0).max(1.0));
                self.toolbar(ui);
                self.timeline(ui, thumbs, lift);
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
            kit::status_pill(ui, if *error { DANGER } else { LIME }, text);
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (key, what) in [
                ("Space", "play"),
                ("S", "split"),
                ("\u{2190} \u{2192}", "frame"),
                ("Del", "delete"),
                ("Ctrl Z", "undo"),
            ] {
                kit::keycap(ui, key);
                ui.label(RichText::new(what).size(12.0).color(MUTED));
                ui.add_space(8.0);
            }
            ui.label(
                RichText::new("Edits stay in this session. Originals are never changed.")
                    .size(12.0)
                    .color(FAINT),
            );
        });
    }
    /// Whether Split would cut something at the playhead.
    fn can_split(&self) -> bool {
        match self.selected_text_layer() {
            Some((_, layer)) => {
                self.playhead - layer.start >= MIN_TEXT && layer.end - self.playhead >= MIN_TEXT
            }
            None => self.playhead > Tick::ZERO && self.playhead < self.session.duration(),
        }
    }
    /// Cuts the selected text layer, or else the video segment, at the playhead.
    fn split(&mut self) {
        if let Some(id) = self.selected_text {
            self.edit(EditorAction::SplitText {
                id,
                at: self.playhead,
            });
            self.select_text_after(id);
        } else {
            self.edit(EditorAction::Split { at: self.playhead });
        }
    }
    fn duplicate_text(&mut self, id: u64) {
        self.edit(EditorAction::DuplicateText { id });
        self.select_text_after(id);
    }
    /// Selects the text layer right after `id`, which is the new half or the copy.
    fn select_text_after(&mut self, id: u64) {
        let texts = self.session.texts();
        if let Some(index) = texts.iter().position(|t| t.id == id)
            && let Some(next) = texts.get(index + 1)
        {
            self.selected_text = Some(next.id);
        }
    }
    fn selected_text_layer(&self) -> Option<(u64, SequenceText)> {
        self.session
            .texts()
            .iter()
            .find(|t| Some(t.id) == self.selected_text)
            .map(|t| (t.id, t.layer.clone()))
    }
    fn selected_look(&self) -> Option<(u64, SegmentLook)> {
        self.session
            .clips()
            .iter()
            .find(|c| Some(c.id) == self.selected)
            .map(|c| (c.id, c.look.clone()))
    }
    /// Transitions and colour effects, applied to the selected segment with one click.
    fn effects_tab(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (name, effects) in [("Transitions", false), ("Effects", true)] {
                if tab_pill(ui, name, self.effect_tab == effects).clicked() {
                    self.effect_tab = effects;
                }
            }
        });
        ui.add_space(10.0);
        let selected = self.selected_look();
        let target_text = self.selected_text_layer();
        let has_target = selected.is_some() || target_text.is_some();
        let enabled = !self.busy();
        if target_text.is_some() {
            ui.label(
                RichText::new("Applies to the selected text layer.")
                    .size(12.0)
                    .color(FAINT),
            );
            ui.add_space(6.0);
        } else if selected.is_none() && !self.effect_tab {
            ui.label(
                RichText::new("Pick a segment on the timeline, or use a preset for every segment.")
                    .size(12.0)
                    .color(FAINT),
            );
            ui.add_space(6.0);
        } else if !has_target {
            ui.label(
                RichText::new("Pick a segment or text layer on the timeline first.")
                    .size(12.0)
                    .color(FAINT),
            );
            ui.add_space(6.0);
        }
        let columns = (((ui.available_width() + 8.0) / 118.0).floor() as usize).clamp(2, 6);
        let card = Vec2::new(
            ((ui.available_width() - 8.0 * (columns - 1) as f32) / columns as f32).max(20.0),
            78.0,
        );
        let mut action = None;
        egui::ScrollArea::vertical()
            .id_salt("editor-effects")
            .max_height((height - 130.0).max(30.0))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                ui.horizontal_wrapped(|ui| {
                    if self.effect_tab {
                        for effect in Effect::ALL {
                            let active = match (&target_text, &selected) {
                                (Some((_, t)), _) => t.effect == effect,
                                (None, Some((_, l))) => l.effect == effect,
                                _ => false,
                            };
                            let response = preset_card(
                                ui,
                                card,
                                effect.name(),
                                active,
                                enabled && has_target,
                                |painter, rect| {
                                    let bars = [
                                        [255, 40, 20],
                                        [20, 200, 20],
                                        [250, 240, 0],
                                        [30, 40, 250],
                                        [250, 40, 250],
                                        [0, 240, 250],
                                    ];
                                    let width = rect.width() / bars.len() as f32;
                                    for (i, bar) in bars.iter().enumerate() {
                                        let mut pixel = [bar[0], bar[1], bar[2], 255];
                                        effect.apply_rgba(&mut pixel);
                                        painter.rect_filled(
                                            egui::Rect::from_min_size(
                                                rect.min + Vec2::new(width * i as f32, 0.0),
                                                Vec2::new(width + 0.5, rect.height()),
                                            ),
                                            0,
                                            Color32::from_rgb(pixel[0], pixel[1], pixel[2]),
                                        );
                                    }
                                },
                            );
                            if response.clicked() {
                                let value = if active { Effect::None } else { effect };
                                if let Some((id, layer)) = &target_text {
                                    let mut layer = layer.clone();
                                    layer.effect = value;
                                    action = Some(EditorAction::UpdateText { id: *id, layer });
                                } else if let Some((id, look)) = &selected {
                                    let mut look = look.clone();
                                    look.effect = value;
                                    action = Some(EditorAction::Look { id: *id, look });
                                }
                            }
                        }
                    } else {
                        for item in TRANSITIONS.iter().filter(|item| {
                            target_text.is_none()
                                || matches!(
                                    item.kind,
                                    TransitionKind::In(Fade::Black)
                                        | TransitionKind::Out(Fade::Black)
                                )
                        }) {
                            let active = match (&target_text, &selected, item.kind) {
                                (Some((_, t)), _, TransitionKind::In(_)) => t.fade_in != Fade::None,
                                (Some((_, t)), _, TransitionKind::Out(_)) => {
                                    t.fade_out != Fade::None
                                }
                                (None, Some((_, l)), TransitionKind::In(f)) => l.fade_in == f,
                                (None, Some((_, l)), TransitionKind::Out(f)) => l.fade_out == f,
                                _ => false,
                            };
                            let local = !matches!(item.kind, TransitionKind::All(_));
                            let response = preset_card(
                                ui,
                                card,
                                item.name,
                                active,
                                enabled && (has_target || !local),
                                |painter, rect| {
                                    painter.rect_filled(rect, 0, Color32::from_gray(72));
                                    let (left, right, color) = match item.kind {
                                        TransitionKind::In(f) => (true, false, f),
                                        TransitionKind::Out(f) => (false, true, f),
                                        TransitionKind::All(TransitionPreset::DipWhite) => {
                                            (true, true, Fade::White)
                                        }
                                        TransitionKind::All(TransitionPreset::None) => {
                                            (false, false, Fade::None)
                                        }
                                        TransitionKind::All(_) => (true, true, Fade::Black),
                                    };
                                    let shade = if color == Fade::White {
                                        Color32::WHITE
                                    } else {
                                        Color32::BLACK
                                    };
                                    for (on, from_left) in [(left, true), (right, false)] {
                                        if !on {
                                            continue;
                                        }
                                        let w = rect.width() * 0.45;
                                        let (edge, inner) = if from_left {
                                            (rect.left(), rect.left() + w)
                                        } else {
                                            (rect.right(), rect.right() - w)
                                        };
                                        painter.add(egui::Shape::convex_polygon(
                                            vec![
                                                egui::pos2(edge, rect.top()),
                                                egui::pos2(edge, rect.bottom()),
                                                egui::pos2(inner, rect.bottom()),
                                            ],
                                            shade,
                                            Stroke::NONE,
                                        ));
                                    }
                                },
                            );
                            if response.clicked() {
                                match (item.kind, &selected) {
                                    (TransitionKind::All(preset), _) => {
                                        action = Some(EditorAction::Preset { preset });
                                    }
                                    (TransitionKind::In(_) | TransitionKind::Out(_), _)
                                        if target_text.is_some() =>
                                    {
                                        if let Some((id, layer)) = &target_text {
                                            let mut layer = layer.clone();
                                            let fade =
                                                if active { Fade::None } else { Fade::Black };
                                            if matches!(item.kind, TransitionKind::In(_)) {
                                                layer.fade_in = fade;
                                            } else {
                                                layer.fade_out = fade;
                                            }
                                            action =
                                                Some(EditorAction::UpdateText { id: *id, layer });
                                        }
                                    }
                                    (TransitionKind::In(f), Some((id, look))) => {
                                        let mut look = look.clone();
                                        look.fade_in = if active { Fade::None } else { f };
                                        action = Some(EditorAction::Look { id: *id, look });
                                    }
                                    (TransitionKind::Out(f), Some((id, look))) => {
                                        let mut look = look.clone();
                                        look.fade_out = if active { Fade::None } else { f };
                                        action = Some(EditorAction::Look { id: *id, look });
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                });
            });
        if let Some(action) = action {
            self.edit(action);
        }
    }
    /// Text layers: add one at the playhead, then style it. Each lives on its own track above the
    /// video, can be moved and trimmed on the timeline and takes effects and fades like a layer.
    fn text_tab(&mut self, ui: &mut egui::Ui, height: f32) {
        let enabled = !self.busy();
        ui.horizontal(|ui| {
            if tool(
                ui,
                kit::Icon::Plus,
                "Add text",
                true,
                enabled && self.has_clips(),
            )
            .on_hover_text("Put a text layer on the text track at the playhead")
            .clicked()
            {
                let total = self.session.duration();
                let length = Tick::from_units(3, 1).min(total);
                let start = self.playhead.min(total - length).max(Tick::ZERO);
                self.edit(EditorAction::AddText {
                    start,
                    duration: length,
                    text: "Your title".into(),
                });
                if let Some(layer) = self.session.texts().last() {
                    self.selected_text = Some(layer.id);
                    self.selected = None;
                }
            }
        });
        ui.add_space(10.0);
        let Some((id, layer)) = self.selected_text_layer() else {
            ui.label(
                RichText::new(
                    "Add text, or pick a text layer on the track above the video. Effects and Transitions apply to it too.",
                )
                .size(12.0)
                .color(FAINT),
            );
            return;
        };
        if self.title_draft.0 != id {
            self.title_draft = (id, layer.text.clone());
        }
        let mut next = layer.clone();
        let total = self.session.duration();
        ui.add_enabled_ui(enabled, |ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.title_draft.1)
                    .hint_text("Type the text, then press Enter")
                    .char_limit(clipcraft_engine::MAX_TITLE_CHARS)
                    .margin(egui::Margin::symmetric(10, 8))
                    .desired_width(ui.available_width()),
            );
            field.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Text layer")
            });
            if field.lost_focus() {
                next.text = self.title_draft.1.replace(char::is_control, " ");
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new("Starts").size(13.0).color(MUTED));
                let mut start = layer.start.seconds();
                if ui
                    .add(
                        egui::DragValue::new(&mut start)
                            .speed(0.05)
                            .range(0.0..=total.seconds())
                            .fixed_decimals(2)
                            .suffix(" s"),
                    )
                    .changed()
                {
                    let moved = Tick::from_units((start * 1000.0).round() as i64, 1000);
                    let length = layer.duration();
                    next.start = moved.min(total - MIN_TEXT).max(Tick::ZERO);
                    next.end = (next.start + length).min(total.max(next.start + MIN_TEXT));
                }
                ui.label(RichText::new("Lasts").size(13.0).color(MUTED));
                let mut length = layer.duration().seconds();
                if ui
                    .add(
                        egui::DragValue::new(&mut length)
                            .speed(0.05)
                            .range(0.04..=total.seconds().max(0.04))
                            .fixed_decimals(2)
                            .suffix(" s"),
                    )
                    .changed()
                {
                    let ticks = Tick::from_units((length * 1000.0).round() as i64, 1000);
                    next.end = (layer.start + ticks).min(total.max(layer.start + MIN_TEXT));
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new("Size").size(13.0).color(MUTED));
                let mut size = u32::from(layer.size);
                if ui
                    .add(
                        egui::Slider::new(&mut size, 20..=250)
                            .show_value(false)
                            .trailing_fill(true),
                    )
                    .on_hover_text("Letter height")
                    .changed()
                {
                    next.size = size.clamp(20, 250) as u16;
                }
                separator(ui);
                ui.label(RichText::new("Colour").size(13.0).color(MUTED));
                for (name, rgb) in TEXT_COLORS {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name)
                    });
                    ui.painter().circle_filled(
                        rect.center(),
                        9.0,
                        Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
                    );
                    if layer.color == rgb {
                        ui.painter()
                            .circle_stroke(rect.center(), 11.0, Stroke::new(2.0_f32, LIME));
                    }
                    if response
                        .on_hover_text(name)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        next.color = rgb;
                    }
                }
            });
            ui.add_space(10.0);
            let columns = (((ui.available_width() + 8.0) / 118.0).floor() as usize).clamp(2, 6);
            let card = Vec2::new(
                ((ui.available_width() - 8.0 * (columns - 1) as f32) / columns as f32).max(20.0),
                78.0,
            );
            egui::ScrollArea::vertical()
                .id_salt("editor-text")
                .max_height((height - 215.0).max(100.0))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                    ui.horizontal_wrapped(|ui| {
                        for (name, x, y) in [
                            ("Title", 500_u16, 500_u16),
                            ("Lower third", 500, 880),
                            ("Top banner", 500, 120),
                            ("Corner tag", 150, 900),
                        ] {
                            let active = layer.x.abs_diff(x) < 25 && layer.y.abs_diff(y) < 25;
                            let response =
                                preset_card(ui, card, name, active, true, |painter, rect| {
                                    painter.rect_filled(rect, 0, Color32::from_gray(72));
                                    painter.text(
                                        egui::pos2(
                                            rect.left() + rect.width() * f32::from(x) / 1000.0,
                                            rect.top() + rect.height() * f32::from(y) / 1000.0,
                                        ),
                                        Align2::CENTER_CENTER,
                                        "Aa",
                                        FontId::new(15.0, kit::semibold()),
                                        Color32::WHITE,
                                    );
                                });
                            if response.clicked() {
                                (next.x, next.y) = (x, y);
                            }
                        }
                    });
                });
            ui.label(
                RichText::new("Drag the text in the preview to move it anywhere.")
                    .size(12.0)
                    .color(FAINT),
            );
        });
        if next != layer {
            self.edit(EditorAction::UpdateText { id, layer: next });
        }
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
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for (tab, name) in [
                        (LibraryTab::Library, "Library"),
                        (LibraryTab::Effects, "Effects"),
                        (LibraryTab::Text, "Text"),
                    ] {
                        if tab_pill(ui, name, self.tab == tab).clicked() {
                            self.tab = tab;
                        }
                    }
                    if self.tab == LibraryTab::Library {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if tool(
                                ui,
                                kit::Icon::Import,
                                "Import clips",
                                false,
                                self.importing.is_none(),
                            )
                            .clicked()
                            {
                                self.import(None, ui.ctx());
                            }
                        });
                    }
                });
                ui.add_space(10.0);
                match self.tab {
                    LibraryTab::Effects => {
                        self.effects_tab(ui, height);
                        return;
                    }
                    LibraryTab::Text => {
                        self.text_tab(ui, height);
                        return;
                    }
                    LibraryTab::Library => {}
                }
                crate::settings::search_box(ui, &mut self.search, "Search clips");
                ui.add_space(10.0);
                let query = self.search.to_lowercase();
                let filtered: Vec<_> = media
                    .iter()
                    .filter(|e| e.title.to_lowercase().contains(&query))
                    .collect();
                let columns = (((ui.available_width() + 8.0) / 148.0).floor() as usize).clamp(2, 6);
                let card_width = ((ui.available_width() - 8.0 * (columns - 1) as f32)
                    / columns as f32)
                    .max(20.0);
                // Thumbnails are 16:9, so wider cards get taller ones.
                let thumb_height = (card_width * 9.0 / 16.0).clamp(48.0, 150.0);
                let card_height = thumb_height + 36.0;
                egui::ScrollArea::vertical()
                    .id_salt("editor-browser")
                    .max_height((height - 174.0).max(30.0))
                    .show_rows(
                        ui,
                        card_height + 6.0,
                        filtered.len().div_ceil(columns),
                        |ui, rows| {
                            for row in rows {
                                ui.horizontal_top(|ui| {
                                    for entry in filtered.iter().skip(row * columns).take(columns) {
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(card_width, card_height),
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
                                                    Vec2::new(card_width, card_height),
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
                                                    Vec2::new(card_width, thumb_height),
                                                );
                                                ui.painter().rect_filled(rect, 10, CARD);
                                                if let Some(texture) = thumbs.texture(
                                                    ui.ctx(),
                                                    &entry.path,
                                                    ui.is_rect_visible(rect),
                                                ) {
                                                    egui::Image::from_texture(
                                                        egui::load::SizedTexture::from_handle(
                                                            texture,
                                                        ),
                                                    )
                                                    .corner_radius(10)
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
                                                let meta = match entry
                                                    .probe
                                                    .as_ref()
                                                    .and_then(|p| p.width.zip(p.height))
                                                {
                                                    Some((w, h)) => format!(
                                                        "{w}\u{00d7}{h} \u{00b7} {}",
                                                        crate::format_bytes(entry.size_bytes)
                                                    ),
                                                    None => crate::format_bytes(entry.size_bytes),
                                                };
                                                ui.painter()
                                                    .with_clip_rect(egui::Rect::from_min_max(
                                                        rect.left_bottom(),
                                                        card.right_bottom(),
                                                    ))
                                                    .text(
                                                        rect.left_bottom() + Vec2::new(0.0, 22.0),
                                                        Align2::LEFT_TOP,
                                                        meta,
                                                        FontId::new(10.5, kit::medium()),
                                                        MUTED,
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
                                                                egui::Button::new(
                                                                    "Add to timeline",
                                                                ),
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
                        },
                    );
                if self.importing.is_some() {
                    ui.spinner();
                    ui.label(RichText::new("Importing clips…").small().color(MUTED));
                } else if filtered.is_empty() {
                    ui.add_space(16.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("No clips here yet")
                                .size(14.0)
                                .family(kit::semibold())
                                .color(kit::TEXT),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new("Import clips, or send them from Clips.")
                                .size(12.5)
                                .color(MUTED),
                        );
                    });
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Double-click or drag a clip to add it.\nRight-click for details.",
                    )
                    .size(12.0)
                    .color(kit::FAINT),
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
    /// Shows the playhead segment's fade, then every text layer that is on at the playhead, as
    /// the export will. The selected text layer can be dragged to place it anywhere.
    fn paint_look(&mut self, ui: &mut egui::Ui, area: egui::Rect) {
        let Some((clip, source_time)) = self.session.at(self.playhead) else {
            return;
        };
        let fade = clip.look.fade_at(source_time - clip.start, clip.duration());
        let (width, height) = if area.width() / area.height().max(1.0) > 16.0 / 9.0 {
            (area.height() * 16.0 / 9.0, area.height())
        } else {
            (area.width(), area.width() * 9.0 / 16.0)
        };
        let canvas = egui::Rect::from_center_size(area.center(), Vec2::new(width, height));
        let painter = ui.painter().with_clip_rect(canvas);
        if let Some((fade, amount)) = fade {
            let color = if fade == Fade::White { 255 } else { 0 };
            painter.rect_filled(
                canvas,
                0,
                Color32::from_rgba_unmultiplied(color, color, color, (amount * 255.0) as u8),
            );
        }
        let layers: Vec<_> = self
            .session
            .texts()
            .iter()
            .filter(|t| {
                let (start, end) = self.live_range(t.id, &t.layer);
                self.playhead >= start && self.playhead < end
            })
            .cloned()
            .collect();
        for text in layers {
            let id = text.id;
            let mut layer = text.layer;
            if let Some((dragged, x, y)) = self.title_drag
                && dragged == id
            {
                (layer.x, layer.y) = (x, y);
            }
            let line = layer.text.trim();
            if line.is_empty() {
                continue;
            }
            let (start, _) = self.live_range(id, &layer);
            let opacity = layer.opacity_at(self.playhead - start);
            let mut ink = [layer.color[0], layer.color[1], layer.color[2], 255];
            layer.effect.apply_rgba(&mut ink);
            let alpha = |a: f32| (opacity * a * 255.0).round().clamp(0.0, 255.0) as u8;
            let color = Color32::from_rgba_unmultiplied(ink[0], ink[1], ink[2], alpha(1.0));
            let galley = painter.layout(
                line.to_owned(),
                FontId::new(
                    (height * f32::from(layer.size) / 1000.0).max(8.0),
                    kit::semibold(),
                ),
                color,
                width * 0.9,
            );
            let size = galley.size();
            let centre = egui::pos2(
                canvas.left() + width * f32::from(layer.x) / 1000.0,
                canvas.top() + height * f32::from(layer.y) / 1000.0,
            );
            let at = egui::pos2(
                (centre.x - size.x / 2.0)
                    .clamp(canvas.left(), (canvas.right() - size.x).max(canvas.left())),
                (centre.y - size.y / 2.0)
                    .clamp(canvas.top(), (canvas.bottom() - size.y).max(canvas.top())),
            );
            let shadow = (height * 0.01).clamp(1.0, 3.0);
            let mut shade = [0, 0, 0, 255];
            layer.effect.apply_rgba(&mut shade);
            painter.galley_with_override_text_color(
                at + Vec2::splat(shadow),
                galley.clone(),
                Color32::from_rgba_unmultiplied(shade[0], shade[1], shade[2], alpha(0.65)),
            );
            painter.galley(at, galley, color);
            if self.selected_text == Some(id) && !self.busy() {
                let rect = egui::Rect::from_min_size(at, size).expand(6.0);
                let response = ui
                    .interact(rect, egui::Id::new("editor-title-drag"), Sense::drag())
                    .on_hover_cursor(egui::CursorIcon::Grab)
                    .on_hover_text("Drag to move the text");
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Move text")
                });
                if response.hovered() || response.dragged() {
                    painter.rect_stroke(
                        rect,
                        4,
                        Stroke::new(1.0_f32, LIME),
                        egui::StrokeKind::Inside,
                    );
                }
                if response.dragged() {
                    let delta = response.drag_delta();
                    let x =
                        (f32::from(layer.x) + delta.x / width.max(1.0) * 1000.0).clamp(0.0, 1000.0);
                    let y = (f32::from(layer.y) + delta.y / height.max(1.0) * 1000.0)
                        .clamp(0.0, 1000.0);
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
        }
    }
    /// A text layer's range, including a move or trim still being dragged on the timeline.
    fn live_range(&self, id: u64, layer: &SequenceText) -> (Tick, Tick) {
        match self.text_drag {
            Some((dragged, start, end)) if dragged == id => (start, end),
            _ => (layer.start, layer.end),
        }
    }
    fn toolbar(&mut self, ui: &mut egui::Ui) {
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
                if let Some(index) = self.session.clips().iter().position(|c| c.id == id) {
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
    fn timeline(&mut self, ui: &mut egui::Ui, thumbs: &mut crate::thumbs::Thumbnails, lift: f32) {
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
        ui.add_space(10.0);
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

/// Shortest a text layer can be.
const MIN_TEXT: Tick = Tick(clipcraft_time::TICKS_PER_SECOND / 30);

/// Height of the text track above the video on the timeline.
const TEXT_LANE: f32 = 34.0;

/// Text colours offered in the Text tab.
const TEXT_COLORS: [(&str, [u8; 3]); 6] = [
    ("White", [255, 255, 255]),
    ("Black", [0, 0, 0]),
    ("Lime", [190, 255, 0]),
    ("Yellow", [255, 220, 40]),
    ("Red", [255, 70, 60]),
    ("Sky", [90, 200, 255]),
];

#[derive(Clone, Copy)]
enum TransitionKind {
    In(Fade),
    Out(Fade),
    All(TransitionPreset),
}
struct TransitionItem {
    name: &'static str,
    kind: TransitionKind,
}
const TRANSITIONS: [TransitionItem; 8] = [
    TransitionItem {
        name: "Fade in",
        kind: TransitionKind::In(Fade::Black),
    },
    TransitionItem {
        name: "Fade out",
        kind: TransitionKind::Out(Fade::Black),
    },
    TransitionItem {
        name: "Flash in",
        kind: TransitionKind::In(Fade::White),
    },
    TransitionItem {
        name: "Flash out",
        kind: TransitionKind::Out(Fade::White),
    },
    TransitionItem {
        name: "Dip to black",
        kind: TransitionKind::All(TransitionPreset::DipBlack),
    },
    TransitionItem {
        name: "Dip to white",
        kind: TransitionKind::All(TransitionPreset::DipWhite),
    },
    TransitionItem {
        name: "Fade in & out",
        kind: TransitionKind::All(TransitionPreset::FadeEnds),
    },
    TransitionItem {
        name: "Clear all",
        kind: TransitionKind::All(TransitionPreset::None),
    },
];

/// A pill tab like the ones above the Library.
fn tab_pill(ui: &mut egui::Ui, label: &str, active: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::new(13.0, kit::medium()),
        Color32::PLACEHOLDER,
    );
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(galley.size().x + 28.0, 28.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let (fill, text) = if active {
        (LIME, Color32::BLACK)
    } else if response.hovered() {
        (kit::CONTROL_HOVER, TEXT)
    } else {
        (kit::CONTROL, MUTED)
    };
    ui.painter().rect_filled(rect, 14, fill);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::new(13.0, kit::medium()),
        text,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A square-ish preset: a small picture of what it does, with its name underneath.
fn preset_card(
    ui: &mut egui::Ui,
    size: Vec2,
    name: &str,
    active: bool,
    enabled: bool,
    swatch: impl FnOnce(&egui::Painter, egui::Rect),
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
    let picture = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width(), size.y - 22.0));
    let painter = ui.painter().with_clip_rect(picture);
    swatch(&painter, picture);
    if !enabled {
        ui.painter()
            .rect_filled(picture, 0, Color32::from_black_alpha(120));
    }
    let outline = if active {
        LIME
    } else if response.hovered() && enabled {
        MUTED
    } else {
        BORDER
    };
    ui.painter().rect_stroke(
        picture,
        0,
        Stroke::new(if active { 2.0_f32 } else { 1.0_f32 }, outline),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        egui::pos2(rect.left(), rect.bottom() - 2.0),
        Align2::LEFT_BOTTOM,
        name,
        FontId::new(12.0, kit::medium()),
        if enabled { TEXT } else { FAINT },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Quiet or lime text button. Standard egui buttons keep keyboard activation and the app's
/// focus/hover states; the colours match the rest of the app.
fn button(ui: &mut egui::Ui, label: &str, primary: bool, enabled: bool) -> egui::Response {
    let text = RichText::new(label).family(kit::medium()).size(12.5);
    let mut button = egui::Button::new(if primary && enabled {
        text.color(Color32::BLACK)
    } else {
        text
    })
    .corner_radius(15)
    .min_size(Vec2::new(30.0, 30.0));
    if primary && enabled {
        button = button.fill(LIME).stroke(Stroke::NONE);
    } else {
        button = button
            .fill(kit::CONTROL)
            .stroke(Stroke::new(1.0_f32, kit::CONTROL_BORDER));
    }
    ui.add_enabled(enabled, button)
}

/// A button with an icon, in the same pill shape as the rest of the app.
fn tool(
    ui: &mut egui::Ui,
    icon: kit::Icon,
    label: &str,
    primary: bool,
    enabled: bool,
) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::new(12.5, kit::medium()),
        Color32::PLACEHOLDER,
    );
    let size = Vec2::new(galley.size().x + 38.0, 28.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let hovered = response.hovered() && enabled;
    let (fill, content) = match (primary, enabled, hovered) {
        (_, false, _) => (CARD, FAINT),
        (true, _, _) => (LIME, Color32::BLACK),
        (false, _, true) => (kit::CONTROL_HOVER, kit::TEXT),
        (false, _, false) => (kit::CONTROL, kit::TEXT),
    };
    ui.painter().rect_filled(rect, 14, fill);
    kit::draw_icon(
        ui.painter(),
        egui::pos2(rect.left() + 15.0, rect.center().y),
        icon,
        content,
        0.55,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 27.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::new(12.5, kit::medium()),
        content,
    );
    response
}

/// The grip drawn in the gap between two panels that can be dragged to resize them.
fn splitter_grip(ui: &egui::Ui, gap: egui::Rect, vertical: bool, active: bool) {
    let size = if vertical {
        Vec2::new(4.0, 40.0)
    } else {
        Vec2::new(40.0, 4.0)
    };
    ui.painter().rect_filled(
        egui::Rect::from_center_size(gap.center(), size),
        2,
        if active { LIME } else { kit::CONTROL_BORDER },
    );
}

/// A thin vertical rule between groups of controls.
fn separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 24.0), Sense::hover());
    ui.painter().line_segment(
        [
            egui::pos2(rect.center().x, rect.top() + 2.0),
            egui::pos2(rect.center().x, rect.bottom() - 2.0),
        ],
        Stroke::new(1.0_f32, kit::CONTROL_BORDER),
    );
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
    fn split_and_duplicate_act_on_the_selected_text_layer_not_the_video() {
        let mut state = state();
        state.edit(EditorAction::AddText {
            start: Tick::from_units(1, 1),
            duration: Tick::from_units(2, 1),
            text: "Hello".into(),
        });
        let id = state.session.texts()[0].id;
        state.selected_text = Some(id);
        state.selected = None;
        state.playhead = Tick::from_units(1, 1);
        assert!(
            !state.can_split(),
            "no room to cut at the layer's first frame"
        );
        state.playhead = Tick::from_units(2, 1);
        assert!(state.can_split());
        state.split();
        assert_eq!(state.session.texts().len(), 2);
        assert_eq!(state.session.clips().len(), 1, "the video must not be cut");
        assert_eq!(state.selected_text, Some(state.session.texts()[1].id));
        let right = state.session.texts()[1].id;
        state.duplicate_text(right);
        assert_eq!(state.session.texts().len(), 3);
        assert_eq!(state.selected_text, Some(state.session.texts()[2].id));
    }
    #[test]
    fn text_layers_draw_select_and_survive_every_tab_and_size() {
        for size in [
            Vec2::new(1.0, 1.0),
            Vec2::new(320.0, 500.0),
            Vec2::new(1440.0, 900.0),
            Vec2::new(6000.0, 4000.0),
        ] {
            let context = crate::kit::test_context();
            let mut state = state();
            state.edit(EditorAction::AddText {
                start: Tick::from_units(1, 1),
                duration: Tick::from_units(2, 1),
                text: "Hello".into(),
            });
            let id = state.session.texts()[0].id;
            state.selected_text = Some(id);
            state.selected = None;
            state.text_drag = Some((id, Tick::ZERO, Tick::from_units(1, 1)));
            state.title_drag = Some((id, 2000, 0));
            for tab in [LibraryTab::Library, LibraryTab::Effects, LibraryTab::Text] {
                for effects in [false, true] {
                    state.tab = tab;
                    state.effect_tab = effects;
                    state.playhead = Tick::from_units(1, 1);
                    for _ in 0..2 {
                        frame(&mut state, &context, vec![], size);
                    }
                }
            }
            // Deleting the selected layer leaves nothing selected and the editor still draws.
            state.edit(EditorAction::DeleteText { id });
            assert_eq!(state.selected_text, None);
            frame(&mut state, &context, vec![], size);
        }
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
    fn dragged_panel_sizes_are_clamped_and_every_size_still_draws() {
        for (library, top) in [
            (None, None),
            (Some(-500.0), Some(-500.0)),
            (Some(1.0e9), Some(1.0e9)),
            (Some(f32::NAN), Some(f32::NAN)),
            (Some(420.0), Some(260.0)),
        ] {
            for size in [
                Vec2::new(320.0, 500.0),
                Vec2::new(960.0, 640.0),
                Vec2::new(1440.0, 900.0),
            ] {
                let context = crate::kit::test_context();
                let mut state = state();
                state.library_width = library;
                state.top_height = top;
                for _ in 0..3 {
                    frame(&mut state, &context, vec![], size);
                }
            }
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
