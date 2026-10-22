//! Favorites, collections, and games: the person's own organisation of the library.

use crate::{EngineError, MediaEntry};

/// Longest collection or game name, in characters.
pub const MAX_NAME_CHARS: usize = 40;
/// Most collections a library may have.
pub const MAX_COLLECTIONS: usize = 64;
/// Most collections one clip may be in.
pub const MAX_COLLECTIONS_PER_CLIP: usize = 16;

/// One change to the person's organisation of the library.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetaEdit {
    /// Marks or unmarks a clip as a favorite.
    SetFavorite { path: String, favorite: bool },
    /// Sets the game a clip belongs to, or `None` to restore its recorded/guessed game.
    SetGame { path: String, game: Option<String> },
    /// Makes an empty collection.
    CreateCollection { name: String },
    /// Renames a collection, keeping its clips.
    RenameCollection { from: String, to: String },
    /// Deletes a collection. The clips stay in the library.
    DeleteCollection { name: String },
    /// Puts a clip in a collection.
    AddToCollection { path: String, name: String },
    /// Takes a clip out of a collection.
    RemoveFromCollection { path: String, name: String },
    /// Forgets a clip whose file is gone. Refused while the file is still on disk, so nothing
    /// that exists can vanish from view; the file itself is never touched.
    RemoveFromLibrary { path: String },
    /// Points a clip whose file moved at its new place, keeping its favorite, game, and
    /// collections.
    RelinkFile { from: String, to: String },
}

/// A name with spaces trimmed, or `None` when it is empty, too long, or has control characters.
pub fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    let usable = !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control);
    usable.then(|| name.to_owned())
}

/// Recorded labels follow the existing game-rule byte bound, independently of manual labels.
pub(crate) fn clean_recorded_game(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name.len() <= 100 && !name.chars().any(char::is_control))
        .then(|| name.to_owned())
}

fn same(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

fn entry_mut<'a>(
    entries: &'a mut [MediaEntry],
    path: &str,
) -> Result<&'a mut MediaEntry, EngineError> {
    entries
        .iter_mut()
        .find(|entry| entry.path == path)
        .ok_or(EngineError::FileUnavailable)
}

/// Applies `edit` to the clips and the collection list. Nothing changes when it is refused.
pub fn apply_edit(
    entries: &mut Vec<MediaEntry>,
    collections: &mut Vec<String>,
    edit: &MetaEdit,
) -> Result<(), EngineError> {
    match edit {
        MetaEdit::SetFavorite { path, favorite } => {
            entry_mut(entries, path)?.favorite = *favorite;
        }
        MetaEdit::SetGame { path, game } => {
            let game = match game {
                Some(name) => Some(clean_name(name).ok_or(EngineError::NameInvalid)?),
                None => None,
            };
            entry_mut(entries, path)?.game = game;
        }
        MetaEdit::CreateCollection { name } => {
            let name = clean_name(name).ok_or(EngineError::NameInvalid)?;
            if collections.iter().any(|existing| same(existing, &name)) {
                return Err(EngineError::NameTaken);
            }
            if collections.len() >= MAX_COLLECTIONS {
                return Err(EngineError::CollectionLimit);
            }
            collections.push(name);
        }
        MetaEdit::RenameCollection { from, to } => {
            let to = clean_name(to).ok_or(EngineError::NameInvalid)?;
            let position = collections
                .iter()
                .position(|existing| existing == from)
                .ok_or(EngineError::NameInvalid)?;
            if !same(from, &to) && collections.iter().any(|existing| same(existing, &to)) {
                return Err(EngineError::NameTaken);
            }
            collections[position] = to.clone();
            for entry in entries.iter_mut() {
                for name in &mut entry.collections {
                    if name == from {
                        *name = to.clone();
                    }
                }
                entry.collections.dedup();
            }
        }
        MetaEdit::DeleteCollection { name } => {
            collections.retain(|existing| existing != name);
            for entry in entries.iter_mut() {
                entry.collections.retain(|existing| existing != name);
            }
        }
        MetaEdit::AddToCollection { path, name } => {
            if !collections.iter().any(|existing| existing == name) {
                return Err(EngineError::NameInvalid);
            }
            let entry = entry_mut(entries, path)?;
            if !entry.collections.contains(name) {
                if entry.collections.len() >= MAX_COLLECTIONS_PER_CLIP {
                    return Err(EngineError::CollectionLimit);
                }
                entry.collections.push(name.clone());
            }
        }
        MetaEdit::RemoveFromLibrary { path } => {
            let position = entries
                .iter()
                .position(|entry| entry.path == *path)
                .ok_or(EngineError::FileUnavailable)?;
            if std::path::Path::new(path).exists() {
                return Err(EngineError::FileStillExists);
            }
            entries.remove(position);
        }
        MetaEdit::RelinkFile { from, to } => {
            let metadata = std::fs::metadata(to)
                .ok()
                .filter(|metadata| metadata.is_file())
                .ok_or(EngineError::FileUnavailable)?;
            if std::path::Path::new(from).exists() {
                return Err(EngineError::FileStillExists);
            }
            if entries.iter().any(|entry| entry.path == *to) {
                return Err(EngineError::NameTaken);
            }
            let target = std::path::Path::new(to);
            let entry = entry_mut(entries, from)?;
            entry.path = to.clone();
            if let Some(name) = target.file_name() {
                entry.title = name.to_string_lossy().into_owned();
            }
            if let Some(extension) = target.extension() {
                entry.extension = extension.to_string_lossy().to_lowercase();
            }
            entry.size_bytes = metadata.len();
            entry.modified_unix_seconds = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|since| since.as_secs());
            entry.offline = false;
            // The file may differ in details; probe it again.
            entry.probe = None;
        }
        MetaEdit::RemoveFromCollection { path, name } => {
            entry_mut(entries, path)?
                .collections
                .retain(|existing| existing != name);
        }
    }
    Ok(())
}

/// Guesses a game from a clip's file name, as recorders name them:
/// `Valorant 2024.03.01 - 21.04.mp4`, `Apex_Legends_2024-03-01.mp4`, `Hades - clutch.mp4`.
pub fn guess_game_from_file_name(file_name: &str) -> Option<String> {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    let spaced = if stem.contains(' ') {
        stem.to_owned()
    } else {
        stem.replace('_', " ")
    };
    let mut kept = Vec::new();
    let mut found_marker = false;
    for word in spaced.split_whitespace() {
        if word == "-" || word == "–" || starts_like_date(word) {
            found_marker = true;
            break;
        }
        kept.push(word);
    }
    // A plain name such as `highlights.mp4` has nothing saying it is a game.
    if !found_marker {
        return None;
    }
    let name = kept.join(" ");
    // A bare number, or a name that is only punctuation, is not a game.
    let has_letter = name.chars().any(char::is_alphabetic);
    if !has_letter {
        return None;
    }
    clean_name(&name)
}

fn starts_like_date(word: &str) -> bool {
    let digits = word.chars().take_while(char::is_ascii_digit).count();
    digits == 4
        && word
            .chars()
            .nth(4)
            .is_some_and(|separator| matches!(separator, '.' | '-' | '_' | '/'))
}

/// The game a clip belongs to: manual choice, recorded game, first child folder, then filename.
pub fn game_of(entry: &MediaEntry, library_directory: &str) -> Option<String> {
    if let Some(game) = &entry.game {
        return Some(game.clone());
    }
    if let Some(game) = &entry.recorded_game {
        return Some(game.clone());
    }
    let path = std::path::Path::new(&entry.path);
    if let Ok(relative) = path.strip_prefix(library_directory)
        && let Some(parent) = relative.parent()
        && let Some(first) = parent.components().next()
        && let Some(folder) = first.as_os_str().to_str()
        && let Some(name) = clean_name(folder)
    {
        return Some(name);
    }
    guess_game_from_file_name(&entry.title)
}

/// A game and how many clips it has.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameGroup {
    /// The game's name.
    pub name: String,
    /// Clips in the library.
    pub clips: usize,
}

/// Every game in the library, busiest first, then by name. Clips with no game are left out.
pub fn games(entries: &[MediaEntry], library_directory: &str) -> Vec<GameGroup> {
    let mut groups: Vec<GameGroup> = Vec::new();
    for entry in entries {
        let Some(name) = game_of(entry, library_directory) else {
            continue;
        };
        match groups.iter_mut().find(|group| same(&group.name, &name)) {
            Some(group) => group.clips += 1,
            None => groups.push(GameGroup { name, clips: 1 }),
        }
    }
    groups.sort_by(|a, b| {
        b.clips
            .cmp(&a.clips)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> MediaEntry {
        MediaEntry {
            path: path.to_owned(),
            title: path.rsplit('/').next().unwrap_or(path).to_owned(),
            size_bytes: 1,
            modified_unix_seconds: None,
            extension: "mp4".to_owned(),
            offline: false,
            favorite: false,
            tags: Vec::new(),
            game: None,
            recorded_game: None,
            collections: Vec::new(),
            probe: None,
        }
    }

    #[test]
    fn games_are_guessed_from_recorder_file_names() {
        for (file, game) in [
            ("Valorant 2024.03.01 - 21.04.55.mp4", Some("Valorant")),
            ("Apex_Legends_2024-03-01_21-04.mp4", Some("Apex Legends")),
            ("Hades - clutch.mp4", Some("Hades")),
            ("Counter-Strike 2 2024.01.02.mp4", Some("Counter-Strike 2")),
            ("2024-03-01 21-04-55.mp4", None),
            ("12345.mp4", None),
            ("highlights.mp4", None),
            ("random clip.mp4", None),
            ("- .mp4", None),
            ("", None),
        ] {
            assert_eq!(guess_game_from_file_name(file).as_deref(), game, "{file}");
        }
    }

    #[test]
    fn a_sub_folder_beats_the_file_name_and_a_choice_beats_both() {
        let mut clip = entry("/clips/Hades/20240301.mp4");
        assert_eq!(game_of(&clip, "/clips").as_deref(), Some("Hades"));
        clip.game = Some("Hades II".to_owned());
        assert_eq!(game_of(&clip, "/clips").as_deref(), Some("Hades II"));
        let loose = entry("/clips/Celeste 2024.01.01.mp4");
        assert_eq!(game_of(&loose, "/clips").as_deref(), Some("Celeste"));
    }

    #[test]
    fn games_are_counted_ignoring_case_busiest_first() {
        let entries = [
            entry("/c/hades 2024.01.01.mp4"),
            entry("/c/Hades 2024.01.02.mp4"),
            entry("/c/Celeste 2024.01.01.mp4"),
            entry("/c/20240101.mp4"),
        ];
        let groups = games(&entries, "/c");
        assert_eq!(groups.len(), 2);
        assert_eq!(
            (groups[0].name.to_lowercase().as_str(), groups[0].clips),
            ("hades", 2)
        );
        assert_eq!((groups[1].name.as_str(), groups[1].clips), ("Celeste", 1));
    }

    #[test]
    fn favorites_and_games_change_only_the_named_clip() {
        let mut entries = vec![entry("/c/a.mp4"), entry("/c/b.mp4")];
        let mut collections = Vec::new();
        apply_edit(
            &mut entries,
            &mut collections,
            &MetaEdit::SetFavorite {
                path: "/c/a.mp4".into(),
                favorite: true,
            },
        )
        .unwrap();
        assert!(entries[0].favorite && !entries[1].favorite);
        apply_edit(
            &mut entries,
            &mut collections,
            &MetaEdit::SetGame {
                path: "/c/b.mp4".into(),
                game: Some("  Hades ".into()),
            },
        )
        .unwrap();
        assert_eq!(entries[1].game.as_deref(), Some("Hades"));
        assert_eq!(
            apply_edit(
                &mut entries,
                &mut collections,
                &MetaEdit::SetFavorite {
                    path: "/missing.mp4".into(),
                    favorite: true
                },
            ),
            Err(EngineError::FileUnavailable)
        );
        assert_eq!(
            apply_edit(
                &mut entries,
                &mut collections,
                &MetaEdit::SetGame {
                    path: "/c/a.mp4".into(),
                    game: Some("   ".into())
                },
            ),
            Err(EngineError::NameInvalid)
        );
    }

    #[test]
    fn collections_are_created_renamed_filled_and_deleted() {
        let mut entries = vec![entry("/c/a.mp4"), entry("/c/b.mp4")];
        let mut collections = Vec::new();
        let run = |edit: MetaEdit, entries: &mut Vec<MediaEntry>, collections: &mut Vec<String>| {
            apply_edit(entries, collections, &edit)
        };
        run(
            MetaEdit::CreateCollection {
                name: "Clutches".into(),
            },
            &mut entries,
            &mut collections,
        )
        .unwrap();
        assert_eq!(
            run(
                MetaEdit::CreateCollection {
                    name: "clutches".into()
                },
                &mut entries,
                &mut collections
            ),
            Err(EngineError::NameTaken)
        );
        assert_eq!(
            run(
                MetaEdit::AddToCollection {
                    path: "/c/a.mp4".into(),
                    name: "Nope".into()
                },
                &mut entries,
                &mut collections
            ),
            Err(EngineError::NameInvalid)
        );
        run(
            MetaEdit::AddToCollection {
                path: "/c/a.mp4".into(),
                name: "Clutches".into(),
            },
            &mut entries,
            &mut collections,
        )
        .unwrap();
        run(
            MetaEdit::AddToCollection {
                path: "/c/a.mp4".into(),
                name: "Clutches".into(),
            },
            &mut entries,
            &mut collections,
        )
        .unwrap();
        assert_eq!(entries[0].collections, ["Clutches"]);
        run(
            MetaEdit::RenameCollection {
                from: "Clutches".into(),
                to: "Best plays".into(),
            },
            &mut entries,
            &mut collections,
        )
        .unwrap();
        assert_eq!(collections, ["Best plays"]);
        assert_eq!(entries[0].collections, ["Best plays"]);
        run(
            MetaEdit::RemoveFromCollection {
                path: "/c/a.mp4".into(),
                name: "Best plays".into(),
            },
            &mut entries,
            &mut collections,
        )
        .unwrap();
        assert!(entries[0].collections.is_empty());
        run(
            MetaEdit::AddToCollection {
                path: "/c/b.mp4".into(),
                name: "Best plays".into(),
            },
            &mut entries,
            &mut collections,
        )
        .unwrap();
        run(
            MetaEdit::DeleteCollection {
                name: "Best plays".into(),
            },
            &mut entries,
            &mut collections,
        )
        .unwrap();
        assert!(collections.is_empty() && entries[1].collections.is_empty());
    }

    #[test]
    fn collection_counts_are_bounded() {
        let mut entries = vec![entry("/c/a.mp4")];
        let mut collections = Vec::new();
        for index in 0..MAX_COLLECTIONS {
            apply_edit(
                &mut entries,
                &mut collections,
                &MetaEdit::CreateCollection {
                    name: format!("c{index}"),
                },
            )
            .unwrap();
        }
        assert_eq!(
            apply_edit(
                &mut entries,
                &mut collections,
                &MetaEdit::CreateCollection {
                    name: "one more".into()
                }
            ),
            Err(EngineError::CollectionLimit)
        );
        for index in 0..MAX_COLLECTIONS_PER_CLIP {
            apply_edit(
                &mut entries,
                &mut collections,
                &MetaEdit::AddToCollection {
                    path: "/c/a.mp4".into(),
                    name: format!("c{index}"),
                },
            )
            .unwrap();
        }
        assert_eq!(
            apply_edit(
                &mut entries,
                &mut collections,
                &MetaEdit::AddToCollection {
                    path: "/c/a.mp4".into(),
                    name: "c20".into()
                }
            ),
            Err(EngineError::CollectionLimit)
        );
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  Best of  ").as_deref(), Some("Best of"));
        assert_eq!(clean_name(""), None);
        assert_eq!(clean_name("a\nb"), None);
        assert_eq!(clean_name(&"x".repeat(MAX_NAME_CHARS + 1)), None);
    }

    #[test]
    fn a_clip_whose_file_is_gone_can_be_forgotten_but_an_existing_one_cannot() {
        let present =
            std::env::temp_dir().join(format!("clipcraft-forget-{}.mp4", std::process::id()));
        std::fs::write(&present, b"x").expect("fixture writes");
        let gone = "/definitely/not/here/gone.mp4".to_owned();
        let mut entries = vec![entry(&gone), entry(&present.to_string_lossy())];
        let mut collections = Vec::new();

        let forget = |path: &str| MetaEdit::RemoveFromLibrary {
            path: path.to_owned(),
        };
        assert_eq!(
            apply_edit(
                &mut entries,
                &mut collections,
                &forget(&present.to_string_lossy())
            ),
            Err(EngineError::FileStillExists)
        );
        assert_eq!(entries.len(), 2, "an existing file stays listed");
        assert_eq!(
            apply_edit(
                &mut entries,
                &mut collections,
                &forget("/not/in/the/library.mp4")
            ),
            Err(EngineError::FileUnavailable)
        );
        apply_edit(&mut entries, &mut collections, &forget(&gone)).expect("forgotten");
        assert_eq!(entries.len(), 1);
        assert!(present.exists(), "the file itself is never touched");
        let _ = std::fs::remove_file(&present);
    }

    #[test]
    fn a_moved_file_can_be_relinked_and_keeps_its_favorite_and_collections() {
        let dir =
            std::env::temp_dir().join(format!("clipcraft-relink-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let new_home = dir.join("Clutch.mp4");
        std::fs::write(&new_home, b"12345").expect("file");
        let new_path = new_home.to_string_lossy().into_owned();
        let mut old = entry("/definitely/not/here/Clutch.mp4");
        old.favorite = true;
        old.collections = vec!["Best".to_owned()];
        old.offline = true;
        let mut entries = vec![old];
        let mut collections = vec!["Best".to_owned()];
        let relink = |from: &str, to: &str| MetaEdit::RelinkFile {
            from: from.to_owned(),
            to: to.to_owned(),
        };

        // The target must exist, and the old file must really be gone.
        assert_eq!(
            apply_edit(
                &mut entries,
                &mut collections,
                &relink("/definitely/not/here/Clutch.mp4", "/nope/x.mp4")
            ),
            Err(EngineError::FileUnavailable)
        );
        assert_eq!(
            apply_edit(
                &mut entries,
                &mut collections,
                &relink(&new_path, &new_path)
            ),
            Err(EngineError::FileStillExists)
        );
        apply_edit(
            &mut entries,
            &mut collections,
            &relink("/definitely/not/here/Clutch.mp4", &new_path),
        )
        .expect("relinked");
        assert_eq!(entries[0].path, new_path);
        assert_eq!(entries[0].size_bytes, 5);
        assert!(!entries[0].offline && entries[0].favorite);
        assert_eq!(entries[0].collections, ["Best"]);
        assert!(entries[0].probe.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
