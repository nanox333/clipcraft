//! A safety net for graphics-card encoding.
//!
//! While the graphics card's encoder is working, ClipCraft keeps a tiny file saying so (a
//! heartbeat every 30 seconds). A clean exit rewrites it. If Windows later reports that the
//! computer restarted without shutting down, and that restart came after the last heartbeat, the
//! previous run ended while the card was encoding, and ClipCraft can offer software encoding. The
//! file holds no paths, names or media; it is replaced atomically and a damaged one is ignored.
//!
//! This proves nothing about the cause: many things restart a computer. It only notices the
//! coincidence and lets the person decide.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds between heartbeats while the graphics card's encoder is working.
pub const HEARTBEAT_SECONDS: u64 = 30;
/// The file is a few dozen bytes; anything larger is not ours.
const MAX_BYTES: u64 = 4096;
const FILE_NAME: &str = "gpu-session.json";
const VERSION: u32 = 1;

/// What the last run left behind.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionMarker {
    version: u32,
    /// The graphics card's encoder was working at `last_seen_unix`.
    pub gpu_encoding: bool,
    /// Unix seconds of the last heartbeat or state change.
    pub last_seen_unix: u64,
    /// ClipCraft closed normally.
    pub clean_exit: bool,
}

/// The verdict on the run before this one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviousRun {
    /// Nothing to report: it ended normally, was not using the card, or no restart followed it.
    Normal,
    /// The computer restarted unexpectedly after ClipCraft was last seen using the card's encoder.
    RestartedWhileEncoding,
}

/// Where the marker lives (beside the settings), or `None` when no data folder is known.
pub fn default_gpu_session_path() -> Option<PathBuf> {
    let path = crate::index::default_index_path()?.with_file_name(FILE_NAME);
    path.is_absolute().then_some(path)
}

/// Where the single-instance lock lives (beside the settings).
pub fn default_instance_lock_path() -> Option<PathBuf> {
    let path = crate::index::default_index_path()?.with_file_name("instance.lock");
    path.is_absolute().then_some(path)
}

/// Where a second launch leaves a request for the running window to come forward.
pub fn default_show_request_path() -> Option<PathBuf> {
    let path = crate::index::default_index_path()?.with_file_name("show-window.request");
    path.is_absolute().then_some(path)
}

/// Seconds since 1970, or 0 when the clock is before it.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl SessionMarker {
    /// Reads the marker. `None` when it is missing, too large, damaged or from another version.
    pub fn read(path: &Path) -> Option<Self> {
        let file = fs::File::open(path).ok()?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
        if u64::try_from(bytes.len()).ok()? > MAX_BYTES {
            return None;
        }
        let marker: Self = serde_json::from_slice(&bytes).ok()?;
        (marker.version == VERSION).then_some(marker)
    }

    fn write(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_vec(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, text)?;
        fs::rename(&temporary, path).inspect_err(|_| {
            let _ = fs::remove_file(&temporary);
        })
    }
}

/// Decides whether the previous run ended while the card's encoder was working and the computer
/// then restarted unexpectedly. `restart_unix` is when Windows last logged an unexpected restart.
pub fn judge_previous_run(marker: Option<SessionMarker>, restart_unix: Option<u64>) -> PreviousRun {
    match (marker, restart_unix) {
        (Some(marker), Some(restart))
            if marker.gpu_encoding && !marker.clean_exit && restart >= marker.last_seen_unix =>
        {
            PreviousRun::RestartedWhileEncoding
        }
        _ => PreviousRun::Normal,
    }
}

/// Whether a marker is worth asking Windows about: only one left by a run that was using the
/// card and did not exit cleanly.
pub fn needs_restart_check(marker: Option<SessionMarker>) -> bool {
    marker.is_some_and(|marker| marker.gpu_encoding && !marker.clean_exit)
}

/// Keeps the marker current for this run. Every write is best effort: a failure is ignored, since
/// the marker is a safety net and must never get in the way of recording.
#[derive(Debug)]
pub struct SessionTracker {
    path: Option<PathBuf>,
    last: SessionMarker,
}

impl SessionTracker {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            last: SessionMarker {
                version: VERSION,
                gpu_encoding: false,
                last_seen_unix: 0,
                clean_exit: false,
            },
        }
    }

    fn save(&mut self, gpu_encoding: bool, clean_exit: bool, now: u64) {
        self.last = SessionMarker {
            version: VERSION,
            gpu_encoding,
            last_seen_unix: now,
            clean_exit,
        };
        if let Some(path) = &self.path {
            let _ = self.last.write(path);
        }
    }

    /// Replaces the previous run's marker. Call once, after reading it.
    pub fn start(&mut self, now: u64) {
        self.save(false, false, now);
    }

    /// Tells the tracker whether the card's encoder is working. Writes when that changes, and
    /// every [`HEARTBEAT_SECONDS`] while it is working.
    pub fn observe(&mut self, gpu_active: bool, now: u64) {
        let changed = gpu_active != self.last.gpu_encoding;
        let due = gpu_active && now >= self.last.last_seen_unix.saturating_add(HEARTBEAT_SECONDS);
        if changed || due {
            self.save(gpu_active, false, now);
        }
    }

    /// Marks a normal exit.
    pub fn finish(&mut self, now: u64) {
        self.save(false, true, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "clipcraft-gpu-session-{name}-{}-{}",
            std::process::id(),
            unix_now()
        ));
        fs::create_dir_all(&directory).unwrap();
        directory.join(FILE_NAME)
    }

    fn marker(gpu: bool, seen: u64, clean: bool) -> SessionMarker {
        SessionMarker {
            version: VERSION,
            gpu_encoding: gpu,
            last_seen_unix: seen,
            clean_exit: clean,
        }
    }

    #[test]
    fn a_restart_after_the_card_was_encoding_is_reported() {
        let unclean = Some(marker(true, 1_000, false));
        assert_eq!(
            judge_previous_run(unclean, Some(1_050)),
            PreviousRun::RestartedWhileEncoding
        );
        assert_eq!(
            judge_previous_run(unclean, Some(1_000)),
            PreviousRun::RestartedWhileEncoding
        );
    }

    #[test]
    fn nothing_is_reported_without_all_three_signs() {
        // A restart logged before the last heartbeat belongs to an earlier run.
        assert_eq!(
            judge_previous_run(Some(marker(true, 1_000, false)), Some(999)),
            PreviousRun::Normal
        );
        // No unexpected restart on record: the program was closed, killed or crashed instead.
        assert_eq!(
            judge_previous_run(Some(marker(true, 1_000, false)), None),
            PreviousRun::Normal
        );
        // The card was idle, or the exit was clean.
        assert_eq!(
            judge_previous_run(Some(marker(false, 1_000, false)), Some(2_000)),
            PreviousRun::Normal
        );
        assert_eq!(
            judge_previous_run(Some(marker(true, 1_000, true)), Some(2_000)),
            PreviousRun::Normal
        );
        // No marker at all (first run, or a damaged file).
        assert_eq!(judge_previous_run(None, Some(2_000)), PreviousRun::Normal);
    }

    #[test]
    fn windows_is_only_asked_when_the_last_run_ended_on_the_card() {
        assert!(needs_restart_check(Some(marker(true, 5, false))));
        assert!(!needs_restart_check(Some(marker(true, 5, true))));
        assert!(!needs_restart_check(Some(marker(false, 5, false))));
        assert!(!needs_restart_check(None));
    }

    #[test]
    fn the_tracker_writes_on_change_and_on_heartbeat_only() {
        let path = scratch("tracker");
        let mut tracker = SessionTracker::new(Some(path.clone()));
        tracker.start(100);
        assert_eq!(
            SessionMarker::read(&path),
            Some(marker(false, 100, false)),
            "a new run replaces the old marker at once"
        );
        tracker.observe(false, 110);
        assert_eq!(SessionMarker::read(&path).unwrap().last_seen_unix, 100);
        tracker.observe(true, 120);
        assert_eq!(SessionMarker::read(&path), Some(marker(true, 120, false)));
        tracker.observe(true, 120 + HEARTBEAT_SECONDS - 1);
        assert_eq!(SessionMarker::read(&path).unwrap().last_seen_unix, 120);
        tracker.observe(true, 120 + HEARTBEAT_SECONDS);
        assert_eq!(
            SessionMarker::read(&path).unwrap().last_seen_unix,
            120 + HEARTBEAT_SECONDS
        );
        tracker.observe(false, 200);
        assert_eq!(SessionMarker::read(&path), Some(marker(false, 200, false)));
        tracker.observe(true, 210);
        tracker.finish(220);
        assert_eq!(SessionMarker::read(&path), Some(marker(false, 220, true)));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_crash_while_encoding_leaves_an_unclean_marker_for_the_next_launch() {
        let path = scratch("crash");
        let mut tracker = SessionTracker::new(Some(path.clone()));
        tracker.start(10);
        tracker.observe(true, 20);
        drop(tracker); // no finish(): the process died
        let left = SessionMarker::read(&path);
        assert!(needs_restart_check(left));
        assert_eq!(
            judge_previous_run(left, Some(500)),
            PreviousRun::RestartedWhileEncoding
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn hostile_files_are_ignored_and_a_missing_folder_never_panics() {
        let path = scratch("hostile");
        fs::write(&path, b"{ not json").unwrap();
        assert_eq!(SessionMarker::read(&path), None);
        fs::write(&path, vec![b' '; 5000]).unwrap();
        assert_eq!(SessionMarker::read(&path), None, "oversized");
        fs::write(
            &path,
            br#"{"version":99,"gpu_encoding":true,"last_seen_unix":1,"clean_exit":false}"#,
        )
        .unwrap();
        assert_eq!(SessionMarker::read(&path), None, "another version");
        fs::write(
            &path,
            br#"{"version":1,"gpu_encoding":true,"last_seen_unix":-5,"clean_exit":false}"#,
        )
        .unwrap();
        assert_eq!(SessionMarker::read(&path), None, "negative time");
        assert_eq!(SessionMarker::read(Path::new("/definitely/not/here")), None);
        // Writing into a folder that does not exist fails quietly.
        let mut tracker =
            SessionTracker::new(Some(path.parent().unwrap().join("missing").join(FILE_NAME)));
        tracker.start(1);
        tracker.observe(true, 2);
        tracker.finish(3);
        // Without any data folder it does nothing at all.
        let mut nowhere = SessionTracker::new(None);
        nowhere.start(1);
        nowhere.observe(true, u64::MAX);
        nowhere.finish(u64::MAX);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }
}
