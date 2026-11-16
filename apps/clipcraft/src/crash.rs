//! When something inside ClipCraft panics: keep going where that is safe, say so, and write down
//! what happened.
//!
//! Three parts:
//! - a panic hook that appends a short note (version, thread, message, source line) to
//!   `crash-reports.log` beside the settings, trimmed to its last 64 KiB, and counts the panic.
//!   Nothing is sent anywhere;
//! - [`FrameGuard`], which runs one frame of the window so that a panic while drawing does not take
//!   the whole program (and the replay in memory) down with it;
//! - [`count`], so the window can tell the person about a panic on any thread, including worker
//!   threads whose own panics were already caught.

use std::fs;
use std::io::Read;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use eframe::egui::{self, RichText};

use crate::capture::notice;
use crate::kit::{self, MUTED, TEXT};
use crate::{ClipCraftApp, tray};

/// How much of the log is kept.
const MAX_LOG_BYTES: usize = 64 * 1024;
/// Notes written in one run; a panic on every frame must not fill the disk or the log.
const MAX_NOTES_PER_RUN: u32 = 20;
/// Frames in a row that may panic before the window shows its fallback screen instead.
pub const FALLBACK_AFTER: u32 = 3;

static PANICS: AtomicU32 = AtomicU32::new(0);
static LOG: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Panics so far in this run, on any thread.
pub fn count() -> u32 {
    PANICS.load(Ordering::SeqCst)
}

/// Where the notes are written: beside the settings, or nowhere when there is no data folder.
pub fn default_log_path() -> Option<PathBuf> {
    let path = clipcraft_engine::default_settings_path()?.with_file_name("crash-reports.log");
    path.is_absolute().then_some(path)
}

/// Starts writing notes to `log` and counting panics. The earlier hook still runs, so nothing that
/// was printed before is lost.
pub fn install(log: Option<PathBuf>) {
    if let Ok(mut slot) = LOG.lock() {
        *slot = log;
    }
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let number = PANICS.fetch_add(1, Ordering::SeqCst) + 1;
        if number <= MAX_NOTES_PER_RUN {
            let thread = std::thread::current();
            let message = info
                .payload()
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "(no message)".to_owned());
            let place = info.location().map_or_else(
                || "unknown place".to_owned(),
                |location| format!("{}:{}", location.file(), location.line()),
            );
            let entry = format_entry(
                clipcraft_engine::unix_now(),
                thread.name().unwrap_or("unnamed thread"),
                &message,
                &place,
            );
            if let Ok(slot) = LOG.lock()
                && let Some(path) = slot.as_deref()
            {
                append_entry(path, &entry);
            }
        }
        previous(info);
    }));
}

/// One note, as plain text.
fn format_entry(unix_time: u64, thread: &str, message: &str, place: &str) -> String {
    // A message can be long (a whole damaged value); the first part says what is needed.
    let message: String = message.chars().take(500).collect();
    format!(
        "[{unix_time}] ClipCraft {} · thread \"{thread}\" · {place}\n    {message}\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// Adds `entry` to the log at `path`, keeping only the last [`MAX_LOG_BYTES`]. Best effort: the
/// hook must never fail, so every step ignores its error.
fn append_entry(path: &Path, entry: &str) {
    let mut kept = Vec::new();
    if let Ok(file) = fs::File::open(path) {
        let _ = file.take((MAX_LOG_BYTES * 2) as u64).read_to_end(&mut kept);
    }
    kept.extend_from_slice(entry.as_bytes());
    if kept.len() > MAX_LOG_BYTES {
        let start = kept.len() - MAX_LOG_BYTES;
        // Start at the next line, so a note is never cut in half.
        let start = kept[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(start, |offset| start + offset + 1);
        kept.drain(..start);
    }
    if let Some(folder) = path.parent() {
        let _ = fs::create_dir_all(folder);
    }
    let _ = fs::write(path, kept);
}

/// The program cannot go on (its window cannot be opened, or an internal error ended it): write
/// it down and say so in a box, since a program without a console would otherwise just vanish.
pub fn report_fatal(what: &str, advice: &str) {
    let entry = format_entry(
        clipcraft_engine::unix_now(),
        "main",
        &format!("fatal: {what}"),
        "startup or exit",
    );
    if let Ok(slot) = LOG.lock()
        && let Some(path) = slot.as_deref()
    {
        append_entry(path, &entry);
    }
    clipcraft_platform::show_error_box("ClipCraft", &fatal_text(what, advice));
}

/// What to try when the window could not be opened, from what went wrong.
pub fn startup_advice(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("display") || lower.contains("wayland") || lower.contains("x11") {
        "No desktop was found to open a window on. Start ClipCraft from your desktop session, not from a plain terminal, a remote shell or a service. "
    } else {
        "This usually means the graphics driver is missing, too old, or not available in this kind of session (remote desktop, some virtual machines): ClipCraft needs OpenGL 3.3. Update the graphics driver and start ClipCraft again. "
    }
}

/// The words of the box.
fn fatal_text(what: &str, advice: &str) -> String {
    let what: String = what.chars().take(600).collect();
    format!(
        "{what}\n\n{advice}Your saved clips are not affected. What happened was also saved in crash-reports.log beside your settings."
    )
}

/// Runs one frame of the window at a time and survives a panic in it.
#[derive(Default)]
pub struct FrameGuard {
    /// Frames in a row that panicked.
    streak: u32,
}

impl FrameGuard {
    /// Runs `frame`. Returns whether it finished. A panic is caught (the hook has already written
    /// its note); the caller decides what to show.
    pub fn run(&mut self, frame: impl FnOnce()) -> bool {
        match panic::catch_unwind(AssertUnwindSafe(frame)) {
            Ok(()) => {
                self.streak = 0;
                true
            }
            Err(_) => {
                self.streak = self.streak.saturating_add(1);
                false
            }
        }
    }

    /// The window panics on frame after frame: it is better to show a plain fallback screen than
    /// to keep drawing the broken one.
    pub fn broken(&self) -> bool {
        self.streak >= FALLBACK_AFTER
    }

    /// The fallback screen was dismissed: try the normal window again.
    pub fn retry(&mut self) {
        self.streak = 0;
    }
}

impl ClipCraftApp {
    /// One frame of the window, guarded against a panic. A panic is written down by the hook and
    /// said on screen; after a few in a row the window shows a plain fallback screen instead of
    /// the one that keeps failing. Recording is a separate thread and is not touched either way.
    pub(crate) fn guarded_frame(&mut self, context: &egui::Context) {
        let mut guard = std::mem::take(&mut self.frame_guard);
        if guard.broken() {
            self.fallback_frame(context, &mut guard);
        } else if !guard.run(|| self.update_frame(context)) {
            context.request_repaint();
        }
        self.frame_guard = guard;
        // A panic on any thread (a worker's was caught there, so nothing else would say so).
        let seen = count();
        if seen != self.panics_told {
            self.panics_told = seen;
            if self
                .capture
                .notice
                .as_ref()
                .is_none_or(|current| !current.error)
            {
                self.capture.notice = notice(
                    "Something went wrong inside ClipCraft and it kept running. What happened was saved in crash-reports.log beside your settings.",
                    true,
                    None,
                );
            }
        }
    }

    /// The screen shown while the normal one keeps panicking. It keeps the recording's housekeeping
    /// (shortcuts, saving) going where that still works.
    fn fallback_frame(&mut self, context: &egui::Context, guard: &mut FrameGuard) {
        FrameGuard::default().run(|| self.capture_frame(context));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(kit::CANVAS))
            .show(context, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(140.0);
                    ui.label(
                        RichText::new("ClipCraft had a problem drawing this screen")
                            .color(TEXT)
                            .size(22.0),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(
                            "Recording and your shortcuts keep working. What happened was saved in crash-reports.log beside your settings.",
                        )
                        .color(MUTED)
                        .size(14.0),
                    );
                    ui.add_space(20.0);
                    if ui.button("Try again").clicked() {
                        guard.retry();
                    }
                    ui.add_space(6.0);
                    if ui.button("Close ClipCraft").clicked() {
                        tray::Tray::quit(context);
                    }
                });
            });
        // Nothing else would draw again until the next input; keep the shortcuts' housekeeping on.
        context.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "clipcraft-crash-{name}-{}-{}",
            std::process::id(),
            clipcraft_engine::unix_now()
        ));
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn a_panicking_frame_is_caught_counted_and_the_next_one_runs() {
        let mut guard = FrameGuard::default();
        assert!(guard.run(|| {}));
        // The hook is not installed here, so this prints the usual message and nothing more.
        assert!(!guard.run(|| panic!("a widget drew outside its list")));
        assert!(!guard.broken());
        assert!(guard.run(|| {}));
        // Three in a row is "broken"; one good frame in between starts the count again.
        for _ in 0..FALLBACK_AFTER {
            assert!(!guard.run(|| panic!("again")));
        }
        assert!(guard.broken());
        guard.retry();
        assert!(!guard.broken());
        assert!(guard.run(|| {}));
    }

    #[test]
    fn the_fatal_box_says_what_happened_what_to_try_and_that_clips_are_safe() {
        let text = fatal_text(
            "ClipCraft could not open its window: no OpenGL",
            "Update the driver. ",
        );
        assert!(
            text.starts_with(
                "ClipCraft could not open its window: no OpenGL\n\nUpdate the driver. "
            )
        );
        assert!(text.contains("saved clips are not affected"));
        assert!(text.contains("crash-reports.log"));
        // A very long reason is cut.
        assert!(fatal_text(&"x".repeat(5_000), "").len() < 900);
    }

    #[test]
    fn the_advice_fits_the_cause() {
        assert!(startup_advice("neither WAYLAND_DISPLAY nor DISPLAY is set").contains("desktop"));
        assert!(startup_advice("NoGlutinConfigs").contains("OpenGL 3.3"));
        assert!(startup_advice("").contains("graphics driver"));
    }

    #[test]
    fn notes_are_plain_short_and_the_log_keeps_only_its_last_part_whole_lines() {
        let folder = scratch("log");
        let path = folder.join("sub").join("crash-reports.log");
        append_entry(&path, &format_entry(1, "main", "first", "a.rs:1"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("thread \"main\" · a.rs:1"));
        assert!(text.contains("    first"));
        // A huge message is cut, and a flood of notes keeps the newest and never splits one.
        let long = "x".repeat(100_000);
        for number in 0..400 {
            append_entry(&path, &format_entry(number, "worker", &long, "b.rs:2"));
        }
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.len() <= MAX_LOG_BYTES, "{}", bytes.len());
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with('['), "starts at a whole note");
        assert!(text.contains("[399]"));
        assert!(!text.contains(&long));
        // A path that cannot be written is ignored, not a second panic.
        append_entry(&folder.join("sub").join("crash-reports.log").join("x"), "y");
        let _ = fs::remove_dir_all(folder);
    }
}
