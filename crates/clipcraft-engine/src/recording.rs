//! Recording preferences: output container, what is captured, and how it is framed.
//!
//! These are pure data and geometry. Nothing here touches the screen or a codec, so every rule is
//! unit-tested without a display; the capture session consumes these settings through the engine.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use clipcraft_capture::reframe::{Framing, MIN_OUTPUT_EDGE, OutputShape, Reframe, reframe};

use crate::EngineError;
use crate::game_detection::GameMatchRule;
use crate::hotkeys::Hotkeys;

/// Largest settings file ClipCraft will read.
pub const MAX_SETTINGS_BYTES: u64 = 64 * 1024;
/// Version of the settings file format.
pub const SETTINGS_VERSION: u32 = 1;

/// Desktop replay behavior between recognized games. Games-only is the privacy-preserving default.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopCaptureMode {
    #[default]
    Never,
    AfterGame,
    Always,
}

/// The file format a clip is saved in.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    /// MP4 with H.264 video and AAC audio.
    #[default]
    Mp4,
    /// Matroska with H.264 video and AAC audio.
    Mkv,
    /// WebM with VP9 video and Opus audio.
    Webm,
}

impl Container {
    /// Every container, in menu order.
    pub const ALL: [Container; 3] = [Container::Mp4, Container::Mkv, Container::Webm];

    /// The file extension, without a dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
        }
    }

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::Mkv => "MKV",
            Self::Webm => "WebM",
        }
    }

    /// A short description of what is inside the file.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Mp4 => "H.264 + AAC. Plays almost everywhere.",
            Self::Mkv => "H.264 + AAC. Survives a crash mid-recording.",
            Self::Webm => "VP9 + Opus. For the web.",
        }
    }

    /// Why this container cannot be written yet, or `None` when it can.
    pub fn unavailable_reason(self) -> Option<&'static str> {
        match self {
            Self::Mp4 | Self::Mkv => None,
            Self::Webm => Some("No VP9 or Opus encoder is available yet."),
        }
    }

    /// True when ClipCraft can write this container today.
    pub fn is_available(self) -> bool {
        self.unavailable_reason().is_none()
    }
}

/// The video compression format inside the file.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    /// H.264 (AVC), written by FilmCraft's pure-Rust encoder or a graphics card.
    #[default]
    H264,
    /// H.265 (HEVC): smaller files at the same quality.
    H265,
}

impl VideoCodec {
    /// Every codec, in menu order.
    pub const ALL: [VideoCodec; 2] = [VideoCodec::H264, VideoCodec::H265];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::H265 => "H.265",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::H264 => "Works everywhere and encodes fast.",
            Self::H265 => {
                "About a third smaller at the same quality. Needs the graphics card to encode it; without one ClipCraft records H.264."
            }
        }
    }

    /// Why this codec cannot be written yet, or `None` when it can.
    pub fn unavailable_reason(self) -> Option<&'static str> {
        match self {
            Self::H264 => None,
            Self::H265 if cfg!(windows) => None,
            Self::H265 => Some(
                "H.265 is only available on Windows for now; it needs the graphics card's encoder and there is no software encoder.",
            ),
        }
    }

    /// True when ClipCraft can write this codec today.
    pub fn is_available(self) -> bool {
        self.unavailable_reason().is_none()
    }
}

/// What part of the machine is recorded.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    /// The window of the running game, including exclusive full-screen games.
    #[default]
    Game,
    /// A whole monitor, whatever is on it.
    Desktop,
}

impl CaptureSource {
    /// Every source, in menu order.
    pub const ALL: [CaptureSource; 2] = [CaptureSource::Game, CaptureSource::Desktop];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Game => "Game",
            Self::Desktop => "Desktop",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Game => "Records the game window, full screen or not.",
            Self::Desktop => "Records everything on one monitor.",
        }
    }
}

/// How the video encoder spends its bitrate.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RateControl {
    /// Spend more on busy scenes and less on calm ones, averaging the chosen bitrate.
    #[default]
    Variable,
    /// Hold the chosen bitrate steadily, which suits streaming and fixed-size limits.
    Constant,
}

impl RateControl {
    /// Every mode, in menu order.
    pub const ALL: [RateControl; 2] = [RateControl::Variable, RateControl::Constant];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Variable => "Variable",
            Self::Constant => "Constant",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Variable => "Best quality for the size. Bitrate follows the action.",
            Self::Constant => "Steady bitrate. Predictable file size.",
        }
    }
}

/// How hard the encoder works to shrink the video.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderPreset {
    /// Least CPU. Larger files at the same quality.
    Fast,
    /// A good balance of CPU and size.
    #[default]
    Balanced,
    /// Most CPU. Smallest files at the same quality.
    Quality,
}

impl EncoderPreset {
    /// Every preset, fastest first.
    pub const ALL: [EncoderPreset; 3] = [Self::Fast, Self::Balanced, Self::Quality];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Balanced => "Balanced",
            Self::Quality => "Quality",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Fast => "Lowest CPU use, so the game keeps its frame rate. Larger files.",
            Self::Balanced => "A good balance of CPU use and file size.",
            Self::Quality => "Smallest files for the quality. Uses the most CPU.",
        }
    }
}

/// A named recording quality, or the person's own choices.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityMode {
    /// Small files for slower computers and quick uploads.
    Low,
    /// Smooth and easy to share.
    Standard,
    /// Sharp, for powerful computers.
    #[default]
    High,
    /// Resolution, frame rate, and bitrate chosen by hand.
    Custom,
}

impl QualityMode {
    /// Every mode, in the order the tiles are shown.
    pub const ALL: [QualityMode; 4] = [Self::Low, Self::Standard, Self::High, Self::Custom];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low Quality",
            Self::Standard => "Standard",
            Self::High => "High Quality",
            Self::Custom => "Custom",
        }
    }

    /// Who it suits.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Low => "Lower end PCs & faster uploads",
            Self::Standard => "Performance & fast sharing",
            Self::High => "Higher quality & slower uploads",
            Self::Custom => "Customize your own settings",
        }
    }

    /// Height, frame rate, and video bitrate in kbps this mode sets, or `None` for Custom.
    pub fn values(self) -> Option<(u32, u32, u32)> {
        match self {
            Self::Low => Some((360, 24, 1_000)),
            Self::Standard => Some((720, 60, 5_000)),
            Self::High => Some((1080, 60, 10_000)),
            Self::Custom => None,
        }
    }
}

/// Which chip does the video encoding.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoEncoder {
    /// The processor. Works on every computer, but takes processor time from the game.
    Cpu,
    /// The graphics card's encoder, which leaves the processor free for the game. Whether this
    /// computer has one is found out when recording starts; without one the CPU is used. This is
    /// the default for that reason.
    #[default]
    Gpu,
}

impl VideoEncoder {
    /// Every encoder, in menu order.
    pub const ALL: [VideoEncoder; 2] = [Self::Cpu, Self::Gpu];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Gpu => "GPU",
        }
    }
}

/// Lowest accepted video bitrate.
pub const MIN_VIDEO_KBPS: u32 = 100;
/// Highest accepted video bitrate.
pub const MAX_VIDEO_KBPS: u32 = 200_000;

/// Everything the user chooses about recording.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct RecordingSettings {
    /// First-run choices have been saved. Older settings already count as configured.
    #[serde(default = "already_configured")]
    pub setup_completed: bool,
    /// Optional Discord disclosure; never part of capture configuration.
    pub discord_presence: crate::presence::PresenceSettings,
    /// Optional local match-data integrations. Separate from capture/game recognition.
    pub game_integrations: crate::game_state::GameIntegrationSettings,
    /// File format.
    pub container: Container,
    /// A named quality, or `Custom` to use the values below.
    pub quality: QualityMode,
    /// Which chip encodes the video.
    pub video_encoder: VideoEncoder,
    /// Video compression format.
    pub codec: VideoCodec,
    /// What is captured.
    pub source: CaptureSource,
    /// The shape of the clip's frame.
    pub shape: OutputShape,
    /// How the capture is fitted to that shape.
    pub framing: Framing,
    /// Clip height in pixels; the width follows from the shape.
    pub height: u32,
    /// Frames per second.
    pub frames_per_second: u32,
    /// Video bitrate in kilobits per second.
    pub video_kbps: u32,
    /// How the encoder spends that bitrate.
    pub rate_control: RateControl,
    /// Audio bitrate in kilobits per second.
    pub audio_kbps: u32,
    /// No longer used (custom games are always shown); kept so older settings files still load.
    pub show_advanced: bool,
    /// How hard the encoder works.
    pub encoder_preset: EncoderPreset,
    /// Seconds between keyframes. Shorter is easier to scrub; longer is smaller.
    pub keyframe_seconds: u32,
    /// Audio sample rate in hertz.
    pub audio_sample_rate: u32,
    /// Audio channels: 1 (mono) or 2 (stereo).
    pub audio_channels: u32,
    /// Draw the mouse cursor in the clip.
    pub capture_cursor: bool,
    /// Record the game's and system's sound.
    pub capture_game_audio: bool,
    /// Record the microphone.
    pub capture_microphone: bool,
    /// Keyboard shortcuts.
    pub hotkeys: Hotkeys,
    /// Microphone to record; empty means the system default.
    pub microphone_device: String,
    /// Output device whose sound is recorded; empty means the system default.
    pub output_device: String,
    /// Keep game sound and microphone on separate tracks in the file.
    pub separate_audio_tracks: bool,
    /// Record the microphone only while the push-to-talk key is held.
    pub push_to_talk: bool,
    /// The microphone is mono: copy it to both channels.
    pub microphone_mono: bool,
    /// Reduce microphone background noise locally with RNNoise, preserving stereo unless mono is selected.
    pub microphone_noise_suppression: bool,
    /// The screen or window to capture, as the capture provider names it; empty for the first
    /// screen.
    pub capture_target: String,
    /// The name of that target, shown when it is not currently available.
    pub capture_target_name: String,
    /// Start recording into the replay buffer when ClipCraft opens.
    pub record_on_launch: bool,
    /// Switch replay sources automatically using game recognition and the chosen desktop policy.
    pub game_only_capture: bool,
    /// Legacy WIP setting migrated to AfterGame; new callers use desktop_capture_mode.
    pub record_desktop_after_game: bool,
    /// Desktop recording policy for automatic source switching.
    pub desktop_capture_mode: DesktopCaptureMode,
    /// How long temporary desktop replay runs after a game, bounded to 15 seconds–1 hour.
    pub desktop_after_game_seconds: u32,
    /// Resolve running Steam IDs from local installation manifests as well as the bundled list.
    pub detect_installed_steam_games: bool,
    /// Exact Steam IDs excluded from automatic recognition. Custom overrides still win.
    pub disabled_steam_app_ids: Vec<String>,
    /// Recognize the bundled game catalog automatically; custom rules still have priority.
    pub detect_built_in_games: bool,
    /// Built-in game IDs explicitly excluded from automatic recognition.
    pub disabled_builtin_games: Vec<String>,
    /// Custom process and Steam-ID rules; these override built-in recognition.
    pub game_rules: Vec<GameMatchRule>,
    /// Capture the game's window even when the game is full screen.
    pub force_window_capture: bool,
    /// Convert HDR games to standard colours so clips look right everywhere. Off by default: the
    /// standard-colour capture is the fast path that stays on the graphics card (no copy to memory).
    pub hdr_compatibility: bool,
    /// Where clips are saved; empty means the default folder.
    pub capture_folder: String,
    /// Most space clips may take, in gigabytes; 0 means no limit.
    pub storage_limit_gb: u32,
    /// Delete the oldest clips to stay under the limit instead of stopping.
    pub delete_oldest_when_full: bool,
    /// Open ClipCraft in the background.
    pub start_minimized: bool,
    /// Closing the window keeps ClipCraft running in the system tray.
    pub close_to_tray: bool,
    /// A pop-up card in the corner of the screen when a clip has been saved.
    pub show_save_notification: bool,
    /// Ask GitHub once at start-up whether a newer ClipCraft has been published.
    pub check_for_updates: bool,
    /// Open ClipCraft when you sign in.
    pub launch_at_startup: bool,
    /// Game sound level, 0 to 100 percent.
    pub game_audio_percent: u32,
    /// Microphone level, 0 to 100 percent.
    pub microphone_percent: u32,
    /// Play a short sound when clipping starts or stops, a clip or screenshot is saved, the
    /// microphone is switched, or something goes wrong.
    pub sound_effects: bool,
    /// How loud those sounds are, 0 to 100 percent.
    pub sound_effects_percent: u32,
    /// How many seconds of play before the hotkey are kept.
    pub replay_seconds: u32,
    /// Seconds ClipCraft keeps recording after the save hotkey, so the clip includes what
    /// happens next. 0 saves immediately.
    pub post_roll_seconds: u32,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            setup_completed: false,
            game_integrations: crate::game_state::GameIntegrationSettings::default(),
            discord_presence: crate::presence::PresenceSettings::default(),
            container: Container::Mp4,
            quality: QualityMode::High,
            video_encoder: VideoEncoder::Gpu,
            codec: VideoCodec::H264,
            source: CaptureSource::Game,
            shape: OutputShape::Source,
            framing: Framing::Original,
            height: 1080,
            frames_per_second: 60,
            video_kbps: 10_000,
            rate_control: RateControl::Variable,
            audio_kbps: 160,
            show_advanced: false,
            encoder_preset: EncoderPreset::Balanced,
            keyframe_seconds: 2,
            audio_sample_rate: 48_000,
            audio_channels: 2,
            capture_cursor: true,
            capture_game_audio: true,
            capture_microphone: false,
            hotkeys: Hotkeys::default(),
            microphone_device: String::new(),
            output_device: String::new(),
            separate_audio_tracks: false,
            push_to_talk: false,
            microphone_mono: false,
            microphone_noise_suppression: false,
            capture_target: String::new(),
            capture_target_name: String::new(),
            record_on_launch: false,
            game_only_capture: true,
            record_desktop_after_game: false,
            desktop_capture_mode: DesktopCaptureMode::Never,
            desktop_after_game_seconds: 60,
            detect_installed_steam_games: true,
            disabled_steam_app_ids: Vec::new(),
            game_rules: Vec::new(),
            detect_built_in_games: true,
            disabled_builtin_games: Vec::new(),
            force_window_capture: false,
            hdr_compatibility: false,
            capture_folder: String::new(),
            storage_limit_gb: 0,
            delete_oldest_when_full: true,
            start_minimized: false,
            close_to_tray: true,
            check_for_updates: true,
            show_save_notification: true,
            launch_at_startup: false,
            game_audio_percent: 100,
            microphone_percent: 100,
            sound_effects: true,
            sound_effects_percent: 60,
            replay_seconds: 30,
            post_roll_seconds: 0,
        }
    }
}

fn already_configured() -> bool {
    true
}

/// Container overhead on top of the encoded streams (sample tables, headers).
const CONTAINER_OVERHEAD_PERCENT: u64 = 2;

/// About how many bytes `seconds` of video at `video_kbps` plus sound at `audio_kbps` take in a
/// file. The bitrate is a target: a still picture in variable-bitrate mode takes less.
pub fn estimated_bytes(video_kbps: u32, audio_kbps: u32, seconds: u32) -> u64 {
    let bits_per_second = (u64::from(video_kbps) + u64::from(audio_kbps)) * 1_000;
    (bits_per_second / 8)
        .saturating_mul(u64::from(seconds))
        .saturating_mul(100 + CONTAINER_OVERHEAD_PERCENT)
        / 100
}

/// A size for people: `850 KB`, `42 MB`, `1.3 GB`.
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1_000.0;
    let value = bytes as f64;
    if value < KB * KB {
        format!("{:.0} KB", (value / KB).max(1.0))
    } else if value < KB * KB * KB {
        let megabytes = value / (KB * KB);
        if megabytes < 10.0 {
            format!("{megabytes:.1} MB")
        } else {
            format!("{megabytes:.0} MB")
        }
    } else {
        format!("{:.1} GB", value / (KB * KB * KB))
    }
}

impl RecordingSettings {
    /// Seconds one saved clip lasts: the replay before the hotkey and the post-roll after it.
    pub fn clip_seconds(&self) -> u32 {
        self.replay_seconds + self.post_roll_seconds
    }

    /// Kilobits per second of sound in a clip: every track at the audio bitrate, or none when no
    /// sound is recorded.
    pub fn audio_total_kbps(&self) -> u32 {
        let sources = u32::from(self.capture_game_audio) + u32::from(self.capture_microphone);
        let tracks = if self.separate_audio_tracks {
            sources
        } else {
            sources.min(1)
        };
        self.audio_kbps * tracks
    }

    /// About how big one saved clip is at the given video bitrate.
    pub fn estimated_clip_bytes(&self, video_kbps: u32) -> u64 {
        estimated_bytes(video_kbps, self.audio_total_kbps(), self.clip_seconds())
    }
}

/// Heights offered in the UI.
pub const HEIGHTS: [u32; 6] = [360, 480, 720, 1080, 1440, 2160];
/// Frame rates offered in the UI.
pub const FRAME_RATES: [u32; 4] = [24, 30, 60, 120];
/// Replay lengths offered in the UI.
pub const REPLAY_SECONDS: [u32; 5] = [15, 30, 60, 120, 300];
/// Seconds recorded after the save hotkey that the UI offers.
pub const POST_ROLL_SECONDS: [u32; 5] = [0, 3, 5, 10, 15];
/// Video bitrates offered in the UI, in kilobits per second.
pub const VIDEO_KBPS: [u32; 9] = [
    1_000, 2_500, 5_000, 8_000, 10_000, 16_000, 25_000, 40_000, 60_000,
];
/// Audio bitrates offered in the UI, in kilobits per second.
/// Keyframe intervals offered in the UI, in seconds.
pub const KEYFRAME_SECONDS: [u32; 4] = [1, 2, 4, 10];
/// Audio sample rates offered in the UI, in hertz.
pub const SAMPLE_RATES: [u32; 2] = [44_100, 48_000];
/// Audio bitrates offered in the UI, in kilobits per second.
pub const AUDIO_KBPS: [u32; 8] = [96, 128, 160, 192, 256, 320, 384, 512];

impl RecordingSettings {
    /// Clamps hand-edited values into safe ranges and swaps an unwritable container for MP4.
    pub fn sanitized(mut self) -> Self {
        self.discord_presence = self.discord_presence.sanitized();
        if !self.container.is_available() {
            self.container = Container::Mp4;
        }
        if !self.codec.is_available() {
            self.codec = VideoCodec::H264;
        }
        self.desktop_after_game_seconds = self.desktop_after_game_seconds.clamp(15, 3600);
        // Compatibility with the earlier WIP boolean. UI choices clear this field.
        if self.record_desktop_after_game && self.desktop_capture_mode == DesktopCaptureMode::Never
        {
            self.desktop_capture_mode = DesktopCaptureMode::AfterGame;
        }
        self.hotkeys = self.hotkeys.sanitized();
        self.disabled_steam_app_ids = self
            .disabled_steam_app_ids
            .into_iter()
            .take(crate::MAX_STEAM_GAMES)
            .filter(|id| crate::steam_games::valid_app_id(id))
            .collect();
        self.disabled_steam_app_ids.sort();
        self.disabled_steam_app_ids.dedup();
        self.disabled_builtin_games = self
            .disabled_builtin_games
            .into_iter()
            .take(crate::MAX_BUILT_IN_GAMES)
            .filter(|id| crate::built_in_games().iter().any(|game| game.id == id))
            .collect();
        self.disabled_builtin_games.sort();
        self.disabled_builtin_games.dedup();
        self.game_rules = self
            .game_rules
            .into_iter()
            .take(64)
            .map(GameMatchRule::sanitized)
            .collect();
        for text in [
            &mut self.microphone_device,
            &mut self.output_device,
            &mut self.capture_target,
            &mut self.capture_target_name,
        ] {
            text.truncate(floor_boundary(text, 200));
        }
        self.capture_folder = self.capture_folder.trim().to_owned();
        if self.capture_folder.len() > 1_024 || self.capture_folder.contains('\0') {
            self.capture_folder.clear();
        }
        self.storage_limit_gb = self.storage_limit_gb.min(100_000);
        self.height = self.height.clamp(MIN_OUTPUT_EDGE, 4_320);
        self.frames_per_second = self.frames_per_second.clamp(1, 240);
        self.video_kbps = self.video_kbps.clamp(MIN_VIDEO_KBPS, MAX_VIDEO_KBPS);
        self.audio_kbps = self.audio_kbps.clamp(32, 512);
        self.keyframe_seconds = self.keyframe_seconds.clamp(1, 30);
        if !SAMPLE_RATES.contains(&self.audio_sample_rate) {
            self.audio_sample_rate = 48_000;
        }
        self.audio_channels = self.audio_channels.clamp(1, 2);
        self.game_audio_percent = self.game_audio_percent.min(100);
        self.microphone_percent = self.microphone_percent.min(100);
        self.sound_effects_percent = self.sound_effects_percent.min(100);
        self.replay_seconds = self.replay_seconds.clamp(5, 600);
        self.post_roll_seconds = self.post_roll_seconds.min(30);
        if let Some((height, frames_per_second, video_kbps)) = self.quality.values() {
            self.height = height;
            self.frames_per_second = frames_per_second;
            self.video_kbps = video_kbps;
        }
        // Distorting or trimming means nothing when the shape is the source's own.
        if self.shape == OutputShape::Source {
            self.framing = Framing::Original;
        } else if self.framing == Framing::Original {
            self.framing = Framing::Stretch;
        }
        self
    }

    /// Where a source of `width` x `height` lands in the output, or `None` for an empty source.
    pub fn reframe(&self, width: u32, height: u32) -> Option<Reframe> {
        reframe(
            width,
            height,
            self.shape,
            self.framing,
            self.height.clamp(MIN_OUTPUT_EDGE, 4_320),
        )
    }
}

/// The largest index at or below `limit` that is a character boundary of `text`.
fn floor_boundary(text: &str, limit: usize) -> usize {
    let mut index = limit.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[derive(Deserialize, Serialize)]
struct SettingsFile {
    version: u32,
    settings: RecordingSettings,
}

/// Default per-user settings file, or `None` when no home directory is known.
pub fn default_settings_path() -> Option<PathBuf> {
    Some(crate::index::default_index_path()?.with_file_name("recording-settings.json"))
}

/// Reads settings. A missing file yields the defaults; a damaged one is an error.
pub(crate) fn load_settings(path: &Path) -> Result<RecordingSettings, EngineError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecordingSettings::default());
        }
        Err(_) => return Err(EngineError::SettingsUnavailable),
    };
    if metadata.len() > MAX_SETTINGS_BYTES {
        set_aside(path);
        return Err(EngineError::SettingsInvalid);
    }
    let bytes = fs::read(path).map_err(|_| EngineError::SettingsUnavailable)?;
    let file: SettingsFile = match serde_json::from_slice(&bytes) {
        Ok(file) => file,
        Err(_) => {
            set_aside(path);
            return Err(EngineError::SettingsInvalid);
        }
    };
    if file.version != SETTINGS_VERSION {
        set_aside(path);
        return Err(EngineError::SettingsInvalid);
    }
    Ok(file.settings.sanitized())
}

/// Moves a settings file ClipCraft cannot read next to itself as `<name>.damaged`, so saving the
/// defaults afterwards does not destroy it (it may be from a newer version, or recoverable by
/// hand). Best effort.
pub(crate) fn set_aside(path: &Path) {
    // An earlier damaged file is kept too: the first free `.damaged`, `.damaged.1`, ...
    for number in 0..100_u32 {
        let mut aside = path.as_os_str().to_owned();
        aside.push(if number == 0 {
            ".damaged".to_owned()
        } else {
            format!(".damaged.{number}")
        });
        let aside = PathBuf::from(aside);
        if fs::symlink_metadata(&aside).is_err() {
            let _ = fs::rename(path, aside);
            return;
        }
    }
}

/// Saves settings with a temp file and rename; returns what was stored.
pub(crate) fn save_settings(
    path: &Path,
    settings: RecordingSettings,
) -> Result<RecordingSettings, EngineError> {
    let settings = settings.sanitized();
    let bytes = serde_json::to_vec_pretty(&SettingsFile {
        version: SETTINGS_VERSION,
        settings: settings.clone(),
    })
    .map_err(|_| EngineError::SettingsSaveFailed)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|_| EngineError::SettingsSaveFailed)?;
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
        return Err(EngineError::SettingsSaveFailed);
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_graphics_card_encoder_is_the_default_and_falls_back_to_the_processor() {
        // Software encoding takes processor time from the game, so it is a choice, not the
        // default; a computer without a hardware encoder uses the CPU anyway.
        assert_eq!(
            RecordingSettings::default().video_encoder,
            VideoEncoder::Gpu
        );
        assert_eq!(VideoEncoder::default(), VideoEncoder::Gpu);
    }

    use super::*;
    use clipcraft_capture::reframe::PixelRect;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> PixelRect {
        PixelRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn source_shape_keeps_a_4_3_game_4_3() {
        let result = reframe(1600, 1200, OutputShape::Source, Framing::Stretch, 1080).unwrap();
        assert_eq!((result.output_width, result.output_height), (1440, 1080));
        assert_eq!(result.source, rect(0, 0, 1600, 1200));
        assert_eq!(result.destination, rect(0, 0, 1440, 1080));
    }

    #[test]
    fn stretching_4_3_to_16_9_fills_the_frame() {
        let result = reframe(1024, 768, OutputShape::Wide16x9, Framing::Stretch, 1080).unwrap();
        assert_eq!((result.output_width, result.output_height), (1920, 1080));
        assert_eq!(result.source, rect(0, 0, 1024, 768));
        assert_eq!(result.destination, rect(0, 0, 1920, 1080));
    }

    #[test]
    fn black_bars_center_a_4_3_game_in_16_9() {
        let result = reframe(1024, 768, OutputShape::Wide16x9, Framing::Letterbox, 1080).unwrap();
        assert_eq!(result.destination, rect(240, 0, 1440, 1080));
    }

    #[test]
    fn black_bars_center_a_wide_game_in_4_3() {
        let result = reframe(
            1920,
            1080,
            OutputShape::Standard4x3,
            Framing::Letterbox,
            1080,
        )
        .unwrap();
        assert_eq!((result.output_width, result.output_height), (1440, 1080));
        assert_eq!(result.destination, rect(0, 134, 1440, 810));
    }

    #[test]
    fn cropping_a_4_3_game_to_16_9_trims_top_and_bottom() {
        let result = reframe(1024, 768, OutputShape::Wide16x9, Framing::Crop, 1080).unwrap();
        assert_eq!(result.source, rect(0, 96, 1024, 576));
        assert_eq!(result.destination, rect(0, 0, 1920, 1080));
    }

    #[test]
    fn cropping_a_wide_game_to_4_3_trims_the_sides() {
        let result = reframe(1920, 1080, OutputShape::Standard4x3, Framing::Crop, 1080).unwrap();
        assert_eq!(result.source, rect(240, 0, 1440, 1080));
    }

    #[test]
    fn every_output_dimension_is_even_and_inside_the_frame() {
        for (width, height) in [(1, 1), (3, 7), (641, 479), (65_535, 2), (2, 65_535)] {
            for shape in OutputShape::ALL {
                for framing in Framing::ALL {
                    for out in [2, 17, 480, 1081, 100_000] {
                        let r = reframe(width, height, shape, framing, out).unwrap();
                        assert_eq!(r.output_width % 2, 0);
                        assert_eq!(r.output_height % 2, 0);
                        assert!(r.destination.x + r.destination.width <= r.output_width);
                        assert!(r.destination.y + r.destination.height <= r.output_height);
                        assert!(r.source.x + r.source.width <= width);
                        assert!(r.source.y + r.source.height <= height);
                        assert!(r.source.width > 0 && r.source.height > 0);
                        assert!(r.destination.width > 0 && r.destination.height > 0);
                    }
                }
            }
        }
    }

    #[test]
    fn an_empty_or_absurd_source_has_no_reframe() {
        assert!(reframe(0, 10, OutputShape::Source, Framing::Original, 1080).is_none());
        assert!(reframe(10, 0, OutputShape::Source, Framing::Original, 1080).is_none());
        assert!(reframe(70_000, 10, OutputShape::Source, Framing::Original, 1080).is_none());
    }

    #[test]
    fn named_qualities_set_the_picture_values_and_custom_keeps_them() {
        let low = RecordingSettings {
            quality: QualityMode::Low,
            height: 2160,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(
            (low.height, low.frames_per_second, low.video_kbps),
            (360, 24, 1_000)
        );
        let custom = RecordingSettings {
            quality: QualityMode::Custom,
            height: 1440,
            frames_per_second: 30,
            video_kbps: 16_000,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(
            (custom.height, custom.frames_per_second, custom.video_kbps),
            (1440, 30, 16_000)
        );
        let default = RecordingSettings::default();
        assert_eq!(default.clone().sanitized(), default);
    }

    #[test]
    fn text_settings_are_bounded() {
        let settings = RecordingSettings {
            microphone_device: "é".repeat(500),
            capture_folder: "x".repeat(2_000),
            storage_limit_gb: u32::MAX,
            video_encoder: VideoEncoder::Gpu,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert!(settings.microphone_device.len() <= 200);
        assert!(settings.capture_folder.is_empty());
        assert_eq!(settings.storage_limit_gb, 100_000);
        // The GPU choice is kept; whether it works is decided when recording starts.
        assert_eq!(settings.video_encoder, VideoEncoder::Gpu);
    }

    #[test]
    fn every_audio_bitrate_fits_the_aac_stereo_limit() {
        // AAC allows 6144 bits per channel per 1024-sample frame: 576 kbps for stereo at 48 kHz,
        // 529 kbps at 44.1 kHz.
        for kbps in AUDIO_KBPS {
            assert!(u64::from(kbps) * 1000 * 1024 / 44_100 <= 6144 * 2);
            assert_eq!(
                RecordingSettings {
                    audio_kbps: kbps,
                    ..Default::default()
                }
                .sanitized()
                .audio_kbps,
                kbps
            );
        }
    }

    #[test]
    fn advanced_values_are_clamped_and_old_files_keep_working() {
        let settings = RecordingSettings {
            keyframe_seconds: 0,
            audio_sample_rate: 12_345,
            audio_channels: 9,
            game_audio_percent: 500,
            microphone_percent: 101,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(settings.keyframe_seconds, 1);
        assert_eq!(settings.audio_sample_rate, 48_000);
        assert_eq!(settings.audio_channels, 2);
        assert_eq!(settings.game_audio_percent, 100);
        assert_eq!(settings.microphone_percent, 100);
        // Standard-colour capture is the default: it keeps frames on the graphics card.
        assert!(!settings.hdr_compatibility);
        // A file written before these options existed still loads with their defaults.
        let old: SettingsFile =
            serde_json::from_str(r#"{"version":1,"settings":{"height":720}}"#).unwrap();
        assert!(old.settings.capture_cursor && !old.settings.show_advanced);
        assert!(!old.settings.microphone_noise_suppression);
        assert!(!RecordingSettings::default().microphone_noise_suppression);
    }

    #[test]
    fn h265_is_a_graphics_card_feature_and_falls_back_to_h264_off_windows() {
        assert!(VideoCodec::H264.is_available());
        assert_eq!(VideoCodec::H265.is_available(), cfg!(windows));
        let settings = RecordingSettings {
            codec: VideoCodec::H265,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(
            settings.codec,
            if cfg!(windows) {
                VideoCodec::H265
            } else {
                VideoCodec::H264
            }
        );
    }

    #[test]
    fn webm_is_unavailable_and_falls_back_to_mp4() {
        assert!(Container::Mp4.is_available() && Container::Mkv.is_available());
        assert!(Container::Webm.unavailable_reason().is_some());
        let settings = RecordingSettings {
            container: Container::Webm,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(settings.container, Container::Mp4);
    }

    #[test]
    fn sanitizing_clamps_values_and_reconciles_framing() {
        let settings = RecordingSettings {
            quality: QualityMode::Custom,
            height: 0,
            frames_per_second: 9_999,
            video_kbps: 0,
            replay_seconds: 0,
            shape: OutputShape::Source,
            framing: Framing::Crop,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(settings.height, MIN_OUTPUT_EDGE);
        assert_eq!(settings.frames_per_second, 240);
        assert_eq!(settings.video_kbps, 100);
        assert_eq!(settings.replay_seconds, 5);
        assert_eq!(settings.framing, Framing::Original);
        let wide = RecordingSettings {
            shape: OutputShape::Wide16x9,
            framing: Framing::Original,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(wide.framing, Framing::Stretch);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("clipcraft-settings-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn settings_round_trip_and_missing_file_gives_defaults() {
        let dir = scratch("round");
        let path = dir.join("nested/settings.json");
        assert_eq!(load_settings(&path).unwrap(), RecordingSettings::default());
        assert!(RecordingSettings::default().game_only_capture);
        let chosen = RecordingSettings {
            container: Container::Mkv,
            microphone_noise_suppression: true,
            source: CaptureSource::Desktop,
            shape: OutputShape::Wide16x9,
            framing: Framing::Letterbox,
            game_rules: vec![
                GameMatchRule::new("Apex Legends", ["r5apex.exe"]).with_steam_app_ids(["1172470"]),
            ],
            ..RecordingSettings::default()
        };
        let saved = save_settings(&path, chosen.clone()).unwrap();
        assert_eq!(saved, chosen);
        assert_eq!(load_settings(&path).unwrap(), chosen);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn damaged_oversized_and_future_files_are_rejected() {
        let dir = scratch("bad");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        fs::write(&path, b"{not json").unwrap();
        assert_eq!(load_settings(&path), Err(EngineError::SettingsInvalid));
        fs::write(&path, vec![b' '; MAX_SETTINGS_BYTES as usize + 1]).unwrap();
        assert_eq!(load_settings(&path), Err(EngineError::SettingsInvalid));
        fs::write(&path, br#"{"version":99,"settings":{}}"#).unwrap();
        assert_eq!(load_settings(&path), Err(EngineError::SettingsInvalid));
        // Unknown or missing fields fall back to defaults rather than losing the file.
        fs::write(
            &path,
            br#"{"version":1,"settings":{"quality":"custom","height":720}}"#,
        )
        .unwrap();
        assert_eq!(load_settings(&path).unwrap().height, 720);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sizes_follow_the_bitrates_and_the_clip_length() {
        // 10 Mbit/s video for 30 s is 37.5 MB, plus 2 % for the container.
        assert_eq!(estimated_bytes(10_000, 0, 30), 38_250_000);
        // Sound adds its own bitrate.
        assert_eq!(
            estimated_bytes(10_000, 160, 30),
            38_250_000 + 160 * 125 * 30 * 102 / 100
        );
        assert_eq!(estimated_bytes(0, 0, 30), 0);
        assert!(estimated_bytes(u32::MAX, u32::MAX, u32::MAX) > 0);
        for (bytes, text) in [
            (0, "1 KB"),
            (850_000, "850 KB"),
            (4_200_000, "4.2 MB"),
            (38_250_000, "38 MB"),
            (1_300_000_000, "1.3 GB"),
        ] {
            assert_eq!(format_bytes(bytes), text);
        }
    }

    #[test]
    fn a_clip_estimate_counts_the_post_roll_and_only_the_sound_that_is_recorded() {
        let mut settings = RecordingSettings {
            replay_seconds: 30,
            post_roll_seconds: 5,
            audio_kbps: 160,
            capture_game_audio: false,
            capture_microphone: false,
            ..RecordingSettings::default()
        };
        assert_eq!(settings.clip_seconds(), 35);
        assert_eq!(settings.audio_total_kbps(), 0);
        settings.capture_game_audio = true;
        assert_eq!(settings.audio_total_kbps(), 160);
        settings.capture_microphone = true;
        assert_eq!(settings.audio_total_kbps(), 160, "mixed into one track");
        settings.separate_audio_tracks = true;
        assert_eq!(settings.audio_total_kbps(), 320);
        assert_eq!(
            settings.estimated_clip_bytes(5_000),
            estimated_bytes(5_000, 320, 35)
        );
    }

    #[test]
    fn a_damaged_settings_file_is_set_aside_not_overwritten() {
        let dir =
            std::env::temp_dir().join(format!("clipcraft-settings-aside-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("recording-settings.json");
        fs::write(&path, b"{ half a file").expect("fixture");
        assert_eq!(load_settings(&path), Err(EngineError::SettingsInvalid));
        // The damaged bytes are kept, and the defaults can now be saved without losing them.
        assert_eq!(
            fs::read(dir.join("recording-settings.json.damaged")).expect("kept"),
            b"{ half a file"
        );
        assert!(!path.exists());
        assert_eq!(
            load_settings(&path).expect("defaults"),
            RecordingSettings::default()
        );
        save_settings(&path, RecordingSettings::default()).expect("saved");
        assert!(dir.join("recording-settings.json.damaged").exists());
        // A settings file from a newer version is kept too, without replacing the first.
        fs::write(&path, br#"{"version":999,"settings":{}}"#).expect("fixture");
        assert_eq!(load_settings(&path), Err(EngineError::SettingsInvalid));
        assert!(
            fs::read(dir.join("recording-settings.json.damaged.1"))
                .expect("kept")
                .starts_with(b"{\"version\":999")
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
