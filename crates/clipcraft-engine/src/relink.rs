//! Finding a clip that was moved: search a folder, and the folders inside it, for a file with the
//! same name and size. Everything is bounded, and links are never followed, so a huge or looping
//! folder tree cannot hang the search or send it outside the chosen folder.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

/// Deepest folder level searched below the chosen one.
const MAX_DEPTH: usize = 8;
/// Most folders opened.
const MAX_FOLDERS: usize = 20_000;
/// Most directory entries looked at.
const MAX_ENTRIES: usize = 400_000;

/// What a search found.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    /// A file with the wanted name and size, if there is one.
    pub found: Option<PathBuf>,
    /// Folders looked into.
    pub folders_searched: usize,
}

/// Looks in `folder` and below for a file named `name` (ignoring case) that is `size_bytes` long.
pub fn find_moved_file(name: &str, size_bytes: u64, folder: &Path) -> SearchResult {
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
    queue.push_back((folder.to_path_buf(), 0));
    let (mut folders, mut entries) = (0_usize, 0_usize);
    while let Some((directory, depth)) = queue.pop_front() {
        if folders >= MAX_FOLDERS {
            break;
        }
        let Ok(listing) = fs::read_dir(&directory) else {
            continue;
        };
        folders += 1;
        for entry in listing.filter_map(Result::ok) {
            entries += 1;
            if entries > MAX_ENTRIES {
                return SearchResult {
                    found: None,
                    folders_searched: folders,
                };
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            // Links are skipped: they can loop, and can lead outside the chosen folder.
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if depth < MAX_DEPTH {
                    queue.push_back((entry.path(), depth + 1));
                }
            } else if kind.is_file()
                && entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(name)
                && entry.metadata().is_ok_and(|meta| meta.len() == size_bytes)
            {
                return SearchResult {
                    found: Some(entry.path()),
                    folders_searched: folders,
                };
            }
        }
    }
    SearchResult {
        found: None,
        folders_searched: folders,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("clipcraft-relink-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("scratch");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn finds_a_moved_file_by_name_and_size_in_a_subfolder() {
        let scratch = Scratch::new("find");
        let deep = scratch.0.join("Games").join("Apex").join("2026");
        fs::create_dir_all(&deep).expect("folders");
        fs::write(deep.join("Clutch.MP4"), b"12345").expect("file");
        // Same name, wrong size, and the right size under another name: neither counts.
        fs::write(scratch.0.join("clutch.mp4"), b"123").expect("decoy");
        fs::write(scratch.0.join("other.mp4"), b"12345").expect("decoy");
        let result = find_moved_file("clutch.mp4", 5, &scratch.0);
        assert_eq!(result.found, Some(deep.join("Clutch.MP4")));
        assert!(result.folders_searched >= 1);
        assert_eq!(find_moved_file("clutch.mp4", 99, &scratch.0).found, None);
    }

    #[test]
    fn a_missing_folder_or_a_file_that_is_not_there_finds_nothing_without_failing() {
        assert_eq!(
            find_moved_file("a.mp4", 1, Path::new("/definitely/not/here")),
            SearchResult {
                found: None,
                folders_searched: 0
            }
        );
        let scratch = Scratch::new("none");
        assert_eq!(find_moved_file("a.mp4", 1, &scratch.0).found, None);
    }

    #[test]
    fn links_are_not_followed_and_depth_is_bounded() {
        let scratch = Scratch::new("links");
        let outside = Scratch::new("outside");
        fs::write(outside.0.join("hidden.mp4"), b"1").expect("file");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside.0, scratch.0.join("link")).expect("link");
            std::os::unix::fs::symlink(&scratch.0, scratch.0.join("loop")).expect("loop");
        }
        assert_eq!(find_moved_file("hidden.mp4", 1, &scratch.0).found, None);
        // Nine folders down is past the limit.
        let mut deep = scratch.0.clone();
        for level in 0..=MAX_DEPTH + 1 {
            deep = deep.join(format!("d{level}"));
        }
        fs::create_dir_all(&deep).expect("deep");
        fs::write(deep.join("far.mp4"), b"1").expect("file");
        assert_eq!(find_moved_file("far.mp4", 1, &scratch.0).found, None);
    }
}
