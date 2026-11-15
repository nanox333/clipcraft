//! One ClipCraft per user.
//!
//! Two copies at once (say one from the Desktop and one installed, both set to start with
//! Windows) would both register the shortcuts, both capture the screen and both open the graphics
//! card's encoder. The first copy to start holds an exclusive lock on a small file in the settings
//! folder; the operating system drops the lock when that program ends, however it ends, so a crash
//! never leaves a stale one behind. A later copy exits at once. When a person started it by hand
//! (not with `--minimized`, which is how Windows starts it at login) it first leaves a note
//! beside the lock, and the running copy, which looks for that note twice a second, brings its
//! window forward. There is no server and nothing is sent anywhere.
//!
//! If the lock cannot be made at all (no settings folder, a read-only disk, a file system without
//! locks) ClipCraft starts anyway: this is a safeguard, not a gate.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use eframe::egui;

/// How often the running copy looks for a note from a later launch.
const LOOK_EVERY: Duration = Duration::from_millis(500);

/// The outcome of trying to be the one copy.
pub enum Claim {
    /// This copy runs. Keep the value until the program ends.
    First(Held),
    /// Another copy already holds the lock.
    AlreadyRunning,
}

/// The lock, kept for as long as this program runs. It holds nothing when the lock could not be
/// made and ClipCraft started anyway.
pub struct Held {
    _file: Option<File>,
}

/// Takes the lock at `path`, or reports that another copy has it.
pub fn claim(path: &Path) -> Claim {
    if let Some(folder) = path.parent() {
        let _ = fs::create_dir_all(folder);
    }
    let Ok(file) = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
    else {
        return Claim::First(Held { _file: None });
    };
    match file.try_lock() {
        Ok(()) => Claim::First(Held { _file: Some(file) }),
        Err(TryLockError::WouldBlock) => Claim::AlreadyRunning,
        Err(TryLockError::Error(_)) => Claim::First(Held { _file: None }),
    }
}

/// Leaves a note for the running copy to bring its window forward. Best effort.
pub fn ask_running_copy_to_show(path: &Path) {
    let _ = fs::write(path, b"");
}

/// Whether a note is waiting; takes it away so it counts once.
fn take_show_request(path: &Path) -> bool {
    path.exists() && fs::remove_file(path).is_ok()
}

/// Starts looking for notes from later launches. The returned flag is set when one arrives, and
/// the window is woken so the next frame can act on it.
pub fn watch_show_requests(path: PathBuf, context: egui::Context) -> Arc<AtomicBool> {
    // A note left by an earlier run is not for this one.
    let _ = fs::remove_file(&path);
    let flag = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&flag);
    let _ = std::thread::Builder::new()
        .name("show-request".into())
        .spawn(move || {
            loop {
                std::thread::sleep(LOOK_EVERY);
                if take_show_request(&path) {
                    seen.store(true, Ordering::SeqCst);
                    context.request_repaint();
                }
            }
        });
    flag
}

/// Brings the window forward from the tray or the taskbar.
pub fn show_window(context: &egui::Context) {
    context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    context.send_viewport_cmd(egui::ViewportCommand::Focus);
    context.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "clipcraft-single-instance-{name}-{}-{}",
            std::process::id(),
            clipcraft_engine::unix_now()
        ));
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn the_second_copy_is_turned_away_until_the_first_ends() {
        let folder = scratch("lock");
        let path = folder.join("instance.lock");
        let first = claim(&path);
        assert!(matches!(first, Claim::First(_)));
        assert!(matches!(claim(&path), Claim::AlreadyRunning));
        assert!(matches!(claim(&path), Claim::AlreadyRunning));
        drop(first);
        // The lock goes with the program that held it: a crash leaves nothing stale.
        let again = claim(&path);
        assert!(matches!(again, Claim::First(_)));
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn a_missing_folder_is_made_and_an_impossible_lock_never_blocks_the_start() {
        let folder = scratch("make");
        let nested = folder.join("a").join("b").join("instance.lock");
        assert!(matches!(
            claim(&nested),
            Claim::First(Held { _file: Some(_) })
        ));
        // The lock path is a folder, so it cannot be opened as a file: start anyway.
        let blocked = folder.join("is-a-folder");
        fs::create_dir_all(&blocked).unwrap();
        assert!(matches!(
            claim(&blocked),
            Claim::First(Held { _file: None })
        ));
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn a_note_counts_once() {
        let folder = scratch("note");
        let path = folder.join("show-window.request");
        assert!(!take_show_request(&path));
        ask_running_copy_to_show(&path);
        ask_running_copy_to_show(&path);
        assert!(take_show_request(&path));
        assert!(!take_show_request(&path));
        // Asking in a folder that is gone does nothing and does not panic.
        ask_running_copy_to_show(&folder.join("gone").join("x"));
        let _ = fs::remove_dir_all(folder);
    }

    #[test]
    fn the_watcher_sets_the_flag_for_a_new_note_and_ignores_an_old_one() {
        let folder = scratch("watch");
        let path = folder.join("show-window.request");
        ask_running_copy_to_show(&path); // left by an earlier run
        let flag = watch_show_requests(path.clone(), egui::Context::default());
        assert!(!path.exists(), "an earlier run's note is cleared at start");
        ask_running_copy_to_show(&path);
        let started = std::time::Instant::now();
        while !flag.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(flag.swap(false, Ordering::SeqCst), "the note was noticed");
        assert!(!path.exists());
        let _ = fs::remove_dir_all(folder);
    }
}
