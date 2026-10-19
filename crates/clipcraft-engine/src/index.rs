//! Versioned, rebuildable local library index.
//!
//! The index remembers scanned folders plus small per-clip metadata (favorite, tags) between
//! launches. It is disposable: ordinary video files stay the source of truth, and deleting or
//! corrupting the index only costs a rescan.

use crate::{
    EngineError, LibraryScanResult, MAX_LIBRARY_RESULTS, MAX_NAME_CHARS, MediaEntry, MetaEdit,
    apply_edit,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

/// Index format version written by this build.
pub const INDEX_VERSION: u32 = 1;
/// Largest index file that will be read or written.
pub const MAX_INDEX_BYTES: u64 = 32 * 1024 * 1024;
/// Most folders remembered; the least recently refreshed are dropped.
pub const MAX_INDEXED_FOLDERS: usize = 32;
/// Most entries kept per folder, including offline ones.
pub const MAX_ENTRIES_PER_FOLDER: usize = MAX_LIBRARY_RESULTS * 2;

const MAX_TAGS_PER_CLIP: usize = 16;
const MAX_TAG_BYTES: usize = 64;
const MAX_PATH_BYTES: usize = 4096;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct LibraryIndex {
    version: u32,
    folders: Vec<FolderIndex>,
    #[serde(default)]
    collections: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct FolderIndex {
    directory: String,
    entries: Vec<MediaEntry>,
}

/// Default per-user index location, or `None` when no home directory is known.
pub fn default_index_path() -> Option<PathBuf> {
    Some(default_data_dir()?.join("library-index.json"))
}

/// Default per-user thumbnail cache folder, or `None` when no home directory is known.
pub fn default_thumbnail_dir() -> Option<PathBuf> {
    Some(default_data_dir()?.join("thumbnails"))
}

fn default_data_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        PathBuf::from(data)
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join(".local/share")
    };
    Some(base.join("ClipCraft"))
}

/// Loads the most recently refreshed folder from the index.
pub(crate) fn load_library(index_path: &Path) -> Result<LibraryScanResult, EngineError> {
    let index = read_index(index_path)?;
    let folder = index
        .folders
        .into_iter()
        .next()
        .ok_or(EngineError::IndexMissing)?;
    Ok(LibraryScanResult {
        directory: folder.directory,
        entries: folder.entries,
        persisted: true,
        collections: index.collections,
    })
}

/// Merges a fresh scan into the index, then saves it atomically.
///
/// A missing or malformed index is rebuilt from the scan. An unreadable index or a failed save
/// does not discard the scan: the result is returned with `persisted == false`.
pub(crate) fn refresh_library(
    scan: LibraryScanResult,
    index_path: &Path,
) -> Result<LibraryScanResult, EngineError> {
    let mut index = match read_index(index_path) {
        Ok(index) => index,
        Err(EngineError::IndexMissing | EngineError::IndexInvalid) => LibraryIndex::default(),
        // The index exists but cannot be read right now. Show the scan, but do not overwrite a
        // file that may still be valid once the problem clears.
        Err(_) => {
            return Ok(LibraryScanResult {
                persisted: false,
                ..scan
            });
        }
    };
    let previous = index
        .folders
        .iter()
        .position(|folder| folder.directory == scan.directory)
        .map(|position| index.folders.remove(position));
    let entries = merge_entries(
        previous.as_ref().map(|folder| folder.entries.as_slice()),
        scan.entries,
    );

    index.version = INDEX_VERSION;
    index.folders.insert(
        0,
        FolderIndex {
            directory: scan.directory.clone(),
            entries: entries.clone(),
        },
    );
    index.folders.truncate(MAX_INDEXED_FOLDERS);

    let persisted = write_index(index_path, &index).is_ok();
    Ok(LibraryScanResult {
        directory: scan.directory,
        entries,
        persisted,
        collections: index.collections,
    })
}

/// Combines scanned files with remembered metadata.
///
/// Scanned files keep the favorite/tags of the same path. Remembered files that are no longer on
/// disk stay in the list as offline entries so a temporarily unplugged drive does not lose them.
pub(crate) fn merge_entries(
    previous: Option<&[MediaEntry]>,
    scanned: Vec<MediaEntry>,
) -> Vec<MediaEntry> {
    let Some(previous) = previous else {
        return scanned;
    };
    let mut merged: Vec<MediaEntry> = scanned
        .into_iter()
        .map(|mut entry| {
            if let Some(old) = previous.iter().find(|old| old.path == entry.path) {
                entry.favorite = old.favorite;
                entry.tags = old.tags.clone();
                entry.game = old.game.clone();
                entry.collections = old.collections.clone();
                // A probe stays valid only while the file looks unchanged.
                if old.size_bytes == entry.size_bytes
                    && old.modified_unix_seconds == entry.modified_unix_seconds
                {
                    entry.probe = old.probe.clone();
                }
            }
            entry
        })
        .collect();
    for old in previous {
        if merged.len() >= MAX_ENTRIES_PER_FOLDER {
            break;
        }
        if !merged.iter().any(|entry| entry.path == old.path) {
            let mut offline = old.clone();
            offline.offline = true;
            merged.push(offline);
        }
    }
    merged
}

/// Most probe results accepted in one call.
const MAX_STORED_PROBES: usize = MAX_ENTRIES_PER_FOLDER;

/// Saves probe results into the remembered folder `directory`. Results for paths that are not in
/// that folder are ignored, so a call can never grow the index beyond what a scan found.
pub(crate) fn store_probes(
    index_path: &Path,
    directory: &str,
    probes: Vec<(String, clipcraft_media::MediaProbe)>,
) -> Result<LibraryScanResult, EngineError> {
    if probes.len() > MAX_STORED_PROBES {
        return Err(EngineError::IndexInvalid);
    }
    let mut index = read_index(index_path)?;
    let folder = index
        .folders
        .iter_mut()
        .find(|folder| folder.directory == directory)
        .ok_or(EngineError::IndexMissing)?;
    for (path, probe) in probes {
        if let Some(entry) = folder.entries.iter_mut().find(|entry| entry.path == path) {
            entry.probe = Some(probe);
        }
    }
    let result = LibraryScanResult {
        directory: folder.directory.clone(),
        entries: folder.entries.clone(),
        persisted: false,
        collections: index.collections.clone(),
    };
    let persisted = write_index(index_path, &index).is_ok();
    Ok(LibraryScanResult {
        persisted,
        ..result
    })
}

/// Applies a favorite, game, or collection change to the saved index and returns the folder.
pub(crate) fn edit_meta(
    index_path: &Path,
    directory: &str,
    edit: &MetaEdit,
) -> Result<LibraryScanResult, EngineError> {
    let mut index = read_index(index_path)?;
    let position = index
        .folders
        .iter()
        .position(|folder| folder.directory == directory)
        .ok_or(EngineError::IndexMissing)?;
    // Work on copies so a refused edit leaves the index untouched.
    let mut collections = index.collections.clone();
    let mut entries = index.folders[position].entries.clone();
    apply_edit(&mut entries, &mut collections, edit)?;
    // Renaming or deleting a collection also reaches the clips of every other remembered folder.
    if matches!(
        edit,
        MetaEdit::RenameCollection { .. } | MetaEdit::DeleteCollection { .. }
    ) {
        for (other, folder) in index.folders.iter_mut().enumerate() {
            if other != position {
                let mut scratch = collections.clone();
                apply_edit(&mut folder.entries, &mut scratch, edit).ok();
            }
        }
    }
    index.folders[position].entries = entries;
    index.collections = collections;
    let persisted = write_index(index_path, &index).is_ok();
    let folder = &index.folders[position];
    Ok(LibraryScanResult {
        directory: folder.directory.clone(),
        entries: folder.entries.clone(),
        persisted,
        collections: index.collections.clone(),
    })
}

fn read_index(path: &Path) -> Result<LibraryIndex, EngineError> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Err(EngineError::IndexMissing),
        Err(_) => return Err(EngineError::IndexUnavailable),
    };
    // Read one byte past the cap so a file that grows after open is still rejected.
    let mut bytes = Vec::new();
    file.take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| EngineError::IndexUnavailable)?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err(EngineError::IndexInvalid);
    }
    let index: LibraryIndex =
        serde_json::from_slice(&bytes).map_err(|_| EngineError::IndexInvalid)?;
    validate(&index)?;
    Ok(index)
}

fn validate(index: &LibraryIndex) -> Result<(), EngineError> {
    if index.version != INDEX_VERSION {
        return Err(EngineError::IndexInvalid);
    }
    if index.folders.len() > MAX_INDEXED_FOLDERS || index.collections.len() > crate::MAX_COLLECTIONS
    {
        return Err(EngineError::IndexInvalid);
    }
    // Names are counted in characters by the editor, so allow four bytes each.
    let name_ok = |name: &String| name.len() <= MAX_NAME_CHARS * 4;
    if !index.collections.iter().all(name_ok) {
        return Err(EngineError::IndexInvalid);
    }
    for folder in &index.folders {
        if folder.directory.len() > MAX_PATH_BYTES || folder.entries.len() > MAX_ENTRIES_PER_FOLDER
        {
            return Err(EngineError::IndexInvalid);
        }
        for entry in &folder.entries {
            let tags_ok = entry.tags.len() <= MAX_TAGS_PER_CLIP
                && entry.tags.iter().all(|tag| tag.len() <= MAX_TAG_BYTES);
            let meta_ok = entry.collections.len() <= crate::MAX_COLLECTIONS_PER_CLIP
                && entry.collections.iter().all(name_ok)
                && entry.game.as_ref().is_none_or(name_ok);
            if entry.path.len() > MAX_PATH_BYTES
                || entry.title.len() > MAX_PATH_BYTES
                || !tags_ok
                || !meta_ok
            {
                return Err(EngineError::IndexInvalid);
            }
        }
    }
    Ok(())
}

/// Writes the index with a temp file and rename so an interrupted save keeps the old index.
fn write_index(path: &Path, index: &LibraryIndex) -> Result<(), EngineError> {
    let bytes = serde_json::to_vec(index).map_err(|_| EngineError::IndexSaveFailed)?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err(EngineError::IndexSaveFailed);
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|_| EngineError::IndexSaveFailed)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);

    let result = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(EngineError::IndexSaveFailed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "clipcraft-index-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("scratch directory");
            Self(path)
        }

        fn index(&self) -> PathBuf {
            self.0.join("data").join("library-index.json")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn clip(path: &str) -> MediaEntry {
        MediaEntry {
            path: path.to_owned(),
            title: path.rsplit('/').next().unwrap_or(path).to_owned(),
            size_bytes: 10,
            modified_unix_seconds: Some(1),
            extension: "mp4".to_owned(),
            offline: false,
            favorite: false,
            tags: Vec::new(),
            game: None,
            collections: Vec::new(),
            probe: None,
        }
    }

    fn scan(directory: &str, paths: &[&str]) -> LibraryScanResult {
        LibraryScanResult {
            directory: directory.to_owned(),
            entries: paths.iter().map(|path| clip(path)).collect(),
            persisted: false,
            collections: Vec::new(),
        }
    }

    #[test]
    fn favorites_games_and_collections_survive_a_restart_and_a_rescan() {
        let scratch = Scratch::new();
        let index = scratch.index();
        refresh_library(scan("/clips", &["/clips/a.mp4", "/clips/b.mp4"]), &index).unwrap();
        let edits = [
            MetaEdit::SetFavorite {
                path: "/clips/a.mp4".into(),
                favorite: true,
            },
            MetaEdit::SetGame {
                path: "/clips/a.mp4".into(),
                game: Some("Hades".into()),
            },
            MetaEdit::CreateCollection {
                name: "Best".into(),
            },
            MetaEdit::AddToCollection {
                path: "/clips/b.mp4".into(),
                name: "Best".into(),
            },
        ];
        for edit in &edits {
            let result = edit_meta(&index, "/clips", edit).unwrap();
            assert!(result.persisted);
        }
        let loaded = load_library(&index).unwrap();
        assert_eq!(loaded.collections, ["Best"]);
        assert!(loaded.entries[0].favorite);
        assert_eq!(loaded.entries[0].game.as_deref(), Some("Hades"));
        assert_eq!(loaded.entries[1].collections, ["Best"]);
        let rescanned =
            refresh_library(scan("/clips", &["/clips/a.mp4", "/clips/b.mp4"]), &index).unwrap();
        assert!(rescanned.entries[0].favorite);
        assert_eq!(rescanned.entries[1].collections, ["Best"]);
        assert_eq!(rescanned.collections, ["Best"]);
    }

    #[test]
    fn a_refused_edit_changes_nothing_and_an_unknown_folder_is_missing() {
        let scratch = Scratch::new();
        let index = scratch.index();
        refresh_library(scan("/clips", &["/clips/a.mp4"]), &index).unwrap();
        let bad = MetaEdit::AddToCollection {
            path: "/clips/a.mp4".into(),
            name: "Nope".into(),
        };
        assert_eq!(
            edit_meta(&index, "/clips", &bad),
            Err(EngineError::NameInvalid)
        );
        assert!(
            load_library(&index).unwrap().entries[0]
                .collections
                .is_empty()
        );
        let fav = MetaEdit::SetFavorite {
            path: "/clips/a.mp4".into(),
            favorite: true,
        };
        assert_eq!(
            edit_meta(&index, "/elsewhere", &fav),
            Err(EngineError::IndexMissing)
        );
    }

    #[test]
    fn deleting_a_collection_reaches_clips_in_other_remembered_folders() {
        let scratch = Scratch::new();
        let index = scratch.index();
        refresh_library(scan("/one", &["/one/a.mp4"]), &index).unwrap();
        refresh_library(scan("/two", &["/two/b.mp4"]), &index).unwrap();
        edit_meta(
            &index,
            "/two",
            &MetaEdit::CreateCollection { name: "X".into() },
        )
        .unwrap();
        edit_meta(
            &index,
            "/two",
            &MetaEdit::AddToCollection {
                path: "/two/b.mp4".into(),
                name: "X".into(),
            },
        )
        .unwrap();
        let mut raw = read_index(&index).unwrap();
        raw.folders[1].entries[0].collections = vec!["X".into()];
        write_index(&index, &raw).unwrap();
        edit_meta(
            &index,
            "/two",
            &MetaEdit::DeleteCollection { name: "X".into() },
        )
        .unwrap();
        let after = read_index(&index).unwrap();
        assert!(after.collections.is_empty());
        assert!(
            after
                .folders
                .iter()
                .all(|f| f.entries.iter().all(|e| e.collections.is_empty()))
        );
    }

    #[test]
    fn refresh_then_load_restores_the_library_after_restart() {
        let scratch = Scratch::new();
        let refreshed = refresh_library(
            scan("/clips", &["/clips/a.mp4", "/clips/b.mp4"]),
            &scratch.index(),
        )
        .expect("refresh");
        assert!(refreshed.persisted);

        let loaded = load_library(&scratch.index()).expect("load");
        assert_eq!(loaded.directory, "/clips");
        assert_eq!(loaded.entries, refreshed.entries);
    }

    #[test]
    fn rescan_keeps_metadata_and_marks_removed_files_offline() {
        let scratch = Scratch::new();
        let mut first = scan("/clips", &["/clips/a.mp4", "/clips/b.mp4"]);
        first.entries[0].favorite = true;
        first.entries[0].tags = vec!["clutch".to_owned()];
        refresh_library(first, &scratch.index()).expect("first refresh");

        let second = refresh_library(scan("/clips", &["/clips/a.mp4"]), &scratch.index())
            .expect("second refresh");

        assert_eq!(second.entries.len(), 2);
        assert!(second.entries[0].favorite);
        assert_eq!(second.entries[0].tags, ["clutch"]);
        assert!(!second.entries[0].offline);
        assert_eq!(second.entries[1].path, "/clips/b.mp4");
        assert!(second.entries[1].offline);
    }

    #[test]
    fn most_recent_folder_loads_first_and_old_folders_are_capped() {
        let scratch = Scratch::new();
        for number in 0..(MAX_INDEXED_FOLDERS + 3) {
            let directory = format!("/folder-{number}");
            refresh_library(scan(&directory, &[]), &scratch.index()).expect("refresh");
        }
        let index = read_index(&scratch.index()).expect("index reads");
        assert_eq!(index.folders.len(), MAX_INDEXED_FOLDERS);
        assert_eq!(
            index.folders[0].directory,
            format!("/folder-{}", MAX_INDEXED_FOLDERS + 2)
        );
    }

    #[test]
    fn missing_index_is_reported_and_malformed_or_oversized_indexes_are_rejected() {
        let scratch = Scratch::new();
        assert_eq!(
            load_library(&scratch.index()),
            Err(EngineError::IndexMissing)
        );

        fs::create_dir_all(scratch.index().parent().expect("parent")).expect("data dir");
        for bad in [
            &b"not json"[..],
            br#"{"version":99,"folders":[]}"#,
            br#"{"version":1}"#,
        ] {
            fs::write(scratch.index(), bad).expect("write bad index");
            assert_eq!(
                load_library(&scratch.index()),
                Err(EngineError::IndexInvalid)
            );
        }

        let oversized = vec![b' '; (MAX_INDEX_BYTES + 1) as usize];
        fs::write(scratch.index(), oversized).expect("write oversized index");
        assert_eq!(
            load_library(&scratch.index()),
            Err(EngineError::IndexInvalid)
        );
    }

    #[test]
    fn hostile_field_sizes_are_rejected() {
        let scratch = Scratch::new();
        let mut entry = clip("/clips/a.mp4");
        entry.tags = vec!["x".repeat(MAX_TAG_BYTES + 1)];
        let index = LibraryIndex {
            version: INDEX_VERSION,
            folders: vec![FolderIndex {
                directory: "/clips".to_owned(),
                entries: vec![entry],
            }],
            collections: Vec::new(),
        };
        write_index(&scratch.index(), &index).expect("write");
        assert_eq!(
            load_library(&scratch.index()),
            Err(EngineError::IndexInvalid)
        );
    }

    #[test]
    fn corrupt_index_is_rebuilt_from_a_scan_and_a_failed_save_keeps_the_scan() {
        let scratch = Scratch::new();
        fs::create_dir_all(scratch.index().parent().expect("parent")).expect("data dir");
        fs::write(scratch.index(), b"garbage").expect("corrupt index");
        let rebuilt = refresh_library(scan("/clips", &["/clips/a.mp4"]), &scratch.index())
            .expect("refresh rebuilds");
        assert!(rebuilt.persisted);
        assert_eq!(
            load_library(&scratch.index()).expect("load").entries.len(),
            1
        );

        // A directory where the index file should be makes the rename fail.
        let blocked = scratch.0.join("blocked");
        fs::create_dir_all(blocked.join("index.json")).expect("blocking directory");
        let result = refresh_library(
            scan("/clips", &["/clips/a.mp4"]),
            &blocked.join("index.json"),
        )
        .expect("scan survives a failed save");
        assert!(!result.persisted);
        assert_eq!(result.entries.len(), 1);
    }

    #[test]
    fn interrupted_write_leaves_the_previous_index_intact() {
        let scratch = Scratch::new();
        refresh_library(scan("/clips", &["/clips/a.mp4"]), &scratch.index()).expect("refresh");
        // A crash after the temp file is written but before the rename leaves a stray .tmp.
        let mut stray = scratch.index().into_os_string();
        stray.push(".tmp");
        fs::write(PathBuf::from(stray), b"half written").expect("stray temp file");

        let loaded = load_library(&scratch.index()).expect("previous index still loads");
        assert_eq!(loaded.entries.len(), 1);
    }

    fn sample_probe(milliseconds: i64) -> clipcraft_media::MediaProbe {
        clipcraft_media::MediaProbe {
            container: "mp4".to_owned(),
            duration: Some(clipcraft_time::Tick::from_rational(milliseconds, 1, 1000)),
            width: Some(1920),
            height: Some(1080),
            frame_rate: Some(clipcraft_time::FrameRate::FPS_60),
            video_codec: Some("avc1".to_owned()),
            audio_tracks: 1,
        }
    }

    #[test]
    fn stored_probes_survive_rescans_until_the_file_changes() {
        let scratch = Scratch::new();
        refresh_library(
            scan("/clips", &["/clips/a.mp4", "/clips/b.mp4"]),
            &scratch.index(),
        )
        .expect("refresh");
        let stored = store_probes(
            &scratch.index(),
            "/clips",
            vec![
                ("/clips/a.mp4".to_owned(), sample_probe(1500)),
                ("/clips/b.mp4".to_owned(), sample_probe(2500)),
                ("/clips/not-in-folder.mp4".to_owned(), sample_probe(1)),
            ],
        )
        .expect("store");
        assert!(stored.persisted);
        assert_eq!(stored.entries.len(), 2, "unknown paths are ignored");

        // a.mp4 is unchanged; b.mp4 grew, so its old probe must be dropped.
        let mut rescan = scan("/clips", &["/clips/a.mp4", "/clips/b.mp4"]);
        rescan.entries[1].size_bytes = 999;
        let merged = refresh_library(rescan, &scratch.index()).expect("rescan");
        assert_eq!(merged.entries[0].probe, Some(sample_probe(1500)));
        assert_eq!(merged.entries[1].probe, None);

        let loaded = load_library(&scratch.index()).expect("load");
        assert_eq!(loaded.entries[0].probe, Some(sample_probe(1500)));
    }

    #[test]
    fn storing_probes_needs_a_known_folder_and_a_bounded_list() {
        let scratch = Scratch::new();
        assert_eq!(
            store_probes(&scratch.index(), "/clips", Vec::new()),
            Err(EngineError::IndexMissing)
        );
        refresh_library(scan("/clips", &["/clips/a.mp4"]), &scratch.index()).expect("refresh");
        assert_eq!(
            store_probes(&scratch.index(), "/other", Vec::new()),
            Err(EngineError::IndexMissing)
        );
        let too_many = vec![("/clips/a.mp4".to_owned(), sample_probe(1)); MAX_STORED_PROBES + 1];
        assert_eq!(
            store_probes(&scratch.index(), "/clips", too_many),
            Err(EngineError::IndexInvalid)
        );
    }
}
