//! **Refresh** on the Clips page: look at the folder again and check every clip.
//!
//! The scan finds new and removed files. The check reopens each clip that is there, in the
//! background, and says which ones do not open, have no picture or no length. Nothing is changed
//! or deleted; a clip that looks damaged is only named.

use std::path::PathBuf;
use std::sync::mpsc;

use clipcraft_engine::ClipCheck;
use eframe::egui;

use crate::ClipCraftApp;
use crate::capture::notice;

/// What the check found.
pub(crate) struct CheckResult {
    checked: usize,
    missing: usize,
    doubtful: Vec<(String, String)>,
}

/// A refresh: first the folder is scanned, then the clips that are there are checked.
pub(crate) struct LibraryCheck {
    /// The check itself, once the scan has landed.
    receiver: Option<mpsc::Receiver<CheckResult>>,
}

/// Clips named in the message before "and N more".
const NAMED: usize = 3;

/// The message for a finished check.
pub(crate) fn summary(result: &CheckResult) -> (String, bool) {
    let CheckResult {
        checked,
        missing,
        doubtful,
    } = result;
    let mut text = if doubtful.is_empty() {
        format!(
            "Refreshed. {checked} clip{} checked, all fine.",
            if *checked == 1 { "" } else { "s" }
        )
    } else {
        let named: Vec<String> = doubtful
            .iter()
            .take(NAMED)
            .map(|(name, why)| format!("{name} ({why})"))
            .collect();
        let more = doubtful.len().saturating_sub(NAMED);
        format!(
            "Refreshed. {} of {checked} clip{} may be damaged: {}{}.",
            doubtful.len(),
            if *checked == 1 { "" } else { "s" },
            named.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        )
    };
    if *missing > 0 {
        text.push_str(&format!(
            " {missing} file{} could not be found.",
            if *missing == 1 { "" } else { "s" }
        ));
    }
    (text, !doubtful.is_empty())
}

impl ClipCraftApp {
    /// Scans the folder again; once the new list has landed, every clip is checked on a thread.
    pub(crate) fn refresh_library(&mut self) {
        if self.library_check.is_some() {
            return;
        }
        self.scan_library();
        self.library_check = Some(LibraryCheck { receiver: None });
        self.capture.notice = notice("Refreshing and checking your clips…", false, None);
    }

    /// Starts the check when the scan has landed, and picks up its answer.
    pub(crate) fn poll_library_check(&mut self, context: &egui::Context) {
        let Some(check) = self.library_check.as_ref() else {
            return;
        };
        let Some(receiver) = check.receiver.as_ref() else {
            // Wait for the scan and its write to the library to finish.
            if self.scanning.is_some() || self.index_store.pending() > 0 {
                context.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            let clips: Vec<(String, PathBuf, bool)> = self
                .library
                .as_ref()
                .map(|library| {
                    library
                        .entries
                        .iter()
                        .map(|entry| {
                            (
                                entry.title.clone(),
                                PathBuf::from(&entry.path),
                                entry.offline,
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(check_all(clips));
            });
            if let Some(check) = self.library_check.as_mut() {
                check.receiver = Some(receiver);
            }
            context.request_repaint();
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.library_check = None;
                let (text, error) = summary(&result);
                self.capture.notice = crate::capture::notice_as(
                    if error {
                        crate::notify::Kind::Warning
                    } else {
                        crate::notify::Kind::Success
                    },
                    text,
                    None,
                );
            }
            Err(mpsc::TryRecvError::Empty) => {
                context.request_repaint_after(std::time::Duration::from_millis(200));
            }
            Err(mpsc::TryRecvError::Disconnected) => self.library_check = None,
        }
    }
}

/// Checks each clip: its title, where it is, and whether the library already knows it is gone.
fn check_all(clips: Vec<(String, PathBuf, bool)>) -> CheckResult {
    let mut result = CheckResult {
        checked: 0,
        missing: 0,
        doubtful: Vec::new(),
    };
    for (title, path, offline) in clips {
        if offline || !path.is_file() {
            result.missing += 1;
            continue;
        }
        result.checked += 1;
        if let ClipCheck::Doubtful(why) = clipcraft_engine::check_library_clip(&path) {
            result.doubtful.push((title, why));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_names_a_few_damaged_clips_and_counts_the_rest() {
        let fine = CheckResult {
            checked: 5,
            missing: 0,
            doubtful: Vec::new(),
        };
        assert_eq!(
            summary(&fine),
            ("Refreshed. 5 clips checked, all fine.".to_owned(), false)
        );
        let bad = CheckResult {
            checked: 9,
            missing: 1,
            doubtful: (0..5)
                .map(|i| (format!("clip{i}"), "it has no picture".to_owned()))
                .collect(),
        };
        let (text, error) = summary(&bad);
        assert!(error);
        assert!(text.contains("5 of 9 clips may be damaged"), "{text}");
        assert!(text.contains("clip0 (it has no picture)"), "{text}");
        assert!(text.contains("and 2 more"), "{text}");
        assert!(text.contains("1 file could not be found"), "{text}");
        assert!(!text.contains("clip4"), "{text}");
    }

    #[test]
    fn a_real_check_names_a_clip_that_is_not_a_video() {
        let dir = std::env::temp_dir().join(format!("clipcraft-refresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let junk = dir.join("junk.mp4");
        std::fs::write(&junk, b"this is not a video").unwrap();
        assert!(matches!(
            clipcraft_engine::check_library_clip(&junk),
            ClipCheck::Doubtful(_)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
