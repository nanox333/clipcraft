//! Noticing that ClipCraft keeps failing to start, so the person can get in.
//!
//! Each start adds one to a small counter file; the count goes back to zero once the program has
//! run for half a minute or is closed normally. If the last few starts never got that far, the
//! next one offers to start with the default settings (the old file is kept, nothing else is
//! touched), because a settings file the program cannot cope with is the usual cause.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Starts in a row that never got going before ClipCraft offers a safe start.
pub const CRASH_LIMIT: u32 = 3;

/// How long the program has to run before a start counts as good.
pub const HEALTHY_AFTER: Duration = Duration::from_secs(30);

/// Where the counter lives, beside the settings.
pub fn default_safe_start_path() -> Option<PathBuf> {
    Some(crate::index::default_index_path()?.with_file_name("startup-count"))
}

/// Counts this start and returns how many starts before it never got going.
pub fn begin(path: &Path) -> u32 {
    let before = read_count(path);
    write_count(path, before.saturating_add(1));
    before
}

/// This start is good (it ran a while, or ended normally): start counting again from zero.
pub fn mark_healthy(path: &Path) {
    if read_count(path) != 0 {
        write_count(path, 0);
    }
}

/// True when the starts before this one failed often enough to offer a safe start.
pub fn needs_safe_start(unclean_starts: u32) -> bool {
    unclean_starts >= CRASH_LIMIT
}

/// Keeps the settings file as `.before-safe-start` (replacing an older such copy) and removes it,
/// so the next load gives the defaults. Clips, the library and drafts are not touched. Returns
/// the kept copy.
pub fn reset_settings(settings: &Path) -> std::io::Result<Option<PathBuf>> {
    if !settings.is_file() {
        return Ok(None);
    }
    let mut kept = settings.as_os_str().to_owned();
    kept.push(".before-safe-start");
    let kept = PathBuf::from(kept);
    fs::copy(settings, &kept)?;
    fs::remove_file(settings)?;
    Ok(Some(kept))
}

fn read_count(path: &Path) -> u32 {
    match fs::metadata(path) {
        Ok(meta) if meta.len() <= 16 => fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0),
        _ => 0,
    }
}

fn write_count(path: &Path, count: u32) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, count.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clipcraft-safe-start-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        dir
    }

    #[test]
    fn starts_that_never_get_going_add_up_and_a_good_start_clears_them() {
        let dir = scratch("count");
        let path = dir.join("startup-count");
        assert_eq!(begin(&path), 0);
        assert_eq!(begin(&path), 1);
        assert_eq!(begin(&path), 2);
        assert!(!needs_safe_start(2));
        assert_eq!(begin(&path), 3);
        assert!(needs_safe_start(3));
        mark_healthy(&path);
        assert_eq!(begin(&path), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_or_oversized_counter_counts_as_nothing() {
        let dir = scratch("junk");
        let path = dir.join("startup-count");
        fs::write(&path, "not a number").expect("fixture");
        assert_eq!(begin(&path), 0);
        fs::write(&path, vec![b'9'; 4096]).expect("fixture");
        assert_eq!(begin(&path), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_safe_start_keeps_a_copy_of_the_settings_and_nothing_else_moves() {
        let dir = scratch("reset");
        let settings = dir.join("recording-settings.json");
        let clip = dir.join("clip.mp4");
        fs::write(&settings, b"my settings").expect("fixture");
        fs::write(&clip, b"video").expect("fixture");
        let kept = reset_settings(&settings).expect("reset").expect("kept");
        assert_eq!(fs::read(&kept).expect("copy"), b"my settings");
        assert!(!settings.exists());
        assert!(clip.exists());
        // With no settings file there is nothing to keep.
        assert_eq!(reset_settings(&settings).expect("ok"), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
