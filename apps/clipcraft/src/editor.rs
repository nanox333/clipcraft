//! Compact media browser, sequence preview and one linked video/audio timeline.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use clipcraft_engine::{
    Command, CommandOutput, EditorAction, EditorSession, Effect, ExportPreset, ExportedSequence,
    Fade, MediaEntry, SegmentLook, SequenceQuality, SequenceRequest, SequenceResolution,
    SequenceSegment, SequenceSettings, SequenceText, TransitionPreset, execute,
};
use clipcraft_time::{FrameRate, Tick};
use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, RichText, Sense, Stroke, Vec2};

use crate::kit::{
    self, BORDER, CANVAS, CARD, CARD_HOVER, FAINT, LIME, MUTED, PANEL, TEXT, sub_tabs,
};

mod inspector;
mod library;
mod timeline;
#[path = "editor_workspace.rs"]
mod workspace;

/// The least height the Library and preview row can be dragged down to.
const MIN_TOP: f32 = 210.0;
const RATE: FrameRate = FrameRate { num: 30, den: 1 };

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreviewTextGeometry {
    key: SequenceText,
    canvas: [u32; 2],
    size: [usize; 2],
}

struct ExportJob {
    receiver: mpsc::Receiver<Result<ExportedSequence, String>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
}

/// Progress still updates every frame, but a fast export must not flood the event loop.
fn export_wake_due(last: &mut Option<Instant>, now: Instant, complete: bool) -> bool {
    if complete || last.is_none_or(|at| now.duration_since(at) >= Duration::from_millis(100)) {
        *last = Some(now);
        true
    } else {
        false
    }
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

/// Everything that makes a prepared source specific to this position in the sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PreviewSource {
    id: u64,
    input: PathBuf,
    start: Tick,
    end: Tick,
    effect: Effect,
}
impl From<&clipcraft_engine::EditorClip> for PreviewSource {
    fn from(clip: &clipcraft_engine::EditorClip) -> Self {
        Self {
            id: clip.id,
            input: clip.input.clone(),
            start: clip.start,
            end: clip.end,
            effect: clip.look.effect,
        }
    }
}
struct PreparedPreview {
    source: PreviewSource,
    playback: crate::playback::Playback,
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
    /// Start again from the beginning when the edit ends.
    looping: bool,
    preview: Option<crate::playback::Playback>,
    preview_id: Option<u64>,
    preview_source: Option<PreviewSource>,
    preview_target: Option<Tick>,
    prepared_preview: Option<PreparedPreview>,
    failed_preload: Option<PreviewSource>,
    /// Stopped decoders still count against the two-player budget until they acknowledge stop.
    retiring_previews: Vec<crate::playback::Playback>,
    seek_pending: bool,
    clock: Option<(Instant, Tick)>,
    search: String,
    browser_search: crate::library_view::BrowserSearch,
    /// Whether the Library shows picture cards or a list, and how big the cards are (0 to 2).
    library_layout: library::LibraryLayout,
    card_scale: u8,
    title: String,
    /// The "add clips" box, while it is open.
    picker: Option<ClipPicker>,
    zoom: f32,
    fit: bool,
    message: Option<(String, bool)>,
    job: Option<ExportJob>,
    export_dialog: bool,
    export_preset: Option<ExportPreset>,
    export_quality: SequenceQuality,
    /// Where the finished video goes; `None` puts it beside the first clip.
    export_folder: Option<PathBuf>,
    saved: Option<PathBuf>,
    /// Width the person dragged the Library to; `None` lets it follow the 16:9 preview.
    selected_text: Option<u64>,
    title_draft: (u64, String),
    title_drag: Option<(u64, u16, u16)>,
    text_drag: Option<(u64, Tick, Tick)>,
    tab: LibraryTab,
    effect_tab: bool,
    /// Which page of the Text tab is open: Style, Font or Look (0 to 2).
    text_page: u8,
    library_width: Option<f32>,
    /// Height the person dragged the preview row to; `None` gives it all the room there is.
    top_height: Option<f32>,
    /// One bounded set of shared FilmCraft rasters uploaded to the GPU for preview.
    text_geometry: Option<PreviewTextGeometry>,
    draft: workspace::DraftState,
    timeline_media: crate::editor_media::TimelineCache,
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
            looping: false,
            preview: None,
            preview_id: None,
            preview_source: None,
            preview_target: None,
            prepared_preview: None,
            failed_preload: None,
            retiring_previews: Vec::new(),
            seek_pending: true,
            clock: None,
            search: String::new(),
            browser_search: crate::library_view::BrowserSearch::default(),
            library_layout: library::LibraryLayout::default(),
            card_scale: 1,
            title: String::new(),
            picker: None,
            zoom: 32.0,
            fit: true,
            message: None,
            job: None,
            export_dialog: false,
            export_preset: None,
            export_quality: SequenceQuality::default(),
            export_folder: None,
            saved: None,
            selected_text: None,
            title_draft: (0, String::new()),
            title_drag: None,
            text_drag: None,
            tab: LibraryTab::Library,
            effect_tab: false,
            text_page: 0,
            library_width: None,
            top_height: None,
            text_geometry: None,
            draft: workspace::DraftState::default(),
            timeline_media: crate::editor_media::TimelineCache::default(),
        }
    }
}
struct Imported {
    entries: Vec<MediaEntry>,
    thumbnails: Vec<(String, PathBuf)>,
    failures: usize,
    recovery: bool,
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
            self.dirty_recovery();
        } else {
            self.message = Some((
                "The editor media library is full. Import a smaller selection.".into(),
                true,
            ));
        }
    }
    fn import(&mut self, paths: Option<Vec<PathBuf>>, context: &egui::Context) {
        self.start_import(paths, false, context);
    }
    fn import_recovery(&mut self, paths: Vec<PathBuf>, context: &egui::Context) {
        self.start_import(Some(paths), true, context);
    }
    fn start_import(
        &mut self,
        paths: Option<Vec<PathBuf>>,
        recovery: bool,
        context: &egui::Context,
    ) {
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
                    let mut failures = 0;
                    let entries = if recovery {
                        let mut entries = Vec::new();
                        for path in paths
                            .into_iter()
                            .take(clipcraft_engine::MAX_LIBRARY_RESULTS)
                        {
                            match execute(Command::ImportEditorMedia { paths: vec![path] }) {
                                Ok(CommandOutput::EditorMedia(imported)) => {
                                    entries.extend(imported)
                                }
                                _ => failures += 1,
                            }
                        }
                        entries
                    } else {
                        let CommandOutput::EditorMedia(entries) =
                            execute(Command::ImportEditorMedia { paths })
                                .map_err(|e| e.to_string())?
                        else {
                            return Err("Import returned an unexpected result.".to_owned());
                        };
                        entries
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
                        failures,
                        recovery,
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
        if self.open || self.playing {
            self.dirty_recovery_view();
        }
        self.open = false;
        self.export_dialog = false;
        self.playing = false;
        self.invalidate_preload();
        self.retire_active_preview();
        self.clock = None;
        self.text_geometry = None;
        self.timeline_media.clear();
    }
    pub fn take_saved(&mut self) -> Option<PathBuf> {
        self.saved.take()
    }
    pub fn has_clips(&self) -> bool {
        !self.session.clips().is_empty()
    }
    /// The clips the editor holds: those in its library and those on the timeline. The storage
    /// limit leaves them alone, since an edit may be using them.
    pub fn library_paths(&self) -> Vec<PathBuf> {
        self.media
            .iter()
            .map(|entry| PathBuf::from(&entry.path))
            .chain(self.session.clips().iter().map(|clip| clip.input.clone()))
            .collect()
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
                self.invalidate_preload();
                if self
                    .preview
                    .as_ref()
                    .is_some_and(|preview| preview.editor_error().is_some())
                {
                    self.retire_active_preview();
                }
                self.clock = None;
                self.message = None;
                self.dirty_recovery();
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
        let next = time.clamp(Tick::ZERO, self.session.duration());
        if self.playhead != next {
            self.playhead = next;
            self.dirty_recovery_view();
        }
        self.seek_pending = true;
        self.invalidate_preload();
        self.clock = None;
    }
    /// The frame rate of the segment at `position`: the clip's own, or 30 when it is not known.
    fn rate_at(&self, position: Tick) -> FrameRate {
        self.session
            .at(position)
            .and_then(|(clip, _)| {
                self.media
                    .iter()
                    .find(|entry| entry.path == clip.input.to_string_lossy())
            })
            .and_then(|entry| entry.probe.as_ref())
            .and_then(|probe| probe.frame_rate)
            .unwrap_or(RATE)
    }
    /// The playhead one frame later or earlier, on the frames of the clip being shown: a 60 FPS
    /// clip steps by 1/60 s, a 24 FPS one by 1/24 s. From between two frames it goes to the next
    /// (or the start of this) frame.
    fn stepped(&self, forward: bool) -> Tick {
        let position = if forward {
            self.playhead
        } else {
            // The instant just before: the segment and frame the playhead is leaving.
            (self.playhead - Tick(1)).max(Tick::ZERO)
        };
        let Some((clip, source)) = self.session.at(position) else {
            return self.playhead;
        };
        let rate = self.rate_at(position);
        let frame = rate.frame_at(source);
        let target = if forward {
            let next = rate.tick_of(frame + 1);
            if next > source {
                next
            } else {
                rate.tick_of(frame + 2)
            }
        } else {
            rate.tick_of(frame)
        };
        let offset = self.session.offset_of(clip.id).unwrap_or(Tick::ZERO);
        (offset + (target - clip.start)).clamp(Tick::ZERO, self.session.duration())
    }
    /// `position` moved to the nearest frame of the clip under it, so a click on the timeline lands
    /// on a frame of that clip whatever its frame rate.
    fn snap_to_frame(&self, position: Tick) -> Tick {
        let Some((clip, source)) = self.session.at(position) else {
            return position.clamp(Tick::ZERO, self.session.duration());
        };
        let rate = self.rate_at(position);
        let nearest = rate.frame_at(source + Tick(rate.frame_duration().0 / 2));
        let snapped = rate.tick_of(nearest).clamp(clip.start, clip.end);
        let offset = self.session.offset_of(clip.id).unwrap_or(Tick::ZERO);
        (offset + (snapped - clip.start)).clamp(Tick::ZERO, self.session.duration())
    }
    fn step_frame(&mut self, forward: bool) {
        self.playing = false;
        let target = self.stepped(forward);
        self.seek(target);
    }
    /// Where each segment starts, then where the edit ends.
    fn cuts(&self) -> Vec<Tick> {
        let mut at = Tick::ZERO;
        let mut cuts = vec![at];
        for clip in self.session.clips() {
            at += clip.duration();
            cuts.push(at);
        }
        cuts
    }
    /// The start of this segment, or of the one before when already at its start.
    fn previous_cut(&mut self) {
        let before = self.playhead - self.rate_at(self.playhead).frame_duration();
        let target = self
            .cuts()
            .into_iter()
            .rev()
            .find(|cut| *cut < before)
            .unwrap_or(Tick::ZERO);
        self.seek(target);
    }
    /// The start of the next segment, or the end of the edit.
    fn next_cut(&mut self) {
        let target = self
            .cuts()
            .into_iter()
            .find(|cut| *cut > self.playhead)
            .unwrap_or_else(|| self.session.duration());
        self.seek(target);
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
        self.dirty_recovery_view();
    }
    fn retire_preview(&mut self, mut playback: crate::playback::Playback) {
        playback.editor_retire();
        self.retiring_previews.push(playback);
    }
    fn retire_active_preview(&mut self) {
        if let Some(playback) = self.preview.take() {
            self.retire_preview(playback);
        }
        self.preview_id = None;
        self.preview_source = None;
        self.preview_target = None;
        self.clock = None;
    }
    fn invalidate_preload(&mut self) {
        if let Some(prepared) = self.prepared_preview.take() {
            self.retire_preview(prepared.playback);
        }
        self.failed_preload = None;
        self.preview_target = None;
    }
    fn preview_workers(&self) -> usize {
        usize::from(self.preview.is_some())
            + usize::from(self.prepared_preview.is_some())
            + self.retiring_previews.len()
    }
    fn poll_preview(&mut self, context: &egui::Context) {
        self.retiring_previews.retain(|p| !p.editor_finished());
        if !self.retiring_previews.is_empty() {
            context.request_repaint_after(Duration::from_millis(16));
        }
        if !self.open {
            return;
        }
        if let Some(prepared) = self.prepared_preview.as_mut() {
            prepared.playback.poll(context);
            if prepared.playback.editor_error().is_some() {
                self.failed_preload = Some(prepared.source.clone());
                if let Some(prepared) = self.prepared_preview.take() {
                    self.retire_preview(prepared.playback);
                }
            }
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
                if self.looping {
                    self.seek(Tick::ZERO);
                } else {
                    self.playing = false;
                    self.clock = None;
                }
            }
        }
        let Some((clip, source_time)) = self
            .session
            .at(self.playhead)
            .map(|(clip, time)| (clip.clone(), time))
        else {
            self.invalidate_preload();
            self.retire_active_preview();
            return;
        };
        let source = PreviewSource::from(&clip);
        let mut source_time = source_time;
        if self.preview_source.as_ref() != Some(&source) {
            let natural_cut = !self.seek_pending && self.preview_source.is_some();
            let same_input = self
                .preview_source
                .as_ref()
                .is_some_and(|previous| previous.input == source.input);
            let continuous = natural_cut
                && same_input
                && self.preview_source.as_ref().is_some_and(|previous| {
                    previous.end == source.start && previous.effect == source.effect
                });
            if !continuous && natural_cut {
                // Start the next cut at its exact boundary, even if the last UI frame ran late.
                self.playhead = self.session.offset_of(clip.id).unwrap_or(self.playhead);
                source_time = clip.start;
            }
            if !same_input {
                self.retire_active_preview();
                let prepared_matches = self
                    .prepared_preview
                    .as_ref()
                    .is_some_and(|prepared| prepared.source == source);
                if prepared_matches {
                    if let Some(prepared) = self.prepared_preview.take() {
                        self.preview = Some(prepared.playback);
                        self.preview_target = Some(clip.start);
                    }
                } else {
                    self.invalidate_preload();
                }
                if self.preview.is_none() && self.preview_workers() < 2 {
                    self.preview = Some(crate::playback::Playback::prepare_editor(
                        &clip.input.to_string_lossy(),
                        source_time,
                        self.preview_recipe(&clip),
                        context,
                    ));
                    self.preview_target = Some(source_time);
                }
            }
            if self.preview.is_none() {
                // A slow cancelled decoder retains its slot; never spawn unbounded replacements.
                self.clock = None;
                context.request_repaint_after(Duration::from_millis(16));
                return;
            }
            self.preview_id = Some(clip.id);
            self.preview_source = Some(source.clone());
            self.seek_pending = !continuous;
            if !continuous {
                self.clock = None;
            }
        }
        let recipe = self.preview_recipe(&clip);
        if let Some(preview) = self.preview.as_mut() {
            if preview.editor_recipe(recipe) {
                self.seek_pending = true;
                self.preview_target = None;
            }
            preview.poll(context);
            if let Some(error) = preview.editor_error() {
                self.message = Some((error.to_owned(), true));
                self.playing = false;
                self.clock = None;
            }
            let seek = (self.seek_pending && self.preview_target.is_none()).then_some(source_time);
            if let Some(target) = seek {
                self.preview_target = Some(target);
                self.clock = None;
            }
            let ready = seek.is_none()
                && self.preview_target.map_or_else(
                    || preview.editor_ready(),
                    |target| preview.editor_target_ready(target),
                );
            let gain = clip.look.audio_gain_at(
                source_time - clip.start,
                clip.duration(),
                clip.volume_percent,
            );
            // Set gain before activation so a newly opened output never starts at stale volume.
            preview.editor_transport(seek, self.playing && ready, gain);
            if ready {
                self.seek_pending = false;
                self.preview_target = None;
                preview.editor_activate();
                if self.playing && self.clock.is_none() {
                    self.clock = Some((Instant::now(), self.playhead));
                }
            }
        }
        self.prepare_next_preview(&source, context);
        if self.playing {
            context.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn preview_recipe(
        &self,
        clip: &clipcraft_engine::EditorClip,
    ) -> clipcraft_engine::PreviewRecipe {
        clipcraft_engine::PreviewRecipe {
            canvas: Some({
                let (w, h) = self.export_canvas().unwrap_or((1280, 720));
                [w, h]
            }),
            framing: self.export_quality.framing,
            source_start: clip.start,
            source_end: clip.end,
            timeline_start: self.session.offset_of(clip.id).unwrap_or(Tick::ZERO),
            look: clip.look.clone(),
            texts: self
                .session
                .texts()
                .iter()
                .map(|text| {
                    let mut layer = text.layer.clone();
                    let (start, end) = self.live_range(text.id, &layer);
                    layer.start = start;
                    layer.end = end;
                    if let Some((id, x, y)) = self.title_drag
                        && id == text.id
                    {
                        layer.x = x;
                        layer.y = y;
                    }
                    layer
                })
                .collect(),
        }
    }

    fn prepare_next_preview(&mut self, source: &PreviewSource, context: &egui::Context) {
        if !self.playing
            || self.seek_pending
            || self.preview_target.is_some()
            || self.prepared_preview.is_some()
            || self.preview_workers() >= 2
            || !self
                .preview
                .as_ref()
                .is_some_and(|preview| preview.editor_ready() && preview.editor_error().is_none())
        {
            return;
        }
        let Some(next) = self
            .session
            .clips()
            .iter()
            .position(|clip| clip.id == source.id)
            .and_then(|index| self.session.clips().get(index + 1))
        else {
            return;
        };
        let recipe = self.preview_recipe(next);
        let next = PreviewSource::from(next);
        if next.input == source.input || self.failed_preload.as_ref() == Some(&next) {
            return;
        }
        self.prepared_preview = Some(PreparedPreview {
            playback: crate::playback::Playback::prepare_editor(
                &next.input.to_string_lossy(),
                next.start,
                recipe,
                context,
            ),
            source: next,
        });
    }
    pub fn poll(&mut self, context: &egui::Context) {
        self.poll_recovery(context);
        self.timeline_media.poll(context);
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
                    if imported.failures > 0 {
                        self.message = Some((
                            format!(
                                "Draft restored, but {} source file(s) could not be opened. Reconnect their drive or restore the files to their original location.",
                                imported.failures
                            ),
                            true,
                        ));
                    } else if imported.recovery {
                        self.message =
                            Some(("Your local editor draft was restored.".into(), false));
                    }
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
                    let saved_to = std::path::Path::new(&report.clip.path)
                        .parent()
                        .map(|folder| {
                            shorten_middle(
                                &clipcraft_engine::plain_path(folder.to_path_buf())
                                    .to_string_lossy(),
                                48,
                            )
                        })
                        .unwrap_or_default();
                    self.saved = Some(report.clip.path.into());
                    let target = report.target_bytes.map_or(String::new(), |target| {
                        format!(
                            " · target {}{}",
                            clipcraft_engine::format_bytes(target),
                            match (report.within_target, report.retried) {
                                (true, false) => "",
                                (true, true) => " (re-encoded once to fit)",
                                (false, _) => " (still over after re-encoding)",
                            }
                        )
                    });
                    self.message = Some((
                        format!(
                            "Edit saved to {saved_to} · {}{target} · {}×{} · {} FPS · {}",
                            clipcraft_engine::format_bytes(report.clip.bytes),
                            report.width,
                            report.height,
                            report.fps,
                            report.encoder
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
        if self.session.clips().is_empty() {
            return;
        }
        let Some(output) = self.export_destination() else {
            self.message = Some((
                "No unused output name is available in that folder.".into(),
                true,
            ));
            return;
        };
        let request = SequenceRequest {
            preset: self.export_preset,
            quality: self.export_quality,
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
                let mut last_wake = None;
                let mut report = |fraction: f32| {
                    job_progress.store(fraction.to_bits(), Ordering::Relaxed);
                    if export_wake_due(&mut last_wake, Instant::now(), fraction >= 1.0) {
                        wake.request_repaint();
                    }
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
        if typing || self.export_dialog || egui::Popup::is_any_open(ui.ctx()) {
            return;
        }
        let pressed = |modifiers, key| ui.input_mut(|i| i.consume_key(modifiers, key));
        let none = egui::Modifiers::NONE;
        let command = egui::Modifiers::COMMAND;
        if pressed(none, egui::Key::Space) {
            self.toggle();
        }
        // Zoom the timeline: + and - by steps, 0 to fit the whole edit.
        if pressed(none, egui::Key::Plus) || pressed(none, egui::Key::Equals) {
            self.fit = false;
            self.zoom = (self.zoom * 1.5).min(160.0);
        }
        if pressed(none, egui::Key::Minus) {
            self.fit = false;
            self.zoom = (self.zoom / 1.5).max(4.0);
        }
        if pressed(none, egui::Key::Num0) {
            self.fit = true;
        }
        if pressed(none, egui::Key::Home) {
            self.seek(Tick::ZERO);
        }
        if pressed(none, egui::Key::End) {
            self.seek(self.session.duration());
        }
        if pressed(none, egui::Key::ArrowLeft) {
            self.step_frame(false);
        }
        if pressed(none, egui::Key::ArrowRight) {
            self.step_frame(true);
        }
        // The worker owns an immutable request. Preview transport remains available, while edits
        // and a second export stay blocked until the worker acknowledges completion/cancellation.
        if self.busy() {
            return;
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
    }
    pub fn show(&mut self, ui: &mut egui::Ui, thumbs: &mut crate::thumbs::Thumbnails) {
        // In a short window the page scrolls instead of squeezing the preview to a sliver.
        let view = ui.available_height();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| self.page(ui, thumbs, view));
    }
    fn page(&mut self, ui: &mut egui::Ui, thumbs: &mut crate::thumbs::Thumbnails, view: f32) {
        for (path, file) in std::mem::take(&mut self.imported_thumbs) {
            thumbs.set_file(path, file);
        }
        self.keys(ui);
        self.recovery_banner(ui);
        self.discard_dialog(ui.ctx());
        ui.horizontal(|ui| {
            ui.set_min_height(44.0);
            // The edit's name is the page heading: type over it to rename the edit. Left empty it
            // stays an unnamed edit.
            let title = ui.add(
                egui::TextEdit::singleline(&mut self.title)
                    .char_limit(100)
                    .hint_text("Unnamed edit")
                    .font(FontId::new(
                        crate::kit::text_size::TITLE + 6.0,
                        kit::semibold(),
                    ))
                    .frame(false)
                    .desired_width(360.0),
            );
            title.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Edit title")
            });
            title
                .clone()
                .on_hover_cursor(egui::CursorIcon::Text)
                .on_hover_text("Click to name this edit");
            if title.changed() {
                self.dirty_recovery();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if tool(
                    ui,
                    kit::Icon::Folder,
                    "Export video",
                    true,
                    !self.session.clips().is_empty() && !self.busy(),
                )
                .on_hover_text(
                    "Choose standard export or an estimated 10 MB, 25 MB, 50 MB, 500 MB or custom file size (10, 50 and 500 MB are what Discord takes).",
                )
                .clicked()
                {
                    self.export_dialog = true;
                }
                // Undo and Redo as one capsule, like the zoom controls under the preview.
                let (can_undo, can_redo) = (
                    self.session.can_undo() && !self.busy(),
                    self.session.can_redo() && !self.busy(),
                );
                let (mut undo, mut redo) = (false, false);
                egui::Frame::new()
                    .fill(kit::CONTROL)
                    .stroke(Stroke::new(1.0_f32, kit::CONTROL_BORDER))
                    .corner_radius((crate::kit::control::COMPACT / 2.0) as u8)
                    .inner_margin(egui::Margin::same(2))
                    .show(ui, |ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(116.0, crate::kit::control::COMPACT - 6.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                undo = timeline::segment_if(ui, "Undo", false, can_undo, 56.0)
                                    .on_hover_text("Undo (Ctrl+Z)")
                                    .clicked();
                                redo = timeline::segment_if(ui, "Redo", false, can_redo, 56.0)
                                    .on_hover_text("Redo (Ctrl+Shift+Z)")
                                    .clicked();
                            },
                        );
                    });
                if undo && can_undo {
                    self.edit(EditorAction::Undo);
                }
                if redo && can_redo {
                    self.edit(EditorAction::Redo);
                }
                // Keep cancellation reachable even when a short window clips the lower timeline.
                if let Some(job) = &self.job {
                    let cancelling = job.cancel.load(Ordering::Relaxed);
                    if button(
                        ui,
                        if cancelling {
                            "Cancelling…"
                        } else {
                            "Cancel export"
                        },
                        false,
                        !cancelling,
                    )
                    .on_hover_text(
                        "Stops after the current codec operation. Your source clips are unchanged.",
                    )
                    .clicked()
                    {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    ui.add(
                        egui::ProgressBar::new(f32::from_bits(
                            job.progress.load(Ordering::Relaxed),
                        ))
                        .desired_width(240.0)
                        .show_percentage(),
                    );
                }
            });
        });
        ui.add_space(crate::kit::space::MD);
        // The panel under the timeline for the selected segment takes room too.
        let segment_height = if self.selected.is_some() { 56.0 } else { 0.0 };
        let status_height = segment_height + if self.message.is_some() { 26.0 } else { 0.0 };
        let full_top = (view - 432.0 - status_height).max(MIN_TOP + 40.0);
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
            // The preview gets at least half the row, so it never looks small beside the Library.
            let preview = (picture_height * 16.0 / 9.0 + 24.0)
                .max(width * 0.5)
                .clamp(360.0, width - 12.0 - 260.0);
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
            ui.spacing_mut().item_spacing.x = crate::kit::space::MD;
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
                        .corner_radius(crate::kit::radius::LG)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_width((preview_width - 24.0).max(1.0));
                            ui.set_min_height((top - 16.0).max(0.0));
                            let size = Vec2::new(ui.available_width(), (top - 62.0).max(40.0));
                            let area = egui::Rect::from_min_size(ui.cursor().min, size);
                            let canvas = self.export_canvas().unwrap_or((1280, 720));
                            if let Some(preview) = self.preview.as_mut() {
                                preview.editor_picture(
                                    ui,
                                    size,
                                    Vec2::new(canvas.0 as f32, canvas.1 as f32),
                                );
                                self.paint_text_handles(ui, area);
                                self.paint_frame_badge(ui, area, canvas);
                            } else {
                                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                                ui.painter().rect_filled(
                                    rect,
                                    f32::from(crate::kit::radius::LG),
                                    Color32::BLACK,
                                );
                                kit::draw_icon(
                                    ui.painter(),
                                    rect.center() - Vec2::new(0.0, 34.0),
                                    kit::Icon::Scissors,
                                    kit::FAINT,
                                    1.6,
                                );
                                ui.painter().text(
                                    rect.center() + Vec2::new(0.0, 8.0),
                                    Align2::CENTER_CENTER,
                                    "Add clips to begin",
                                    FontId::new(crate::kit::text_size::LEAD, kit::medium()),
                                    MUTED,
                                );
                                // A child ui, so placing the button leaves the layout cursor alone.
                                let mut cta = ui.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(egui::Rect::from_center_size(
                                            rect.center() + Vec2::new(0.0, 56.0),
                                            Vec2::new(160.0, crate::kit::control::COMPACT),
                                        ))
                                        .layout(Layout::centered_and_justified(
                                            egui::Direction::LeftToRight,
                                        )),
                                );
                                if tool(
                                    &mut cta,
                                    kit::Icon::Import,
                                    "Import clips",
                                    true,
                                    self.importing.is_none(),
                                )
                                .clicked()
                                {
                                    self.picker = Some(ClipPicker::default());
                                }
                            }
                            ui.add_space(crate::kit::space::SM);
                            // Transport centred under the picture, the time at the right edge.
                            let (row, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 34.0),
                                Sense::hover(),
                            );
                            let any = !self.session.clips().is_empty();
                            let playing = self.playing;
                            ui.painter().circle_filled(
                                row.center(),
                                17.0,
                                if any { LIME } else { kit::CONTROL },
                            );
                            let hit = ui.interact(
                                egui::Rect::from_center_size(row.center(), Vec2::splat(34.0)),
                                egui::Id::new("editor-play"),
                                if any { Sense::click() } else { Sense::hover() },
                            );
                            hit.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    any,
                                    if playing { "Pause edit" } else { "Play edit" },
                                )
                            });
                            // The icon is dark on the lime circle, and quiet while there is nothing
                            // to play.
                            kit::draw_icon(
                                ui.painter(),
                                row.center(),
                                if playing {
                                    kit::Icon::Pause
                                } else {
                                    kit::Icon::Play
                                },
                                if any { Color32::BLACK } else { FAINT },
                                0.9,
                            );
                            if hit
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text(if playing { "Pause edit" } else { "Play edit" })
                                .clicked()
                                && any
                            {
                                self.toggle();
                            }
                            // Around play: a frame at a time, and from cut to cut.
                            let mut step = |dx: f32, id: &str, glyph: Glyph, tip: &str| {
                                transport_button(
                                    ui,
                                    row.center() + Vec2::new(dx, 0.0),
                                    id,
                                    glyph,
                                    false,
                                    any,
                                    tip,
                                )
                            };
                            let previous = step(
                                -88.0,
                                "cut-back",
                                Glyph::Icon(kit::Icon::StepBack),
                                "Previous cut",
                            );
                            let back = step(
                                -46.0,
                                "frame-back",
                                Glyph::Chevron(-1.0),
                                "Back one frame (\u{2190})",
                            );
                            let forward = step(
                                46.0,
                                "frame-forward",
                                Glyph::Chevron(1.0),
                                "Forward one frame (\u{2192})",
                            );
                            let next = step(
                                88.0,
                                "cut-forward",
                                Glyph::Icon(kit::Icon::StepForward),
                                "Next cut",
                            );
                            if transport_button(
                                ui,
                                egui::pos2(row.left() + 14.0, row.center().y),
                                "loop",
                                Glyph::Icon(kit::Icon::Loop),
                                self.looping,
                                any,
                                if self.looping {
                                    "Loop is on: playback starts over at the end"
                                } else {
                                    "Loop: start over at the end"
                                },
                            ) {
                                self.looping = !self.looping;
                            }
                            // The shape of the picture: a chip that opens the shape and framing choices.
                            if row.width() >= 460.0 {
                                let label = frame_label(self.export_quality);
                                let width = ui
                                    .painter()
                                    .layout_no_wrap(
                                        label.clone(),
                                        FontId::new(crate::kit::text_size::CAPTION, kit::medium()),
                                        Color32::PLACEHOLDER,
                                    )
                                    .size()
                                    .x
                                    + 34.0;
                                let chip = egui::Rect::from_min_size(
                                    egui::pos2(row.left() + 34.0, row.center().y - 13.0),
                                    Vec2::new(width, 26.0),
                                );
                                let response = ui.interact(
                                    chip,
                                    egui::Id::new("editor-frame-chip"),
                                    if any { Sense::click() } else { Sense::hover() },
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        any,
                                        "Picture shape",
                                    )
                                });
                                let open = egui::Popup::is_id_open(
                                    ui.ctx(),
                                    egui::Popup::default_response_id(&response),
                                );
                                ui.painter().rect_filled(
                                    chip,
                                    13.0,
                                    if open || response.hovered() {
                                        kit::CONTROL_HOVER
                                    } else {
                                        kit::CONTROL
                                    },
                                );
                                ui.painter().text(
                                    egui::pos2(chip.left() + 11.0, chip.center().y),
                                    Align2::LEFT_CENTER,
                                    label,
                                    FontId::new(crate::kit::text_size::CAPTION, kit::medium()),
                                    if any { TEXT } else { FAINT },
                                );
                                kit::draw_icon(
                                    ui.painter(),
                                    egui::pos2(chip.right() - 12.0, chip.center().y),
                                    kit::Icon::ChevronDown,
                                    MUTED,
                                    0.5,
                                );
                                let before = self.export_quality;
                                if any {
                                    egui::Popup::menu(&response)
                                        .close_behavior(
                                            egui::PopupCloseBehavior::CloseOnClickOutside,
                                        )
                                        .show(|ui| frame_menu(ui, &mut self.export_quality));
                                }
                                response
                                    .on_hover_text("Picture shape: 16:9, 9:16 and more")
                                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                                if self.export_quality != before {
                                    self.dirty_recovery();
                                }
                            }
                            if previous {
                                self.previous_cut();
                            }
                            if back {
                                self.step_frame(false);
                            }
                            if forward {
                                self.step_frame(true);
                            }
                            if next {
                                self.next_cut();
                            }
                            // The time passed in the text colour, the total beside it quieter.
                            let total = ui.painter().layout_no_wrap(
                                format!(" / {}", stamp_precise(self.session.duration())),
                                FontId::monospace(13.0),
                                MUTED,
                            );
                            let passed = ui.painter().layout_no_wrap(
                                stamp_precise(self.playhead),
                                FontId::monospace(13.0),
                                if any { TEXT } else { MUTED },
                            );
                            let left = row.right() - total.size().x - passed.size().x;
                            let top = row.center().y - passed.size().y / 2.0;
                            ui.painter()
                                .galley(egui::pos2(left, top), passed.clone(), TEXT);
                            ui.painter().galley(
                                egui::pos2(left + passed.size().x, top),
                                total,
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
            .corner_radius(crate::kit::radius::LG)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width((width - 24.0).max(1.0));
                self.toolbar(ui);
                self.timeline(ui, thumbs, lift);
                self.range_controls(ui);
            });
        if let Some((text, error)) = self.message.clone() {
            let kind = crate::notify::Kind::from_message(&text, error);
            if crate::notify::banner(ui, kind, &text, true) {
                self.message = None;
            }
        }
        ui.add_space(crate::kit::space::XS);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = crate::kit::space::SM;
            for (key, what) in [
                ("Space", "play"),
                ("S", "split"),
                ("\u{2190} \u{2192}", "frame"),
                ("Home", "start"),
                ("End", "end"),
                ("Del", "delete"),
                ("Ctrl Z", "undo"),
            ] {
                kit::keycap(ui, key);
                ui.label(
                    RichText::new(what)
                        .size(crate::kit::text_size::CAPTION)
                        .color(MUTED),
                );
                ui.add_space(crate::kit::space::SM);
            }
            self.recovery_footer(ui);
        });
        self.show_export_dialog(ui.ctx());
    }
    /// A label on the viewer that says which shape and size the edit is being made in, so it is
    /// never a guess from the black bars.
    fn paint_frame_badge(&self, ui: &egui::Ui, area: egui::Rect, canvas: (u32, u32)) {
        let shape = self.export_quality.aspect;
        let name = if shape == clipcraft_engine::SequenceAspect::Source {
            "Original".to_owned()
        } else {
            shape.label()
        };
        let text = format!("{name}  \u{00b7}  {} \u{00d7} {}", canvas.0, canvas.1);
        let galley = ui.painter().layout_no_wrap(
            text,
            FontId::proportional(crate::kit::text_size::CAPTION),
            MUTED,
        );
        let icon = 12.0;
        let size = Vec2::new(galley.size().x + icon + 26.0, 22.0);
        let frame = crate::playback::fit(Vec2::new(canvas.0 as f32, canvas.1 as f32), area);
        let origin = if frame.width() >= size.x + 16.0 {
            frame.min + Vec2::splat(8.0)
        } else {
            area.min + Vec2::splat(6.0)
        };
        let pill = egui::Rect::from_min_size(origin, size);
        ui.painter()
            .rect_filled(pill, 11.0, Color32::from_black_alpha(150));
        // A tiny drawing of the shape at its own proportions.
        let (across, down) = shape
            .ratio()
            .map_or((canvas.0 as f32, canvas.1 as f32), |(a, d)| {
                (a as f32, d as f32)
            });
        let longest = across.max(down).max(1.0);
        let drawn = Vec2::new(
            (icon * across / longest).max(5.0),
            (icon * down / longest).max(5.0),
        );
        ui.painter().rect_stroke(
            egui::Rect::from_center_size(egui::pos2(pill.left() + 14.0, pill.center().y), drawn),
            1.5,
            Stroke::new(1.2_f32, MUTED),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            egui::pos2(
                pill.left() + 14.0 + icon / 2.0 + 7.0,
                pill.center().y - galley.size().y / 2.0,
            ),
            galley,
            MUTED,
        );
    }
    fn export_canvas(&self) -> Result<(u32, u32), String> {
        self.export_settings().map(|s| (s.width, s.height))
    }
    fn export_settings(&self) -> Result<SequenceSettings, String> {
        let first = self.session.clips().first();
        let probe = first
            .and_then(|clip| {
                self.media
                    .iter()
                    .find(|m| m.path == clip.input.to_string_lossy())
            })
            .and_then(|m| m.probe.as_ref());
        clipcraft_engine::sequence_settings(
            probe.and_then(|p| p.width).unwrap_or(1280),
            probe.and_then(|p| p.height).unwrap_or(720),
            self.export_preset,
            self.session.duration(),
            self.export_quality,
        )
        .map_err(|error| error.to_string())
    }
    /// The file the export would write right now: the edit's title (or the first clip's name) in the
    /// chosen folder, or beside the first clip, with a free name.
    fn export_destination(&self) -> Option<PathBuf> {
        let first = self.session.clips().first()?;
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
        let file = if name.is_empty() {
            PathBuf::from(first.input.file_name()?)
        } else {
            PathBuf::from(format!("{name}.mp4"))
        };
        let base = match &self.export_folder {
            Some(folder) => folder.join(file),
            None => first.input.with_file_name(file),
        };
        clipcraft_engine::edited_path(&base)
    }
    fn choose_export_folder(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Save the exported video in");
        let current = self
            .export_destination()
            .and_then(|path| path.parent().map(PathBuf::from));
        if let Some(folder) = current.filter(|folder| folder.is_dir()) {
            dialog = dialog.set_directory(folder);
        }
        if let Some(folder) = dialog.pick_folder() {
            self.export_folder = Some(folder);
        }
    }
    fn show_export_dialog(&mut self, context: &egui::Context) {
        if !self.export_dialog {
            return;
        }
        if context.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.export_dialog = false;
            return;
        }
        let mut start = false;
        let mut cancel = false;
        let mut change_folder = false;
        let mut reset_folder = false;
        let previous_preset = self.export_preset;
        let previous_quality = self.export_quality;
        let destination = self.export_destination();
        let window = egui::Window::new("Export video")
            .id(egui::Id::new("editor-export-window"))
            .title_bar(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .fixed_size(Vec2::new(500.0, 0.0))
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(egui::CornerRadius::same(crate::kit::radius::LG))
                    .inner_margin(egui::Margin::same(24))
                    .shadow(egui::Shadow {
                        offset: [0, 8],
                        blur: 32,
                        spread: 0,
                        color: Color32::from_black_alpha(140),
                    }),
            )
            .show(context, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                ui.label(
                    RichText::new("Export video")
                        .size(crate::kit::text_size::TITLE)
                        .family(kit::semibold())
                        .color(TEXT),
                );
                ui.label(
                    RichText::new("Choose where it goes, how big the file is and how it looks.")
                        .size(crate::kit::text_size::SMALL)
                        .color(MUTED),
                );
                ui.add_space(crate::kit::space::SM);

                // Where the file goes.
                export_heading(ui, "Save to");
                egui::Frame::new()
                    .fill(kit::CARD)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(egui::CornerRadius::same(crate::kit::radius::MD))
                    .inner_margin(egui::Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let (icon, _) =
                                ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                            kit::draw_icon(ui.painter(), icon.center(), kit::Icon::Folder, LIME, 0.9);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                match &destination {
                                    Some(path) => {
                                        let folder = path
                                            .parent()
                                            .map(|p| {
                                                clipcraft_engine::plain_path(p.to_path_buf())
                                                    .to_string_lossy()
                                                    .into_owned()
                                            })
                                            .unwrap_or_default();
                                        let file = path
                                            .file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_default();
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(file)
                                                    .family(kit::semibold())
                                                    .size(crate::kit::text_size::BODY)
                                                    .color(TEXT),
                                            )
                                            .truncate(),
                                        );
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(shorten_middle(&folder, 58))
                                                    .size(crate::kit::text_size::SMALL)
                                                    .color(MUTED),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(folder);
                                    }
                                    None => {
                                        crate::notify::inline(
                                            ui,
                                            crate::notify::Kind::Error,
                                            "No free file name here",
                                        );
                                    }
                                }
                            });
                        });
                        ui.add_space(crate::kit::space::SM);
                        ui.horizontal(|ui| {
                            change_folder =
                                kit::labelled_button(ui, kit::Icon::Folder, "Change folder", false, true)
                                    .clicked();
                            if self.export_folder.is_some() {
                                reset_folder = button(ui, "Use the clip's folder", false, true).clicked();
                            }
                        });
                        ui.label(
                            RichText::new(if self.export_folder.is_some() {
                                "The name comes from the edit's title. If it is taken, a number is added."
                            } else {
                                "Saved beside the first clip. The name comes from the edit's title; if it is taken, a number is added."
                            })
                            .size(crate::kit::text_size::CAPTION)
                            .color(kit::FAINT),
                        );
                    });

                // File size.
                export_heading(ui, "File size");
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                    for (value, label) in
                        kit::export_size_choices(self.export_preset, "Standard quality")
                    {
                        let short = match value {
                            None => "Standard",
                            Some(ExportPreset::Small) => "10 MB",
                            Some(ExportPreset::Medium) => "25 MB",
                            Some(ExportPreset::Large) => "50 MB",
                            Some(ExportPreset::Largest) => "500 MB",
                            Some(ExportPreset::Custom { .. }) => "Custom",
                        };
                        let on = std::mem::discriminant(&value)
                            == std::mem::discriminant(&self.export_preset);
                        if tab_pill(ui, short, on).on_hover_text(label).clicked() {
                            self.export_preset = value;
                        }
                    }
                });
                if let Some(ExportPreset::Custom { megabytes }) = &mut self.export_preset {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Target size").color(MUTED));
                        ui.add(egui::DragValue::new(megabytes).range(1..=500).suffix(" MB"))
                            .on_hover_text("Estimated output size, from 1 to 500 MB");
                    });
                }

                // How it looks.
                export_heading(ui, "Picture");
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    export_label(ui, "Quality");
                    for (value, name) in [
                        (SequenceResolution::Hd720, "720p"),
                        (SequenceResolution::FullHd1080, "1080p"),
                        (SequenceResolution::Source, "Source"),
                    ] {
                        let on = self.export_quality.resolution == value;
                        if tab_pill(ui, name, on).clicked() {
                            self.export_quality.resolution = value;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    export_label(ui, "Frame rate");
                    for fps in [30, 60] {
                        let on = self.export_quality.fps == fps;
                        if tab_pill(ui, &format!("{fps} FPS"), on).clicked() {
                            self.export_quality.fps = fps;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    export_label(ui, "Shape");
                    egui::ComboBox::from_id_salt("editor-export-shape")
                        .selected_text(self.export_quality.aspect.label())
                        .show_ui(ui, |ui| {
                            for aspect in clipcraft_engine::SequenceAspect::PRESETS {
                                ui.selectable_value(
                                    &mut self.export_quality.aspect,
                                    aspect,
                                    aspect.label(),
                                );
                            }
                        });
                    egui::ComboBox::from_id_salt("editor-export-framing")
                        .selected_text(self.export_quality.framing.name())
                        .show_ui(ui, |ui| {
                            for framing in clipcraft_engine::SequenceFraming::ALL {
                                ui.selectable_value(
                                    &mut self.export_quality.framing,
                                    framing,
                                    framing.name(),
                                )
                                .on_hover_text(framing.about());
                            }
                        });
                });

                // What will be written.
                let settings = self.export_settings();
                ui.add_space(crate::kit::space::XS);
                egui::Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(190, 255, 43, 14))
                    .stroke(Stroke::new(
                        1.0_f32,
                        Color32::from_rgba_unmultiplied(190, 255, 43, 60),
                    ))
                    .corner_radius(egui::CornerRadius::same(crate::kit::radius::MD))
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        match &settings {
                            Ok(settings) => {
                                ui.label(
                                    RichText::new(format!(
                                        "{} \u{00d7} {}  \u{00b7}  {} FPS  \u{00b7}  H.264 MP4",
                                        settings.width, settings.height, settings.fps
                                    ))
                                    .family(kit::semibold())
                                    .size(crate::kit::text_size::BODY)
                                    .color(TEXT),
                                );
                                let mut notes = vec![format!(
                                    "Video target {} kbps, stereo AAC audio. Sizes are estimates.",
                                    settings.video_kbps
                                )];
                                if settings.resolution_limited {
                                    notes.push(
                                        "Resolution reduced to fit the file-size target.".into(),
                                    );
                                } else if settings.bitrate_limited {
                                    notes.push(
                                        "Video bitrate reduced to fit the file-size target.".into(),
                                    );
                                }
                                for note in notes {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(note)
                                                .size(crate::kit::text_size::CAPTION)
                                                .color(MUTED),
                                        )
                                        .wrap(),
                                    );
                                }
                            }
                            Err(error) => {
                                crate::notify::inline(ui, crate::notify::Kind::Error, error);
                            }
                        }
                    });

                ui.add_space(crate::kit::space::SM);
                let can_export =
                    settings.is_ok() && destination.is_some() && self.has_clips() && !self.busy();
                egui::Sides::new().show(
                    ui,
                    |ui| {
                        cancel = kit::labelled_button(ui, kit::Icon::ChevronLeft, "Cancel", false, true)
                            .clicked();
                    },
                    |ui| {
                        start = kit::labelled_button(ui, kit::Icon::Import, "Export", true, can_export)
                            .clicked();
                    },
                );
            });
        if let Some(window) = window {
            // Keep keyboard navigation in the dialog and its popups, not the editor behind it.
            context.memory_mut(|memory| memory.set_modal_layer(window.response.layer_id));
        }
        if change_folder {
            self.choose_export_folder();
            context.request_repaint();
        }
        if reset_folder {
            self.export_folder = None;
        }
        self.export_dialog = !cancel && !start;
        if self.export_preset != previous_preset || self.export_quality != previous_quality {
            self.dirty_recovery();
        }
        if start {
            self.export(context);
        }
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

    fn browser(&mut self, ui: &mut egui::Ui, thumbs: &mut crate::thumbs::Thumbnails, height: f32) {
        let media = Arc::clone(&self.media);
        let mut add = None;
        let mut details = None;
        egui::Frame::new()
            .fill(PANEL)
            .corner_radius(crate::kit::radius::LG)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_min_height((height - 24.0).max(0.0));
                // Every tab fills the panel, so the divider never moves when the tab changes.
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = crate::kit::space::SM;
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
                            ui.spacing_mut().item_spacing.x = crate::kit::space::SM;
                            if tool(
                                ui,
                                kit::Icon::Import,
                                "Import clips",
                                false,
                                self.importing.is_none(),
                            )
                            .clicked()
                            {
                                self.picker = Some(ClipPicker::default());
                            }
                            kit::help(
                                ui,
                                "Double-click a clip, or drag it onto the timeline, to add it. Right-click for details.",
                            );
                        });
                    }
                });
                ui.add_space(crate::kit::space::MD);
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
                self.library_list(ui, thumbs, height, &media, &mut add, &mut details);
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

/// What the frame chip says: the shape, and how a clip that is not that shape is framed.
fn frame_label(quality: SequenceQuality) -> String {
    let shape = quality.aspect.label();
    if quality.framing == clipcraft_engine::SequenceFraming::Fit {
        shape
    } else {
        format!("{shape} \u{00b7} {}", quality.framing.name())
    }
}

/// The choices behind the frame chip: the shape of the picture and how a clip is framed in it.
fn frame_menu(ui: &mut egui::Ui, quality: &mut SequenceQuality) {
    use clipcraft_engine::{SequenceAspect, SequenceFraming};
    ui.set_min_width(268.0);
    ui.set_max_width(268.0);
    ui.label(
        RichText::new("Shape")
            .size(crate::kit::text_size::CAPTION)
            .family(kit::semibold())
            .color(MUTED),
    );
    ui.add_space(crate::kit::space::XS);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        for aspect in SequenceAspect::PRESETS {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(38.0, 50.0), Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, aspect.label())
            });
            let on = quality.aspect == aspect;
            if on {
                ui.painter().rect_filled(
                    rect,
                    f32::from(crate::kit::radius::MD),
                    Color32::from_rgba_unmultiplied(190, 255, 43, 24),
                );
            } else if response.hovered() {
                ui.painter().rect_filled(
                    rect,
                    f32::from(crate::kit::radius::MD),
                    Color32::from_white_alpha(12),
                );
            }
            // A small picture of the shape, at its own proportions.
            let (across, down) = aspect.ratio().unwrap_or((16, 9));
            let longest = across.max(down) as f32;
            let size = Vec2::new(22.0 * across as f32 / longest, 22.0 * down as f32 / longest);
            let shape = egui::Rect::from_center_size(
                egui::pos2(rect.center().x, rect.top() + 18.0),
                size.max(Vec2::splat(8.0)),
            );
            let colour = if on { LIME } else { MUTED };
            if aspect == SequenceAspect::Source {
                ui.painter().rect_stroke(
                    shape,
                    2.0,
                    Stroke::new(1.3_f32, colour),
                    egui::StrokeKind::Inside,
                );
                ui.painter().line_segment(
                    [shape.left_bottom(), shape.right_top()],
                    Stroke::new(1.0_f32, colour),
                );
            } else {
                ui.painter().rect_stroke(
                    shape,
                    2.0,
                    Stroke::new(if on { 2.0_f32 } else { 1.3_f32 }, colour),
                    egui::StrokeKind::Inside,
                );
            }
            ui.painter().text(
                egui::pos2(rect.center().x, rect.bottom() - 9.0),
                Align2::CENTER_CENTER,
                if aspect == SequenceAspect::Source {
                    "Original".to_owned()
                } else {
                    aspect.label()
                },
                FontId::new(10.5, kit::medium()),
                if on { TEXT } else { MUTED },
            );
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                quality.aspect = aspect;
            }
        }
    });
    // Any other shape, as two whole numbers.
    ui.add_space(crate::kit::space::SM);
    let (mut across, mut down) = quality.aspect.ratio().map_or((16_u16, 9_u16), |(a, d)| {
        (
            u16::try_from(a).unwrap_or(16),
            u16::try_from(d).unwrap_or(9),
        )
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = crate::kit::space::SM;
        ui.label(
            RichText::new("Custom")
                .size(crate::kit::text_size::SMALL)
                .color(MUTED),
        );
        let moved = ui
            .add(egui::DragValue::new(&mut across).range(1..=99).speed(0.1))
            .changed()
            | {
                ui.label(RichText::new(":").color(MUTED));
                ui.add(egui::DragValue::new(&mut down).range(1..=99).speed(0.1))
                    .changed()
            };
        let wanted = SequenceAspect::Ratio(across, down);
        if moved && wanted.is_valid() {
            quality.aspect = wanted;
        } else if !wanted.is_valid() && quality.aspect.ratio().is_some() {
            crate::notify::inline(ui, crate::notify::Kind::Error, "Between 1:4 and 4:1");
        }
    });
    ui.add_space(crate::kit::space::MD);
    ui.label(
        RichText::new("When a clip is another shape")
            .size(crate::kit::text_size::CAPTION)
            .family(kit::semibold())
            .color(MUTED),
    );
    ui.add_space(crate::kit::space::XS);
    let names: Vec<&str> = SequenceFraming::ALL.iter().map(|f| f.name()).collect();
    let current = SequenceFraming::ALL
        .iter()
        .position(|f| *f == quality.framing)
        .unwrap_or(0);
    if let Some(index) = sub_tabs(ui, &names, current) {
        quality.framing = SequenceFraming::ALL[index];
    }
    ui.add_space(crate::kit::space::XS);
    ui.label(
        RichText::new(quality.framing.about())
            .size(crate::kit::text_size::CAPTION)
            .color(MUTED),
    );
}

/// What a transport button shows.
#[derive(Clone, Copy)]
enum Glyph {
    Icon(kit::Icon),
    /// A chevron pointing left (-1) or right (1).
    Chevron(f32),
}

/// A small round-cornered icon button centred on `centre`, placed without moving the layout cursor.
/// Lime while it is switched on.
fn transport_button(
    ui: &mut egui::Ui,
    centre: egui::Pos2,
    id: &str,
    glyph: Glyph,
    on: bool,
    enabled: bool,
    tip: &str,
) -> bool {
    let rect = egui::Rect::from_center_size(centre, Vec2::splat(30.0));
    let response = ui.interact(
        rect,
        egui::Id::new(("editor-transport", id)),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tip));
    if response.hovered() && enabled {
        ui.painter().rect_filled(
            rect,
            f32::from(crate::kit::radius::MD),
            Color32::from_white_alpha(16),
        );
    }
    let colour = if !enabled {
        FAINT
    } else if on {
        LIME
    } else if response.hovered() {
        TEXT
    } else {
        MUTED
    };
    match glyph {
        Glyph::Icon(icon) => kit::draw_icon(ui.painter(), centre, icon, colour, 0.8),
        // A plain chevron: one frame at a time.
        Glyph::Chevron(way) => {
            let stroke = Stroke::new(2.0_f32, colour);
            let tip = centre + Vec2::new(4.0 * way, 0.0);
            let wing = Vec2::new(-5.0 * way, 6.0);
            ui.painter().line_segment([tip + wing, tip], stroke);
            ui.painter()
                .line_segment([tip, tip + Vec2::new(wing.x, -wing.y)], stroke);
        }
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip)
        .clicked()
        && enabled
}

/// `text` cut to at most `max` characters by replacing the middle with an ellipsis, so both the
/// start of a long path and the folder it ends in stay readable.
fn shorten_middle(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max || max < 8 {
        return text.to_owned();
    }
    let keep = max - 1;
    let head = keep / 3;
    let tail = keep - head;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}\u{2026}{end}")
}

/// A small heading above a group of export choices.
fn export_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(crate::kit::space::XS);
    ui.label(
        RichText::new(text.to_uppercase())
            .size(crate::kit::text_size::CAPTION)
            .family(kit::semibold())
            .color(kit::FAINT),
    );
}

/// The name of a row of export choices, at a fixed width so the choices line up.
fn export_label(ui: &mut egui::Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(84.0, crate::kit::control::COMPACT),
        Sense::hover(),
    );
    ui.painter().text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        text,
        FontId::new(crate::kit::text_size::SMALL, kit::medium()),
        MUTED,
    );
}
/// A pill tab like the ones above the Library.
fn tab_pill(ui: &mut egui::Ui, label: &str, active: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::new(crate::kit::text_size::SMALL, kit::medium()),
        Color32::PLACEHOLDER,
    );
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(galley.size().x + 28.0, crate::kit::control::COMPACT),
        Sense::click(),
    );
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
        FontId::new(crate::kit::text_size::SMALL, kit::medium()),
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
    // Rounded corners: the picture is painted square, so the corners are filled with the panel.
    let corner = f32::from(crate::kit::radius::MD);
    round_corners(ui.painter(), picture, corner, PANEL);
    let outline = if active {
        LIME
    } else if response.hovered() && enabled {
        MUTED
    } else {
        BORDER
    };
    ui.painter().rect_stroke(
        picture,
        corner,
        Stroke::new(if active { 2.0_f32 } else { 1.0_f32 }, outline),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 2.0, rect.bottom() - 2.0),
        Align2::LEFT_BOTTOM,
        name,
        FontId::new(crate::kit::text_size::CAPTION, kit::medium()),
        if !enabled {
            FAINT
        } else if active {
            LIME
        } else {
            TEXT
        },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Fills the four corners of `rect` outside a rounded rectangle of radius `radius` with `colour`,
/// to round off something that was painted square.
fn round_corners(painter: &egui::Painter, rect: egui::Rect, radius: f32, colour: Color32) {
    const STEPS: usize = 8;
    for (corner, sx, sy) in [
        (rect.left_top(), 1.0_f32, 1.0_f32),
        (rect.right_top(), -1.0, 1.0),
        (rect.left_bottom(), 1.0, -1.0),
        (rect.right_bottom(), -1.0, -1.0),
    ] {
        let centre = corner + Vec2::new(sx * radius, sy * radius);
        let mut mesh = egui::epaint::Mesh::default();
        mesh.colored_vertex(corner, colour);
        for step in 0..=STEPS {
            let turn = step as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
            mesh.colored_vertex(
                centre + Vec2::new(-sx * radius * turn.sin(), -sy * radius * turn.cos()),
                colour,
            );
        }
        for step in 0..STEPS as u32 {
            mesh.add_triangle(0, step + 1, step + 2);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// Quiet or lime text button. Standard egui buttons keep keyboard activation and the app's
/// focus/hover states; the colours match the rest of the app.
fn button(ui: &mut egui::Ui, label: &str, primary: bool, enabled: bool) -> egui::Response {
    let text = RichText::new(label)
        .family(kit::medium())
        .size(crate::kit::text_size::SMALL);
    let mut button = egui::Button::new(if primary && enabled {
        text.color(Color32::BLACK)
    } else {
        text
    })
    .corner_radius((crate::kit::control::COMPACT / 2.0) as u8)
    .min_size(Vec2::splat(crate::kit::control::COMPACT));
    if primary && enabled {
        button = button.fill(LIME).stroke(Stroke::NONE);
    } else {
        button = button
            .fill(kit::CONTROL)
            .stroke(Stroke::new(1.0_f32, kit::CONTROL_BORDER));
    }
    // Editor buttons are compact: as tall as the tab pills and tools next to them.
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = crate::kit::control::COMPACT;
        ui.spacing_mut().button_padding.y = crate::kit::space::XS;
        ui.add_enabled(enabled, button)
    })
    .inner
}

/// A button with an icon, in the same pill shape as the rest of the app.
/// The "add clips to the edit" box: the clips in the library, with a way to pick other files.
#[derive(Default)]
struct ClipPicker {
    search: String,
    chosen: std::collections::BTreeSet<String>,
}

impl EditorState {
    /// Shows the box for adding clips while it is open. `library` is what the Clips page shows.
    pub fn clip_picker(
        &mut self,
        context: &egui::Context,
        library: Option<&clipcraft_engine::LibraryScanResult>,
    ) {
        let Some(mut picker) = self.picker.take() else {
            return;
        };
        let mut close = false;
        let mut browse = false;
        let mut add = false;
        let modal = egui::Modal::new(egui::Id::new("editor-clip-picker"))
            .frame(crate::kit::card_frame().inner_margin(egui::Margin::same(20)))
            .show(context, |ui| {
                ui.set_width(520.0);
                ui.label(
                    RichText::new("Add clips to your edit")
                        .color(TEXT)
                        .size(crate::kit::text_size::LEAD)
                        .family(kit::semibold()),
                );
                ui.label(
                    RichText::new(
                        "Tick the clips from your library, or browse for files elsewhere.",
                    )
                    .color(MUTED)
                    .size(crate::kit::text_size::SMALL),
                );
                ui.add_space(crate::kit::space::MD);
                ui.add(
                    egui::TextEdit::singleline(&mut picker.search)
                        .hint_text("Search clips")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(crate::kit::space::SM);
                let query = picker.search.trim().to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let mut shown = 0;
                        for entry in library.map(|l| l.entries.as_slice()).unwrap_or_default() {
                            if entry.offline
                                || (!query.is_empty()
                                    && !entry.title.to_lowercase().contains(&query))
                            {
                                continue;
                            }
                            shown += 1;
                            let mut ticked = picker.chosen.contains(&entry.path);
                            let label = format!(
                                "{}   {}",
                                entry.title,
                                clipcraft_engine::format_bytes(entry.size_bytes)
                            );
                            if ui.checkbox(&mut ticked, label).changed() {
                                if ticked {
                                    picker.chosen.insert(entry.path.clone());
                                } else {
                                    picker.chosen.remove(&entry.path);
                                }
                            }
                        }
                        if shown == 0 {
                            ui.label(
                                RichText::new("No clips in your library match.")
                                    .color(MUTED)
                                    .size(crate::kit::text_size::SMALL),
                            );
                        }
                    });
                ui.add_space(crate::kit::space::LG);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = crate::kit::space::MD;
                    let count = picker.chosen.len();
                    let label = if count == 0 {
                        "Add clips".to_owned()
                    } else {
                        format!("Add {count} clip{}", if count == 1 { "" } else { "s" })
                    };
                    if tool(ui, kit::Icon::Plus, &label, true, count > 0).clicked() {
                        add = true;
                    }
                    if tool(
                        ui,
                        kit::Icon::Folder,
                        "Browse other files\u{2026}",
                        false,
                        true,
                    )
                    .clicked()
                    {
                        browse = true;
                    }
                    if tool(ui, kit::Icon::Restart, "Cancel", false, true).clicked() {
                        close = true;
                    }
                });
            });
        if modal.should_close() {
            close = true;
        }
        if add {
            let paths: Vec<PathBuf> = picker.chosen.iter().map(PathBuf::from).collect();
            self.import(Some(paths), context);
        } else if browse {
            self.import(None, context);
        } else if !close {
            self.picker = Some(picker);
        }
    }
}

fn tool(
    ui: &mut egui::Ui,
    icon: kit::Icon,
    label: &str,
    primary: bool,
    enabled: bool,
) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::new(crate::kit::text_size::SMALL, kit::medium()),
        Color32::PLACEHOLDER,
    );
    let size = Vec2::new(galley.size().x + 44.0, crate::kit::control::COMPACT);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let hovered = response.hovered() && enabled;
    let (fill, content) = match (primary, enabled, hovered) {
        (_, false, _) => (CARD, Color32::from_gray(112)),
        (true, _, _) => (LIME, Color32::BLACK),
        (false, _, true) => (kit::CONTROL_HOVER, kit::TEXT),
        (false, _, false) => (kit::CONTROL, kit::TEXT),
    };
    ui.painter()
        .rect_filled(rect, crate::kit::control::COMPACT / 2.0, fill);
    kit::draw_icon(
        ui.painter(),
        egui::pos2(rect.left() + 15.0, rect.center().y),
        icon,
        content,
        0.55,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 30.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::new(crate::kit::text_size::SMALL, kit::medium()),
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

/// Stable, whole-second analysis windows are clipped to the actual source in/out points.
/// Display coordinates select a view only; the edit's integer boundaries never change here.
fn timeline_window(
    start: Tick,
    end: Tick,
    card: egui::Rect,
    viewport: egui::Rect,
) -> Option<(Tick, Tick, u16, u16)> {
    let visible = card.intersect(viewport);
    if visible.width() < 8.0 || card.width() <= 0.0 || end <= start {
        return None;
    }
    let second = clipcraft_time::TICKS_PER_SECOND;
    let span = end.0 - start.0;
    let fraction = |x: f32| f64::from(((x - card.left()) / card.width()).clamp(0.0, 1.0));
    let first = start.0 + (span as f64 * fraction(visible.left())) as i64;
    let last = start.0 + (span as f64 * fraction(visible.right())).ceil() as i64;
    let a = Tick((first / second * second).max(start.0));
    let b = Tick(((last.saturating_add(second - 1) / second) * second).min(end.0));
    if !(second / 30..=second * 1800).contains(&(b.0 - a.0)) {
        return None;
    }
    let frames = (visible.width() / (card.height() * 16.0 / 9.0).max(1.0))
        .ceil()
        .clamp(1.0, 12.0) as u16;
    let buckets = ((visible.width() / 128.0).ceil() * 64.0).clamp(16.0, 2048.0) as u16;
    Some((a, b, frames, buckets))
}
fn source_x(card: egui::Rect, start: Tick, end: Tick, at: f64) -> f32 {
    card.left() + card.width() * ((at - start.0 as f64) / (end.0 - start.0).max(1) as f64) as f32
}
fn cover_uv(size: Vec2, aspect: f32) -> egui::Rect {
    let ratio = size.x / size.y.max(1.0);
    let (w, h) = if ratio > aspect {
        (1.0, aspect / ratio)
    } else {
        (ratio / aspect, 1.0)
    };
    egui::Rect::from_center_size(egui::pos2(0.5, 0.5), Vec2::new(w, h))
}

/// Minutes, seconds and hundredths: fine enough to see a frame go by at 60 FPS.
fn stamp_precise(time: Tick) -> String {
    let ms = time.to_units_floor(1000).max(0);
    format!("{}:{:02}.{:02}", ms / 60_000, ms / 1000 % 60, ms / 10 % 100)
}

fn stamp(time: Tick) -> String {
    let ms = time.to_units_floor(1000).max(0);
    format!("{}:{:02}.{:01}", ms / 60_000, ms / 1000 % 60, ms / 100 % 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_paths_keep_their_start_and_end() {
        assert_eq!(shorten_middle("C:\\short", 58), "C:\\short");
        let long = format!("C:\\Users\\someone\\{}\\clips", "deep\\".repeat(30));
        let cut = shorten_middle(&long, 58);
        assert_eq!(cut.chars().count(), 58);
        assert!(cut.starts_with("C:\\Users"));
        assert!(cut.ends_with("\\clips"));
        assert!(cut.contains('\u{2026}'));
        // Multi-byte characters are counted, never split.
        assert_eq!(shorten_middle(&"é".repeat(100), 20).chars().count(), 20);
    }

    #[test]
    fn export_progress_wakes_are_bounded_and_completion_is_immediate() {
        let start = Instant::now();
        let mut last = None;
        let wakes = (0..1000)
            .filter(|ms| export_wake_due(&mut last, start + Duration::from_millis(*ms), false))
            .count();
        assert_eq!(wakes, 10);
        assert!(export_wake_due(
            &mut last,
            start + Duration::from_millis(999),
            true
        ));
        assert!(!export_wake_due(
            &mut last,
            start + Duration::from_millis(1000),
            false
        ));
    }

    #[test]
    fn short_editor_keeps_export_cancellation_visible_and_clickable() {
        let context = crate::kit::test_context();
        context.enable_accesskit();
        let mut state = state();
        let (_sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        state.job = Some(ExportJob {
            receiver,
            cancel: cancel.clone(),
            progress: Arc::new(AtomicU32::new(0)),
        });
        // The editor's space inside a 960x640 desktop window, after rail/top bar/margins.
        let size = Vec2::new(858.0, 554.0);
        let mut bounds = None;
        for _ in 0..3 {
            if let Some(update) = frame(&mut state, &context, vec![], size)
                .platform_output
                .accesskit_update
            {
                for (_, node) in update.nodes {
                    if node.label() == Some("Cancel export") {
                        bounds = node.bounds();
                    }
                }
            }
        }
        let bounds = bounds.expect("cancel must have an accessible target");
        assert!(
            bounds.y0 >= 0.0 && bounds.y1 <= f64::from(size.y),
            "cancel clipped: {bounds:?}"
        );
        assert!(bounds.x0 >= 0.0 && bounds.x1 <= f64::from(size.x));
        let pos = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        frame(
            &mut state,
            &context,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            size,
        );
        assert!(
            cancel.load(Ordering::Relaxed),
            "the visible target must accept a cancel click"
        );
        assert!(
            state.busy(),
            "the worker must still acknowledge cancellation"
        );
    }

    #[test]
    fn export_keeps_keyboard_transport_and_blocks_mutations_until_worker_acknowledges() {
        let context = crate::kit::test_context();
        let mut state = state();
        let original = state.session.clips().to_vec();
        let (sender, receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        state.job = Some(ExportJob {
            receiver,
            cancel: cancel.clone(),
            progress: Arc::new(AtomicU32::new(0)),
        });
        frame(&mut state, &context, vec![], Vec2::new(1440.0, 900.0));
        key(
            &mut state,
            &context,
            egui::Key::Space,
            egui::Modifiers::NONE,
        );
        assert!(
            state.playing,
            "Space must work during export, just like mouse playback"
        );
        state.playhead = RATE.tick_of(30);
        key(
            &mut state,
            &context,
            egui::Key::ArrowRight,
            egui::Modifiers::NONE,
        );
        assert!(!state.playing);
        assert_eq!(state.playhead, RATE.tick_of(31));
        key(
            &mut state,
            &context,
            egui::Key::ArrowLeft,
            egui::Modifiers::NONE,
        );
        assert_eq!(state.playhead, RATE.tick_of(30));
        for key_value in [egui::Key::S, egui::Key::Delete, egui::Key::Z, egui::Key::D] {
            key(&mut state, &context, key_value, egui::Modifiers::NONE);
        }
        state.edit(EditorAction::Delete { id: original[0].id });
        state.export(&context);
        assert_eq!(state.session.clips(), original);
        assert!(Arc::ptr_eq(&state.job.as_ref().unwrap().cancel, &cancel));
        cancel.store(true, Ordering::Relaxed);
        frame(&mut state, &context, vec![], Vec2::new(960.0, 640.0));
        assert!(state.busy(), "cancellation cannot free a worker slot early");
        state.open = false;
        sender.send(Err("Export cancelled".into())).unwrap();
        state.poll(&context);
        assert!(!state.busy());
        assert_eq!(state.message.as_ref().unwrap().0, "Export cancelled");
        // A failed worker also releases the slot and surfaces an actionable error.
        let (sender, receiver) = mpsc::sync_channel(1);
        state.job = Some(ExportJob {
            receiver,
            cancel,
            progress: Arc::new(AtomicU32::new(0)),
        });
        drop(sender);
        state.poll(&context);
        assert!(!state.busy());
        assert!(
            state
                .message
                .as_ref()
                .unwrap()
                .0
                .contains("Export stopped unexpectedly")
        );
    }

    #[test]
    fn visible_analysis_stays_in_trimmed_source_and_maps_back_to_the_same_cut() {
        let start = Tick::from_units(37, 10);
        let end = Tick::from_units(127, 10);
        let card = egui::Rect::from_min_max(egui::pos2(-300.0, 0.0), egui::pos2(600.0, 72.0));
        let viewport = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(300.0, 90.0));
        let (a, b, frames, buckets) = timeline_window(start, end, card, viewport).unwrap();
        assert_eq!(a, Tick::from_units(6, 1));
        assert_eq!(b, Tick::from_units(10, 1));
        assert!((1..=12).contains(&frames) && (16..=2048).contains(&buckets));
        assert!((source_x(card, start, end, Tick::from_units(67, 10).0 as f64)).abs() < 0.01);
        assert!(
            (source_x(card, start, end, Tick::from_units(97, 10).0 as f64) - 300.0).abs() < 0.01
        );
        let full = timeline_window(start, end, card, card).unwrap();
        assert_eq!((full.0, full.1), (start, end));
        let tiny = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(4.0, 72.0));
        assert!(timeline_window(start, end, tiny, tiny).is_none());
        assert!(
            timeline_window(Tick::ZERO, Tick::from_units(1801, 1), viewport, viewport).is_none()
        );
        // Subsecond trims stay precise and cannot ask a decoder outside their in/out points.
        let short_end = start + RATE.tick_of(1);
        assert_eq!(
            timeline_window(start, short_end, viewport, viewport).map(|v| (v.0, v.1)),
            Some((start, short_end))
        );
        let uv = cover_uv(Vec2::new(100.0, 100.0), 16.0 / 9.0);
        assert!((uv.width() - 9.0 / 16.0).abs() < 0.001);
        assert_eq!(uv.height(), 1.0);
    }

    fn two_preview_sources(fixture: &crate::playback::editor_fixtures::Fixture) -> EditorState {
        let mut state = EditorState {
            open: true,
            ..Default::default()
        };
        for name in ["first.mp4", "second.mp4"] {
            state.edit(EditorAction::Append {
                input: fixture.file(name),
                name: name.into(),
                duration: Tick::from_units(1, 1),
            });
        }
        state
    }
    fn preview_until(
        state: &mut EditorState,
        context: &egui::Context,
        ready: impl Fn(&EditorState) -> bool,
    ) {
        // Debug builds composite 720p pictures in software. Parallel tests on a busy CI runner
        // can take longer than a normal interactive preview; still fail if no frame arrives.
        let deadline = Instant::now() + Duration::from_secs(30);
        while !ready(state) && Instant::now() < deadline {
            // Test decode completion without depending on UI frame rate or advancing wall time.
            state.clock = None;
            state.poll_preview(context);
            assert!(state.preview_workers() <= 2);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            ready(state),
            "preview did not become ready: message={:?}, workers={}, seeking={}, target={:?}",
            state.message,
            state.preview_workers(),
            state.seek_pending,
            state.preview_target
        );
    }

    #[test]
    fn prepared_second_source_keeps_its_picture_when_promoted_at_a_cut() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        let context = crate::kit::test_context();
        let mut state = two_preview_sources(&fixture);
        let next = state.session.clips()[1].id;
        state.edit(EditorAction::Trim {
            id: next,
            start: Tick::from_units(1, 3),
            end: Tick::from_units(1, 1),
        });
        state.playing = true;
        preview_until(&mut state, &context, |state| {
            state.prepared_preview.as_ref().is_some_and(|prepared| {
                prepared.playback.editor_target_ready(prepared.source.start)
            })
        });
        let texture = state
            .prepared_preview
            .as_ref()
            .unwrap()
            .playback
            .editor_texture_id();
        assert!(texture.is_some());
        state.playhead = Tick::from_units(1, 1);
        state.clock = None;
        state.poll_preview(&context);
        assert_eq!(state.preview_id, Some(next));
        assert_eq!(state.preview.as_ref().unwrap().editor_texture_id(), texture);
        assert!(
            state
                .preview
                .as_ref()
                .unwrap()
                .editor_target_ready(Tick::from_units(1, 3))
        );
        assert!(!state.seek_pending);
        assert!(
            state.preview_target.is_none(),
            "promotion must not request another seek"
        );
        assert!(state.playing);
        assert!(state.preview_workers() <= 2);
        state.close();
        preview_until(&mut state, &context, |state| state.preview_workers() == 0);
    }

    #[test]
    fn same_source_cuts_reuse_decoder_and_seek_only_when_ranges_are_discontinuous() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        let context = crate::kit::test_context();
        let mut state = EditorState {
            open: true,
            ..Default::default()
        };
        state.edit(EditorAction::Append {
            input: fixture.file("same.mp4"),
            name: "Same".into(),
            duration: Tick::from_units(1, 1),
        });
        state.edit(EditorAction::Split {
            at: Tick::from_units(1, 2),
        });
        state.playing = true;
        preview_until(&mut state, &context, |state| !state.seek_pending);
        let texture = state.preview.as_ref().unwrap().editor_texture_id();
        state.playhead = Tick::from_units(1, 2);
        state.clock = None;
        state.poll_preview(&context);
        assert_eq!(state.preview.as_ref().unwrap().editor_texture_id(), texture);
        assert_eq!(state.preview_workers(), 1);
        assert!(state.prepared_preview.is_none());
        assert!(state.preview_target.is_none());

        let second = state.session.clips()[1].id;
        state.edit(EditorAction::Trim {
            id: second,
            start: Tick::from_units(3, 4),
            end: Tick::from_units(1, 1),
        });
        state.seek(Tick::ZERO);
        state.playing = true;
        preview_until(&mut state, &context, |state| !state.seek_pending);
        state.playhead = Tick::from_units(1, 2);
        state.clock = None;
        state.poll_preview(&context);
        assert_eq!(state.preview_target, Some(Tick::from_units(3, 4)));
        assert_eq!(state.preview_workers(), 1);
        preview_until(&mut state, &context, |state| !state.seek_pending);
        assert!(
            state
                .preview
                .as_ref()
                .unwrap()
                .editor_target_ready(Tick::from_units(3, 4))
        );
        assert_eq!(state.preview.as_ref().unwrap().editor_texture_id(), texture);
        state.close();
        preview_until(&mut state, &context, |state| state.preview_workers() == 0);
    }

    #[test]
    fn rapid_seek_reorder_and_reopen_keep_active_prepared_and_retiring_workers_bounded() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        let context = crate::kit::test_context();
        let mut state = two_preview_sources(&fixture);
        state.playing = true;
        preview_until(&mut state, &context, |state| {
            state.prepared_preview.is_some()
        });
        for index in 0..40 {
            state.seek(Tick::from_units(index % 2, 1));
            if index % 3 == 0 {
                let id = state.session.clips()[1].id;
                state.edit(EditorAction::Move { id, index: 0 });
            }
            state.playing = true;
            state.poll_preview(&context);
            assert!(state.preview_workers() <= 2);
            assert!(
                state.prepared_preview.is_none(),
                "a seek invalidates preparation"
            );
            if index % 5 == 0 {
                state.close();
                assert!(state.preview.is_none());
                assert!(state.prepared_preview.is_none());
                state.open = true;
                state.poll_preview(&context);
                assert!(state.preview_workers() <= 2);
            }
        }
        state.close();
        preview_until(&mut state, &context, |state| state.preview_workers() == 0);
        assert!(state.preview.is_none());
        assert!(state.prepared_preview.is_none());
    }

    #[test]
    fn failed_next_source_does_not_stop_or_repeatedly_reopen_healthy_current_source() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        let context = crate::kit::test_context();
        let mut state = two_preview_sources(&fixture);
        std::fs::write(&state.session.clips()[1].input, b"damaged media").unwrap();
        state.playing = true;
        preview_until(&mut state, &context, |state| state.failed_preload.is_some());
        let failed = state.failed_preload.clone();
        for _ in 0..20 {
            state.clock = None;
            state.poll_preview(&context);
            assert!(state.playing);
            assert!(state.preview.as_ref().unwrap().editor_error().is_none());
            assert!(state.prepared_preview.is_none());
            assert_eq!(state.failed_preload, failed);
        }
        state.close();
        preview_until(&mut state, &context, |state| state.preview_workers() == 0);
    }
    #[test]
    fn export_presets_keep_edit_shortcuts_out_of_the_dialog_and_reject_impossible_sizes() {
        let context = crate::kit::test_context();
        let mut editor = state();
        editor.playhead = Tick::from_units(2, 1);
        editor.export_dialog = true;
        editor.export_quality = SequenceQuality {
            resolution: SequenceResolution::Source,
            fps: 60,
            ..Default::default()
        };
        for preset in [
            None,
            Some(ExportPreset::Small),
            Some(ExportPreset::Medium),
            Some(ExportPreset::Custom { megabytes: 1 }),
        ] {
            editor.export_preset = preset;
            assert!(editor.export_canvas().is_ok());
            for size in [Vec2::new(900.0, 650.0), Vec2::new(1440.0, 900.0)] {
                for _ in 0..3 {
                    frame(&mut editor, &context, vec![], size);
                }
            }
        }
        let original = editor.session.clone();
        assert!(
            !context.memory(|memory| memory.allows_interaction(egui::LayerId::background())),
            "export keyboard focus must not escape to the editor behind it"
        );
        for pressed in [egui::Key::Space, egui::Key::S, egui::Key::Delete] {
            key(&mut editor, &context, pressed, egui::Modifiers::NONE);
        }
        assert_eq!(editor.session, original);
        assert!(!editor.playing);
        key(
            &mut editor,
            &context,
            egui::Key::Escape,
            egui::Modifiers::NONE,
        );
        assert!(!editor.export_dialog);
        let mut long = EditorState::default();
        long.edit(EditorAction::Append {
            input: "long.mp4".into(),
            name: "Long".into(),
            duration: Tick::from_units(1800, 1),
        });
        long.export_preset = Some(ExportPreset::Custom { megabytes: 1 });
        assert!(long.export_canvas().is_err());
    }
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
        assert!(
            app.editor.media[0].probe.is_none(),
            "send must resolve playable source bounds"
        );
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
    fn library_draws_as_a_grid_in_every_size_and_as_a_list_at_any_window_size() {
        let context = crate::kit::test_context();
        let mut state = state();
        let mut second = media();
        second.path = "second.mp4".into();
        second.title = "Valorant 2026-11-16 08-33-52".into();
        state.media = Arc::new(vec![media(), second]);
        state.tab = LibraryTab::Library;
        let sizes = [
            Vec2::new(1.0, 1.0),
            Vec2::new(320.0, 500.0),
            Vec2::new(900.0, 650.0),
            Vec2::new(1440.0, 900.0),
            Vec2::new(6000.0, 4000.0),
        ];
        for scale in 0..3_u8 {
            state.library_layout = library::LibraryLayout::Grid;
            state.card_scale = scale;
            for size in sizes {
                frame(&mut state, &context, vec![], size);
            }
        }
        // Out-of-range sizes (from a damaged value) are clamped, not a crash.
        state.card_scale = 200;
        frame(&mut state, &context, vec![], Vec2::new(1440.0, 900.0));
        state.library_layout = library::LibraryLayout::List;
        for size in sizes {
            frame(&mut state, &context, vec![], size);
        }
        // Searching narrows both layouts, and an empty result still draws.
        state.search = "valorant".into();
        frame(&mut state, &context, vec![], Vec2::new(1440.0, 900.0));
        state.search = "no such clip".into();
        frame(&mut state, &context, vec![], Vec2::new(1440.0, 900.0));
        state.library_layout = library::LibraryLayout::Grid;
        frame(&mut state, &context, vec![], Vec2::new(1440.0, 900.0));
    }

    #[test]
    fn frame_step_follows_the_frame_rate_of_the_clip_being_shown() {
        for rate in [
            FrameRate { num: 24, den: 1 },
            FrameRate { num: 30, den: 1 },
            FrameRate { num: 60, den: 1 },
            FrameRate {
                num: 60_000,
                den: 1_001,
            },
        ] {
            let mut state = state();
            let mut entry = media();
            entry.probe.as_mut().unwrap().frame_rate = Some(rate);
            state.media = Arc::new(vec![entry]);
            state.seek(Tick::ZERO);
            state.step_frame(true);
            assert_eq!(state.playhead, rate.frame_duration(), "{rate:?}");
            state.step_frame(true);
            assert_eq!(state.playhead, rate.tick_of(2), "{rate:?}");
            state.step_frame(false);
            assert_eq!(state.playhead, rate.tick_of(1), "{rate:?}");
            // From between two frames: forward to the next one, back to the start of this one.
            state.seek(rate.tick_of(5) + Tick(1_000));
            state.step_frame(true);
            assert_eq!(state.playhead, rate.tick_of(6), "{rate:?}");
            state.seek(rate.tick_of(5) + Tick(1_000));
            state.step_frame(false);
            assert_eq!(state.playhead, rate.tick_of(5), "{rate:?}");
            // The edges hold.
            state.seek(Tick::ZERO);
            state.step_frame(false);
            assert_eq!(state.playhead, Tick::ZERO);
            state.seek(state.session.duration());
            state.step_frame(true);
            assert_eq!(state.playhead, state.session.duration());
        }
    }
    #[test]
    fn timeline_clicks_land_on_frames_of_the_clip_under_the_pointer() {
        for rate in [
            FrameRate { num: 24, den: 1 },
            FrameRate { num: 60, den: 1 },
            FrameRate {
                num: 60_000,
                den: 1_001,
            },
        ] {
            let mut state = state();
            let mut entry = media();
            entry.probe.as_mut().unwrap().frame_rate = Some(rate);
            state.media = Arc::new(vec![entry]);
            // Just under half a frame past frame 7 snaps back to it; just over snaps to frame 8.
            let half = rate.frame_duration().0 / 2;
            assert_eq!(
                state.snap_to_frame(rate.tick_of(7) + Tick(half - 10)),
                rate.tick_of(7),
                "{rate:?}"
            );
            assert_eq!(
                state.snap_to_frame(rate.tick_of(7) + Tick(half + 10)),
                rate.tick_of(8),
                "{rate:?}"
            );
            // The ends hold.
            assert_eq!(state.snap_to_frame(Tick::ZERO), Tick::ZERO);
            assert!(state.snap_to_frame(state.session.duration()) <= state.session.duration());
            assert!(state.snap_to_frame(Tick(i64::MAX / 4)) <= state.session.duration());
        }
    }
    #[test]
    fn every_page_of_the_text_tab_draws_at_any_window_size_and_a_damaged_page_is_clamped() {
        let context = crate::kit::test_context();
        let mut state = state();
        state.edit(EditorAction::AddText {
            start: Tick::ZERO,
            duration: Tick::from_units(2, 1),
            text: "Your title".into(),
        });
        state.selected_text = state.session.texts().last().map(|layer| layer.id);
        state.selected = None;
        state.tab = LibraryTab::Text;
        for page in [0_u8, 1, 2, 3, 200] {
            state.text_page = page;
            for size in [
                Vec2::new(1.0, 1.0),
                Vec2::new(320.0, 500.0),
                Vec2::new(900.0, 650.0),
                Vec2::new(1440.0, 900.0),
                Vec2::new(6000.0, 4000.0),
            ] {
                frame(&mut state, &context, vec![], size);
            }
        }
        // With nothing selected the tab says what text is for, whatever the page.
        state.selected_text = None;
        frame(&mut state, &context, vec![], Vec2::new(1440.0, 900.0));
    }

    #[test]
    fn zoom_keys_step_the_timeline_and_fit_returns_to_the_whole_edit() {
        let mut state = state();
        let context = crate::kit::test_context();
        frame(&mut state, &context, vec![], Vec2::new(1440.0, 900.0));
        assert!(state.fit);
        key(&mut state, &context, egui::Key::Plus, egui::Modifiers::NONE);
        assert!(!state.fit);
        let in_once = state.zoom;
        key(&mut state, &context, egui::Key::Plus, egui::Modifiers::NONE);
        assert!(state.zoom > in_once);
        key(
            &mut state,
            &context,
            egui::Key::Minus,
            egui::Modifiers::NONE,
        );
        assert!((state.zoom - in_once).abs() < 1e-3);
        // The ends hold, however many times the key is pressed.
        for _ in 0..40 {
            key(&mut state, &context, egui::Key::Plus, egui::Modifiers::NONE);
        }
        assert!((state.zoom - 160.0).abs() < 1e-3);
        for _ in 0..40 {
            key(
                &mut state,
                &context,
                egui::Key::Minus,
                egui::Modifiers::NONE,
            );
        }
        assert!((state.zoom - 4.0).abs() < 1e-3);
        key(&mut state, &context, egui::Key::Num0, egui::Modifiers::NONE);
        assert!(state.fit);
    }
    #[test]
    fn transport_jumps_between_cuts_steps_frames_and_loops() {
        let mut state = state();
        let context = crate::kit::test_context();
        frame(&mut state, &context, vec![], Vec2::new(1440.0, 900.0));
        // Cut at one second and again at three: segments of 1, 2 and 1 seconds.
        state.seek(Tick::from_units(1, 1));
        key(&mut state, &context, egui::Key::S, egui::Modifiers::NONE);
        state.seek(Tick::from_units(3, 1));
        key(&mut state, &context, egui::Key::S, egui::Modifiers::NONE);
        assert_eq!(
            state.cuts(),
            [0, 1, 3, 4]
                .map(|seconds| Tick::from_units(seconds, 1))
                .to_vec()
        );
        state.seek(Tick::from_units(2, 1));
        state.previous_cut();
        assert_eq!(state.playhead, Tick::from_units(1, 1));
        state.previous_cut();
        assert_eq!(state.playhead, Tick::ZERO);
        state.previous_cut();
        assert_eq!(state.playhead, Tick::ZERO, "stays at the start");
        state.next_cut();
        assert_eq!(state.playhead, Tick::from_units(1, 1));
        state.next_cut();
        state.next_cut();
        assert_eq!(state.playhead, Tick::from_units(4, 1));
        state.next_cut();
        assert_eq!(state.playhead, Tick::from_units(4, 1), "stays at the end");
        // Home and End, and a frame at a time.
        key(&mut state, &context, egui::Key::Home, egui::Modifiers::NONE);
        assert_eq!(state.playhead, Tick::ZERO);
        key(
            &mut state,
            &context,
            egui::Key::ArrowRight,
            egui::Modifiers::NONE,
        );
        assert_eq!(state.playhead, RATE.frame_duration());
        key(&mut state, &context, egui::Key::End, egui::Modifiers::NONE);
        assert_eq!(state.playhead, state.session.duration());
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
    fn preview_transport_fades_audio_without_rewriting_the_segment_volume() {
        let context = crate::kit::test_context();
        let mut state = state();
        let id = state.session.clips()[0].id;
        state.edit(EditorAction::Volume { id, percent: 50 });
        state.edit(EditorAction::Look {
            id,
            look: SegmentLook {
                fade_in: Fade::Black,
                fade_out: Fade::White,
                ..Default::default()
            },
        });
        for (time, expected) in [
            (Tick::ZERO, 0.0),
            (Tick::from_units(1, 4), 0.25),
            (Tick::from_units(1, 1), 0.5),
        ] {
            state.seek(time);
            state.poll_preview(&context);
            assert_eq!(state.preview.as_ref().unwrap().prefs().volume, expected);
            assert_eq!(state.session.clips()[0].volume_percent, 50);
        }
    }

    #[test]
    fn preview_keeps_only_selected_text_geometry_and_releases_it_on_deselect_or_close() {
        fn draw(state: &mut EditorState, context: &egui::Context, size: Vec2) {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        state.paint_text_handles(ui, ui.max_rect());
                    });
                },
            );
        }
        let context = crate::kit::test_context();
        let mut state = state();
        for text in ["Selected", "Other layer"] {
            state.edit(EditorAction::AddText {
                start: Tick::ZERO,
                duration: Tick::from_units(2, 1),
                text: text.into(),
            });
        }
        let id = state.session.texts()[0].id;
        state.selected_text = Some(id);
        state.playhead = Tick::from_units(1, 1);
        for _ in 0..3 {
            draw(&mut state, &context, Vec2::new(1440.0, 900.0));
        }
        let geometry = state.text_geometry.clone().unwrap();
        assert_eq!(geometry.key.text, "Selected");
        let key_pointer = state.text_geometry.as_ref().unwrap().key.text.as_ptr();
        state.title_drag = Some((id, 850, 200));
        draw(&mut state, &context, Vec2::new(1080.0, 760.0));
        assert_eq!(state.text_geometry.as_ref(), Some(&geometry));
        assert_eq!(
            state.text_geometry.as_ref().unwrap().key.text.as_ptr(),
            key_pointer
        );
        state.title_drag = None;
        state.selected_text = None;
        draw(&mut state, &context, Vec2::new(1440.0, 900.0));
        assert!(state.text_geometry.is_none());
        state.selected_text = Some(id);
        draw(&mut state, &context, Vec2::new(1440.0, 900.0));
        assert!(state.text_geometry.is_some());
        state.close();
        assert!(state.text_geometry.is_none());
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
        // Move buttons need a second segment; trim and volume are behind a toggle.
        state.edit(EditorAction::Split {
            at: Tick::from_units(1, 1),
        });
        state.selected = state.session.clips().last().map(|c| c.id);
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
