//! Shared command and local library behavior for the ClipCraft UI and CLI.

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::AtomicBool;

pub mod brand;
mod diagnose;
mod editor;
mod editor_draft;
pub use editor::{
    EditorAction, EditorClip, EditorSession, EditorText, MAX_EDITOR_DURATION, MAX_EDITOR_HISTORY,
    MIN_EDITOR_CLIP, TransitionPreset,
};
pub use editor_draft::{
    EDITOR_DRAFT_FILE_NAME, EditorDraft, EditorDraftError, MAX_EDITOR_DRAFT_BYTES,
    default_editor_draft_path, discard_editor_draft, load_editor_draft, save_editor_draft,
};
mod game_catalog;
mod game_detection;
pub mod game_state;
pub mod presence;
pub mod presence_art;
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

pub use clipcraft_export::compositor::{PreviewRecipe, PreviewRenderer};
pub use clipcraft_export::{
    Effect, FADE_LENGTH, Fade, MAX_TEXT_LAYERS, MAX_TITLE_CHARS, SegmentLook, SequenceQuality,
    SequenceRequest, SequenceResolution, SequenceSegment, SequenceSettings, SequenceText,
    TEXT_SIZE_RANGE, TextRaster, sequence_canvas, sequence_settings, text_position,
};
pub use clipcraft_export::{
    ExportPreset, GifOptions, LOUDNESS_TARGETS, MAX_GIF_SECONDS, TrimRequest as ExportRange,
};
pub use clipcraft_media::MediaProbe;
pub use clipcraft_player::timeline::{
    TimelineAnalysis, TimelineFrame, TimelineRequest, WaveformBucket,
};
use clipcraft_record::{HardwareEncoder, HardwareProvider, HardwareRequest};

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
    CLIP_PREFIX, CaptureSession, MicrophoneTestPlan, SaveJob, SavedClipReport, clip_format,
    clip_stem, record_config,
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
    PreviewDiscordCard {
        settings: presence::PresenceSettings,
        context: Box<presence::PresenceContext>,
    },
    BuildDiscordPresence {
        settings: presence::PresenceSettings,
        context: Box<presence::PresenceContext>,
    },
    ParseGameState {
        game: game_state::GameIntegration,
        bytes: Vec<u8>,
    },
    InstallGameIntegration {
        game: game_state::GameIntegration,
        directory: PathBuf,
        key_path: PathBuf,
    },
    AnalyzeEditorTimeline {
        request: Box<TimelineRequest>,
    },
    LoadEditorDraft {
        path: PathBuf,
    },
    SaveEditorDraft {
        path: PathBuf,
        draft: Box<EditorDraft>,
    },
    DiscardEditorDraft {
        path: PathBuf,
    },
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
    /// Validates the processing settings for a local, unsaved microphone listening test.
    PrepareMicrophoneTest {
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
    /// Saves a short range (up to 30 seconds) as a looping GIF.
    ExportGif {
        input: PathBuf,
        output: PathBuf,
        start_ms: u64,
        end_ms: u64,
        max_width: u32,
        fps: u32,
    },
    /// Evens out a range's loudness to `target_lufs` (-30 to -8) and saves it as a new MP4.
    NormalizeClip {
        input: PathBuf,
        output: PathBuf,
        start_ms: u64,
        end_ms: u64,
        target_lufs: i8,
    },
}

impl Command {
    /// Returns this operation's stable command ID.
    pub const fn id(&self) -> &'static str {
        match self {
            Self::PreviewDiscordCard { .. } => "presence.card",
            Self::BuildDiscordPresence { .. } => "presence.preview",
            Self::ParseGameState { .. } => "game_state.parse",
            Self::InstallGameIntegration { .. } => "game_state.install",
            Self::AnalyzeEditorTimeline { .. } => "editor.timeline",
            Self::LoadEditorDraft { .. } => "editor.draft.load",
            Self::SaveEditorDraft { .. } => "editor.draft.save",
            Self::DiscardEditorDraft { .. } => "editor.draft.discard",
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
            Self::PrepareMicrophoneTest { .. } => "recording.test_microphone",
            Self::TrimClip { .. } => "clip.trim",
            Self::ExportClip { .. } => "clip.export",
            Self::ExportGif { .. } => "clip.export_gif",
            Self::NormalizeClip { .. } => "clip.normalize",
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
    /// `Software`, or the hardware encoder that made the picture.
    pub encoder: String,
    /// Whether a hardware decoder read the source.
    pub hardware_decode: bool,
}

/// A rendered editor sequence, with the selected size target and actual output settings.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExportedSequence {
    #[serde(flatten)]
    pub clip: TrimmedClip,
    pub target_bytes: Option<u64>,
    pub within_target: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub video_kbps: u32,
    pub resolution_limited: bool,
    pub encoder: String,
    pub hardware_decode: bool,
}

/// A clip saved with its loudness evened out. Loudness values are in tenths, so they compare
/// exactly.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct NormalizedClip {
    pub clip: TrimmedClip,
    /// Integrated loudness before the change, in tenths of a LUFS.
    pub measured_lufs_tenths: i32,
    /// Gain applied, in tenths of a dB.
    pub gain_db_tenths: i32,
    /// The gain was held back to keep the peaks from clipping.
    pub peak_limited: bool,
}

/// Command `clip.normalize`: saves `start_ms..end_ms` with its sound at `target_lufs`.
pub fn normalize_clip(
    input: &std::path::Path,
    output: &std::path::Path,
    start_ms: u64,
    end_ms: u64,
    target_lufs: i8,
    progress: &mut dyn FnMut(f32),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<NormalizedClip, EngineError> {
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
        clipcraft_export::normalize_to_mp4(&request, f64::from(target_lufs), progress, cancel)
            .map_err(EngineError::Export)?;
    Ok(NormalizedClip {
        clip: trimmed_clip(report.clip),
        measured_lufs_tenths: (report.measured_lufs * 10.0).round() as i32,
        gain_db_tenths: (report.gain_db * 10.0).round() as i32,
        peak_limited: report.peak_limited,
    })
}

/// The hardware encoder exports may use, and whether they may. Exports try it first and fall back
/// to the software encoder on any trouble.
static EXPORT_HARDWARE: std::sync::Mutex<Option<(HardwareProvider, bool)>> =
    std::sync::Mutex::new(None);

/// Registers the hardware encoder provider exports can use, if the computer has one. Off until
/// [`enable_export_hardware`] turns it on.
pub fn set_export_hardware(provider: Option<HardwareProvider>) {
    if let Ok(mut slot) = EXPORT_HARDWARE.lock() {
        *slot = provider.map(|provider| (provider, false));
    }
}

/// Lets exports use the registered hardware encoder, or stops them. Follows the recording
/// setting that chooses between GPU and CPU encoding.
pub fn enable_export_hardware(enabled: bool) {
    if let Ok(mut slot) = EXPORT_HARDWARE.lock()
        && let Some((_, on)) = slot.as_mut()
    {
        *on = enabled;
    }
}

/// The registered hardware encoder as the export crate's opener, when it is enabled.
fn export_hardware() -> Option<clipcraft_export::FastOpener> {
    let provider = {
        let slot = EXPORT_HARDWARE.lock().ok()?;
        match slot.as_ref() {
            Some((provider, true)) => provider.clone(),
            _ => return None,
        }
    };
    Some(std::sync::Arc::new(
        move |request: &clipcraft_export::FastRequest| {
            let encoder = provider.open(&HardwareRequest {
                codec: clipcraft_record::StreamCodec::H264,
                width: request.width,
                height: request.height,
                fps: request.fps,
                video_kbps: request.video_kbps,
                constant_bitrate: false,
                keyframe_frames: request.keyframe_frames,
                effort: clipcraft_record::EncoderSpeed::Balanced,
            })?;
            Ok(Box::new(HardwareExport(encoder)) as Box<dyn clipcraft_export::FastEncoder>)
        },
    ))
}

/// A recorder hardware encoder seen as an export one. A picture that comes out of order is an
/// error, which makes the export fall back to the software encoder.
struct HardwareExport(Box<dyn HardwareEncoder>);

impl HardwareExport {
    fn packets(
        packets: Vec<clipcraft_record::HardwarePacket>,
    ) -> Result<Vec<clipcraft_export::FastPacket>, String> {
        packets
            .into_iter()
            .map(|packet| {
                if packet.reordered {
                    Err("the encoder reordered pictures".to_owned())
                } else {
                    Ok(clipcraft_export::FastPacket {
                        data: packet.data,
                        keyframe: packet.keyframe,
                    })
                }
            })
            .collect()
    }
}

impl clipcraft_export::FastEncoder for HardwareExport {
    fn encode(
        &mut self,
        rgba: &[u8],
        index: u64,
    ) -> Result<Vec<clipcraft_export::FastPacket>, String> {
        Self::packets(self.0.encode(rgba, index)?)
    }

    fn flush(&mut self) -> Result<Vec<clipcraft_export::FastPacket>, String> {
        Self::packets(self.0.flush()?)
    }

    fn avcc(&self) -> Vec<u8> {
        self.0.avcc()
    }

    fn name(&self) -> String {
        self.0.name()
    }
}

/// Shared `editor.export` implementation; edits and source files are never modified.
pub fn export_sequence(
    request: &SequenceRequest,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<ExportedSequence, EngineError> {
    let hardware = export_hardware();
    let report =
        clipcraft_export::sequence_to_mp4_with(request, hardware.as_ref(), progress, cancel)
            .map_err(EngineError::Export)?;
    Ok(ExportedSequence {
        clip: trimmed_clip(report.clip),
        target_bytes: report.target_bytes,
        within_target: report.within_target,
        width: report.width,
        height: report.height,
        fps: report.fps,
        video_kbps: report.video_kbps,
        resolution_limited: report.resolution_limited,
        encoder: report.encoder,
        hardware_decode: report.hardware_decode,
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
    let hardware = export_hardware();
    let report = clipcraft_export::compress_to_mp4_with(
        request,
        preset,
        hardware.as_ref(),
        progress,
        cancel,
    )
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
        encoder: report.encoder,
        hardware_decode: report.hardware_decode,
    })
}

/// Command `clip.export_gif`: saves `start_ms..end_ms` of `input` as a looping GIF.
pub fn export_gif_clip(
    input: &std::path::Path,
    output: &std::path::Path,
    start_ms: u64,
    end_ms: u64,
    options: clipcraft_export::GifOptions,
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
    clipcraft_export::export_gif(&request, options, progress, cancel)
        .map(trimmed_clip)
        .map_err(EngineError::Export)
}

fn trimmed_clip(report: clipcraft_export::ExportReport) -> TrimmedClip {
    TrimmedClip {
        path: report.path.to_string_lossy().into_owned(),
        duration_ms: u64::try_from(report.duration.to_units_floor(1_000)).unwrap_or(0),
        pictures: report.pictures,
        audio_tracks: report.audio_tracks,
        bytes: report.bytes,
    }
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
    copy_path(input, "trimmed", "mp4")
}

/// An unused name for an exported copy beside the source.
pub fn exported_path(input: &std::path::Path) -> Option<PathBuf> {
    copy_path(input, "exported", "mp4")
}

/// An unused name for a loudness-adjusted copy beside the source.
pub fn normalized_path(input: &std::path::Path) -> Option<PathBuf> {
    copy_path(input, "normalized", "mp4")
}

/// An unused name for a GIF made from a clip, beside the source.
pub fn gif_path(input: &std::path::Path) -> Option<PathBuf> {
    copy_path(input, "gif", "gif")
}

/// An unused MP4 name beside the first source in a sequence.
pub fn edited_path(input: &std::path::Path) -> Option<PathBuf> {
    copy_path(input, "edit", "mp4")
}

fn copy_path(input: &std::path::Path, suffix: &str, extension: &str) -> Option<PathBuf> {
    let folder = input.parent()?;
    let stem = input.file_stem()?.to_string_lossy().into_owned();
    (1..1_000).find_map(|n| {
        let tag = if n == 1 {
            suffix.to_owned()
        } else {
            format!("{suffix} {n}")
        };
        let candidate = folder.join(format!("{stem} ({tag}).{extension}"));
        // A crashed or concurrent writer may own its temporary even when final output is absent.
        // Broken links and unreadable paths also reserve a name; never erase them to make room.
        let available = ["", ".partial", ".recovery"].into_iter().all(|suffix| {
            let mut path = candidate.as_os_str().to_owned();
            path.push(suffix);
            std::fs::symlink_metadata(PathBuf::from(path))
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        });
        available.then_some(candidate)
    })
}

/// Output produced by a registered engine command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CommandOutput {
    DiscordCard(Option<presence::PresenceActivity>),
    DiscordPresence(Option<presence::PresenceTarget>),
    GameState(Box<game_state::GameSnapshot>),
    GameIntegrationInstalled(PathBuf),
    EditorTimeline(Box<TimelineAnalysis>),
    EditorDraft(Option<Box<EditorDraft>>),
    EditorDraftSaved,
    EditorDraftDiscarded,
    EditorMedia(Vec<MediaEntry>),
    EditorSession(Box<EditorSession>),
    SequenceExported(ExportedSequence),
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
    MicrophoneTest(MicrophoneTestPlan),
    /// Folder and disk usage.
    StorageUsage(StorageUsage),
    /// A clip that was cut and saved.
    Trimmed(TrimmedClip),
    /// A compressed clip and the actual/target size comparison.
    Exported(ExportedClip),
    /// A clip whose loudness was evened out.
    Normalized(NormalizedClip),
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
    GameState(game_state::GameStateError),
    EditorTimeline(clipcraft_player::timeline::TimelineError),
    EditorDraft(EditorDraftError),
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
            Self::GameState(error) => error.fmt(formatter),
            Self::EditorTimeline(error) => error.fmt(formatter),
            Self::EditorDraft(error) => error.fmt(formatter),
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
        Command::PrepareMicrophoneTest { settings } => Ok(CommandOutput::MicrophoneTest(
            MicrophoneTestPlan::from_settings(&settings),
        )),
        Command::PreviewDiscordCard { settings, context } => Ok(CommandOutput::DiscordCard(
            presence::preview_activity(&settings, &context),
        )),
        Command::BuildDiscordPresence { settings, context } => Ok(CommandOutput::DiscordPresence(
            presence::build_presence(&settings, &context),
        )),
        Command::ParseGameState { game, bytes } => game_state::parse_game_state(game, &bytes)
            .map(|s| CommandOutput::GameState(Box::new(s)))
            .map_err(EngineError::GameState),
        Command::InstallGameIntegration {
            game,
            directory,
            key_path,
        } => game_state::install_game_config(game, &directory, &key_path)
            .map(CommandOutput::GameIntegrationInstalled)
            .map_err(EngineError::GameState),
        Command::AnalyzeEditorTimeline { request } => {
            clipcraft_player::timeline::analyze_timeline(&request, cancel)
                .map(|analysis| CommandOutput::EditorTimeline(Box::new(analysis)))
                .map_err(EngineError::EditorTimeline)
        }
        Command::LoadEditorDraft { path } => load_editor_draft(&path)
            .map(|draft| CommandOutput::EditorDraft(draft.map(Box::new)))
            .map_err(EngineError::EditorDraft),
        Command::SaveEditorDraft { path, draft } => save_editor_draft(&path, &draft)
            .map(|_| CommandOutput::EditorDraftSaved)
            .map_err(EngineError::EditorDraft),
        Command::DiscardEditorDraft { path } => discard_editor_draft(&path)
            .map(|_| CommandOutput::EditorDraftDiscarded)
            .map_err(EngineError::EditorDraft),
        Command::ImportEditorMedia { paths } => {
            editor::import_media(paths).map(CommandOutput::EditorMedia)
        }
        Command::EditSequence { session, action } => session
            .apply(action)
            .map(|state| CommandOutput::EditorSession(Box::new(state))),
        Command::ExportSequence { request } => {
            export_sequence(&request, &mut |_| {}, cancel).map(CommandOutput::SequenceExported)
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
        Command::ExportGif {
            input,
            output,
            start_ms,
            end_ms,
            max_width,
            fps,
        } => {
            let never = std::sync::atomic::AtomicBool::new(false);
            export_gif_clip(
                &input,
                &output,
                start_ms,
                end_ms,
                clipcraft_export::GifOptions { max_width, fps },
                &mut |_| {},
                &never,
            )
            .map(CommandOutput::Trimmed)
        }
        Command::NormalizeClip {
            input,
            output,
            start_ms,
            end_ms,
            target_lufs,
        } => {
            let never = std::sync::atomic::AtomicBool::new(false);
            normalize_clip(
                &input,
                &output,
                start_ms,
                end_ms,
                target_lufs,
                &mut |_| {},
                &never,
            )
            .map(CommandOutput::Normalized)
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
    // A folder is opened, not selected inside its parent.
    if path.is_dir() {
        let program = match manager {
            FileManager::Explorer => "explorer",
            FileManager::Finder => "open",
            FileManager::XdgOpen => "xdg-open",
        };
        return (program, vec![path.as_os_str().to_owned()]);
    }
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
        "presence.card",
        "presence.preview",
        "game_state.parse",
        "game_state.install",
        "editor.timeline",
        "editor.draft.load",
        "editor.draft.save",
        "editor.draft.discard",
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
        "clip.export_gif",
        "clip.normalize",
        "library.find_file",
        "capture.toggle_microphone",
        "capture.push_to_talk",
        "recording.save_settings",
        "recording.test_microphone",
        "recording.set_builtin_games",
    ]
}

#[cfg(test)]
mod offline_workflow_tests;

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

    #[test]
    fn editor_export_json_keeps_existing_fields_and_accepts_optional_size_presets() {
        let normal: super::SequenceRequest =
            serde_json::from_str(r#"{"segments":[],"output":"generated.mp4"}"#).unwrap();
        assert_eq!(normal.preset, None);
        assert_eq!(normal.quality, super::SequenceQuality::default());
        let small: super::SequenceRequest = serde_json::from_str(
            r#"{"segments":[],"output":"generated.mp4","preset":{"preset":"small"}}"#,
        )
        .unwrap();
        assert_eq!(small.preset, Some(super::ExportPreset::Small));
        let report = super::ExportedSequence {
            clip: super::TrimmedClip {
                path: "generated.mp4".into(),
                duration_ms: 1000,
                pictures: 30,
                audio_tracks: 1,
                bytes: 1234,
            },
            target_bytes: Some(10_000_000),
            within_target: true,
            width: 640,
            height: 360,
            fps: 30,
            video_kbps: 4000,
            resolution_limited: false,
            encoder: "Software".into(),
            hardware_decode: false,
        };
        let json = serde_json::to_value(report).unwrap();
        assert_eq!(json["path"], "generated.mp4");
        assert_eq!(json["duration_ms"], 1000);
        assert_eq!(json["target_bytes"], 10_000_000);
        assert_eq!(json["fps"], 30);
        assert!(
            json.get("clip").is_none(),
            "existing CLI fields remain at the top level"
        );
    }

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
    fn editor_quality_json_uses_the_shared_validation_before_opening_or_writing_files() {
        let dir = TestDirectory::new();
        let output = dir.path().join("out.mp4");
        let json = serde_json::json!({
            "segments": [{"input":dir.path().join("missing.mp4"),"start":0,"end":254016000000_i64,"volume_percent":100}],
            "output":output,
            "quality":{"resolution":"source","fps":60}
        });
        let valid: super::SequenceRequest = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(
            valid.quality,
            super::SequenceQuality {
                resolution: super::SequenceResolution::Source,
                fps: 60
            }
        );
        let mut invalid = json.clone();
        invalid["quality"]["fps"] = serde_json::json!(120);
        let request = serde_json::from_value(invalid).unwrap();
        assert!(matches!(
            super::execute(Command::ExportSequence {
                request: Box::new(request)
            }),
            Err(EngineError::Export(
                clipcraft_export::ExportError::Unsupported(_)
            ))
        ));
        let mut invalid = json;
        invalid["quality"]["resolution"] = serde_json::json!("unlimited");
        assert!(serde_json::from_value::<super::SequenceRequest>(invalid).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn timeline_and_recovery_commands_share_validated_engine_dispatch() {
        let directory = TestDirectory::new();
        let path = directory
            .path()
            .canonicalize()
            .unwrap()
            .join("editor-draft.json");
        assert_eq!(
            super::execute(Command::LoadEditorDraft { path: path.clone() }),
            Ok(CommandOutput::EditorDraft(None))
        );
        let draft = Box::new(super::EditorDraft::default());
        assert_eq!(
            super::execute(Command::SaveEditorDraft {
                path: path.clone(),
                draft: draft.clone()
            }),
            Ok(CommandOutput::EditorDraftSaved)
        );
        assert_eq!(
            super::execute(Command::LoadEditorDraft { path: path.clone() }),
            Ok(CommandOutput::EditorDraft(Some(draft)))
        );
        assert_eq!(
            super::execute(Command::DiscardEditorDraft { path: path.clone() }),
            Ok(CommandOutput::EditorDraftDiscarded)
        );
        assert!(!path.exists());
        let request = super::TimelineRequest {
            path: PathBuf::from("missing.mp4"),
            start: clipcraft_time::Tick::ZERO,
            end: clipcraft_time::Tick::ZERO,
            frames: 0,
            buckets: 0,
        };
        assert!(matches!(
            super::execute(Command::AnalyzeEditorTimeline {
                request: Box::new(request)
            }),
            Err(EngineError::EditorTimeline(_))
        ));
        for id in [
            "editor.timeline",
            "editor.draft.load",
            "editor.draft.save",
            "editor.draft.discard",
        ] {
            assert!(super::command_ids().contains(&id));
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
    fn interrupted_exports_reserve_temporary_names_without_deleting_them() {
        let directory = TestDirectory::new();
        let input = directory.path().join("Run.mkv");
        let partial = directory.path().join("Run (edit).mp4.partial");
        let recovery = directory.path().join("Run (edit 2).mp4.recovery");
        fs::write(&partial, b"unfinished export").unwrap();
        fs::write(&recovery, b"recoverable recording").unwrap();
        fs::create_dir(directory.path().join("Run (edit 3).mp4")).unwrap();
        assert_eq!(
            super::edited_path(&input),
            Some(directory.path().join("Run (edit 4).mp4"))
        );
        assert_eq!(fs::read(&partial).unwrap(), b"unfinished export");
        assert_eq!(fs::read(&recovery).unwrap(), b"recoverable recording");
        #[cfg(unix)]
        {
            let dangling = directory.path().join("Run (edit 4).mp4");
            std::os::unix::fs::symlink(directory.path().join("absent"), &dangling).unwrap();
            assert_eq!(
                super::edited_path(&input),
                Some(directory.path().join("Run (edit 5).mp4"))
            );
            assert!(fs::symlink_metadata(dangling).is_ok());
        }
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

    #[test]
    fn gif_and_normalize_commands_validate_before_touching_files() {
        let directory = TestDirectory::new();
        let missing = directory.path().join("missing.mp4");
        let out = directory.path().join("out.mp4");
        for id in ["clip.export_gif", "clip.normalize"] {
            assert!(super::command_ids().contains(&id));
        }
        let gif = Command::ExportGif {
            input: PathBuf::new(),
            output: out.clone(),
            start_ms: 0,
            end_ms: 1_000,
            max_width: 480,
            fps: 12,
        };
        assert_eq!(gif.id(), "clip.export_gif");
        assert_eq!(super::execute(gif), Err(EngineError::EmptyMediaPath));
        let normalize = |target_lufs| Command::NormalizeClip {
            input: missing.clone(),
            output: out.clone(),
            start_ms: 0,
            end_ms: 1_000,
            target_lufs,
        };
        assert_eq!(normalize(-16).id(), "clip.normalize");
        assert_eq!(
            super::execute(normalize(0)),
            Err(EngineError::Export(
                clipcraft_export::ExportError::BadPreset
            ))
        );
        assert_eq!(
            super::execute(normalize(-16)),
            Err(EngineError::Export(
                clipcraft_export::ExportError::Unreadable
            ))
        );
        assert!(!out.exists());
        let named = super::gif_path(&directory.path().join("run.mp4")).expect("a free name");
        assert_eq!(
            named.file_name().and_then(|n| n.to_str()),
            Some("run (gif).gif")
        );
    }
}
