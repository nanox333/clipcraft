//! Internal editor recovery; this is not a project-file workflow.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clipcraft_engine::{EditorDraft, MediaEntry};
use eframe::egui::{self, RichText};

use super::EditorState;
use crate::editor_recovery::{EXIT_WAIT, Recovery, RecoveryResult};
use crate::kit::{self, DANGER, MUTED};

const SAVE_DELAY: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(super) struct DraftState {
    worker: Option<Recovery>,
    context: Option<egui::Context>,
    revision: u64,
    saved_revision: u64,
    saving_revision: Option<u64>,
    changed: Option<Instant>,
    loading: bool,
    load_revision: u64,
    pending: Option<Box<EditorDraft>>,
    blocked: bool,
    discarding: bool,
    suppressed: bool,
    recovered: bool,
    error: Option<String>,
}

impl EditorState {
    pub fn enable_recovery(&mut self, path: PathBuf, context: &egui::Context) {
        if self.draft.worker.is_some() {
            return;
        }
        self.draft.context = Some(context.clone());
        self.draft.loading = true;
        self.draft.load_revision = self.draft.revision;
        self.draft.worker = Some(Recovery::new(path, context));
    }

    pub fn take_recovered(&mut self) -> bool {
        std::mem::take(&mut self.draft.recovered)
    }

    pub fn flush_recovery(&mut self) {
        let Some(context) = self.draft.context.clone() else {
            return;
        };
        let Some(mut worker) = self.draft.worker.take() else {
            return;
        };
        let deadline = Instant::now() + EXIT_WAIT;
        let result = (|| {
            for result in worker.finish_pending(deadline)? {
                self.recovery_result(result, &context, false);
            }
            let has_workspace = self.has_recoverable_workspace();
            let snapshot = if has_workspace && self.can_save_draft() {
                Some(self.editor_draft()?)
            } else {
                None
            };
            worker.flush(snapshot, &context, deadline)?;
            Ok::<_, String>(())
        })();
        self.draft.worker = Some(worker);
        if result.is_err() {
            // Engine details may include private paths, so shutdown reports only a generic error.
            eprintln!(
                "ClipCraft could not finish its editor recovery checkpoint before exit. The newest changes may be missing from the saved draft."
            );
        }
    }

    pub(super) fn dirty_recovery(&mut self) {
        self.draft.suppressed = false;
        self.dirty_recovery_view();
    }

    pub(super) fn dirty_recovery_view(&mut self) {
        self.draft.revision = self.draft.revision.saturating_add(1);
        self.draft.changed = Some(Instant::now());
        if let Some(context) = &self.draft.context {
            context.request_repaint_after(SAVE_DELAY);
        }
    }

    fn can_save_draft(&self) -> bool {
        !self.draft.loading
            && self.draft.pending.is_none()
            && !self.draft.blocked
            && !self.draft.suppressed
            && !self.draft.discarding
    }

    fn has_recoverable_workspace(&self) -> bool {
        !self.media.is_empty()
            || !self.session.clips().is_empty()
            || !self.session.texts().is_empty()
    }

    fn editor_draft(&self) -> Result<EditorDraft, String> {
        EditorDraft::new(
            &self.session,
            self.media
                .iter()
                .map(|entry| PathBuf::from(&entry.path))
                .collect(),
            self.title.clone(),
            self.playhead,
            self.export_preset,
        )
        .and_then(|draft| draft.with_quality(self.export_quality))
        .map_err(|error| error.to_string())
    }

    pub(super) fn poll_recovery(&mut self, context: &egui::Context) {
        while let Some(result) = self.draft.worker.as_mut().and_then(Recovery::poll) {
            self.recovery_result(result, context, true);
        }
        if !self.can_save_draft()
            || !self.has_recoverable_workspace()
            || self.draft.revision == self.draft.saved_revision
            || self.draft.worker.as_ref().is_none_or(Recovery::busy)
        {
            return;
        }
        let Some(changed) = self.draft.changed else {
            return;
        };
        if let Some(remaining) = SAVE_DELAY.checked_sub(changed.elapsed()) {
            context.request_repaint_after(remaining);
            return;
        }
        match self.editor_draft() {
            Ok(draft) => {
                if self
                    .draft
                    .worker
                    .as_mut()
                    .is_some_and(|worker| worker.save(draft, context))
                {
                    self.draft.saving_revision = Some(self.draft.revision);
                    self.draft.changed = None;
                }
            }
            Err(error) => {
                self.draft.changed = None;
                self.draft.error = Some(error);
            }
        }
    }

    fn recovery_result(&mut self, result: RecoveryResult, context: &egui::Context, restore: bool) {
        match result {
            RecoveryResult::Loaded(draft) => {
                self.draft.loading = false;
                if !self.draft.discarding {
                    if let Some(draft) = draft {
                        if restore
                            && self.draft.revision == self.draft.load_revision
                            && !self.busy()
                            && self.importing.is_none()
                        {
                            self.restore_draft(*draft, context);
                        } else {
                            self.draft.pending = Some(draft);
                        }
                    } else if self.draft.revision != self.draft.saved_revision {
                        self.draft.changed = Some(Instant::now());
                    }
                }
            }
            RecoveryResult::Saved => {
                if let Some(revision) = self.draft.saving_revision.take() {
                    self.draft.saved_revision = revision;
                }
                self.draft.error = None;
            }
            RecoveryResult::Discarded => {
                self.draft.discarding = false;
                self.draft.blocked = false;
                self.draft.error = None;
                if !self.draft.suppressed && self.draft.revision != self.draft.saved_revision {
                    self.draft.changed.get_or_insert_with(Instant::now);
                    context.request_repaint_after(SAVE_DELAY);
                }
            }
            RecoveryResult::Failed(error) => {
                let failed_revision = self.draft.saving_revision.take();
                if self.draft.loading {
                    self.draft.loading = false;
                    self.draft.blocked = true;
                } else if failed_revision.is_none() && self.draft.discarding {
                    self.draft.discarding = false;
                    self.draft.blocked = true;
                }
                if failed_revision.is_none_or(|revision| revision >= self.draft.revision) {
                    self.draft.changed = None;
                }
                self.draft.error = Some(error);
            }
        }
    }

    fn restore_draft(&mut self, draft: EditorDraft, context: &egui::Context) {
        // close() retires both active and prepared decoders before installing another source set.
        self.close();
        let paths = draft.media;
        self.media = Arc::new(
            paths
                .iter()
                .map(|path| recovery_placeholder(path))
                .collect(),
        );
        self.session = draft.session;
        self.title = draft.title;
        self.playhead = draft.playhead;
        self.export_preset = draft.preset;
        self.export_quality = draft.quality;
        self.selected = self.session.clips().first().map(|clip| clip.id);
        self.selected_text = None;
        self.title_draft = (0, String::new());
        self.title_drag = None;
        self.text_drag = None;
        self.details = None;
        self.imported_thumbs.clear();
        self.seek_pending = true;
        self.open = true;
        self.draft.pending = None;
        self.draft.saved_revision = self.draft.revision;
        self.draft.changed = None;
        self.draft.suppressed = false;
        self.draft.blocked = false;
        self.draft.error = None;
        self.draft.recovered = true;
        self.message = Some((
            "Your local editor draft was restored. Checking its source files…".into(),
            false,
        ));
        if !paths.is_empty() {
            self.import_recovery(paths, context);
        }
    }

    fn discard_draft(&mut self, context: &egui::Context) {
        if self
            .draft
            .worker
            .as_mut()
            .is_some_and(|worker| worker.discard(context))
        {
            self.draft.pending = None;
            self.draft.discarding = true;
            self.draft.suppressed = true;
            self.draft.changed = None;
            self.draft.saved_revision = self.draft.revision;
        }
    }

    pub(super) fn recovery_banner(&mut self, ui: &mut egui::Ui) {
        if self.draft.pending.is_none() && self.draft.error.is_none() {
            return;
        }
        let mut restore = false;
        let mut discard = false;
        let mut retry = false;
        egui::Frame::new()
            .fill(kit::PANEL)
            .corner_radius(8)
            .inner_margin(10)
            .show(ui, |ui| {
                if let Some(error) = &self.draft.error {
                    ui.add(egui::Label::new(RichText::new(error).color(DANGER)).wrap());
                } else {
                    ui.add(
                        egui::Label::new(
                            RichText::new("A saved editor draft is available on this device.")
                                .color(MUTED),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if self.draft.pending.is_some() {
                        restore = super::button(
                            ui,
                            "Restore draft",
                            true,
                            !self.busy() && self.importing.is_none() && !self.draft.discarding,
                        )
                        .clicked();
                    } else if !self.draft.blocked {
                        retry = super::button(ui, "Retry saving", false, !self.draft.discarding)
                            .clicked();
                    }
                    discard =
                        super::button(ui, "Discard saved draft", false, !self.draft.discarding)
                            .clicked();
                });
            });
        ui.add_space(8.0);
        if discard {
            self.discard_draft(ui.ctx());
        } else if restore && let Some(draft) = self.draft.pending.take() {
            self.restore_draft(*draft, ui.ctx());
        } else if retry {
            self.dirty_recovery();
        }
    }

    pub(super) fn recovery_footer(&mut self, ui: &mut egui::Ui) {
        let enabled = self.draft.worker.is_some();
        let status = if !enabled {
            "Originals are never changed."
        } else if self.draft.suppressed {
            "Saved draft discarded. Your current edit is still open."
        } else {
            "Editor drafts save automatically on this device. Originals are never changed."
        };
        ui.label(RichText::new(status).size(12.0).color(kit::FAINT));
        if enabled
            && !self.draft.loading
            && !self.draft.suppressed
            && !self.media.is_empty()
            && ui
                .add_enabled(
                    !self.draft.discarding,
                    egui::Button::new(RichText::new("Discard saved draft").small()).frame(false),
                )
                .clicked()
        {
            self.discard_draft(ui.ctx());
        }
    }
}

fn recovery_placeholder(path: &Path) -> MediaEntry {
    MediaEntry {
        path: path.to_string_lossy().into_owned(),
        title: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        size_bytes: 0,
        modified_unix_seconds: None,
        extension: path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase(),
        offline: true,
        favorite: false,
        tags: Vec::new(),
        game: None,
        recorded_game: None,
        collections: Vec::new(),
        probe: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipcraft_engine::{
        EditorAction, EditorSession, ExportPreset, load_editor_draft, save_editor_draft,
    };
    use clipcraft_time::Tick;

    fn until(
        state: &mut EditorState,
        context: &egui::Context,
        ready: impl Fn(&EditorState) -> bool,
    ) {
        let start = Instant::now();
        while !ready(state) {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "recovery did not finish"
            );
            state.poll_recovery(context);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn snapshot(paths: &[PathBuf], title: &str) -> EditorDraft {
        let mut session = EditorSession::default();
        for path in paths {
            let clipcraft_engine::CommandOutput::EditorSession(updated) =
                clipcraft_engine::execute(clipcraft_engine::Command::EditSequence {
                    session: Box::new(session),
                    action: EditorAction::Append {
                        input: path.clone(),
                        name: "Clip".into(),
                        duration: Tick::from_units(1, 1),
                    },
                })
                .unwrap()
            else {
                panic!("edit command output");
            };
            session = *updated;
        }
        EditorDraft::new(
            &session,
            paths.to_vec(),
            title.into(),
            Tick::from_units(1, 2),
            Some(ExportPreset::Small),
        )
        .unwrap()
    }

    #[test]
    fn restore_preserves_missing_references_and_probes_each_file_independently() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        // Real editor imports store canonical paths. On Windows canonicalization also adds
        // the verbatim drive prefix, so build the draft with the same path representation.
        let healthy = std::fs::canonicalize(fixture.file("healthy.mp4")).unwrap();
        let missing = healthy.with_file_name("missing.mp4");
        let checkpoint = healthy.with_file_name(clipcraft_engine::EDITOR_DRAFT_FILE_NAME);
        let original = std::fs::read(&healthy).unwrap();
        let quality = clipcraft_engine::SequenceQuality {
            resolution: clipcraft_engine::SequenceResolution::Source,
            fps: 60,
        };
        let draft = snapshot(&[healthy.clone(), missing.clone()], "Recovered edit")
            .with_quality(quality)
            .unwrap();
        assert!(!draft.session.can_undo());
        save_editor_draft(&checkpoint, &draft).unwrap();
        let context = egui::Context::default();
        let mut state = EditorState::default();
        state.enable_recovery(checkpoint, &context);
        until(&mut state, &context, |state| !state.draft.loading);
        assert!(state.take_recovered());
        assert!(!state.take_recovered());
        assert!(state.open);
        assert!(!state.playing);
        assert_eq!(state.session.clips().len(), 2);
        assert!(!state.session.can_undo());
        assert_eq!(state.title, "Recovered edit");
        assert_eq!(state.export_preset, Some(ExportPreset::Small));
        assert_eq!(state.export_quality, quality);
        assert!(
            state
                .media
                .iter()
                .any(|entry| entry.path == missing.to_string_lossy() && entry.offline)
        );
        state.close();
        let started = Instant::now();
        while state.importing.is_some() {
            assert!(started.elapsed() < Duration::from_secs(5));
            state.poll(&context);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            state
                .media
                .iter()
                .any(|entry| entry.path == healthy.to_string_lossy()
                    && entry.probe.is_some()
                    && !entry.offline)
        );
        assert!(
            state
                .media
                .iter()
                .any(|entry| entry.path == missing.to_string_lossy() && entry.offline)
        );
        assert!(state.message.as_ref().unwrap().0.contains("1 source file"));
        assert_eq!(std::fs::read(healthy).unwrap(), original);
    }

    #[test]
    fn edits_before_loading_require_a_choice_and_discard_stays_discarded_until_a_new_edit() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        let source = fixture.file("clip.mp4");
        let checkpoint = source.with_file_name(clipcraft_engine::EDITOR_DRAFT_FILE_NAME);
        save_editor_draft(
            &checkpoint,
            &snapshot(std::slice::from_ref(&source), "Earlier draft"),
        )
        .unwrap();
        let context = egui::Context::default();
        let mut state = EditorState::default();
        state.enable_recovery(checkpoint.clone(), &context);
        state.title = "New edit".into();
        state.dirty_recovery();
        until(&mut state, &context, |state| !state.draft.loading);
        assert_eq!(state.title, "New edit");
        assert!(state.draft.pending.is_some());
        state.draft.changed = Some(Instant::now() - SAVE_DELAY);
        state.poll_recovery(&context);
        assert_eq!(
            load_editor_draft(&checkpoint).unwrap().unwrap().title,
            "Earlier draft"
        );
        state.discard_draft(&context);
        until(&mut state, &context, |state| !state.draft.discarding);
        assert!(load_editor_draft(&checkpoint).unwrap().is_none());
        state.open = true;
        state.close();
        state.draft.changed = Some(Instant::now() - SAVE_DELAY);
        state.poll_recovery(&context);
        state.flush_recovery();
        assert!(load_editor_draft(&checkpoint).unwrap().is_none());
        assert_eq!(state.title, "New edit");
        state.title = "Newer edit".into();
        state.media = Arc::new(vec![recovery_placeholder(&source)]);
        state.dirty_recovery();
        state.draft.changed = Some(Instant::now() - SAVE_DELAY);
        until(&mut state, &context, |state| {
            state.draft.saved_revision == state.draft.revision
        });
        assert_eq!(
            load_editor_draft(&checkpoint).unwrap().unwrap().title,
            "Newer edit"
        );
    }

    #[test]
    fn invalid_checkpoint_is_preserved_until_explicit_discard() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        let source = fixture.file("clip.mp4");
        let checkpoint = source.with_file_name(clipcraft_engine::EDITOR_DRAFT_FILE_NAME);
        let bad = b"invalid recovery metadata";
        std::fs::write(&checkpoint, bad).unwrap();
        let context = egui::Context::default();
        let mut state = EditorState::default();
        state.enable_recovery(checkpoint.clone(), &context);
        until(&mut state, &context, |state| !state.draft.loading);
        assert!(state.draft.blocked);
        assert!(state.draft.error.is_some());
        state.title = "Keep this edit".into();
        state.media = Arc::new(vec![recovery_placeholder(&source)]);
        state.dirty_recovery();
        state.draft.changed = Some(Instant::now() - SAVE_DELAY);
        state.poll_recovery(&context);
        state.flush_recovery();
        assert_eq!(std::fs::read(&checkpoint).unwrap(), bad);
        state.discard_draft(&context);
        until(&mut state, &context, |state| !state.draft.discarding);
        assert!(!state.draft.blocked);
        assert!(load_editor_draft(&checkpoint).unwrap().is_none());
        state.dirty_recovery();
        state.draft.changed = Some(Instant::now() - SAVE_DELAY);
        until(&mut state, &context, |state| {
            state.draft.saved_revision == state.draft.revision
        });
        assert_eq!(
            load_editor_draft(&checkpoint).unwrap().unwrap().title,
            "Keep this edit"
        );
    }

    #[test]
    fn normal_exit_checkpoints_the_final_playback_position_even_without_a_content_edit() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        let source = fixture.file("clip.mp4");
        let checkpoint = source.with_file_name(clipcraft_engine::EDITOR_DRAFT_FILE_NAME);
        save_editor_draft(&checkpoint, &snapshot(&[source], "Current edit")).unwrap();
        let context = egui::Context::default();
        let mut state = EditorState::default();
        state.enable_recovery(checkpoint.clone(), &context);
        until(&mut state, &context, |state| !state.draft.loading);
        assert_eq!(state.draft.saved_revision, state.draft.revision);
        state.playhead = Tick::from_units(3, 4);
        state.flush_recovery();
        assert_eq!(
            load_editor_draft(&checkpoint).unwrap().unwrap().playhead,
            Tick::from_units(3, 4)
        );
    }

    #[test]
    fn debounce_and_revision_tracking_keep_edits_made_during_an_earlier_save() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        let source = fixture.file("clip.mp4");
        let checkpoint = source.with_file_name(clipcraft_engine::EDITOR_DRAFT_FILE_NAME);
        let context = egui::Context::default();
        let mut state = EditorState::default();
        state.enable_recovery(checkpoint.clone(), &context);
        until(&mut state, &context, |state| !state.draft.loading);
        state.media = Arc::new(vec![recovery_placeholder(&source)]);
        state.title = "First revision".into();
        state.dirty_recovery();
        state.poll_recovery(&context);
        assert!(!state.draft.worker.as_ref().unwrap().busy());
        assert!(!checkpoint.exists());
        state.draft.changed = Some(Instant::now() - SAVE_DELAY);
        state.poll_recovery(&context);
        assert!(state.draft.worker.as_ref().unwrap().busy());
        state.title = "Second revision".into();
        state.dirty_recovery();
        until(&mut state, &context, |state| {
            state.draft.saving_revision.is_none()
        });
        assert!(state.draft.saved_revision < state.draft.revision);
        assert!(state.draft.changed.is_some());
        assert_eq!(
            load_editor_draft(&checkpoint).unwrap().unwrap().title,
            "First revision"
        );
        state.draft.changed = Some(Instant::now() - SAVE_DELAY);
        until(&mut state, &context, |state| {
            state.draft.saved_revision == state.draft.revision
        });
        assert_eq!(
            load_editor_draft(&checkpoint).unwrap().unwrap().title,
            "Second revision"
        );
    }

    #[test]
    fn opening_and_closing_an_empty_editor_never_creates_a_checkpoint() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        let source = fixture.file("clip.mp4");
        let checkpoint = source.with_file_name(clipcraft_engine::EDITOR_DRAFT_FILE_NAME);
        let context = egui::Context::default();
        let mut state = EditorState::default();
        state.enable_recovery(checkpoint.clone(), &context);
        until(&mut state, &context, |state| !state.draft.loading);
        state.open = true;
        state.close();
        state.draft.changed = Some(Instant::now() - SAVE_DELAY);
        state.poll_recovery(&context);
        state.flush_recovery();
        assert!(!checkpoint.exists());
    }

    #[test]
    fn exit_after_an_edit_drains_the_initial_load_before_deciding_what_to_save() {
        for existing in [false, true] {
            let fixture = crate::playback::editor_fixtures::Fixture::new(false);
            let source = fixture.file("clip.mp4");
            let checkpoint = source.with_file_name(clipcraft_engine::EDITOR_DRAFT_FILE_NAME);
            if existing {
                save_editor_draft(
                    &checkpoint,
                    &snapshot(std::slice::from_ref(&source), "Earlier draft"),
                )
                .unwrap();
            }
            let context = egui::Context::default();
            let mut state = EditorState::default();
            state.enable_recovery(checkpoint.clone(), &context);
            state.media = Arc::new(vec![recovery_placeholder(&source)]);
            state.title = "New edit before load acknowledgement".into();
            state.dirty_recovery();
            assert!(state.draft.loading);
            state.flush_recovery();
            let saved = load_editor_draft(&checkpoint).unwrap().unwrap();
            assert_eq!(
                saved.title,
                if existing {
                    "Earlier draft"
                } else {
                    "New edit before load acknowledgement"
                }
            );
            assert_eq!(state.draft.pending.is_some(), existing);
        }
    }

    #[test]
    fn exit_after_discard_and_a_new_edit_saves_the_newer_snapshot() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        let source = fixture.file("clip.mp4");
        let checkpoint = source.with_file_name(clipcraft_engine::EDITOR_DRAFT_FILE_NAME);
        let context = egui::Context::default();
        let mut state = EditorState::default();
        state.enable_recovery(checkpoint.clone(), &context);
        until(&mut state, &context, |state| !state.draft.loading);
        state.media = Arc::new(vec![recovery_placeholder(&source)]);
        state.title = "Earlier edit".into();
        state.dirty_recovery();
        state.draft.changed = Some(Instant::now() - SAVE_DELAY);
        until(&mut state, &context, |state| {
            state.draft.saved_revision == state.draft.revision
        });
        state.discard_draft(&context);
        state.title = "Edit after discard".into();
        state.dirty_recovery();
        assert!(state.draft.discarding);
        assert!(!state.draft.suppressed);
        state.flush_recovery();
        assert_eq!(
            load_editor_draft(&checkpoint).unwrap().unwrap().title,
            "Edit after discard"
        );
    }

    #[test]
    fn an_older_save_failure_and_discard_acknowledgement_preserve_the_newer_dirty_revision() {
        let fixture = crate::playback::editor_fixtures::Fixture::new(false);
        let source = fixture.file("clip.mp4");
        let checkpoint = source.with_file_name(clipcraft_engine::EDITOR_DRAFT_FILE_NAME);
        let context = egui::Context::default();
        let mut state = EditorState::default();
        state.enable_recovery(checkpoint.clone(), &context);
        until(&mut state, &context, |state| !state.draft.loading);
        state.media = Arc::new(vec![recovery_placeholder(&source)]);
        state.title = "Earlier save".into();
        state.dirty_recovery();
        // Model the accepted save revision, then deterministically deliver its failure before
        // polling the real discard reply. Worker barrier ordering is covered separately.
        state.draft.saving_revision = Some(state.draft.revision);
        state.draft.changed = None;
        state.discard_draft(&context);
        state.title = "New edit while discard is pending".into();
        state.dirty_recovery();
        state.recovery_result(
            RecoveryResult::Failed("The previous save failed.".into()),
            &context,
            true,
        );
        assert!(state.draft.changed.is_some());
        until(&mut state, &context, |state| !state.draft.discarding);
        assert!(state.draft.changed.is_some());
        assert!(state.draft.error.is_none());
        state.draft.changed = Some(Instant::now() - SAVE_DELAY);
        until(&mut state, &context, |state| {
            state.draft.saved_revision == state.draft.revision
        });
        assert_eq!(
            load_editor_draft(&checkpoint).unwrap().unwrap().title,
            "New edit while discard is pending"
        );
    }
}
