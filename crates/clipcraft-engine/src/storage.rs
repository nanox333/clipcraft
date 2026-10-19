//! How much room clips take and how much the disk has left.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::VIDEO_EXTENSIONS;

/// Most directory entries looked at when adding up a capture folder.
pub const MAX_USAGE_ENTRIES: usize = 50_000;
/// Deepest sub-folder looked at.
pub const MAX_USAGE_DEPTH: usize = 4;

/// What a capture folder holds and what its disk has left.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct StorageUsage {
    /// The folder measured.
    pub folder: String,
    /// True when the folder exists.
    pub exists: bool,
    /// Number of videos in the folder.
    pub clip_count: u64,
    /// Total size of those videos, in bytes.
    pub used_bytes: u64,
    /// True when the folder had more entries than ClipCraft counts, so the total is a floor.
    pub truncated: bool,
    /// Space still free for ClipCraft on that disk, if the system says.
    pub free_bytes: Option<u64>,
    /// Size of that disk, if the system says.
    pub total_bytes: Option<u64>,
}

/// A video found while measuring.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredClip {
    /// Where it is.
    pub path: PathBuf,
    /// Its size in bytes.
    pub size_bytes: u64,
    /// When it last changed, in seconds since 1970.
    pub modified_unix_seconds: u64,
}

/// Default capture folder, or `None` when no home directory is known.
pub fn default_capture_folder() -> Option<PathBuf> {
    let home = if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("USERPROFILE")?)
    } else {
        PathBuf::from(std::env::var_os("HOME")?)
    };
    let videos = if cfg!(target_os = "macos") {
        "Movies"
    } else {
        "Videos"
    };
    Some(home.join(videos).join("ClipCraft"))
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            VIDEO_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

/// Lists the videos under `folder`, within the entry and depth limits. The flag is true when a
/// limit cut the listing short.
pub fn list_clips(folder: &Path) -> (Vec<StoredClip>, bool) {
    let mut clips = Vec::new();
    let mut seen = 0_usize;
    let mut truncated = false;
    let mut pending = vec![(folder.to_path_buf(), 0_usize)];
    while let Some((directory, depth)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_USAGE_ENTRIES {
                return (clips, true);
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            // Symbolic links are not followed, so a loop cannot trap the walk.
            if kind.is_dir() {
                if depth < MAX_USAGE_DEPTH {
                    pending.push((path, depth + 1));
                } else {
                    truncated = true;
                }
            } else if kind.is_file() && is_video(&path) {
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                clips.push(StoredClip {
                    path,
                    size_bytes: metadata.len(),
                    modified_unix_seconds: metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |age| age.as_secs()),
                });
            }
        }
    }
    (clips, truncated)
}

/// Measures a capture folder and its disk.
pub fn storage_usage(folder: &Path) -> StorageUsage {
    let exists = folder.is_dir();
    let (clips, truncated) = if exists {
        list_clips(folder)
    } else {
        (Vec::new(), false)
    };
    // Disk figures come from the nearest folder that exists, so a folder not yet created still
    // shows the room on its disk.
    let probe = folder.ancestors().find(|path| path.is_dir());
    StorageUsage {
        folder: folder.to_string_lossy().into_owned(),
        exists,
        clip_count: clips.len() as u64,
        used_bytes: clips
            .iter()
            .fold(0_u64, |total, clip| total.saturating_add(clip.size_bytes)),
        truncated,
        free_bytes: probe.and_then(|path| fs4::available_space(path).ok()),
        total_bytes: probe.and_then(|path| fs4::total_space(path).ok()),
    }
}

/// The clips to delete, oldest first, so the rest fit in `limit_bytes`. Nothing is deleted when
/// they already fit. A single clip bigger than the limit is deleted too.
pub fn cleanup_plan(clips: &[StoredClip], limit_bytes: u64) -> Vec<PathBuf> {
    let mut ordered: Vec<&StoredClip> = clips.iter().collect();
    ordered.sort_by(|a, b| {
        a.modified_unix_seconds
            .cmp(&b.modified_unix_seconds)
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut total: u128 = ordered.iter().map(|clip| u128::from(clip.size_bytes)).sum();
    let limit_bytes = u128::from(limit_bytes);
    let mut doomed = Vec::new();
    for clip in ordered {
        if total <= limit_bytes {
            break;
        }
        total -= u128::from(clip.size_bytes);
        doomed.push(clip.path.clone());
    }
    doomed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(name: &str, size: u64, modified: u64) -> StoredClip {
        StoredClip {
            path: PathBuf::from(name),
            size_bytes: size,
            modified_unix_seconds: modified,
        }
    }

    #[test]
    fn cleanup_removes_the_oldest_clips_until_the_rest_fit() {
        let clips = [clip("c", 40, 30), clip("a", 40, 10), clip("b", 40, 20)];
        assert!(cleanup_plan(&clips, 120).is_empty());
        assert_eq!(cleanup_plan(&clips, 100), vec![PathBuf::from("a")]);
        assert_eq!(
            cleanup_plan(&clips, 40),
            vec![PathBuf::from("a"), PathBuf::from("b")]
        );
        assert_eq!(cleanup_plan(&clips, 0).len(), 3);
        assert!(cleanup_plan(&[], 0).is_empty());
    }

    #[test]
    fn cleanup_never_overflows() {
        let clips = [clip("a", u64::MAX, 1), clip("b", u64::MAX, 2)];
        assert_eq!(cleanup_plan(&clips, 5).len(), 2);
    }

    #[test]
    fn usage_counts_videos_in_sub_folders_and_ignores_other_files() {
        let dir = std::env::temp_dir().join(format!("clipcraft-usage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("game/deeper")).unwrap();
        fs::write(dir.join("a.mp4"), [0_u8; 10]).unwrap();
        fs::write(dir.join("game/b.MKV"), [0_u8; 20]).unwrap();
        fs::write(dir.join("game/deeper/c.webm"), [0_u8; 5]).unwrap();
        fs::write(dir.join("notes.txt"), [0_u8; 99]).unwrap();
        let usage = storage_usage(&dir);
        assert!(usage.exists && !usage.truncated);
        assert_eq!((usage.clip_count, usage.used_bytes), (3, 35));
        assert!(usage.total_bytes.is_some_and(|total| total > 0));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_reports_zero_use_and_still_shows_disk_space() {
        let missing = std::env::temp_dir().join("clipcraft-does-not-exist/nested");
        let usage = storage_usage(&missing);
        assert!(!usage.exists);
        assert_eq!((usage.clip_count, usage.used_bytes), (0, 0));
        assert!(usage.free_bytes.is_some());
    }
}
