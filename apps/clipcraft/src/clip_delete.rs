//! Deleting a clip: ask first, then move the file to the Recycle Bin, so it can be put back.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use clipcraft_engine::{Command, EngineError, MediaEntry, execute, format_bytes};
use eframe::egui::{self, RichText};

use crate::capture::notice;
use crate::kit::{self, DANGER, MUTED, TEXT};
use crate::{ClipCraftApp, Icon};

/// A clip being moved to the Recycle Bin on another thread.
pub(crate) struct Trashing {
    title: String,
    receiver: mpsc::Receiver<Result<(), EngineError>>,
}

impl ClipCraftApp {
    /// Opens the "are you sure" box for `entry`.
    pub(crate) fn ask_to_delete(&mut self, entry: MediaEntry) {
        if self.trashing.is_none() {
            self.confirm_delete = Some(entry);
        }
    }

    /// The confirmation box, while there is one. Enter or the red button moves the clip; Escape or
    /// Cancel keeps it.
    pub(crate) fn delete_dialog(&mut self, context: &egui::Context) {
        let Some(entry) = self.confirm_delete.clone() else {
            return;
        };
        let mut decision = None;
        let modal = egui::Modal::new(egui::Id::new("delete-clip-dialog"))
            .frame(
                egui::Frame::new()
                    .fill(kit::PANEL)
                    .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::same(22)),
            )
            .show(context, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new("Delete this clip?")
                        .color(TEXT)
                        .size(18.0)
                        .family(kit::semibold()),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(&entry.title)
                        .color(TEXT)
                        .size(14.0)
                        .family(kit::medium()),
                );
                ui.label(
                    RichText::new(format_bytes(entry.size_bytes))
                        .color(MUTED)
                        .size(12.5),
                );
                ui.add_space(10.0);
                ui.label(
                    RichText::new(
                        "It goes to the Recycle Bin, so you can put it back from there if you change your mind.",
                    )
                    .color(MUTED)
                    .size(13.0),
                );
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    if danger_button(ui, "Move to Recycle Bin").clicked()
                        || ui.input(|input| input.key_pressed(egui::Key::Enter))
                    {
                        decision = Some(true);
                    }
                    if kit::labelled_button(ui, Icon::Restart, "Keep it", false, true).clicked() {
                        decision = Some(false);
                    }
                });
            });
        if modal.should_close() {
            decision = decision.or(Some(false));
        }
        match decision {
            Some(true) => {
                self.confirm_delete = None;
                self.start_trash(entry.path, entry.title);
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }

    /// Moves the clip to the Recycle Bin on a thread. The player may still hold the file for a
    /// moment after it is closed, so a refusal is tried again for a couple of seconds.
    pub(crate) fn start_trash(&mut self, path: String, title: String) {
        let path = PathBuf::from(path);
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| std::path::Path::new(&selected.path) == path)
        {
            self.selected = None;
        }
        self.playback = None;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut last = Err(EngineError::TrashUnavailable);
            for _ in 0..20 {
                last = execute(Command::TrashClip { path: path.clone() }).map(|_| ());
                match &last {
                    Err(EngineError::TrashUnavailable) if path.exists() => {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    _ => break,
                }
            }
            let _ = sender.send(last);
        });
        self.trashing = Some(Trashing { title, receiver });
    }

    /// Picks up the result of a move to the Recycle Bin.
    pub(crate) fn poll_trash(&mut self, context: &egui::Context) {
        let Some(trashing) = self.trashing.as_ref() else {
            return;
        };
        let result = match trashing.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                context.request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(EngineError::TrashUnavailable),
        };
        let title = trashing.title.clone();
        self.trashing = None;
        match result {
            Ok(()) => {
                self.capture.notice =
                    notice(format!("Moved {title} to the Recycle Bin"), false, None);
                self.scan_library();
            }
            Err(error) => {
                self.capture.notice = notice(format!("{title}: {error}"), true, None);
            }
        }
    }
}

/// A red button for a step that removes something.
fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let font = egui::FontId::new(13.5, kit::medium());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, egui::Color32::PLACEHOLDER);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(galley.size().x + 36.0, 36.0),
        egui::Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let fill = if response.hovered() {
        egui::Color32::from_rgb(255, 132, 132)
    } else {
        DANGER
    };
    ui.painter().rect_filled(rect, 10.0, fill);
    ui.painter().galley(
        egui::pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        egui::Color32::BLACK,
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_bin(path: &std::path::Path) -> Result<(), String> {
        std::fs::remove_file(path).map_err(|error| error.to_string())
    }

    fn entry(path: &std::path::Path) -> MediaEntry {
        MediaEntry {
            path: path.to_string_lossy().into_owned(),
            title: "ClipCraft test.mp4".to_owned(),
            size_bytes: 4,
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
    fn a_clip_is_only_moved_after_the_person_confirms_and_the_library_is_refreshed() {
        clipcraft_engine::set_trash_handler(fake_bin);
        let directory =
            std::env::temp_dir().join(format!("clipcraft-delete-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let clip = directory.join("ClipCraft test.mp4");
        std::fs::write(&clip, b"clip").unwrap();
        let context = kit::test_context();
        let mut app = ClipCraftApp {
            selected: Some(entry(&clip)),
            ..Default::default()
        };

        app.ask_to_delete(entry(&clip));
        assert!(app.confirm_delete.is_some());
        assert!(clip.exists(), "asking does not delete");

        let confirmed = app.confirm_delete.take().unwrap();
        app.start_trash(confirmed.path, confirmed.title);
        assert!(app.selected.is_none(), "the open clip is closed first");
        assert!(app.trashing.is_some());
        app.ask_to_delete(entry(&clip));
        assert!(
            app.confirm_delete.is_none(),
            "nothing new while one is moving"
        );
        for _ in 0..200 {
            app.poll_trash(&context);
            if app.trashing.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(app.trashing.is_none());
        assert!(!clip.exists());
        let notice = app.capture.notice.as_ref().expect("a notice");
        assert!(
            !notice.error && notice.text.contains("Recycle Bin"),
            "{}",
            notice.text
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}
