//! Deleting a clip: ask first, then move the file to the Recycle Bin, so it can be put back.

use std::path::PathBuf;
use std::time::Duration;

use clipcraft_engine::{Command, EngineError, MediaEntry, execute, format_bytes};
use eframe::egui::{self, RichText};

use crate::kit::{self, DANGER, MUTED, TEXT};
use crate::{ClipCraftApp, Icon};

impl ClipCraftApp {
    /// Opens the "are you sure" box for `entry`.
    pub(crate) fn ask_to_delete(&mut self, entry: MediaEntry) {
        self.confirm_delete = Some(entry);
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
                    .corner_radius(egui::CornerRadius::same(crate::kit::radius::LG))
                    .inner_margin(egui::Margin::same(24)),
            )
            .show(context, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new("Delete this clip?")
                        .color(TEXT)
                        .size(crate::kit::text_size::LEAD)
                        .family(kit::semibold()),
                );
                ui.add_space(crate::kit::space::SM);
                ui.label(
                    RichText::new(&entry.title)
                        .color(TEXT)
                        .size(crate::kit::text_size::BODY)
                        .family(kit::medium()),
                );
                ui.label(
                    RichText::new(format_bytes(entry.size_bytes))
                        .color(MUTED)
                        .size(crate::kit::text_size::CAPTION),
                );
                if entry.favorite {
                    ui.add_space(crate::kit::space::SM);
                    crate::notify::inline(
                        ui,
                        crate::notify::Kind::Warning,
                        "This clip is one of your favorites.",
                    );
                }
                ui.add_space(crate::kit::space::MD);
                ui.label(
                    RichText::new(
                        "It leaves the page now. A message with Undo stays on screen; when you close it, the clip moves to the Recycle Bin, where you can still put it back.",
                    )
                    .color(MUTED)
                    .size(crate::kit::text_size::SMALL),
                );
                ui.add_space(crate::kit::space::XL);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = crate::kit::space::MD;
                    // Enter does not confirm: the default for a destructive step is to keep.
                    if danger_button(ui, "Move to Recycle Bin").clicked() {
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
                self.hold_for_undo(vec![entry.path]);
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }
}

/// Moves one clip's file to the Recycle Bin. The player may still hold the file for a moment, so
/// a refusal is tried again for a couple of seconds. Never deletes for good.
pub(crate) fn move_clip_to_trash(path: PathBuf) -> Result<(), EngineError> {
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
    last
}

/// A red button for a step that removes something.
pub(crate) fn danger_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let font = egui::FontId::new(crate::kit::text_size::SMALL, kit::medium());
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
    ui.painter()
        .rect_filled(rect, f32::from(crate::kit::radius::MD), fill);
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
    fn a_clip_waits_for_undo_and_is_only_moved_when_the_message_is_closed() {
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

        // Confirming takes the clip off the page and starts the Undo message; nothing moves yet.
        let confirmed = app.confirm_delete.take().unwrap();
        app.hold_for_undo(vec![confirmed.path.clone()]);
        assert!(app.selected.is_none(), "the open clip is closed first");
        assert!(clip.exists(), "nothing moves while the message is up");
        assert!(
            app.capture
                .notice
                .as_ref()
                .is_some_and(|notice| notice.undo)
        );
        // Another delete joins the wait instead of replacing it.
        app.hold_for_undo(vec!["/clips/other.mp4".to_owned()]);
        assert!(
            app.capture
                .notice
                .as_ref()
                .is_some_and(|notice| notice.text.starts_with("2 clips"))
        );

        // Undo brings them back with nothing moved.
        app.undo_pending_trash();
        assert!(clip.exists());

        // Closing the message (or Delete now) moves the clip to the Recycle Bin.
        app.hold_for_undo(vec![confirmed.path]);
        app.delete_pending_now();
        for _ in 0..200 {
            app.poll_bulk_trash(&context);
            if !clip.exists() && app.selection_idle() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(!clip.exists());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
