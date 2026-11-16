//! Sharing a clip: dragging it out of the window into a chat (Discord), at a size that fits.
//!
//! A clip that already fits is dragged as it is. A bigger one is first made into a copy of the
//! size chosen in Settings (kept in a `Shared` folder beside it), which takes a few seconds, so it
//! is started when the pointer rests on the clip, not when the drag starts. A drag that begins
//! before the copy is ready says so and waits for the next try.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use clipcraft_engine::{
    EngineError, MediaEntry, SharePlan, SharedCopy, make_share_copy, share_plan,
};
use eframe::egui;

use crate::capture::notice;
use crate::{ClipCraftApp, kit};

/// A share copy being made on another thread.
pub(crate) struct ShareJob {
    source: String,
    megabytes: u16,
    /// 0 to 1000.
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<SharedCopy, EngineError>>,
}

impl Drop for ShareJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// What the Share control should show for a clip.
pub(crate) enum ShareState {
    /// Ready to drag as it is or from a copy already made.
    Ready,
    /// A copy is being made, this far along (0 to 1).
    Making(f32),
    /// A copy has to be made first.
    NotMade,
}

impl ClipCraftApp {
    /// The size, in megabytes, clips are made to fit when shared.
    fn share_megabytes(&self) -> u16 {
        u16::try_from(self.recording.share_megabytes.clamp(1, 500)).unwrap_or(10)
    }

    /// Where the share of `entry` stands.
    pub(crate) fn share_state(&self, entry: &MediaEntry) -> ShareState {
        if let Some(job) = self.sharing.as_ref().filter(|job| job.source == entry.path) {
            return ShareState::Making(job.progress.load(Ordering::Relaxed) as f32 / 1000.0);
        }
        match share_plan(std::path::Path::new(&entry.path), self.share_megabytes()) {
            Ok(SharePlan::Original(_) | SharePlan::Existing(_)) => ShareState::Ready,
            _ => ShareState::NotMade,
        }
    }

    /// Starts making the share copy of `entry` when one is needed and nothing else is being made.
    /// Not while clipping: encoding would compete with the game, so the person asks for it.
    pub(crate) fn prepare_share(&mut self, entry: &MediaEntry, even_while_clipping: bool) {
        if entry.offline || self.sharing.is_some() {
            return;
        }
        if self.capture.is_active() && !even_while_clipping {
            return;
        }
        let megabytes = self.share_megabytes();
        if !matches!(
            share_plan(std::path::Path::new(&entry.path), megabytes),
            Ok(SharePlan::Needed(_))
        ) {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (thread_progress, thread_cancel) = (progress.clone(), cancel.clone());
        let source = PathBuf::from(&entry.path);
        let spawned = std::thread::Builder::new()
            .name("clipcraft-share".to_owned())
            .spawn(move || {
                let mut report = |fraction: f32| {
                    thread_progress.store(
                        (fraction.clamp(0.0, 1.0) * 1000.0) as u32,
                        Ordering::Relaxed,
                    );
                };
                let _ = sender.send(make_share_copy(
                    &source,
                    megabytes,
                    &mut report,
                    &thread_cancel,
                ));
            });
        if spawned.is_ok() {
            self.capture.notice = notice(
                format!("Making a {megabytes} MB copy so it can be dragged into a chat\u{2026}"),
                false,
                None,
            );
            self.sharing = Some(ShareJob {
                source: entry.path.clone(),
                megabytes,
                progress,
                cancel,
                receiver,
            });
        }
    }

    /// Picks up a finished share copy.
    pub(crate) fn poll_share(&mut self, context: &egui::Context) {
        let Some(job) = self.sharing.as_ref() else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                context.request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(EngineError::FileUnavailable),
        };
        let megabytes = job.megabytes;
        self.sharing = None;
        self.capture.notice = match result {
            Ok(copy) => {
                let size = std::fs::metadata(&copy.path)
                    .map(|metadata| clipcraft_engine::format_bytes(metadata.len()))
                    .unwrap_or_default();
                notice(
                    format!(
                        "The {megabytes} MB copy is ready ({size}). Drag the clip into your chat."
                    ),
                    false,
                    None,
                )
            }
            Err(error) => notice(
                format!("Could not make a {megabytes} MB copy: {error}"),
                true,
                None,
            ),
        };
        context.request_repaint();
    }

    /// Starts dragging `entry` out of the window, at the size chosen for sharing. When the copy it
    /// needs is not made yet, starts making it and says so instead. Where the system cannot drag
    /// out of the window, the file is put on the clipboard instead.
    pub(crate) fn drag_clip_out(&mut self, context: &egui::Context, entry: &MediaEntry) {
        if entry.offline {
            return;
        }
        let path = match share_plan(std::path::Path::new(&entry.path), self.share_megabytes()) {
            Ok(SharePlan::Original(path) | SharePlan::Existing(path)) => path,
            Ok(SharePlan::Needed(_)) => {
                self.prepare_share(entry, true);
                if self.sharing.is_none() {
                    self.capture.notice = notice(
                        "A copy of this size is still being made. Try dragging again in a moment.",
                        false,
                        None,
                    );
                }
                return;
            }
            Err(error) => {
                self.capture.notice = notice(error.to_string(), true, None);
                return;
            }
        };
        if let Err(reason) = clipcraft_platform::start_file_drag(vec![path.clone()]) {
            // No drag out of the window here: the next best thing is a paste.
            self.copy_clip_file(context, path);
            if self
                .capture
                .notice
                .as_ref()
                .is_some_and(|notice| !notice.error)
            {
                self.capture.notice = notice(
                    format!("{reason}. The file was copied instead: paste it into your chat."),
                    false,
                    None,
                );
            }
        }
    }
}

/// The "Drag into Discord" control on a clip's page: drag it into a chat, or press it to get the
/// copy made. Returns what the person did with it.
pub(crate) enum ShareChip {
    Nothing,
    /// The pointer has rested on it (time to get the copy ready).
    Hover,
    /// A drag began.
    Drag,
    /// It was pressed.
    Press,
}

/// The words on the drag tile, for each state: (title, line under it).
fn share_words(state: &ShareState, megabytes: u16) -> (String, String) {
    match state {
        ShareState::Ready => (
            "Drag into Discord".to_owned(),
            format!("Drop it in a chat \u{b7} {megabytes} MB"),
        ),
        ShareState::Making(done) => (
            format!("Making a {megabytes} MB copy\u{2026} {:.0}%", done * 100.0),
            "Then drag it into a chat".to_owned(),
        ),
        ShareState::NotMade => (
            "Get ready to drag".to_owned(),
            format!("Click to make a {megabytes} MB copy for chats"),
        ),
    }
}

/// The tile you pick up and drag into a chat. The clip's own picture is the thing to pick up: it
/// rises when the pointer is on it, the cursor becomes a hand, and it follows the pointer a little
/// once dragging starts. Around it, only a quiet card and two lines of words.
pub(crate) fn share_chip(
    ui: &mut egui::Ui,
    state: &ShareState,
    megabytes: u16,
    enabled: bool,
    entry: &MediaEntry,
    thumbs: &mut crate::thumbs::Thumbnails,
) -> ShareChip {
    use eframe::egui::{Color32, CursorIcon, Sense, Stroke, Vec2};
    let (title, subtitle) = share_words(state, megabytes);
    let ready = matches!(state, ShareState::Ready);
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 84.0), sense);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, title.clone())
    });
    let dragging = enabled && response.dragged();
    let hovered = enabled && (response.hovered() || dragging);
    if enabled {
        ui.ctx().set_cursor_icon(if dragging {
            CursorIcon::Grabbing
        } else if hovered && ready {
            CursorIcon::Grab
        } else if hovered {
            CursorIcon::PointingHand
        } else {
            CursorIcon::Default
        });
    }
    let lift = ui.ctx().animate_bool_responsive(response.id, hovered);
    let accent = if enabled { kit::LIME } else { kit::FAINT };
    let alpha = |value: f32| value.clamp(0.0, 255.0) as u8;
    let texture = thumbs.texture(ui.ctx(), &entry.path, ui.is_rect_visible(rect));
    let painter = ui.painter().clone();
    // The card.
    painter.rect_filled(
        rect,
        16.0,
        if enabled {
            Color32::from_rgba_unmultiplied(190, 255, 43, alpha(14.0 + 16.0 * lift))
        } else {
            kit::CARD
        },
    );
    painter.rect_stroke(
        rect,
        16.0,
        Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(
                accent.r(),
                accent.g(),
                accent.b(),
                alpha(if ready { 80.0 + 150.0 * lift } else { 70.0 }),
            ),
        ),
        egui::StrokeKind::Inside,
    );
    if let ShareState::Making(done) = state {
        let mut filled = rect;
        filled.set_right(rect.left() + rect.width() * done.clamp(0.0, 1.0));
        painter.with_clip_rect(rect).rect_filled(
            filled,
            16.0,
            Color32::from_rgba_unmultiplied(190, 255, 43, 40),
        );
    }
    // The picture to pick up: bigger and raised on hover, with a shadow that grows.
    let base = egui::Rect::from_min_size(
        rect.left_top() + Vec2::new(14.0, 16.0),
        Vec2::new(82.0, 52.0),
    );
    let raised = base
        .expand2(
            Vec2::new(5.0, 3.0) * lift
                + if dragging {
                    Vec2::new(4.0, 2.0)
                } else {
                    Vec2::ZERO
                },
        )
        .translate(Vec2::new(0.0, -2.0 * lift));
    if enabled {
        painter.rect_filled(
            raised
                .translate(Vec2::new(0.0, 4.0 + 3.0 * lift))
                .expand(1.0),
            10.0,
            Color32::from_black_alpha(alpha(70.0 + 70.0 * lift)),
        );
    }
    crate::clip_card::paint_video_placeholder(ui, raised, entry, texture, 10);
    painter.rect_stroke(
        raised,
        10.0,
        Stroke::new(1.0_f32, Color32::from_white_alpha(40)),
        egui::StrokeKind::Inside,
    );
    // The words.
    let text_left = base.right() + 18.0;
    let room = (rect.right() - text_left - 38.0).max(40.0);
    let fit = |text: String, font: egui::FontId, color: Color32| {
        let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
        job.wrap.max_width = room;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('\u{2026}');
        painter.layout_job(job)
    };
    let title_galley = fit(
        title,
        egui::FontId::new(crate::kit::text_size::LEAD, kit::semibold()),
        if enabled { kit::TEXT } else { kit::MUTED },
    );
    let subtitle_galley = fit(
        subtitle,
        egui::FontId::proportional(crate::kit::text_size::CAPTION),
        if enabled { kit::MUTED } else { kit::FAINT },
    );
    let total = title_galley.size().y + 3.0 + subtitle_galley.size().y;
    let top = rect.center().y - total / 2.0;
    painter.galley(egui::pos2(text_left, top), title_galley.clone(), kit::TEXT);
    painter.galley(
        egui::pos2(text_left, top + title_galley.size().y + 3.0),
        subtitle_galley,
        kit::MUTED,
    );
    // An arrow leaving the card, the one cue about where it goes.
    let arrow = egui::pos2(rect.right() - 22.0, rect.center().y);
    let ink = Color32::from_rgba_unmultiplied(
        accent.r(),
        accent.g(),
        accent.b(),
        alpha(if enabled { 150.0 + 105.0 * lift } else { 90.0 }),
    );
    let stroke = Stroke::new(1.8_f32, ink);
    let nudge = Vec2::new(2.0, -2.0) * lift;
    painter.line_segment(
        [
            arrow + nudge + Vec2::new(-5.0, 5.0),
            arrow + nudge + Vec2::new(5.0, -5.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            arrow + nudge + Vec2::new(-2.0, -5.0),
            arrow + nudge + Vec2::new(5.0, -5.0),
        ],
        stroke,
    );
    painter.line_segment(
        [
            arrow + nudge + Vec2::new(5.0, -5.0),
            arrow + nudge + Vec2::new(5.0, 2.0),
        ],
        stroke,
    );
    let response = response.on_hover_text(if ready {
        "Click and hold the picture, then drag it into a Discord chat (or any program that takes files). A clip that is too big is shared as a smaller copy."
    } else {
        "Make a copy small enough to drag into a chat. Bigger clips are re-encoded to fit."
    });
    if response.drag_started() {
        ShareChip::Drag
    } else if response.clicked() {
        ShareChip::Press
    } else if response.hovered() {
        ShareChip::Hover
    } else {
        ShareChip::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &std::path::Path) -> MediaEntry {
        MediaEntry {
            path: path.to_string_lossy().into_owned(),
            title: "clip.mp4".to_owned(),
            size_bytes: 1,
            modified_unix_seconds: None,
            extension: "mp4".to_owned(),
            offline: false,
            favorite: false,
            tags: Vec::new(),
            game: None,
            recorded_game: None,
            collections: Vec::new(),
            probe: None,
        }
    }

    #[test]
    fn a_clip_over_the_size_starts_a_copy_once_and_reports_when_it_is_done_or_fails() {
        let folder =
            std::env::temp_dir().join(format!("clipcraft-share-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let big = folder.join("big.mp4");
        std::fs::write(&big, vec![0_u8; 3_000_000]).unwrap();
        let context = egui::Context::default();
        let mut app = ClipCraftApp::default();
        app.recording.share_megabytes = 1;
        let entry = entry(&big);
        assert!(matches!(app.share_state(&entry), ShareState::NotMade));

        app.prepare_share(&entry, false);
        assert!(app.sharing.is_some(), "a copy was started");
        assert!(matches!(app.share_state(&entry), ShareState::Making(_)));
        // Asking again while one is being made does nothing new.
        app.prepare_share(&entry, true);
        // The file is not a real video, so the copy fails, and the failure is said.
        for _ in 0..400 {
            app.poll_share(&context);
            if app.sharing.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(app.sharing.is_none());
        let message = app.capture.notice.as_ref().expect("a message");
        assert!(message.error && message.text.contains("Could not make a 1 MB copy"));
        // A clip that is not over the size needs nothing.
        let small = folder.join("small.mp4");
        std::fs::write(&small, vec![0_u8; 10_000]).unwrap();
        app.prepare_share(&entry_for(&small), false);
        assert!(app.sharing.is_none());
        assert!(matches!(
            app.share_state(&entry_for(&small)),
            ShareState::Ready
        ));
        let _ = std::fs::remove_dir_all(&folder);
    }

    fn entry_for(path: &std::path::Path) -> MediaEntry {
        entry(path)
    }

    #[test]
    fn while_clipping_a_copy_is_not_started_by_hovering_but_is_by_asking() {
        let folder =
            std::env::temp_dir().join(format!("clipcraft-share-ui2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let big = folder.join("big.mp4");
        std::fs::write(&big, vec![0_u8; 3_000_000]).unwrap();
        let mut app = ClipCraftApp::default();
        app.recording.share_megabytes = 1;
        let settings = app.recording.clone();
        app.capture.start_synthetic(&settings);
        assert!(app.capture.is_active());
        let entry = entry(&big);
        app.prepare_share(&entry, false);
        assert!(app.sharing.is_none(), "hovering never competes with a game");
        app.prepare_share(&entry, true);
        assert!(app.sharing.is_some(), "asking does");
        drop(app);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
