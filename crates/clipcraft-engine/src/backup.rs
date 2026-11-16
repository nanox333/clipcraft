//! Rolling copies of the files that hold a person's choices: the settings, and the library index
//! (where favorites, collections and game labels live).
//!
//! Before such a file is replaced, its last good version is copied to `<name>.bak1`, the older
//! copies moving to `.bak2` and `.bak3`. A new copy is made at most once a day, so the three copies
//! cover three days of changes instead of the last three saves. When a file turns out to be
//! damaged, the newest copy that is readable takes its place and the person is told.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// How many copies are kept.
pub(crate) const KEEP: u32 = 3;
/// The least time between two new copies.
const GAP: Duration = Duration::from_secs(20 * 60 * 60);

static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn copy_name(path: &Path, number: u32) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".bak{number}"));
    PathBuf::from(name)
}

/// Copies `path` as it is now into the rolling set, unless a copy was made within a day.
pub(crate) fn keep_backup(path: &Path) {
    keep_backup_with(path, GAP);
}

pub(crate) fn keep_backup_with(path: &Path, gap: Duration) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if !meta.is_file() || meta.len() == 0 {
        return;
    }
    let newest = copy_name(path, 1);
    let recent = fs::metadata(&newest)
        .and_then(|meta| meta.modified())
        .ok()
        .is_some_and(|modified| modified.elapsed().map_or(true, |age| age < gap));
    if recent {
        return;
    }
    // Older copies move up one place; the oldest falls off the end.
    let _ = fs::remove_file(copy_name(path, KEEP));
    for number in (1..KEEP).rev() {
        let _ = fs::rename(copy_name(path, number), copy_name(path, number + 1));
    }
    let mut temporary = newest.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    if fs::copy(path, &temporary).is_ok() && fs::rename(&temporary, &newest).is_err() {
        let _ = fs::remove_file(&temporary);
    }
}

/// The copies that exist for `path`, newest first.
pub(crate) fn backups(path: &Path) -> Vec<PathBuf> {
    (1..=KEEP)
        .map(|number| copy_name(path, number))
        .filter(|copy| copy.is_file())
        .collect()
}

/// Puts `copy` back in place of `path` (which the caller has already set aside). Best effort: the
/// copy stays where it is.
pub(crate) fn restore_from(copy: &Path, path: &Path) -> bool {
    fs::copy(copy, path).is_ok()
}

/// Remembers that something was restored from a copy, for the app to tell the person.
pub(crate) fn note_restored(what: &str) {
    if let Ok(mut notes) = NOTES.lock()
        && notes.len() < 8
    {
        notes.push(what.to_owned());
    }
}

/// What was restored from a copy since this was last asked (for example `"settings"` or
/// `"library"`), so the person can be told. Empty when nothing was.
pub fn take_restore_notes() -> Vec<String> {
    NOTES
        .lock()
        .map(|mut notes| std::mem::take(&mut *notes))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("clipcraft-backup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn old(path: &Path, hours: u64) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(hours * 3600))
            .unwrap();
    }

    #[test]
    fn copies_roll_oldest_out_and_are_made_at_most_once_a_day() {
        let dir = scratch("roll");
        let file = dir.join("settings.json");
        // Nothing to copy, or an empty file: no copy.
        keep_backup(&file);
        fs::write(&file, b"").unwrap();
        keep_backup(&file);
        assert!(backups(&file).is_empty());
        // The first copy; a second save the same day does not make another.
        fs::write(&file, b"one").unwrap();
        keep_backup(&file);
        fs::write(&file, b"two").unwrap();
        keep_backup(&file);
        assert_eq!(fs::read(copy_name(&file, 1)).unwrap(), b"one");
        assert!(!copy_name(&file, 2).exists());
        // A day later the next copy is made and the first moves up.
        old(&copy_name(&file, 1), 30);
        keep_backup(&file);
        assert_eq!(fs::read(copy_name(&file, 1)).unwrap(), b"two");
        assert_eq!(fs::read(copy_name(&file, 2)).unwrap(), b"one");
        // Only three are ever kept, the newest first.
        for (content, age) in [("three", 30), ("four", 30), ("five", 30)] {
            fs::write(&file, content).unwrap();
            old(&copy_name(&file, 1), age);
            keep_backup(&file);
        }
        let kept = backups(&file);
        assert_eq!(kept.len(), 3);
        assert_eq!(fs::read(&kept[0]).unwrap(), b"five");
        assert_eq!(fs::read(&kept[2]).unwrap(), b"three");
        assert!(!copy_name(&file, 4).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_restore_puts_the_copy_back_and_is_reported_once() {
        let dir = scratch("restore");
        let file = dir.join("index.json");
        fs::write(&file, b"good").unwrap();
        keep_backup(&file);
        fs::write(&file, b"broken").unwrap();
        let copy = backups(&file).remove(0);
        assert!(restore_from(&copy, &file));
        assert_eq!(fs::read(&file).unwrap(), b"good");
        assert!(copy.exists(), "the copy stays");
        let _ = take_restore_notes();
        note_restored("library");
        assert_eq!(take_restore_notes(), ["library"]);
        assert!(take_restore_notes().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
