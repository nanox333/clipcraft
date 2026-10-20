//! Shared command and local library behavior for the ClipCraft UI and CLI.

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::UNIX_EPOCH;

mod hotkeys;
mod index;
mod library_meta;
mod recording;
mod session;
mod startup;
mod storage;

pub use clipcraft_media::MediaProbe;

pub use clipcraft_capture::reframe::{
    Framing, MAX_OUTPUT_EDGE, MIN_OUTPUT_EDGE, OutputShape, PixelRect, Reframe, reframe,
};
pub use hotkeys::{HotkeyAction, Hotkeys, normalize_hotkey};
pub use library_meta::{
    GameGroup, MAX_COLLECTIONS, MAX_COLLECTIONS_PER_CLIP, MAX_NAME_CHARS, MetaEdit, apply_edit,
    clean_name, game_of, games, guess_game_from_file_name,
};
pub use recording::{
    AUDIO_KBPS, CaptureSource, Container, EncoderPreset, FRAME_RATES, HEIGHTS, KEYFRAME_SECONDS,
    QualityMode, REPLAY_SECONDS, RateControl, RecordingSettings, SAMPLE_RATES, VIDEO_KBPS,
    VideoCodec, VideoEncoder, default_settings_path,
};
pub use session::{
    CLIP_PREFIX, CaptureSession, SavedClipReport, clip_format, clip_stem, record_config,
};
pub use storage::{
    StorageUsage, StoredClip, cleanup_plan, default_capture_folder, list_clips, storage_usage,
};

pub use index::{
    INDEX_VERSION, MAX_ENTRIES_PER_FOLDER, MAX_INDEX_BYTES, MAX_INDEXED_FOLDERS,
    default_index_path, default_thumbnail_dir,
};

/// Maximum directory entries inspected by one library scan.
pub const MAX_DIRECTORY_ENTRIES: usize = 20_000;
/// Maximum number of recognized videos returned by one scan.
pub const MAX_LIBRARY_RESULTS: usize = 5_000;

const VIDEO_EXTENSIONS: [&str; 7] = ["avi", "m4v", "mkv", "mov", "mp4", "webm", "wmv"];

/// A typed operation exposed by the shared command registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    /// Scans one local directory without recursing into child folders.
    ScanLibrary { directory: PathBuf },
    /// Returns a local media path for display or clipboard use.
    LocalMediaPath { path: PathBuf },
    /// Scans a folder, merges it into the local index, and saves the index atomically.
    RefreshLibrary {
        directory: PathBuf,
        index_path: PathBuf,
    },
    /// Restores the most recently refreshed folder from the local index without scanning.
    LoadLibrary { index_path: PathBuf },
    /// Makes sure a thumbnail for one clip exists in the thumbnail cache, generating it if needed.
    EnsureThumbnail { path: PathBuf, cache_dir: PathBuf },
    /// Reads one clip's container headers for duration, size, frame rate, and codecs.
    ProbeClip { path: PathBuf },
    /// Saves probe results for clips of a remembered folder into the local index.
    /// Reads the saved recording settings, or the defaults when none are saved.
    LoadRecordingSettings { path: PathBuf },
    /// Measures a capture folder and the disk it is on.
    StorageUsage { folder: PathBuf },
    /// Turns launching at sign-in on or off.
    SetLaunchAtStartup { enabled: bool, executable: PathBuf },
    /// Validates and saves recording settings.
    SaveRecordingSettings {
        path: PathBuf,
        settings: Box<RecordingSettings>,
    },
    /// Changes a favorite, game, or collection and saves it in the local index.
    EditLibraryMeta {
        index_path: PathBuf,
        directory: String,
        edit: MetaEdit,
    },
    StoreProbes {
        index_path: PathBuf,
        directory: String,
        probes: Vec<(String, MediaProbe)>,
    },
    /// Shows a local media file in the operating system's file manager.
    RevealInFileManager { path: PathBuf },
}

impl Command {
    /// Returns this operation's stable command ID.
    pub const fn id(&self) -> &'static str {
        match self {
            Self::ScanLibrary { .. } => "library.scan",
            Self::LocalMediaPath { .. } => "library.path",
            Self::RevealInFileManager { .. } => "library.reveal",
            Self::RefreshLibrary { .. } => "library.refresh",
            Self::LoadLibrary { .. } => "library.load",
            Self::ProbeClip { .. } => "library.probe",
            Self::EnsureThumbnail { .. } => "library.thumbnail",
            Self::StoreProbes { .. } => "library.store_probes",
            Self::EditLibraryMeta { .. } => "library.edit",
            Self::LoadRecordingSettings { .. } => "recording.load_settings",
            Self::StorageUsage { .. } => "storage.usage",
            Self::SetLaunchAtStartup { .. } => "app.launch_at_startup",
            Self::SaveRecordingSettings { .. } => "recording.save_settings",
        }
    }
}

/// Result of one local library scan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LibraryScanResult {
    /// Folder scanned by the command.
    pub directory: String,
    /// Video files discovered in the folder.
    pub entries: Vec<MediaEntry>,
    /// Whether these entries are saved in the local index (false for a plain scan or a failed save).
    pub persisted: bool,
    /// Names of the person's collections, including empty ones.
    pub collections: Vec<String>,
}

/// Local metadata that can be shown in a library card.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaEntry {
    /// Full local path. This is never sent outside the process.
    pub path: String,
    /// Display name of the file.
    pub title: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Unix timestamp when available.
    pub modified_unix_seconds: Option<u64>,
    /// Lowercase file extension without a leading dot.
    pub extension: String,
    /// True when the file was remembered by the index but is no longer on disk.
    #[serde(default)]
    pub offline: bool,
    /// User favorite flag, kept across rescans.
    #[serde(default)]
    pub favorite: bool,
    /// User tags, kept across rescans.
    #[serde(default)]
    pub tags: Vec<String>,
    /// The game the person assigned, overriding the guess from folder and file name.
    #[serde(default)]
    pub game: Option<String>,
    /// Collections this clip is in.
    #[serde(default)]
    pub collections: Vec<String>,
    /// Duration, size, and codec facts. Reused across rescans while the file is unchanged.
    #[serde(default)]
    pub probe: Option<MediaProbe>,
}

/// A thumbnail stored in the local cache.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ThumbnailFile {
    /// The cached JPEG.
    pub file: String,
    /// True when this call generated it; false when it was already cached.
    pub created: bool,
}

/// Output produced by a registered engine command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CommandOutput {
    /// Result of scanning a local directory.
    LibraryScan(LibraryScanResult),
    /// A local file path requested by the library UI or CLI.
    LocalMediaPath { path: String },
    /// A local file was handed to the file manager.
    Revealed { path: String },
    /// Container facts read from one clip.
    ClipProbe(MediaProbe),
    /// A clip's thumbnail in the cache.
    Thumbnail(ThumbnailFile),
    /// Recording settings as stored.
    RecordingSettings(Box<RecordingSettings>),
    /// Folder and disk usage.
    StorageUsage(StorageUsage),
    /// Whether ClipCraft now launches at sign-in.
    LaunchAtStartup { enabled: bool },
}

/// Actionable errors returned by engine commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineError {
    /// The selected folder does not exist or cannot be read.
    DirectoryUnavailable,
    /// The selected path is not a directory.
    NotADirectory,
    /// The directory contains more entries than the bounded scanner accepts.
    DirectoryEntryLimit,
    /// The scan would return more results than the bounded library accepts.
    ResultLimit,
    /// A local media path was empty.
    EmptyMediaPath,
    /// The media file no longer exists or cannot be read.
    FileUnavailable,
    /// The system file manager could not be started.
    FileManagerUnavailable,
    /// No saved library index exists yet.
    IndexMissing,
    /// The saved library index could not be read.
    IndexUnavailable,
    /// The saved library index is malformed, oversized, or from an unsupported version.
    IndexInvalid,
    /// The library index could not be saved.
    IndexSaveFailed,
    /// The video's headers are larger than ClipCraft will read.
    MediaTooLarge,
    /// The file is not an MP4 or MOV video, or its headers are damaged.
    MediaUnsupported,
    /// A thumbnail could not be made from this video.
    ThumbnailFailed,
    /// The thumbnail cache folder could not be written.
    ThumbnailCacheFailed,
    /// The saved recording settings could not be read.
    SettingsUnavailable,
    /// The saved recording settings are damaged or from a newer version.
    SettingsInvalid,
    /// The recording settings could not be saved.
    SettingsSaveFailed,
    /// Launching at sign-in could not be changed.
    StartupFailed,
    /// A collection or game name is empty, too long, or does not exist.
    NameInvalid,
    /// Recording could not start or a clip could not be saved.
    Record(clipcraft_record::RecordError),
    /// A recording is already running.
    AlreadyRecording,
    /// No recording is running.
    NotRecording,
    /// The capture folder is at its storage limit and the setting is to stop saving.
    StorageLimit,
    /// A collection with that name already exists.
    NameTaken,
    /// There are too many collections.
    CollectionLimit,
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectoryUnavailable => formatter.write_str(
                "The selected folder could not be read. Check that it exists and that ClipCraft has permission.",
            ),
            Self::NotADirectory => formatter.write_str("Choose a folder to scan."),
            Self::DirectoryEntryLimit => formatter.write_str(
                "This folder has too many entries to scan safely. Choose a narrower folder.",
            ),
            Self::ResultLimit => formatter.write_str(
                "This folder contains more videos than one scan can show. Choose a narrower folder.",
            ),
            Self::EmptyMediaPath => formatter.write_str("Choose a local media file first."),
            Self::FileUnavailable => formatter.write_str(
                "This file could not be found. It may have been moved or deleted.",
            ),
            Self::MediaTooLarge => {
                formatter.write_str("This video's headers are too large to read safely.")
            }
            Self::MediaUnsupported => {
                formatter.write_str("This file is not a readable MP4 or MOV video.")
            }
            Self::ThumbnailFailed => {
                formatter.write_str("A thumbnail could not be made from this video.")
            }
            Self::ThumbnailCacheFailed => {
                formatter.write_str("The thumbnail cache could not be written. Check free disk space.")
            }
            Self::SettingsUnavailable => {
                formatter.write_str("The recording settings could not be read.")
            }
            Self::SettingsInvalid => formatter.write_str(
                "The saved recording settings are damaged or from a newer ClipCraft. Defaults are in use.",
            ),
            Self::SettingsSaveFailed => {
                formatter.write_str("The recording settings could not be saved. Check free disk space.")
            }
            Self::Record(error) => std::fmt::Display::fmt(error, formatter),
            Self::AlreadyRecording => formatter.write_str("A recording is already running."),
            Self::NotRecording => formatter.write_str("Start recording first."),
            Self::StorageLimit => formatter.write_str(
                "The capture folder is at its storage limit, so the clip was not saved. Raise the limit or delete clips.",
            ),
            Self::NameInvalid => formatter.write_str(
                "Use a name of 1 to 40 characters, and pick a collection that exists.",
            ),
            Self::NameTaken => formatter.write_str("A collection with that name already exists."),
            Self::CollectionLimit => formatter.write_str(
                "That is the most collections, or the most collections for one clip, ClipCraft keeps.",
            ),
            Self::StartupFailed => formatter
                .write_str("Launching at sign-in could not be changed on this computer."),
            Self::IndexMissing => formatter.write_str("No saved library was found."),
            Self::IndexUnavailable => formatter.write_str(
                "The saved library could not be read. Scan your clips folder again.",
            ),
            Self::IndexInvalid => formatter.write_str(
                "The saved library is damaged or from a newer version. Scan your clips folder to rebuild it.",
            ),
            Self::IndexSaveFailed => formatter.write_str(
                "The library could not be saved. Your clips are untouched; check free disk space.",
            ),
            Self::FileManagerUnavailable => formatter.write_str(
                "The system file manager could not be opened. Copy the file path instead.",
            ),
        }
    }
}

impl std::error::Error for EngineError {}

/// Executes a typed user operation through the shared UI/CLI command path.
pub fn execute(command: Command) -> Result<CommandOutput, EngineError> {
    match command {
        Command::ScanLibrary { directory } => {
            scan_library_directory(&directory).map(CommandOutput::LibraryScan)
        }
        Command::RefreshLibrary {
            directory,
            index_path,
        } => {
            let scan = scan_library_directory(&directory)?;
            index::refresh_library(scan, &index_path).map(CommandOutput::LibraryScan)
        }
        Command::ProbeClip { path } => {
            if path.as_os_str().is_empty() {
                return Err(EngineError::EmptyMediaPath);
            }
            clipcraft_media::probe_file(&path)
                .map(CommandOutput::ClipProbe)
                .map_err(|error| match error {
                    clipcraft_media::ProbeError::Unreadable => EngineError::FileUnavailable,
                    clipcraft_media::ProbeError::TooLarge => EngineError::MediaTooLarge,
                    clipcraft_media::ProbeError::Unsupported => EngineError::MediaUnsupported,
                })
        }
        Command::LoadRecordingSettings { path } => recording::load_settings(&path)
            .map(|settings| CommandOutput::RecordingSettings(Box::new(settings))),
        Command::SaveRecordingSettings { path, settings } => {
            recording::save_settings(&path, *settings)
                .map(|settings| CommandOutput::RecordingSettings(Box::new(settings)))
        }
        Command::StorageUsage { folder } => {
            if folder.as_os_str().is_empty() {
                return Err(EngineError::DirectoryUnavailable);
            }
            Ok(CommandOutput::StorageUsage(storage_usage(&folder)))
        }
        Command::SetLaunchAtStartup {
            enabled,
            executable,
        } => startup::set_launch_at_startup(enabled, &executable)
            .map(|()| CommandOutput::LaunchAtStartup { enabled }),
        Command::EnsureThumbnail { path, cache_dir } => {
            if path.as_os_str().is_empty() {
                return Err(EngineError::EmptyMediaPath);
            }
            clipcraft_player::thumbnail::ensure_cached(&path, &cache_dir)
                .map(|cached| {
                    CommandOutput::Thumbnail(ThumbnailFile {
                        file: cached.file.to_string_lossy().into_owned(),
                        created: cached.created,
                    })
                })
                .map_err(|error| match error {
                    clipcraft_player::thumbnail::ThumbnailError::Unreadable => {
                        EngineError::FileUnavailable
                    }
                    clipcraft_player::thumbnail::ThumbnailError::Unsupported => {
                        EngineError::MediaUnsupported
                    }
                    clipcraft_player::thumbnail::ThumbnailError::CacheFailed => {
                        EngineError::ThumbnailCacheFailed
                    }
                    clipcraft_player::thumbnail::ThumbnailError::DecodeFailed
                    | clipcraft_player::thumbnail::ThumbnailError::Crashed => {
                        EngineError::ThumbnailFailed
                    }
                })
        }
        Command::StoreProbes {
            index_path,
            directory,
            probes,
        } => index::store_probes(&index_path, &directory, probes).map(CommandOutput::LibraryScan),
        Command::EditLibraryMeta {
            index_path,
            directory,
            edit,
        } => index::edit_meta(&index_path, &directory, &edit).map(CommandOutput::LibraryScan),
        Command::LoadLibrary { index_path } => {
            index::load_library(&index_path).map(CommandOutput::LibraryScan)
        }
        Command::RevealInFileManager { path } => {
            reveal_in_file_manager(&path).map(|path| CommandOutput::Revealed { path })
        }
        Command::LocalMediaPath { path } => {
            if path.as_os_str().is_empty() {
                return Err(EngineError::EmptyMediaPath);
            }
            Ok(CommandOutput::LocalMediaPath {
                path: path.to_string_lossy().into_owned(),
            })
        }
    }
}

/// File managers ClipCraft can hand a path to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileManager {
    Explorer,
    Finder,
    XdgOpen,
}

impl FileManager {
    const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Explorer
        } else if cfg!(target_os = "macos") {
            Self::Finder
        } else {
            Self::XdgOpen
        }
    }
}

/// Returns the program and arguments that show `path` in a file manager.
///
/// `path` must be absolute so it can never be read as a command-line option.
fn reveal_invocation(manager: FileManager, path: &Path) -> (&'static str, Vec<OsString>) {
    match manager {
        FileManager::Explorer => {
            let mut selection = OsString::from("/select,");
            selection.push(path);
            ("explorer", vec![selection])
        }
        FileManager::Finder => ("open", vec![OsString::from("-R"), path.into()]),
        FileManager::XdgOpen => (
            "xdg-open",
            vec![path.parent().unwrap_or(path).as_os_str().to_owned()],
        ),
    }
}

fn reveal_in_file_manager(path: &Path) -> Result<String, EngineError> {
    if path.as_os_str().is_empty() {
        return Err(EngineError::EmptyMediaPath);
    }
    let absolute = std::path::absolute(path).map_err(|_| EngineError::FileUnavailable)?;
    fs::metadata(&absolute).map_err(|_| EngineError::FileUnavailable)?;

    let (program, arguments) = reveal_invocation(FileManager::current(), &absolute);
    let mut child = std::process::Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| EngineError::FileManagerUnavailable)?;
    // Reap the launcher without blocking the caller; Explorer exits non-zero on success.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(absolute.to_string_lossy().into_owned())
}

/// Scans one folder for common local video containers.
pub fn scan_library_directory(directory: &Path) -> Result<LibraryScanResult, EngineError> {
    scan_library_directory_with_limit(directory, MAX_DIRECTORY_ENTRIES, MAX_LIBRARY_RESULTS)
}

fn scan_library_directory_with_limit(
    directory: &Path,
    max_entries: usize,
    max_results: usize,
) -> Result<LibraryScanResult, EngineError> {
    let directory_metadata =
        fs::metadata(directory).map_err(|_| EngineError::DirectoryUnavailable)?;
    if !directory_metadata.is_dir() {
        return Err(EngineError::NotADirectory);
    }

    let read_dir = fs::read_dir(directory).map_err(|_| EngineError::DirectoryUnavailable)?;
    let mut seen_entries = 0usize;
    let mut media = Vec::new();

    for directory_entry in read_dir {
        seen_entries = seen_entries
            .checked_add(1)
            .ok_or(EngineError::DirectoryEntryLimit)?;
        if seen_entries > max_entries {
            return Err(EngineError::DirectoryEntryLimit);
        }

        let directory_entry = directory_entry.map_err(|_| EngineError::DirectoryUnavailable)?;
        let path = directory_entry.path();
        let file_type = directory_entry
            .file_type()
            .map_err(|_| EngineError::DirectoryUnavailable)?;
        if !file_type.is_file() {
            continue;
        }

        let metadata =
            fs::symlink_metadata(&path).map_err(|_| EngineError::DirectoryUnavailable)?;
        if !metadata.file_type().is_file() {
            continue;
        }

        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        let Some(extension) = extension.filter(|value| VIDEO_EXTENSIONS.contains(&value.as_str()))
        else {
            continue;
        };

        if media.len() >= max_results {
            return Err(EngineError::ResultLimit);
        }

        let title = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled clip".to_owned());
        let modified_unix_seconds = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());

        media.push(MediaEntry {
            path: path.to_string_lossy().into_owned(),
            title,
            size_bytes: metadata.len(),
            modified_unix_seconds,
            extension,
            offline: false,
            favorite: false,
            tags: Vec::new(),
            game: None,
            collections: Vec::new(),
            probe: None,
        });
    }

    media.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.path.cmp(&right.path))
    });

    Ok(LibraryScanResult {
        directory: directory.to_string_lossy().into_owned(),
        entries: media,
        persisted: false,
        collections: Vec::new(),
    })
}

/// Lists stable user-facing command IDs available to the first application slice.
pub const fn command_ids() -> &'static [&'static str] {
    &[
        "library.scan",
        "library.path",
        "library.reveal",
        "library.refresh",
        "library.load",
        "library.probe",
        "library.thumbnail",
        "library.store_probes",
        "library.edit",
        "recording.load_settings",
        "storage.usage",
        "app.launch_at_startup",
        "capture.start",
        "capture.stop",
        "capture.status",
        "capture.save_clip",
        "capture.toggle_microphone",
        "capture.push_to_talk",
        "recording.save_settings",
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        Command, CommandOutput, EngineError, FileManager, MAX_DIRECTORY_ENTRIES,
        MAX_LIBRARY_RESULTS, reveal_invocation, scan_library_directory,
        scan_library_directory_with_limit,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("clipcraft-engine-test-{}-{id}", std::process::id()));
            fs::create_dir(&path).expect("test directory can be created");
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn scan_finds_videos_sorts_them_and_ignores_other_files() {
        let directory = TestDirectory::new();
        fs::write(directory.path().join("zeta.MP4"), b"1234").expect("video fixture writes");
        fs::write(directory.path().join("alpha.mkv"), b"12").expect("video fixture writes");
        fs::write(directory.path().join("notes.txt"), b"ignore").expect("text fixture writes");
        fs::create_dir(directory.path().join("nested")).expect("nested directory creates");

        let result = scan_library_directory(directory.path()).expect("folder scan succeeds");

        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.entries[0].title, "alpha.mkv");
        assert_eq!(result.entries[0].size_bytes, 2);
        assert_eq!(result.entries[1].extension, "mp4");
        assert_eq!(result.entries[1].size_bytes, 4);
    }

    #[test]
    fn scan_rejects_a_file_and_a_missing_folder() {
        let directory = TestDirectory::new();
        let file = directory.path().join("clip.mp4");
        fs::write(&file, b"video").expect("fixture writes");
        assert_eq!(
            scan_library_directory(&file),
            Err(EngineError::NotADirectory)
        );

        let missing = directory.path().join("missing");
        assert_eq!(
            scan_library_directory(&missing),
            Err(EngineError::DirectoryUnavailable)
        );
    }

    #[test]
    fn scan_enforces_directory_and_result_bounds() {
        let directory = TestDirectory::new();
        fs::write(directory.path().join("one.mp4"), b"1").expect("fixture writes");
        fs::write(directory.path().join("two.mp4"), b"2").expect("fixture writes");
        assert_eq!(
            scan_library_directory_with_limit(directory.path(), 1, MAX_LIBRARY_RESULTS),
            Err(EngineError::DirectoryEntryLimit)
        );
        assert_eq!(
            scan_library_directory_with_limit(directory.path(), MAX_DIRECTORY_ENTRIES, 1),
            Err(EngineError::ResultLimit)
        );
    }

    #[cfg(unix)]
    #[test]
    fn scan_ignores_symbolic_links() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let target = directory.path().join("real.mp4");
        let link = directory.path().join("linked.mp4");
        fs::write(&target, b"video").expect("fixture writes");
        symlink(&target, &link).expect("symbolic link creates");

        let result = scan_library_directory(directory.path()).expect("folder scan succeeds");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].title, "real.mp4");
    }

    #[test]
    fn ui_and_cli_share_the_same_typed_command() {
        let directory = TestDirectory::new();
        let command = Command::ScanLibrary {
            directory: directory.0.clone(),
        };
        assert_eq!(command.id(), "library.scan");
        assert!(matches!(
            super::execute(command),
            Ok(CommandOutput::LibraryScan(_))
        ));
    }

    #[test]
    fn local_media_path_command_returns_path_and_rejects_empty_input() {
        let path = PathBuf::from("/clips/example.mp4");
        let command = Command::LocalMediaPath { path: path.clone() };

        assert_eq!(command.id(), "library.path");
        assert_eq!(
            super::execute(command),
            Ok(CommandOutput::LocalMediaPath {
                path: path.to_string_lossy().into_owned()
            })
        );
        assert_eq!(
            super::execute(Command::LocalMediaPath {
                path: PathBuf::new()
            }),
            Err(EngineError::EmptyMediaPath)
        );
    }

    #[test]
    fn reveal_invocation_selects_the_file_on_each_platform() {
        let file = PathBuf::from("/clips/a.mp4");

        let (program, arguments) = reveal_invocation(FileManager::Explorer, &file);
        assert_eq!(program, "explorer");
        assert_eq!(arguments, ["/select,/clips/a.mp4"]);

        let (program, arguments) = reveal_invocation(FileManager::Finder, &file);
        assert_eq!(program, "open");
        assert_eq!(arguments, ["-R", "/clips/a.mp4"]);

        let (program, arguments) = reveal_invocation(FileManager::XdgOpen, &file);
        assert_eq!(program, "xdg-open");
        assert_eq!(arguments, ["/clips"]);
    }

    #[test]
    fn reveal_command_rejects_empty_and_missing_files_without_launching_anything() {
        assert_eq!(
            Command::RevealInFileManager {
                path: PathBuf::new()
            }
            .id(),
            "library.reveal"
        );
        assert_eq!(
            super::execute(Command::RevealInFileManager {
                path: PathBuf::new()
            }),
            Err(EngineError::EmptyMediaPath)
        );
        let directory = TestDirectory::new();
        assert_eq!(
            super::execute(Command::RevealInFileManager {
                path: directory.path().join("missing.mp4")
            }),
            Err(EngineError::FileUnavailable)
        );
    }

    #[test]
    fn probe_command_maps_failures_to_actionable_errors() {
        assert_eq!(
            Command::ProbeClip {
                path: PathBuf::new()
            }
            .id(),
            "library.probe"
        );
        assert_eq!(
            super::execute(Command::ProbeClip {
                path: PathBuf::new()
            }),
            Err(EngineError::EmptyMediaPath)
        );
        let directory = TestDirectory::new();
        assert_eq!(
            super::execute(Command::ProbeClip {
                path: directory.path().join("missing.mp4")
            }),
            Err(EngineError::FileUnavailable)
        );
        let fake = directory.path().join("fake.mp4");
        fs::write(&fake, b"not really a video").expect("fixture writes");
        assert_eq!(
            super::execute(Command::ProbeClip { path: fake }),
            Err(EngineError::MediaUnsupported)
        );
    }

    #[test]
    fn thumbnail_command_maps_failures_to_actionable_errors() {
        let directory = TestDirectory::new();
        let cache_dir = directory.path().join("cache");
        let command = |path: PathBuf| Command::EnsureThumbnail {
            path,
            cache_dir: cache_dir.clone(),
        };
        assert_eq!(command(PathBuf::new()).id(), "library.thumbnail");
        assert_eq!(
            super::execute(command(PathBuf::new())),
            Err(EngineError::EmptyMediaPath)
        );
        assert_eq!(
            super::execute(command(directory.path().join("missing.mp4"))),
            Err(EngineError::FileUnavailable)
        );
        let fake = directory.path().join("fake.mp4");
        fs::write(&fake, b"not really a video").expect("fixture writes");
        assert_eq!(
            super::execute(command(fake)),
            Err(EngineError::MediaUnsupported)
        );
    }
}
