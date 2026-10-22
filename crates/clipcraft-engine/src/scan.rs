//! Bounded, cancellable traversal shared by the desktop and CLI.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use crate::{EngineError, LibraryScanResult, MediaEntry, VIDEO_EXTENSIONS};

/// Child-folder levels visited below the chosen library root.
pub const MAX_SCAN_DEPTH: usize = 8;

pub(crate) fn scan(
    root: &Path,
    max_entries: usize,
    max_results: usize,
    cancel: &AtomicBool,
) -> Result<LibraryScanResult, EngineError> {
    if root
        .to_str()
        .is_none_or(|root| root.is_empty() || root.len() > 4096)
    {
        return Err(EngineError::LibraryPathInvalid);
    }
    let check_cancel = || {
        if cancel.load(Ordering::Relaxed) {
            Err(EngineError::ScanCancelled)
        } else {
            Ok(())
        }
    };
    check_cancel()?;
    let metadata = fs::metadata(root).map_err(|_| EngineError::DirectoryUnavailable)?;
    if !metadata.is_dir() {
        return Err(EngineError::NotADirectory);
    }
    // A stack of directory iterators bounds open handles to depth + 1, even for wide trees.
    let open = |path: &Path| fs::read_dir(path).map_err(|_| EngineError::DirectoryUnavailable);
    let mut stack = vec![open(root)?];
    let mut visited = HashSet::new();
    visited.insert(fs::canonicalize(root).map_err(|_| EngineError::DirectoryUnavailable)?);
    let mut seen = 0usize;
    let mut entries = Vec::new();
    while !stack.is_empty() {
        check_cancel()?;
        let next = stack.last_mut().and_then(Iterator::next);
        let Some(next) = next else {
            stack.pop();
            continue;
        };
        seen = seen
            .checked_add(1)
            .ok_or(EngineError::DirectoryEntryLimit)?;
        if seen > max_entries {
            return Err(EngineError::DirectoryEntryLimit);
        }
        let item = next.map_err(|_| EngineError::DirectoryUnavailable)?;
        let path = item.path();
        // Non-UTF-8 names cannot round-trip through the JSON index. Refuse rather than conflate
        // two different paths through lossy decoding or silently hide a file.
        if path.to_str().is_none_or(|path| path.len() > 4096) {
            return Err(EngineError::LibraryPathInvalid);
        }
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| EngineError::DirectoryUnavailable)?;
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            if stack.len() > MAX_SCAN_DEPTH {
                return Err(EngineError::ScanDepthLimit);
            }
            let canonical =
                fs::canonicalize(&path).map_err(|_| EngineError::DirectoryUnavailable)?;
            if visited.insert(canonical) {
                stack.push(open(&path)?);
            }
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
            continue;
        };
        let extension = extension.to_ascii_lowercase();
        if !VIDEO_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        if entries.len() >= max_results {
            return Err(EngineError::ResultLimit);
        }
        entries.push(MediaEntry {
            path: path.to_string_lossy().into_owned(),
            title: item.file_name().to_string_lossy().into_owned(),
            size_bytes: metadata.len(),
            modified_unix_seconds: metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                .map(|value| value.as_secs()),
            extension,
            offline: false,
            favorite: false,
            tags: Vec::new(),
            game: None,
            recorded_game: None,
            collections: Vec::new(),
            probe: None,
        });
    }
    check_cancel()?;
    entries.sort_by_cached_key(|entry| (entry.title.to_lowercase(), entry.path.clone()));
    Ok(LibraryScanResult {
        directory: root
            .to_str()
            .filter(|root| root.len() <= 4096)
            .ok_or(EngineError::LibraryPathInvalid)?
            .to_owned(),
        entries,
        persisted: false,
        collections: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;

    struct Tree(PathBuf);
    impl Tree {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "clipcraft-tree-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn write(&self, name: &str) {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"fixture").unwrap();
        }
        fn scan(&self) -> Result<LibraryScanResult, EngineError> {
            scan(&self.0, 20_000, 5_000, &AtomicBool::new(false))
        }
    }
    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn nested_clips_are_stably_sorted_and_grouped_by_the_first_child_folder() {
        let tree = Tree::new();
        tree.write("Hades/session/b.MP4");
        tree.write("Celeste/a.mkv");
        tree.write("Hades/a.mkv");
        tree.write("notes.txt");
        let result = tree.scan().unwrap();
        assert_eq!(result.entries.len(), 3);
        assert_eq!(
            result
                .entries
                .iter()
                .map(|entry| entry.title.as_str())
                .collect::<Vec<_>>(),
            ["a.mkv", "a.mkv", "b.MP4"]
        );
        assert!(result.entries[0].path.contains("Celeste"));
        assert_eq!(
            crate::games(&result.entries, &result.directory),
            [
                crate::GameGroup {
                    name: "Hades".into(),
                    clips: 2
                },
                crate::GameGroup {
                    name: "Celeste".into(),
                    clips: 1
                }
            ]
        );
        assert_eq!(result, tree.scan().unwrap());
    }

    #[test]
    fn entry_and_result_limits_apply_to_the_whole_tree() {
        let tree = Tree::new();
        tree.write("one/a.mp4");
        tree.write("two/b.mp4");
        let cancel = AtomicBool::new(false);
        assert_eq!(
            scan(&tree.0, 3, 5_000, &cancel),
            Err(EngineError::DirectoryEntryLimit)
        );
        assert_eq!(
            scan(&tree.0, 20_000, 1, &cancel),
            Err(EngineError::ResultLimit)
        );
    }

    #[test]
    fn accepts_eight_levels_and_refuses_a_ninth_without_publishing_partial_results() {
        let tree = Tree::new();
        let parent = (0..MAX_SCAN_DEPTH)
            .map(|_| "child")
            .collect::<Vec<_>>()
            .join("/");
        tree.write(&format!("{parent}/clip.mp4"));
        assert_eq!(tree.scan().unwrap().entries.len(), 1);
        tree.write(&format!("{parent}/extra/too-deep.mp4"));
        assert_eq!(tree.scan(), Err(EngineError::ScanDepthLimit));
    }

    #[test]
    fn cancellation_returns_no_partial_scan_and_never_changes_an_index() {
        let tree = Tree::new();
        tree.write("child/clip.mp4");
        let index = tree.0.join("index.json");
        crate::execute(crate::Command::RefreshLibrary {
            directory: tree.0.clone(),
            index_path: index.clone(),
        })
        .unwrap();
        let before = fs::read(&index).unwrap();
        assert_eq!(
            crate::scan_library_directory_cancellable(&tree.0, &AtomicBool::new(true)),
            Err(EngineError::ScanCancelled)
        );
        assert_eq!(fs::read(index).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn directory_links_and_cycles_are_not_followed() {
        use std::os::unix::fs::symlink;
        let tree = Tree::new();
        let outside = Tree::new();
        tree.write("game/clip.mp4");
        outside.write("private.mp4");
        symlink(&tree.0, tree.0.join("game/loop")).unwrap();
        symlink(&outside.0, tree.0.join("outside")).unwrap();
        symlink(tree.0.join("game"), tree.0.join("alias")).unwrap();
        assert_eq!(tree.scan().unwrap().entries.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_child_returns_an_error_instead_of_an_incomplete_library() {
        use std::os::unix::fs::PermissionsExt;
        let tree = Tree::new();
        tree.write("private/clip.mp4");
        let child = tree.0.join("private");
        fs::set_permissions(&child, fs::Permissions::from_mode(0o000)).unwrap();
        // Privileged environments bypass filesystem permissions. The same test is also run as
        // an unprivileged user for evidence; do not infer permission handling from root.
        let permission_enforced = fs::read_dir(&child).is_err();
        let result = tree.scan();
        fs::set_permissions(&child, fs::Permissions::from_mode(0o700)).unwrap();
        if permission_enforced {
            assert_eq!(result, Err(EngineError::DirectoryUnavailable));
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_are_refused_instead_of_becoming_duplicate_lossy_names() {
        use std::os::unix::ffi::OsStringExt;
        let tree = Tree::new();
        fs::write(
            tree.0
                .join(std::ffi::OsString::from_vec(b"invalid-\xff.mp4".to_vec())),
            b"x",
        )
        .unwrap();
        assert_eq!(tree.scan(), Err(EngineError::LibraryPathInvalid));
    }
}
