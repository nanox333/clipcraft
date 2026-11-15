//! After the computer restarts unexpectedly while ClipCraft was using the graphics card's
//! encoder, offer to record with the processor instead.
//!
//! This does not say what went wrong; many things restart a computer. It notices the coincidence
//! (see `clipcraft_engine::gpu_session`), asks once, and leaves the choice to the person. Windows
//! only: other systems have no way to tell an unexpected restart from a normal one here.

use std::sync::mpsc;
use std::time::Duration;

use clipcraft_engine::{
    HEARTBEAT_SECONDS, PreviousRun, SessionMarker, SessionTracker, VideoEncoder,
    default_gpu_session_path, judge_previous_run, needs_restart_check, unix_now,
};
use clipcraft_record::RecordState;
use eframe::egui::{self, RichText};

use crate::kit::{self, MUTED, TEXT};
use crate::{ClipCraftApp, Icon};

/// What launch learned about the run before this one, and the tracker for this one.
pub(crate) struct GpuSession {
    tracker: SessionTracker,
    /// The answer from Windows, while it is still being fetched.
    check: Option<mpsc::Receiver<PreviousRun>>,
    /// The box is showing.
    pub(crate) asking: bool,
}

impl GpuSession {
    /// Reads the previous run's marker, replaces it with this run's, and, only when the previous
    /// run ended on the graphics card without a clean exit, asks Windows on a worker thread
    /// whether the computer restarted unexpectedly afterwards.
    pub(crate) fn begin() -> Self {
        let path = default_gpu_session_path();
        let previous = path.as_deref().and_then(SessionMarker::read);
        let check = needs_restart_check(previous).then(|| {
            let (sender, receiver) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("restart-judge".into())
                .spawn(move || {
                    let restart = clipcraft_platform::last_unexpected_restart();
                    let _ = sender.send(judge_previous_run(previous, restart));
                });
            spawned.ok().map(|_| receiver)
        });
        let mut tracker = SessionTracker::new(path);
        tracker.start(unix_now());
        Self {
            tracker,
            check: check.flatten(),
            asking: false,
        }
    }

    /// Marks a normal exit.
    pub(crate) fn finish(&mut self) {
        self.tracker.finish(unix_now());
    }
}

/// Whether the graphics card's encoder is the one working: the recorder labels it `GPU (name)`,
/// and `CPU` when the processor encodes (including after the card failed).
pub(crate) fn card_encoder_active(state: &RecordState, encoder: &str) -> bool {
    matches!(state, RecordState::Running)
        && encoder.starts_with("GPU (")
        && !encoder.contains("failed")
}

impl ClipCraftApp {
    /// Keeps the marker current and picks up Windows' answer about the previous run.
    pub(crate) fn sync_gpu_session(&mut self, context: &egui::Context) {
        let active = self
            .capture
            .status
            .as_ref()
            .is_some_and(|status| card_encoder_active(&status.state, &status.encoder));
        let Some(session) = self.gpu_session.as_mut() else {
            return;
        };
        session.tracker.observe(active, unix_now());
        if active {
            // A quiet window still beats at the pace the safety net needs.
            context.request_repaint_after(Duration::from_secs(HEARTBEAT_SECONDS));
        }
        if let Some(receiver) = session.check.as_ref() {
            match receiver.try_recv() {
                Ok(verdict) => {
                    session.check = None;
                    session.asking = verdict == PreviousRun::RestartedWhileEncoding
                        && self.recording.video_encoder == VideoEncoder::Gpu;
                    context.request_repaint();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    context.request_repaint_after(Duration::from_millis(500));
                }
                Err(mpsc::TryRecvError::Disconnected) => session.check = None,
            }
        }
    }

    /// The question, while there is one. Escape or the second button keeps the card.
    pub(crate) fn gpu_restart_dialog(&mut self, context: &egui::Context) {
        if !self
            .gpu_session
            .as_ref()
            .is_some_and(|session| session.asking)
        {
            return;
        }
        let mut use_processor = false;
        let mut keep_card = false;
        let modal = egui::Modal::new(egui::Id::new("gpu-restart-dialog"))
            .frame(
                egui::Frame::new()
                    .fill(kit::PANEL)
                    .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
                    .corner_radius(egui::CornerRadius::same(crate::kit::radius::LG))
                    .inner_margin(egui::Margin::same(24)),
            )
            .show(context, |ui| {
                ui.set_width(440.0);
                ui.label(
                    RichText::new("Your computer restarted unexpectedly")
                        .color(TEXT)
                        .size(crate::kit::text_size::LEAD)
                        .family(kit::semibold()),
                );
                ui.add_space(crate::kit::space::SM);
                ui.label(
                    RichText::new(
                        "Windows reported an unexpected restart after ClipCraft was last using \
                         your graphics card to encode video. ClipCraft cannot tell what caused \
                         it, and many things can.",
                    )
                    .color(TEXT)
                    .size(crate::kit::text_size::BODY),
                );
                ui.add_space(crate::kit::space::MD);
                ui.label(
                    RichText::new(
                        "If your computer has been freezing, recording with the processor \
                         instead is a safe way to find out. It uses more of the processor while \
                         you play. You can switch back any time in Settings \u{203a} Video.",
                    )
                    .color(MUTED)
                    .size(crate::kit::text_size::SMALL),
                );
                ui.add_space(crate::kit::space::XL);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = crate::kit::space::MD;
                    if kit::labelled_button(ui, Icon::Play, "Record with the processor", true, true)
                        .clicked()
                    {
                        use_processor = true;
                    }
                    if kit::labelled_button(
                        ui,
                        Icon::Restart,
                        "Keep the graphics card",
                        false,
                        true,
                    )
                    .clicked()
                    {
                        keep_card = true;
                    }
                });
            });
        if modal.should_close() {
            keep_card = true;
        }
        if use_processor || keep_card {
            self.answer_gpu_restart(use_processor);
        }
    }

    /// Closes the question. Choosing the processor saves that setting like any other, which
    /// restarts recording with the processor encoder.
    pub(crate) fn answer_gpu_restart(&mut self, use_processor: bool) {
        if use_processor {
            let before = self.recording.clone();
            let mut working = before.clone();
            working.video_encoder = VideoEncoder::Cpu;
            self.apply_recording(before, working);
        }
        if let Some(session) = self.gpu_session.as_mut() {
            session.asking = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipcraft_record::RecordError;

    #[test]
    fn only_a_running_recording_on_the_card_counts() {
        assert!(card_encoder_active(&RecordState::Running, "GPU (AMD h264)"));
        assert!(!card_encoder_active(&RecordState::Running, "CPU"));
        assert!(!card_encoder_active(
            &RecordState::Running,
            "CPU (H.265 needs the graphics card, so this is H.264)"
        ));
        assert!(!card_encoder_active(&RecordState::Running, "Starting"));
        assert!(!card_encoder_active(
            &RecordState::Running,
            "GPU (drain failed: lost)"
        ));
        assert!(!card_encoder_active(
            &RecordState::Stopped,
            "GPU (AMD h264)"
        ));
        assert!(!card_encoder_active(
            &RecordState::Failed(RecordError::EncoderFailed),
            "GPU (AMD h264)"
        ));
        assert!(!card_encoder_active(&RecordState::Running, ""));
    }

    fn app_asking() -> ClipCraftApp {
        let mut app = ClipCraftApp::restored_from(None);
        app.recording.video_encoder = VideoEncoder::Gpu;
        app.settings_path = None;
        let mut session = GpuSession {
            tracker: SessionTracker::new(None),
            check: None,
            asking: true,
        };
        session.tracker.start(1);
        app.gpu_session = Some(session);
        app
    }

    fn run_frame(app: &mut ClipCraftApp, context: &egui::Context) {
        let _ = context.run(egui::RawInput::default(), |context| {
            app.gpu_restart_dialog(context);
        });
    }

    #[test]
    fn the_box_draws_and_leaves_the_encoder_alone_until_a_choice_is_made() {
        let mut app = app_asking();
        let context = egui::Context::default();
        kit::install_fonts(&context);
        run_frame(&mut app, &context);
        run_frame(&mut app, &context);
        assert!(app.gpu_session.as_ref().is_some_and(|s| s.asking));
        assert_eq!(app.recording.video_encoder, VideoEncoder::Gpu);
    }

    #[test]
    fn choosing_the_processor_saves_it_and_keeping_the_card_changes_nothing() {
        let mut app = app_asking();
        app.answer_gpu_restart(false);
        assert_eq!(app.recording.video_encoder, VideoEncoder::Gpu);
        assert!(!app.gpu_session.as_ref().unwrap().asking);
        let mut app = app_asking();
        app.answer_gpu_restart(true);
        assert_eq!(app.recording.video_encoder, VideoEncoder::Cpu);
        assert!(!app.gpu_session.as_ref().unwrap().asking);
    }

    #[test]
    fn nothing_is_shown_when_there_is_nothing_to_ask() {
        let mut app = ClipCraftApp::restored_from(None);
        let context = egui::Context::default();
        kit::install_fonts(&context);
        run_frame(&mut app, &context); // no session at all
        assert!(app.gpu_session.is_none());
    }

    #[test]
    fn a_verdict_only_asks_while_the_card_is_the_chosen_encoder() {
        let context = egui::Context::default();
        for (encoder, expect) in [(VideoEncoder::Gpu, true), (VideoEncoder::Cpu, false)] {
            let mut app = ClipCraftApp::restored_from(None);
            app.recording.video_encoder = encoder;
            let (sender, receiver) = mpsc::channel();
            sender.send(PreviousRun::RestartedWhileEncoding).unwrap();
            app.gpu_session = Some(GpuSession {
                tracker: SessionTracker::new(None),
                check: Some(receiver),
                asking: false,
            });
            app.sync_gpu_session(&context);
            let session = app.gpu_session.as_ref().unwrap();
            assert_eq!(session.asking, expect);
            assert!(session.check.is_none());
        }
        let mut app = ClipCraftApp::restored_from(None);
        let (sender, receiver) = mpsc::channel();
        sender.send(PreviousRun::Normal).unwrap();
        app.gpu_session = Some(GpuSession {
            tracker: SessionTracker::new(None),
            check: Some(receiver),
            asking: false,
        });
        app.sync_gpu_session(&context);
        assert!(!app.gpu_session.as_ref().unwrap().asking);
        // A worker that ended without answering leaves nothing pending.
        let (sender, receiver) = mpsc::channel::<PreviousRun>();
        drop(sender);
        let mut app = ClipCraftApp::restored_from(None);
        app.gpu_session = Some(GpuSession {
            tracker: SessionTracker::new(None),
            check: Some(receiver),
            asking: false,
        });
        app.sync_gpu_session(&context);
        assert!(app.gpu_session.as_ref().unwrap().check.is_none());
    }
}
