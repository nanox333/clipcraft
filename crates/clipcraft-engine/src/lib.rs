//! Shared command and local library behavior for the ClipCraft UI and CLI.

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::AtomicBool;

mod diagnose;
mod editor;
pub use editor::{
    EditorAction, EditorClip, EditorSession, MAX_EDITOR_DURATION, MAX_EDITOR_HISTORY,
    MIN_EDITOR_CLIP,
};
mod game_catalog;
mod game_detection;
pub use game_catalog::{BuiltInGame, MAX_BUILT_IN_GAMES, built_in_games};
mod hotkeys;
mod index;
mod library_meta;
mod recording;
mod recovery;
mod relink;
mod scan;
mod screenshot;
pub use scan::MAX_SCAN_DEPTH;
mod session;
mod startup;
mod steam_games;
pub use steam_games::{MAX_STEAM_GAMES, SteamGame, SteamGames};
mod storage;

pub use clipcraft_export::{ExportPreset, TrimRequest as ExportRange};
pub use clipcraft_export::{SequenceRequest, SequenceSegment};
pub use clipcraft_media::MediaProbe;

pub use clipcraft_capture::reframe::{
    Framing, MAX_OUTPUT_EDGE, MIN_OUTPUT_EDGE, OutputShape, PixelRect, Reframe, reframe,
};
pub use diagnose::{Check, Diagnostics};
pub use game_detection::{
    GameArmingDecision, GameArmingState, GameMatchRule, GameProcessMatch, best_game_match,
    match_game_process, match_game_with_user_priority,
};
pub use hotkeys::{HotkeyAction, Hotkeys, normalize_hotkey};
pub use library_meta::{
    GameGroup, MAX_COLLECTIONS, MAX_COLLECTIONS_PER_CLIP, MAX_NAME_CHARS, MetaEdit, apply_edit,
    clean_name, game_of, games, guess_game_from_file_name,
};
pub use recording::{
    AUDIO_KBPS, CaptureSource, Container, DesktopCaptureMode, EncoderPreset, FRAME_RATES, HEIGHTS,
    KEYFRAME_SECONDS, POST_ROLL_SECONDS, QualityMode, REPLAY_SECONDS, RateControl,
    RecordingSettings, SAMPLE_RATES, VIDEO_KBPS, VideoCodec, VideoEncoder, default_settings_path,
    estimated_bytes, format_bytes,
};
pub use recovery::{Recovery, recover_interrupted};
pub use relink::{SearchResult, find_moved_file};
pub use screenshot::{ScreenshotJob, screenshot_name};
pub use session::{
    CLIP_PREFIX, CaptureSession, SaveJob, SavedClipReport, clip_format, clip_stem, record_config,
};
pub use storage::{
    StorageUsage, StoredClip, cleanup_plan, default_capture_folder, list_clips, storage_usage,
};

pub use index::merge_entries as merge_library_entries;
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
    ImportEditorMedia {
        paths: Vec<PathBuf>,
    },
    EditSequence {
        session: Box<EditorSession>,
        action: EditorAction,
    },
    ExportSequence {
        request: Box<SequenceRequest>,
    },
    /// Read only local Steam installation manifests.
    ScanSteamGames {
        roots: Vec<PathBuf>,
    },
    /// Enables/disables built-in identifiers while preserving custom rules and the global switch.
    SetBuiltInGames {
        settings: Box<RecordingSettings>,
        ids: Vec<String>,
        enabled: bool,
    },
    /// Scans a local folder tree without following symbolic links.
    ScanLibrary {
        directory: PathBuf,
    },
    /// Returns a local media path for display or clipboard use.
    LocalMediaPath {
        path: PathBuf,
    },
    /// Scans a folder, merges it into the local index, and saves the index atomically.
    RefreshLibrary {
        directory: PathBuf,
        index_path: PathBuf,
    },
    /// Merges a completed worker scan on the caller's thread, preserving current metadata.
    StoreLibraryScan {
        index_path: PathBuf,
        scan: LibraryScanResult,
    },
    /// Registers a saved recording and its captured game without replacing manual labels.
    RegisterSavedClip {
        index_path: PathBuf,
        directory: PathBuf,
        path: PathBuf,
        game: String,
    },
    /// Restores the most recently refreshed folder from the local index without scanning.
    LoadLibrary {
        index_path: PathBuf,
    },
    /// Makes sure a thumbnail for one clip exists in the thumbnail cache, generating it if needed.
    EnsureThumbnail {
        path: PathBuf,
        cache_dir: PathBuf,
    },
    /// Reads one clip's container headers for duration, size, frame rate, and codecs.
    ProbeClip {
        path: PathBuf,
    },
    /// Saves probe results for clips of a remembered folder into the local index.
    /// Reads the saved recording settings, or the defaults when none are saved.
    LoadRecordingSettings {
        path: PathBuf,
    },
    /// Measures a capture folder and the disk it is on.
    StorageUsage {
        folder: PathBuf,
    },
    /// Turns launching at sign-in on or off.
    SetLaunchAtStartup {
        enabled: bool,
        executable: PathBuf,
    },
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
    RevealInFileManager {
        path: PathBuf,
    },
    /// Looks for a moved clip by name and size in a folder and the folders inside it.
    FindMovedFile {
        name: String,
        size_bytes: u64,
        folder: PathBuf,
    },
    /// Cuts a clip to a range and saves it as a new MP4 without re-encoding.
    TrimClip {
        input: PathBuf,
        output: PathBuf,
        start_ms: u64,
        end_ms: u64,
    },
    /// Re-encodes a range locally with an estimated size target.
    ExportClip {
        input: PathBuf,
        output: PathBuf,
        start_ms: u64,
        end_ms: u64,
        preset: ExportPreset,
    },
}

impl Command {
    /// Returns this operation's stable command ID.
    pub const fn id(&self) -> &'static str {
        match self {
            Self::ImportEditorMedia { .. } => "editor.import",
            Self::EditSequence { .. } => "editor.edit",
            Self::ExportSequence { .. } => "editor.export",
            Self::ScanSteamGames { .. } => "recording.scan_steam_games",
            Self::SetBuiltInGames { .. } => "recording.set_builtin_games",
            Self::ScanLibrary { .. } => "library.scan",
            Self::LocalMediaPath { .. } => "library.path",
            Self::RevealInFileManager { .. } => "library.reveal",
            Self::RefreshLibrary { .. } => "library.refresh",
            Self::LoadLibrary { .. } => "library.load",
            Self::StoreLibraryScan { .. } => "library.store_scan",
            Self::RegisterSavedClip { .. } => "library.register_recording",
            Self::ProbeClip { .. } => "library.probe",
            Self::EnsureThumbnail { .. } => "library.thumbnail",
            Self::StoreProbes { .. } => "library.store_probes",
            Self::EditLibraryMeta { .. } => "library.edit",
            Self::LoadRecordingSettings { .. } => "recording.load_settings",
            Self::StorageUsage { .. } => "storage.usage",
            Self::SetLaunchAtStartup { .. } => "app.launch_at_startup",
            Self::SaveRecordingSettings { .. } => "recording.save_settings",
            Self::TrimClip { .. } => "clip.trim",
            Self::ExportClip { .. } => "clip.export",
            Self::FindMovedFile { .. } => "library.find_file",
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
    /// Game active when this recording was saved; a manual game overrides it.
    #[serde(default)]
    pub recorded_game: Option<String>,
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

/// A clip that was cut and saved.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TrimmedClip {
    /// The new file.
    pub path: String,
    /// Its length in milliseconds.
    pub duration_ms: u64,
    /// Pictures written.
    pub pictures: u64,
    /// Tracks of sound written.
    pub audio_tracks: usize,
    /// File size in bytes.
    pub bytes: u64,
}

/// A compressed local clip, with its estimate and actual file size.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExportedClip {
    pub clip: TrimmedClip,
    pub target_bytes: u64,
    pub within_target: bool,
    pub width: u32,
    pub height: u32,
}

/// Shared `editor.export` implementation; edits and source files are never modified.
pub fn export_sequence(
    request: &SequenceRequest,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<TrimmedClip, EngineError> {
    let report = clipcraft_export::sequence_to_mp4(request, progress, cancel)
        .map_err(EngineError::Export)?;
    Ok(TrimmedClip {
        path: report.path.to_string_lossy().into_owned(),
        duration_ms: report.duration.to_units_floor(1000) as u64,
        pictures: report.pictures,
        audio_tracks: report.audio_tracks,
        bytes: report.bytes,
    })
}

/// Command `clip.export`: compress a range without modifying or uploading its source.
pub fn export_clip(
    request: &ExportRange,
    preset: ExportPreset,
    progress: &mut dyn FnMut(f32),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<ExportedClip, EngineError> {
    if request.input.as_os_str().is_empty() || request.output.as_os_str().is_empty() {
        return Err(EngineError::EmptyMediaPath);
    }
    let report = clipcraft_export::compress_to_mp4(request, preset, progress, cancel)
        .map_err(EngineError::Export)?;
    Ok(ExportedClip {
        clip: TrimmedClip {
            path: report.clip.path.to_string_lossy().into_owned(),
            duration_ms: u64::try_from(report.clip.duration.to_units_floor(1_000)).unwrap_or(0),
            pictures: report.clip.pictures,
            audio_tracks: report.clip.audio_tracks,
            bytes: report.clip.bytes,
        },
        target_bytes: report.target_bytes,
        within_target: report.within_target,
        width: report.width,
        height: report.height,
    })
}

/// Cuts `input` from `start_ms` to `end_ms` into a new MP4 at `output`, reporting progress from 0
/// to 1 and stopping when `cancel` is set. Nothing is re-encoded and the input is never changed.
/// Command id `clip.trim`.
pub fn trim_clip(
    input: &std::path::Path,
    output: &std::path::Path,
    start_ms: u64,
    end_ms: u64,
    progress: &mut dyn FnMut(f32),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<TrimmedClip, EngineError> {
    if input.as_os_str().is_empty() || output.as_os_str().is_empty() {
        return Err(EngineError::EmptyMediaPath);
    }
    let to_tick = |ms: u64| {
        clipcraft_time::Tick::from_units(i64::try_from(ms).unwrap_or(i64::MAX / 4), 1_000)
    };
    let request = clipcraft_export::TrimRequest {
        input: input.to_path_buf(),
        output: output.to_path_buf(),
        start: to_tick(start_ms),
        end: to_tick(end_ms),
    };
    let report =
        clipcraft_export::trim_to_mp4(&request, progress, cancel).map_err(EngineError::Export)?;
    Ok(TrimmedClip {
        path: report.path.to_string_lossy().into_owned(),
        duration_ms: u64::try_from(report.duration.to_units_floor(1_000)).unwrap_or(0),
        pictures: report.pictures,
        audio_tracks: report.audio_tracks,
        bytes: report.bytes,
    })
}

/// A file name beside `input` for its trimmed copy (`Name (trimmed).mp4`, then `(trimmed 2)`),
/// one that does not exist yet.
pub fn trimmed_path(input: &std::path::Path) -> Option<PathBuf> {
    copy_path(input, "trimmed")
}

/// An unused name for an exported copy beside the source.
pub fn exported_path(input: &std::path::Path) -> Option<PathBuf> {
    copy_path(input, "exported")
}

/// An unused MP4 name beside the first source in a sequence.
pub fn edited_path(input: &std::path::Path) -> Option<PathBuf> {
    copy_path(input, "edit")
}

fn copy_path(input: &std::path::Path, suffix: &str) -> Option<PathBuf> {
    let folder = input.parent()?;
    let stem = input.file_stem()?.to_string_lossy().into_owned();
    (1..1_000).find_map(|n| {
        let tag = if n == 1 {
            suffix.to_owned()
        } else {
            format!("{suffix} {n}")
        };
        let candidate = folder.join(format!("{stem} ({tag}).mp4"));
        (!candidate.exists()).then_some(candidate)
    })
}

/// Output produced by a registered engine command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CommandOutput {
    EditorMedia(Vec<MediaEntry>),
    EditorSession(Box<EditorSession>),
    SteamGames(SteamGames),
    /// Result of scanning a local directory.
    LibraryScan(LibraryScanResult),
    /// A local file path requested by the library UI or CLI.
    LocalMediaPath {
        path: String,
    },
    /// A local file was handed to the file manager.
    Revealed {
        path: String,
    },
    /// Container facts read from one clip.
    ClipProbe(MediaProbe),
    /// A clip's thumbnail in the cache.
    Thumbnail(ThumbnailFile),
    /// Recording settings as stored.
    RecordingSettings(Box<RecordingSettings>),
    /// Folder and disk usage.
    StorageUsage(StorageUsage),
    /// A clip that was cut and saved.
    Trimmed(TrimmedClip),
    /// A compressed clip and the actual/target size comparison.
    Exported(ExportedClip),
    /// Where a moved clip was found, if it was.
    FoundFile {
        path: Option<String>,
        folders_searched: usize,
    },
    /// Whether ClipCraft now launches at sign-in.
    LaunchAtStartup {
        enabled: bool,
    },
}

/// Actionable errors returned by engine commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineError {
    EditorInvalid,
    SteamMetadataInvalid,
    /// A requested starter game does not exist in the shipped catalog.
    GamePresetInvalid,
    /// A built-in game change is empty or exceeds the bounded request size.
    GameRuleLimit,
    /// The selected folder does not exist or cannot be read.
    DirectoryUnavailable,
    /// The selected path is not a directory.
    NotADirectory,
    /// The directory contains more entries than the bounded scanner accepts.
    DirectoryEntryLimit,
    /// The scan would return more results than the bounded library accepts.
    ResultLimit,
    /// The scan was cancelled before publishing any result.
    ScanCancelled,
    /// A child directory exceeds the bounded traversal depth.
    ScanDepthLimit,
    /// A path cannot be represented safely in the local JSON index.
    LibraryPathInvalid,
    /// A local media path was empty.
    EmptyMediaPath,
    /// The media file no longer exists or cannot be read.
    FileUnavailable,
    /// The file is still on disk, so it cannot be removed from the library.
    FileStillExists,
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
    /// A clip could not be cut and saved.
    Export(clipcraft_export::ExportError),
    /// A screenshot could not be taken or saved.
    ScreenshotFailed,
    /// A collection with that name already exists.
    NameTaken,
    /// There are too many collections.
    CollectionLimit,
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EditorInvalid => formatter.write_str("This edit is outside its limits. Use at most 30 minutes total, valid source ranges at least 1/30 second long, and volume from 0 to 100%."),
            Self::SteamMetadataInvalid => formatter.write_str("Steam library metadata could not be read safely. Check the installation or refresh again; no game files were changed."),
            Self::GamePresetInvalid => formatter.write_str("Choose a game from the built-in catalog, or add a custom game."),
            Self::GameRuleLimit => formatter.write_str("Choose one or more known games, with at most 1024 IDs in a request."),
            Self::DirectoryUnavailable => formatter.write_str(
                "The selected folder could not be read. Check that it exists and that ClipCraft has permission.",
            ),
            Self::NotADirectory => formatter.write_str("Choose a folder to scan."),
            Self::DirectoryEntryLimit => formatter.write_str(
                "This folder has too many entries to scan safely. Choose a narrower folder.",
            ),
            Self::ScanCancelled => formatter.write_str("Library scan cancelled. The previous library is unchanged."),
            Self::ScanDepthLimit => formatter.write_str("This folder tree is deeper than 8 levels. Choose a narrower folder."),
            Self::LibraryPathInvalid => formatter.write_str("A library path is too long or is not valid UTF-8. Choose a narrower folder or rename the file."),
            Self::ResultLimit => formatter.write_str(
                "This folder contains more videos than one scan can show. Choose a narrower folder.",
            ),
            Self::EmptyMediaPath => formatter.write_str("Choose a local media file first."),
            Self::FileUnavailable => formatter.write_str(
                "This file could not be found. It may have been moved or deleted.",
            ),
            Self::FileStillExists => formatter.write_str(
                "This file is still on your computer. Delete or move it first, then remove it from the library.",
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
            Self::Export(error) => std::fmt::Display::fmt(error, formatter),
            Self::AlreadyRecording => formatter.write_str("A recording is already running."),
            Self::NotRecording => formatter.write_str("Start clipping first."),
            Self::StorageLimit => formatter.write_str(
                "The capture folder is at its storage limit, so the clip was not saved. Raise the limit or delete clips.",
            ),
            Self::NameInvalid => formatter.write_str(
                "Use a name of 1 to 40 characters, and pick a collection that exists.",
            ),
            Self::ScreenshotFailed => formatter.write_str(
                "The screenshot could not be taken or saved. Check free disk space and that clipping is running.",
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
    execute_cancellable(command, &AtomicBool::new(false))
}

/// Executes a command; traversal commands cooperate with cancellation between filesystem calls.
/// Other commands retain their existing atomic operation semantics.
pub fn execute_cancellable(
    command: Command,
    cancel: &AtomicBool,
) -> Result<CommandOutput, EngineError> {
    match command {
        Command::ImportEditorMedia { paths } => {
            editor::import_media(paths).map(CommandOutput::EditorMedia)
        }
        Command::EditSequence { session, action } => session
            .apply(action)
            .map(|state| CommandOutput::EditorSession(Box::new(state))),
        Command::ExportSequence { request } => {
            export_sequence(&request, &mut |_| {}, cancel).map(CommandOutput::Trimmed)
        }
        Command::ScanSteamGames { roots } => {
            steam_games::scan(&roots, cancel).map(CommandOutput::SteamGames)
        }
        Command::SetBuiltInGames {
            settings,
            ids,
            enabled,
        } => game_catalog::set_games(*settings, &ids, enabled)
            .map(|settings| CommandOutput::RecordingSettings(Box::new(settings))),
        Command::ScanLibrary { directory } => {
            scan_library_directory_cancellable(&directory, cancel).map(CommandOutput::LibraryScan)
        }
        Command::RefreshLibrary {
            directory,
            index_path,
        } => {
            let scan = scan_library_directory_cancellable(&directory, cancel)?;
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(EngineError::ScanCancelled);
            }
            index::refresh_library(scan, &index_path).map(CommandOutput::LibraryScan)
        }
        Command::StoreLibraryScan { index_path, scan } => {
            index::refresh_library(scan, &index_path).map(CommandOutput::LibraryScan)
        }
        Command::RegisterSavedClip {
            index_path,
            directory,
            path,
            game,
        } => index::register_recording(&index_path, &directory, &path, &game)
            .map(CommandOutput::LibraryScan),
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
        Command::TrimClip {
            input,
            output,
            start_ms,
            end_ms,
        } => {
            let never = std::sync::atomic::AtomicBool::new(false);
            trim_clip(&input, &output, start_ms, end_ms, &mut |_| {}, &never)
                .map(CommandOutput::Trimmed)
        }
        Command::ExportClip {
            input,
            output,
            start_ms,
            end_ms,
            preset,
        } => {
            let never = std::sync::atomic::AtomicBool::new(false);
            let to_tick = |ms: u64| {
                clipcraft_time::Tick::from_units(i64::try_from(ms).unwrap_or(i64::MAX), 1_000)
            };
            let request = clipcraft_export::TrimRequest {
                input,
                output,
                start: to_tick(start_ms),
                end: to_tick(end_ms),
            };
            export_clip(&request, preset, &mut |_| {}, &never).map(CommandOutput::Exported)
        }
        Command::FindMovedFile {
            name,
            size_bytes,
            folder,
        } => {
            let result = find_moved_file(&name, size_bytes, &folder);
            Ok(CommandOutput::FoundFile {
                path: result.found.map(|path| path.to_string_lossy().into_owned()),
                folders_searched: result.folders_searched,
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

/// Scans a folder tree for common local video containers with bounded work.
pub fn scan_library_directory(directory: &Path) -> Result<LibraryScanResult, EngineError> {
    scan_library_directory_cancellable(directory, &AtomicBool::new(false))
}

/// Same scanner as `library.scan`, with cooperative cancellation between filesystem operations.
/// A cancelled or failed traversal never returns partial results or mutates the index.
pub fn scan_library_directory_cancellable(
    directory: &Path,
    cancel: &AtomicBool,
) -> Result<LibraryScanResult, EngineError> {
    scan::scan(
        directory,
        MAX_DIRECTORY_ENTRIES,
        MAX_LIBRARY_RESULTS,
        cancel,
    )
}

#[cfg(test)]
fn scan_library_directory_with_limit(
    directory: &Path,
    max_entries: usize,
    max_results: usize,
) -> Result<LibraryScanResult, EngineError> {
    scan::scan(directory, max_entries, max_results, &AtomicBool::new(false))
}

/// Lists stable user-facing command IDs available to the first application slice.
pub const fn command_ids() -> &'static [&'static str] {
    &[
        "editor.import",
        "editor.edit",
        "editor.export",
        "library.scan",
        "library.path",
        "library.reveal",
        "library.refresh",
        "library.load",
        "library.store_scan",
        "library.register_recording",
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
        "clip.trim",
        "clip.export",
        "library.find_file",
        "capture.toggle_microphone",
        "capture.push_to_talk",
        "recording.save_settings",
        "recording.set_builtin_games",
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
    fn export_command_validates_paths_ranges_and_size_before_creating_output() {
        let directory = TestDirectory::new();
        let output = directory.path().join("out.mp4");
        let export = |input: &str, start_ms, end_ms, megabytes| {
            super::execute(Command::ExportClip {
                input: input.into(),
                output: output.clone(),
                start_ms,
                end_ms,
                preset: super::ExportPreset::Custom { megabytes },
            })
        };
        assert_eq!(export("", 0, 1_000, 10), Err(EngineError::EmptyMediaPath));
        assert_eq!(
            export("missing.mp4", 0, 1_000, 0),
            Err(EngineError::Export(
                clipcraft_export::ExportError::BadPreset
            ))
        );
        assert_eq!(
            export("missing.mp4", 2_000, 1_000, 10),
            Err(EngineError::Export(clipcraft_export::ExportError::BadRange))
        );
        assert!(!output.exists());
        assert!(super::command_ids().contains(&"clip.export"));
        assert_eq!(
            super::exported_path(&directory.path().join("run.mp4")),
            Some(directory.path().join("run (exported).mp4"))
        );
    }

    #[test]
    fn trim_command_refuses_bad_input_without_writing_anything() {
        let directory = TestDirectory::new();
        let output = directory.path().join("out.mp4");
        let trim = |input: &str, start_ms: u64, end_ms: u64| {
            super::execute(Command::TrimClip {
                input: input.into(),
                output: output.clone(),
                start_ms,
                end_ms,
            })
        };
        assert_eq!(trim("", 0, 1_000), Err(EngineError::EmptyMediaPath));
        assert_eq!(
            trim("/definitely/not/here.mp4", 0, 1_000),
            Err(EngineError::Export(
                clipcraft_export::ExportError::Unreadable
            ))
        );
        assert_eq!(
            trim("/definitely/not/here.mp4", 2_000, 1_000),
            Err(EngineError::Export(clipcraft_export::ExportError::BadRange))
        );
        assert!(!output.exists());
        assert_eq!(
            Command::TrimClip {
                input: "a".into(),
                output: "b".into(),
                start_ms: 0,
                end_ms: 1
            }
            .id(),
            "clip.trim"
        );
    }

    #[test]
    fn trimmed_copies_get_free_names_beside_the_original() {
        let directory = TestDirectory::new();
        let input = directory.path().join("Run 1.mkv");
        assert_eq!(
            super::trimmed_path(&input),
            Some(directory.path().join("Run 1 (trimmed).mp4"))
        );
        fs::write(directory.path().join("Run 1 (trimmed).mp4"), b"x").expect("fixture writes");
        assert_eq!(
            super::trimmed_path(&input),
            Some(directory.path().join("Run 1 (trimmed 2).mp4"))
        );
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
