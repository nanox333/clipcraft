//! Renaming a clip in its folder, keeping what the library knows about it (command
//! `library.rename`). The file never replaces another, and a clip still open elsewhere is left
//! as it was.

use std::fs;
use std::path::Path;

use crate::{EngineError, LibraryScanResult, MetaEdit, VIDEO_EXTENSIONS};

/// Longest new name, in characters, without the extension.
pub const MAX_CLIP_NAME_CHARS: usize = 120;

/// What a rename did.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct RenamedClip {
    /// The clip's new path.
    pub path: String,
    /// The remembered folder after the rename, when an index was given and could be updated.
    pub library: Option<LibraryScanResult>,
}

/// The name as it will be used (trimmed), or why it cannot be a file name on any system
/// ClipCraft runs on: empty, too long, a path separator or a character Windows refuses, a name
/// Windows reserves, or one ending in a dot or space.
pub fn clean_clip_name(name: &str) -> Result<String, EngineError> {
    let name = name.trim();
    let reserved = |stem: &str| {
        let upper = stem.to_ascii_uppercase();
        matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((upper.starts_with("COM") || upper.starts_with("LPT"))
                && upper.len() == 4
                && upper.as_bytes()[3].is_ascii_digit()
                && upper.as_bytes()[3] != b'0')
    };
    let usable = !name.is_empty()
        && name.chars().count() <= MAX_CLIP_NAME_CHARS
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        && !name.ends_with('.')
        && name != "."
        && name != ".."
        && !reserved(name.split('.').next().unwrap_or(name));
    if usable {
        Ok(name.to_owned())
    } else {
        Err(EngineError::ClipNameInvalid)
    }
}

/// Renames the clip at `path` to `name` plus its own extension, in the same folder. With an
/// index, the clip's favorite, collections and game label move with it.
pub(crate) fn rename_clip(
    index: Option<(&Path, &str)>,
    path: &Path,
    name: &str,
) -> Result<RenamedClip, EngineError> {
    let name = clean_clip_name(name)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| EngineError::FileUnavailable)?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| VIDEO_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .ok_or(EngineError::NotAClip)?
        .to_owned();
    if !metadata.is_file() {
        return Err(EngineError::NotAClip);
    }
    let folder = path.parent().ok_or(EngineError::FileUnavailable)?;
    let target = folder.join(format!("{name}.{extension}"));
    if target == path {
        return Ok(RenamedClip {
            path: path.to_string_lossy().into_owned(),
            library: None,
        });
    }
    move_without_replacing(path, &target)?;
    let library = index.and_then(|(index_path, directory)| {
        crate::index::edit_meta(
            index_path,
            directory,
            &MetaEdit::RelinkFile {
                from: path.to_string_lossy().into_owned(),
                to: target.to_string_lossy().into_owned(),
            },
        )
        .ok()
    });
    Ok(RenamedClip {
        path: target.to_string_lossy().into_owned(),
        library,
    })
}

/// Moves `from` to `to` in the same folder, never replacing a file at `to`. A hard link makes the
/// new name atomically; if the old name then cannot go (the clip is open in another program), the
/// new name is taken back and nothing changes.
fn move_without_replacing(from: &Path, to: &Path) -> Result<(), EngineError> {
    // Only the letter case changes: on a case-insensitive disk `to` is the same file.
    let same_file = to
        .to_string_lossy()
        .eq_ignore_ascii_case(&from.to_string_lossy())
        && fs::canonicalize(to).ok() == fs::canonicalize(from).ok();
    if same_file {
        return fs::rename(from, to).map_err(|_| EngineError::ClipInUse);
    }
    match fs::hard_link(from, to) {
        Ok(()) => {
            if fs::remove_file(from).is_err() {
                let _ = fs::remove_file(to);
                return Err(EngineError::ClipInUse);
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(EngineError::ClipNameTaken)
        }
        // No hard links on this disk (exFAT, FAT, some network shares): a checked rename.
        Err(_) => {
            if fs::symlink_metadata(to).is_ok() {
                return Err(EngineError::ClipNameTaken);
            }
            fs::rename(from, to).map_err(|_| EngineError::ClipInUse)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "clipcraft-rename-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join("clips")).expect("scratch directory");
            Self(path)
        }

        fn clips(&self) -> PathBuf {
            self.0.join("clips")
        }

        fn index(&self) -> PathBuf {
            self.0.join("library-index.json")
        }

        fn clip(&self, name: &str) -> PathBuf {
            let path = self.clips().join(name);
            fs::write(&path, name.as_bytes()).expect("fixture writes");
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn names_that_cannot_be_files_are_refused() {
        for bad in [
            "",
            "   ",
            "a/b",
            "a\\b",
            "what?",
            "x:y",
            "quote\"",
            "pipe|",
            "dot.",
            ".",
            "..",
            "CON",
            "nul",
            "com1",
            "LPT9.clip",
            "tab\there",
        ] {
            assert_eq!(
                clean_clip_name(bad),
                Err(EngineError::ClipNameInvalid),
                "{bad:?}"
            );
        }
        assert!(clean_clip_name(&"x".repeat(MAX_CLIP_NAME_CHARS + 1)).is_err());
        assert_eq!(
            clean_clip_name("  Ace clutch  "),
            Ok("Ace clutch".to_owned())
        );
        assert_eq!(clean_clip_name("COM0"), Ok("COM0".to_owned()));
        assert_eq!(clean_clip_name("Console"), Ok("Console".to_owned()));
        assert!(clean_clip_name(&"é".repeat(MAX_CLIP_NAME_CHARS)).is_ok());
    }

    #[test]
    fn a_renamed_clip_keeps_its_extension_favorite_and_collections() {
        let scratch = Scratch::new();
        let path = scratch.clip("ClipCraft 2026-11-10.mp4");
        let directory = scratch.clips().to_string_lossy().into_owned();
        let scanned =
            crate::scan::scan(&scratch.clips(), 100, 100, &AtomicBool::new(false)).expect("scan");
        crate::index::refresh_library(scanned, &scratch.index()).expect("index");
        let key = path.to_string_lossy().into_owned();
        crate::index::edit_meta(
            &scratch.index(),
            &directory,
            &MetaEdit::SetFavorite {
                path: key.clone(),
                favorite: true,
            },
        )
        .expect("favorite");

        let renamed = rename_clip(Some((&scratch.index(), &directory)), &path, "Ace clutch")
            .expect("renamed");

        let target = scratch.clips().join("Ace clutch.mp4");
        assert_eq!(renamed.path, target.to_string_lossy());
        assert!(target.is_file());
        assert!(!path.exists());
        let library = renamed.library.expect("index updated");
        let entry = library
            .entries
            .iter()
            .find(|entry| entry.path == renamed.path)
            .expect("listed under its new name");
        assert!(entry.favorite);
        assert_eq!(entry.title, "Ace clutch.mp4");
        assert!(!library.entries.iter().any(|entry| entry.path == key));
    }

    #[test]
    fn a_game_read_from_the_old_file_name_is_kept() {
        let scratch = Scratch::new();
        let path = scratch.clip("Hades 2026-10-19 21-04-55.mp4");
        let directory = scratch.clips().to_string_lossy().into_owned();
        let scanned =
            crate::scan::scan(&scratch.clips(), 100, 100, &AtomicBool::new(false)).expect("scan");
        crate::index::refresh_library(scanned, &scratch.index()).expect("index");
        let renamed =
            rename_clip(Some((&scratch.index(), &directory)), &path, "Boss rush").expect("renamed");
        let library = renamed.library.expect("index updated");
        let entry = library
            .entries
            .iter()
            .find(|entry| entry.path == renamed.path)
            .expect("listed");
        assert_eq!(crate::game_of(entry, &directory).as_deref(), Some("Hades"));
    }

    #[test]
    fn a_rename_never_replaces_another_file() {
        let scratch = Scratch::new();
        let path = scratch.clip("one.mp4");
        let other = scratch.clip("two.mp4");
        assert_eq!(
            rename_clip(None, &path, "two"),
            Err(EngineError::ClipNameTaken)
        );
        assert_eq!(fs::read(&other).expect("other kept"), b"two.mp4");
        assert_eq!(fs::read(&path).expect("clip kept"), b"one.mp4");
    }

    #[test]
    fn an_unchanged_name_is_a_no_op_and_a_case_change_is_kept() {
        let scratch = Scratch::new();
        let path = scratch.clip("clip.mp4");
        let same = rename_clip(None, &path, " clip ").expect("unchanged");
        assert_eq!(same.path, path.to_string_lossy());
        assert!(path.is_file());

        let renamed = rename_clip(None, &path, "Clip").expect("case change");
        let names: Vec<_> = fs::read_dir(scratch.clips())
            .expect("folder reads")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Clip.mp4"]);
        assert!(renamed.path.ends_with("Clip.mp4"));
    }

    #[test]
    fn only_existing_video_files_are_renamed() {
        let scratch = Scratch::new();
        let notes = scratch.clip("notes.txt");
        assert_eq!(rename_clip(None, &notes, "x"), Err(EngineError::NotAClip));
        assert_eq!(
            rename_clip(None, &scratch.clips().join("gone.mp4"), "x"),
            Err(EngineError::FileUnavailable)
        );
        assert!(notes.is_file());
    }

    #[test]
    fn the_command_is_registered() {
        let scratch = Scratch::new();
        let path = scratch.clip("a.mkv");
        let output = crate::execute(crate::Command::RenameClip {
            path,
            name: "b".to_owned(),
            index_path: None,
            directory: None,
        })
        .expect("command runs");
        assert!(
            matches!(output, crate::CommandOutput::Renamed(renamed) if renamed.path.ends_with("b.mkv"))
        );
        assert!(crate::command_ids().contains(&"library.rename"));
    }
}
