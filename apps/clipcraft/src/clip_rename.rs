//! Renaming a clip: a small box with the current name, Enter renames, Escape keeps it. The
//! file stays in its folder with its extension, and its favorite, collections and game follow it.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use clipcraft_engine::{
    Command, CommandOutput, EngineError, MAX_CLIP_NAME_CHARS, MediaEntry, MetaEdit,
    clean_clip_name, execute,
};
use eframe::egui::{self, RichText};

use crate::capture::notice;
use crate::kit::{self, DANGER, MUTED, TEXT};
use crate::{ClipCraftApp, Icon};

/// The rename box while it is open.
pub(crate) struct RenameDraft {
    entry: MediaEntry,
    text: String,
    focused: bool,
}

/// A rename running on another thread (the player may hold the file for a moment).
pub(crate) struct Renaming {
    from: String,
    reopen: bool,
    receiver: mpsc::Receiver<Result<String, EngineError>>,
}

/// The clip's file name without its extension.
fn stem(entry: &MediaEntry) -> String {
    Path::new(&entry.path).file_stem().map_or_else(
        || entry.title.clone(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

impl ClipCraftApp {
    /// Opens the rename box for `entry`.
    pub(crate) fn ask_to_rename(&mut self, entry: MediaEntry) {
        if self.renaming.is_none() && !entry.offline {
            self.rename_draft = Some(RenameDraft {
                text: stem(&entry),
                entry,
                focused: false,
            });
        }
    }

    /// The rename box, while there is one.
    pub(crate) fn rename_dialog(&mut self, context: &egui::Context) {
        let Some(mut draft) = self.rename_draft.take() else {
            return;
        };
        let mut decision = None;
        let checked = clean_clip_name(&draft.text);
        let modal = egui::Modal::new(egui::Id::new("rename-clip-dialog"))
            .frame(
                egui::Frame::new()
                    .fill(kit::PANEL)
                    .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::same(22)),
            )
            .show(context, |ui| {
                ui.set_width(400.0);
                ui.label(
                    RichText::new("Rename clip")
                        .color(TEXT)
                        .size(18.0)
                        .family(kit::semibold()),
                );
                ui.add_space(10.0);
                let field = ui.add(
                    egui::TextEdit::singleline(&mut draft.text)
                        .char_limit(MAX_CLIP_NAME_CHARS)
                        .margin(egui::Margin::symmetric(10, 8))
                        .desired_width(ui.available_width()),
                );
                field.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "New clip name")
                });
                if !draft.focused {
                    field.request_focus();
                    draft.focused = true;
                }
                let entered =
                    field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                ui.add_space(6.0);
                match &checked {
                    Ok(_) => {
                        let extension = Path::new(&draft.entry.path)
                            .extension()
                            .map(|extension| extension.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ui.label(
                            RichText::new(format!(
                                "Stays in the same folder as a .{extension} file. Favorites, collections and the game label stay with it."
                            ))
                            .color(MUTED)
                            .size(12.5),
                        );
                    }
                    Err(error) => {
                        ui.label(RichText::new(error.to_string()).color(DANGER).size(12.5));
                    }
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let rename =
                        kit::labelled_button(ui, Icon::Pencil, "Rename", true, checked.is_ok());
                    if (rename.clicked() || entered) && checked.is_ok() {
                        decision = Some(true);
                    }
                    if kit::labelled_button(ui, Icon::Restart, "Cancel", false, true).clicked() {
                        decision = Some(false);
                    }
                });
            });
        if modal.should_close() {
            decision = decision.or(Some(false));
        }
        match (decision, checked) {
            (Some(true), Ok(name)) => self.start_rename(&draft.entry, name),
            (Some(_), _) => {}
            (None, _) => self.rename_draft = Some(draft),
        }
    }

    /// Renames the clip on a thread, closing the player first if it shows this clip. The player
    /// may hold the file a moment longer, so "in use" is tried again for two seconds.
    pub(crate) fn start_rename(&mut self, entry: &MediaEntry, name: String) {
        let reopen = self
            .selected
            .as_ref()
            .is_some_and(|selected| selected.path == entry.path);
        if self
            .playback
            .as_ref()
            .is_some_and(|open| open.path() == entry.path)
        {
            self.playback = None;
        }
        let path = PathBuf::from(&entry.path);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut last = Err(EngineError::ClipInUse);
            for _ in 0..20 {
                last = execute(Command::RenameClip {
                    path: path.clone(),
                    name: name.clone(),
                    index_path: None,
                    directory: None,
                })
                .and_then(|output| match output {
                    CommandOutput::Renamed(renamed) => Ok(renamed.path),
                    _ => Err(EngineError::FileUnavailable),
                });
                match &last {
                    Err(EngineError::ClipInUse) => std::thread::sleep(Duration::from_millis(100)),
                    _ => break,
                }
            }
            let _ = sender.send(last);
        });
        self.renaming = Some(Renaming {
            from: entry.path.clone(),
            reopen,
            receiver,
        });
    }

    /// Picks up a finished rename and moves the clip's library entry to its new name.
    pub(crate) fn poll_rename(&mut self, context: &egui::Context) {
        let Some(renaming) = self.renaming.as_ref() else {
            return;
        };
        let result = match renaming.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                context.request_repaint_after(Duration::from_millis(50));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(EngineError::FileUnavailable),
        };
        let Some(renaming) = self.renaming.take() else {
            return;
        };
        match result {
            Ok(to) if to == renaming.from => {}
            Ok(to) => {
                // The index write goes through the same queue as every other library edit.
                self.edit_library(MetaEdit::RelinkFile {
                    from: renaming.from.clone(),
                    to: to.clone(),
                });
                let fresh = self
                    .library
                    .as_ref()
                    .and_then(|library| library.entries.iter().find(|entry| entry.path == to))
                    .cloned();
                if fresh.is_none() {
                    // Not in the list (another folder, or the edit was refused): read it again.
                    self.scan_library();
                }
                if renaming.reopen {
                    self.selected = fresh;
                }
                let name = Path::new(&to)
                    .file_name()
                    .map_or(to.clone(), |name| name.to_string_lossy().into_owned());
                self.capture.notice = notice(format!("Renamed to {name}"), false, None);
            }
            Err(error) => {
                self.capture.notice = notice(error.to_string(), true, None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: egui::Key) -> Vec<egui::Event> {
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .collect()
    }

    fn frame(app: &mut ClipCraftApp, context: &egui::Context, events: Vec<egui::Event>) {
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 1000.0),
                )),
                events,
                ..Default::default()
            },
            |context| {
                kit::apply_theme(context);
                app.draw_screen(context);
            },
        );
    }

    #[test]
    fn a_clip_is_renamed_with_the_keyboard_alone_and_stays_a_favorite() {
        let folder =
            std::env::temp_dir().join(format!("clipcraft-rename-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let clip = folder.join("Hades 2026-10-19 21-04-55.mp4");
        std::fs::write(&clip, b"not really a video").unwrap();
        let entry = MediaEntry {
            path: clip.to_string_lossy().into_owned(),
            title: "Hades 2026-10-19 21-04-55.mp4".to_owned(),
            size_bytes: 18,
            modified_unix_seconds: None,
            extension: "mp4".to_owned(),
            offline: false,
            favorite: true,
            tags: Vec::new(),
            game: None,
            recorded_game: None,
            collections: vec!["Best".to_owned()],
            probe: None,
        };
        let directory = folder.to_string_lossy().into_owned();
        let context = kit::test_context();
        let mut app = ClipCraftApp {
            library: Some(clipcraft_engine::LibraryScanResult {
                directory: directory.clone(),
                entries: vec![entry.clone()],
                persisted: false,
                collections: vec!["Best".to_owned()],
            }),
            directory_text: directory.clone(),
            ..ClipCraftApp::default()
        };

        // Select: Tab until the clip's card has focus.
        let card = crate::library_grid::card_id(&entry);
        let mut focused = false;
        for _ in 0..80 {
            frame(&mut app, &context, key(egui::Key::Tab));
            if context.memory(|memory| memory.focused()) == Some(card) {
                focused = true;
                break;
            }
        }
        assert!(focused, "Tab reaches the clip card");

        // F2, clear the name, type a new one, Enter.
        frame(&mut app, &context, key(egui::Key::F2));
        assert!(app.rename_draft.is_some(), "F2 opens the rename box");
        frame(&mut app, &context, Vec::new());
        let mut typing: Vec<egui::Event> =
            (0..40).flat_map(|_| key(egui::Key::Backspace)).collect();
        typing.push(egui::Event::Text("Boss rush".to_owned()));
        frame(&mut app, &context, typing);
        frame(&mut app, &context, key(egui::Key::Enter));
        assert!(app.rename_draft.is_none(), "Enter renames");
        for _ in 0..200 {
            frame(&mut app, &context, Vec::new());
            if app.renaming.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let renamed = folder.join("Boss rush.mp4");
        assert!(renamed.is_file());
        assert!(!clip.exists());
        let listed = app
            .library
            .as_ref()
            .and_then(|library| {
                library
                    .entries
                    .iter()
                    .find(|listed| listed.path == renamed.to_string_lossy())
            })
            .cloned()
            .expect("the library lists the new name");
        assert!(listed.favorite);
        assert_eq!(listed.collections, ["Best"]);
        assert_eq!(
            clipcraft_engine::game_of(&listed, &directory).as_deref(),
            Some("Hades")
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn an_unusable_name_keeps_the_box_open_and_the_file_unchanged() {
        let context = kit::test_context();
        let entry = MediaEntry {
            path: "/nowhere/clip.mp4".to_owned(),
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
        };
        let mut app = ClipCraftApp::default();
        app.ask_to_rename(entry);
        frame(&mut app, &context, Vec::new());
        let mut typing: Vec<egui::Event> =
            (0..10).flat_map(|_| key(egui::Key::Backspace)).collect();
        typing.push(egui::Event::Text("a/b".to_owned()));
        frame(&mut app, &context, typing);
        frame(&mut app, &context, key(egui::Key::Enter));
        assert!(app.rename_draft.is_some(), "a refused name is not used");
        assert!(app.renaming.is_none());
    }

    #[test]
    fn the_box_starts_with_the_name_without_its_extension() {
        let entry = MediaEntry {
            path: "/clips/Hades 2026-10-19.mp4".to_owned(),
            title: "Hades 2026-10-19.mp4".to_owned(),
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
        };
        let mut app = ClipCraftApp::default();
        app.ask_to_rename(entry.clone());
        assert_eq!(
            app.rename_draft.as_ref().map(|draft| draft.text.as_str()),
            Some("Hades 2026-10-19")
        );
        app.rename_draft = None;
        app.ask_to_rename(MediaEntry {
            offline: true,
            ..entry
        });
        assert!(
            app.rename_draft.is_none(),
            "a missing file cannot be renamed"
        );
    }
}
